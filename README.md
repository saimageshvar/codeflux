# CodeFlux

Runtime-traced test impact analysis for Ruby projects. Answers **"which tests are affected by my code changes?"** in milliseconds.

## How It Works

1. **Trace** — Hook Ruby's `TracePoint` API during your test suite. Each test records the set of methods it invokes into a `.cft` trace file.
2. **Ingest** — A Rust CLI parses trace files in parallel and builds a compact binary index (`.cfx`) with inverted and forward method→test mappings.
3. **Query** — When you change code, the CLI uses Tree-sitter to resolve changed lines → method names, looks them up in the index, and returns the minimal set of affected tests.

## Repository Layout

```
crates/
  codeflux-core/      String interning, graph structures, binary index format, filters
  codeflux-ingest/    .cft file parser, parallel index builder (rayon)
  codeflux-query/     Tree-sitter Ruby mapper, affected/coverage/untested queries
  codeflux-cli/       Command-line interface (clap)
gem/
  codeflux-trace/     Ruby gem with Minitest TracePoint hooks
test-fixtures/        Sample .cft files for unit tests
integration-test/     Minimal Ruby project exercising the full pipeline
```

## Prerequisites

- Rust (stable, 2021 edition or later)
- Ruby 3.0+
- Git

## Build

```bash
cargo build --release
```

The binary is at `target/release/codeflux`.

## Quick Start

### 1. Initialize in your Ruby project

```bash
cd /path/to/your-project
codeflux init
```

Creates `.codeflux/` (with `config.toml` and `traces/`) and appends it to `.gitignore`.

### 2. Add the gem to your test group

```ruby
# Gemfile
group :test do
  gem 'codeflux-trace', path: '/path/to/codeflux/gem/codeflux-trace'
end
```

```bash
bundle install
```

### 3. Hook it into your test helper

```ruby
# test/test_helper.rb
require 'codeflux/trace' if ENV['CODEFLUX_TRACE']
```

### 4. Run tests with tracing enabled

```bash
CODEFLUX_TRACE=1 bundle exec rake test
```

Each test writes a `.cft` file to `.codeflux/traces/`. Expect ~2–3x slowdown — this is a one-time cost typically run in CI.

### 5. Ingest traces into the index

```bash
codeflux ingest
```

### 6. Query

```bash
# Tests affected by uncommitted changes
codeflux affected

# Tests affected by changes since a branch/ref
codeflux affected --diff main

# Which tests cover a method
codeflux coverage "User#deactivate!"

# Methods with no test coverage
codeflux untested
codeflux untested --path app/models/

# Index metadata
codeflux stats
```

## CLI Reference

```
codeflux <command> [options]

Commands:
  init        Initialize .codeflux/ directory and config.toml
  ingest      Build .cfx index from .cft trace files
  affected    Show tests affected by current changes
  coverage    Show which tests cover a method
  untested    List methods with no test coverage
  stats       Show index metadata and statistics

Global options:
  --project <path>  Target project root (default: current directory)
  --index <path>    Path to .cfx index file (default: .codeflux/index.cfx)
  --quiet           Suppress non-essential output
```

### `ingest`

| Flag | Description |
|------|-------------|
| `--keep-traces` | Do not delete `.cft` files after ingestion |
| `--output <path>` | Write index to a custom path |

### `affected`

| Flag | Description |
|------|-------------|
| `--diff <ref>` | Diff against a git ref (e.g. `main`, `HEAD~3`). Defaults to uncommitted changes. |
| `--format text\|json` | Output format (default: `text`) |

### `untested`

| Flag | Description |
|------|-------------|
| `--path <prefix>` | Restrict to files under a path prefix |

Without `--path`, only project-defined methods are considered — Ruby core/stdlib methods (`Integer#+`, `BasicObject#initialize`, etc.), gem paths, and test files are excluded. Providing `--path` bypasses this filtering.

## File Layout (in your project)

```
.codeflux/
  config.toml    Trace filter configuration
  traces/        .cft files (removed after ingest unless --keep-traces)
  index.cfx      Binary index
```

## CI Integration

Typical workflow: run tracing on a nightly/scheduled job, then use the cached index in PR builds to run only affected tests.

```bash
# Nightly job
CODEFLUX_TRACE=1 bundle exec rake test
codeflux ingest
# Archive .codeflux/index.cfx as a CI artifact

# PR build (after restoring the index artifact)
codeflux affected --format json > affected.json
# Feed affected.json to your test runner to execute only the relevant tests
```

## Development

Run the full test suite:

```bash
cargo test --workspace
```

Run tests for a single crate:

```bash
cargo test -p codeflux-core
cargo test -p codeflux-ingest
cargo test -p codeflux-query
```

End-to-end verification using the bundled `integration-test/` project:

```bash
cd integration-test
git init && git add -A && git commit -m "init"
CODEFLUX_TRACE=1 ruby -Itest test/models/calculator_test.rb
cd ..
cargo run -p codeflux-cli -- --project integration-test ingest --keep-traces
cargo run -p codeflux-cli -- --project integration-test stats
cargo run -p codeflux-cli -- --project integration-test coverage "Calculator#add"
```

## Troubleshooting

**`could not open index file`** — Run `codeflux ingest` first. The index must exist before querying.

**No trace files generated** — Confirm `CODEFLUX_TRACE=1` is set in the environment and that `require 'codeflux/trace'` runs in your `test_helper.rb`.

**Stale index** — The `stats` command shows the commit the index was built from. If it's far behind your working copy, re-run tracing and `ingest`. The `affected` command still works across commits via git diff.

**Method shows as untested but has tests** — The tracer records methods actually invoked at runtime. If a code path is skipped during tracing (e.g. guarded by a feature flag that's off), the method won't appear as covered. Enable the path during tracing or add a direct test.

**TracePoint overhead** — Expect 2–3x slowdown while tracing. Run tracing in CI and share the resulting index with developers rather than tracing on every local test run.

## Binary Index Format

The `.cfx` file is a little-endian binary format:

1. **Header** (32 bytes): magic `CFX\0`, version, min reader version, commit SHA, method count
2. **String table**: interned strings (method names, test IDs, file paths) as offset/length entries plus a contiguous UTF-8 blob
3. **Inverted index**: `method_id → sorted list of test_ids`
4. **Forward index**: `test_id → sorted list of method_ids`
5. **File→method map**: `file_id → list of method_ids defined in that file`

The format is designed for fast load via a single read; all lookups are hash-table operations after parsing.

## License

MIT
