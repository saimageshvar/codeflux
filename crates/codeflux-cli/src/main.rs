use std::path::{Path, PathBuf};
use clap::{Parser, Subcommand};
use anyhow::{Result, Context};

#[derive(Parser)]
#[command(name = "codeflux", version, about = "Runtime-traced test impact analysis")]
struct Cli {
    /// Path to the target project root
    #[arg(long, default_value = ".")]
    project: PathBuf,

    /// Path to the .cfx index file
    #[arg(long)]
    index: Option<PathBuf>,

    /// Suppress non-essential output
    #[arg(long)]
    quiet: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize .codeflux/ directory
    Init,

    /// Build .cfx index from .cft trace files
    Ingest {
        /// Keep trace files after ingestion
        #[arg(long)]
        keep_traces: bool,

        /// Output path for .cfx file
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Show tests affected by current changes
    Affected {
        /// Git ref to diff against (default: uncommitted changes)
        #[arg(long)]
        diff: Option<String>,

        /// Output format: text, json
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Show which tests cover a method
    Coverage {
        /// Method name, e.g., "User#deactivate!"
        method: String,
    },

    /// List methods with no test coverage
    Untested {
        /// Filter by file path prefix
        #[arg(long)]
        path: Option<String>,
    },

    /// Show index metadata and statistics
    Stats,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let project_root = cli.project.canonicalize()
        .context("could not resolve project path")?;
    let codeflux_dir = project_root.join(".codeflux");
    let index_path = cli.index.unwrap_or_else(|| codeflux_dir.join("index.cfx"));

    match cli.command {
        Commands::Init => cmd_init(&project_root, &codeflux_dir),
        Commands::Ingest { keep_traces, output } => {
            let traces_dir = codeflux_dir.join("traces");
            let output = output.unwrap_or(index_path);
            cmd_ingest(&traces_dir, &output, keep_traces, cli.quiet)
        }
        Commands::Affected { diff, format } => {
            cmd_affected(&project_root, &index_path, diff.as_deref(), &format, cli.quiet)
        }
        Commands::Coverage { method } => cmd_coverage(&index_path, &method),
        Commands::Untested { path } => cmd_untested(&index_path, path.as_deref()),
        Commands::Stats => cmd_stats(&index_path),
    }
}

fn cmd_init(project_root: &Path, codeflux_dir: &Path) -> Result<()> {
    // Create .codeflux/ and .codeflux/traces/
    std::fs::create_dir_all(codeflux_dir.join("traces"))?;

    // Create default config
    let config_path = codeflux_dir.join("config.toml");
    if !config_path.exists() {
        std::fs::write(&config_path, r#"# CodeFlux configuration
[trace]
include = ["app/", "lib/", "test/"]
exclude = []
"#)?;
    }

    // Append .codeflux/ to .gitignore if not present
    let gitignore_path = project_root.join(".gitignore");
    let needs_entry = if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path)?;
        !content.lines().any(|l| l.trim() == ".codeflux/" || l.trim() == ".codeflux")
    } else {
        true
    };

    if needs_entry {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&gitignore_path)?;
        writeln!(f, "\n.codeflux/")?;
    }

    println!("Initialized .codeflux/ in {}", project_root.display());
    Ok(())
}

fn cmd_ingest(traces_dir: &Path, output: &Path, keep_traces: bool, quiet: bool) -> Result<()> {
    let built = codeflux_ingest::builder::build_index(traces_dir)?;

    if !quiet {
        println!("Ingested {} trace files ({} empty, {} skipped)",
            built.stats.files_processed, built.stats.files_empty, built.stats.files_skipped);
        println!("  {} unique methods, {} tests",
            built.stats.total_methods, built.stats.total_tests);
    }

    // Ensure output directory exists
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    codeflux_core::index::write_cfx(
        output,
        &built.strings,
        &built.inverted,
        &built.forward,
        &built.file_methods,
        &built.commit_sha,
    )?;

    if !quiet {
        println!("Wrote index to {}", output.display());
    }

    // Delete traces unless keep_traces
    if !keep_traces {
        let files = codeflux_ingest::builder::discover_cft_files(traces_dir)?;
        for f in files {
            std::fs::remove_file(f)?;
        }
        if !quiet {
            println!("Cleaned up trace files");
        }
    }

    Ok(())
}

fn cmd_affected(
    project_root: &Path,
    index_path: &Path,
    diff_ref: Option<&str>,
    format: &str,
    quiet: bool,
) -> Result<()> {
    let index = codeflux_core::index::CfxReader::open(index_path)
        .context("could not open index file — run `codeflux ingest` first")?;

    let result = codeflux_query::affected::affected_tests(project_root, &index, diff_ref)?;

    // Print warnings
    if !quiet {
        for w in &result.warnings {
            eprintln!("warning: {}", w);
        }
    }

    match format {
        "json" => {
            // Simple JSON array of test IDs
            let test_ids: Vec<&str> = result.tests.iter().map(|t| t.test_id.as_str()).collect();
            println!("{}", serde_json::to_string_pretty(&test_ids)
                .unwrap_or_else(|_| "[]".to_string()));
        }
        _ => {
            // Text format
            if result.tests.is_empty() {
                println!("No affected tests found.");
            } else {
                if !quiet {
                    println!("{} affected test(s):", result.tests.len());
                    if !result.changed_methods.is_empty() {
                        println!("\nChanged methods:");
                        for m in &result.changed_methods {
                            println!("  {}", m);
                        }
                    }
                    if !result.fallback_files.is_empty() {
                        println!("\nFile-level fallback (method resolution failed):");
                        for f in &result.fallback_files {
                            println!("  {}", f);
                        }
                    }
                    println!("\nAffected tests:");
                }
                for t in &result.tests {
                    println!("{}", t.test_id);
                }
            }
        }
    }

    Ok(())
}

fn cmd_coverage(index_path: &Path, method: &str) -> Result<()> {
    let index = codeflux_core::index::CfxReader::open(index_path)
        .context("could not open index file — run `codeflux ingest` first")?;

    let result = codeflux_query::coverage::method_coverage(&index, method)?;

    if result.test_count == 0 {
        println!("No tests found covering '{}'", method);
    } else {
        println!("{} test(s) cover '{}':", result.test_count, method);
        for test in &result.tests {
            println!("  {}", test);
        }
    }

    Ok(())
}

fn cmd_untested(index_path: &Path, path_filter: Option<&str>) -> Result<()> {
    let index = codeflux_core::index::CfxReader::open(index_path)
        .context("could not open index file — run `codeflux ingest` first")?;

    let result = codeflux_query::untested::untested_methods(&index, path_filter)?;

    if result.untested_count == 0 {
        println!("All {} methods have test coverage!", result.total_methods);
    } else {
        println!("{}/{} methods have no test coverage:",
            result.untested_count, result.total_methods);
        for m in &result.methods {
            println!("  {} ({})", m.qualified_name, m.file_path);
        }
    }

    Ok(())
}

fn cmd_stats(index_path: &Path) -> Result<()> {
    let index = codeflux_core::index::CfxReader::open(index_path)
        .context("could not open index file — run `codeflux ingest` first")?;

    let metadata = std::fs::metadata(index_path)?;
    let size_kb = metadata.len() as f64 / 1024.0;

    println!("CodeFlux Index: {}", index_path.display());
    println!("  Commit:      {}", index.commit_sha());
    println!("  Methods:     {}", index.method_count());
    println!("  Tests:       {}", index.test_count());
    println!("  Index size:  {:.1} KB", size_kb);

    Ok(())
}
