$LOAD_PATH.unshift File.expand_path("../../gem/codeflux-trace/lib", __dir__)
$LOAD_PATH.unshift File.expand_path("../app/models", __dir__)

require "minitest/autorun"
require "codeflux/trace"

Codeflux.project_root = File.expand_path("..", __dir__)
