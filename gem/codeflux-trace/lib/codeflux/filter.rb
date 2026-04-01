module Codeflux
  class Filter
    DEFAULT_INCLUDE = %w[app/ lib/ vendor/engines/ test/].freeze
    DEFAULT_EXCLUDE = %w[
      kernel/ basicobject ruby/ rubygems/ bundler/
      monitor.rb delegate.rb set.rb
      codeflux-trace/
    ].freeze

    def initialize(include_paths: DEFAULT_INCLUDE, exclude_patterns: DEFAULT_EXCLUDE, gem_depth: 1)
      @include_paths = include_paths
      @exclude_patterns = exclude_patterns
      @gem_depth = gem_depth
    end

    # Returns true if this path should be traced.
    def accept?(path)
      return false if path.nil? || path.empty?

      # Fast reject: stdlib/internal
      path_lower = path.downcase
      return false if @exclude_patterns.any? { |pat| path_lower.include?(pat) }

      # Accept project paths
      return true if @include_paths.any? { |prefix| path.include?(prefix) }

      # Gem paths: accept only first-level entry points
      return @gem_depth > 0 if path.include?("gems/")

      false
    end
  end
end
