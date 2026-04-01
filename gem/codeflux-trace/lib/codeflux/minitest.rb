require "set"
require "fileutils"
require_relative "filter"
require_relative "writer"

module Codeflux
  module MinitestHook
    def before_setup
      super
      @_codeflux_methods = Set.new
      @_codeflux_filter = Codeflux::Filter.new

      project_prefix = "#{Codeflux.project_root}/"

      @_codeflux_trace = TracePoint.new(:call, :c_call) do |tp|
        path = tp.path
        next unless @_codeflux_filter.accept?(path)

        klass = tp.defined_class
        next unless klass

        # Build qualified name: "ClassName#method" or "ClassName.method"
        separator = tp.self.is_a?(Class) || tp.self.is_a?(Module) ? "." : "#"
        klass_name = begin
          klass.name || klass.to_s
        rescue StandardError
          klass.to_s
        end
        qualified = "#{klass_name}#{separator}#{tp.method_id}"

        # Relativize path against project root
        relative_path = path.start_with?(project_prefix) ? path[project_prefix.length..] : path

        @_codeflux_methods.add("#{qualified} #{relative_path}:#{tp.lineno}")
      end

      @_codeflux_trace.enable
    end

    def after_teardown
      @_codeflux_trace&.disable

      if @_codeflux_methods && !@_codeflux_methods.empty?
        test_id = "#{method_name_for_codeflux}"
        output_dir = File.join(
          Codeflux.project_root,
          ".codeflux",
          "traces"
        )
        writer = Codeflux::Writer.new(output_dir)
        writer.write(
          test_id: test_id,
          commit_sha: Codeflux.commit_sha,
          methods: @_codeflux_methods.to_a.sort
        )
      end

      super
    end

    private

    def method_name_for_codeflux
      # e.g., "test/unit/models/user_test.rb::UserTest#test_deactivate"
      file = self.class.instance_method(name).source_location&.first || ""
      relative = file.sub(%r{^.*/test/}, "test/")
      "#{relative}::#{self.class.name}##{name}"
    end
  end
end
