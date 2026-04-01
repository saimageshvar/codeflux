require_relative "minitest"

module Codeflux
  class << self
    def project_root
      @project_root ||= Dir.pwd
    end

    def project_root=(path)
      @project_root = path
    end

    def commit_sha
      @commit_sha ||= `git rev-parse --short HEAD 2>/dev/null`.strip
    end
  end
end

# Auto-hook into Minitest when required.
# This file should only be required when CODEFLUX_TRACE is set:
#   require 'codeflux/trace' if ENV['CODEFLUX_TRACE']
#
# If Minitest::Test is already loaded, hook immediately.
# Otherwise, hook after Minitest is loaded via at_exit.
if defined?(Minitest::Test)
  Minitest::Test.prepend(Codeflux::MinitestHook)
else
  # Minitest hasn't been required yet. Register a hook that fires
  # after all requires are done but before tests run.
  at_exit do
    if defined?(Minitest::Test)
      Minitest::Test.prepend(Codeflux::MinitestHook)
    else
      warn "[codeflux-trace] Minitest::Test not found. Tracing disabled."
    end
  end
end
