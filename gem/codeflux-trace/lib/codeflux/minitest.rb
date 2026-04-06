require "set"
require "fileutils"
require_relative "filter"
require_relative "writer"

module Codeflux
  # Ruby core/stdlib class names whose methods should never be indexed.
  # These get attributed to project call-site paths by TracePoint, causing
  # false positives in `affected` and unnecessary index bloat.
  # Use a Set for O(1) membership checks inside the hot TracePoint loop.
  STDLIB_CLASS_NAMES = Set.new(%w[
    BasicObject Object Class Module
    Kernel Comparable Enumerable
    Integer Float String Symbol
    Array Hash
    NilClass TrueClass FalseClass
    IO File Dir
    Proc Thread Mutex
    Enumerator Regexp Pathname
  ]).freeze

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

        # Resolve the real class name, not metaclass garbage.
        # tp.defined_class can return:
        #   - A normal class/module: User, ActiveRecord::Base
        #   - A singleton class: #<Class:User> (for class methods)
        #   - An anonymous class: #<Class:0x00007f...>
        #   - AR inspect dump: #<Class:User(id: integer, ...)>
        klass_name = resolve_class_name(klass, tp.self)
        next unless klass_name

        # Skip Ruby core/stdlib methods. TracePoint attributes them to the
        # project call-site path, not their definition, so they pollute the
        # index with entries like `Hash#[] => app/helpers/foo.rb:42` that
        # cover every test and cause false positives in `affected`.
        next if Codeflux::STDLIB_CLASS_NAMES.include?(klass_name)

        # Determine if this is a class/module method (.) or instance method (#).
        # A class method means the receiver itself is a Class or Module.
        is_class_method = tp.self.is_a?(Class) || tp.self.is_a?(Module)
        separator = is_class_method ? "." : "#"

        qualified = "#{klass_name}#{separator}#{tp.method_id}"

        # Relativize path against project root
        relative_path = path.start_with?(project_prefix) ? path[project_prefix.length..] : path

        @_codeflux_methods.add("#{qualified} #{relative_path}:#{tp.lineno}")
      end

      # TracePoint is created but NOT enabled here. Fixture loading happens
      # in before_setup (via super above) — skipping it avoids fixture-loading
      # noise in the index. Tracing starts in setup (after fixtures load).
    end

    def setup
      super  # fixtures are loaded here
      @_codeflux_trace.enable
    end

    def before_teardown
      @_codeflux_trace&.disable  # stop before teardown cleanup runs
      super
    end

    def after_teardown
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

    # Resolve a stable, readable class name from tp.defined_class.
    #
    # Handles:
    #   User                          → "User"
    #   #<Class:User>                 → "User" (singleton/class method)
    #   #<Class:User(id: integer...)> → "User" (AR metaclass with schema dump)
    #   #<Class:#<Object:0x...>>      → nil (anonymous, skip)
    #   #<Module:0x...>               → nil (anonymous, skip)
    def resolve_class_name(defined_class, receiver)
      # Try the simple case first: named class/module
      name = begin
        defined_class.name
      rescue StandardError
        nil
      end
      return name if name && !name.empty? && !name.start_with?("#<")

      # It's a singleton class or anonymous. Try to get the attached object.
      # Note: respond_to?(:attached_object) is true for all Class instances in Ruby 3.2+,
      # but attached_object raises TypeError unless it's actually a singleton class.
      is_singleton = begin
        defined_class.singleton_class?
      rescue StandardError
        false
      end
      if is_singleton
        # Ruby 3.2+: singleton_class.attached_object returns the original
        obj = defined_class.attached_object
        name = begin
          obj.is_a?(Module) ? obj.name : obj.class.name
        rescue StandardError
          nil
        end
        return name if name && !name.empty? && !name.start_with?("#<")
      end

      # Fallback: use the receiver's class
      name = begin
        if receiver.is_a?(Class) || receiver.is_a?(Module)
          receiver.name
        else
          receiver.class.name
        end
      rescue StandardError
        nil
      end
      return name if name && !name.empty? && !name.start_with?("#<")

      # Give up — anonymous class, not useful for indexing
      nil
    end

    def method_name_for_codeflux
      # e.g., "test/unit/models/user_test.rb::UserTest#test_deactivate"
      file = self.class.instance_method(name).source_location&.first || ""
      relative = file.sub(%r{^.*/test/}, "test/")
      "#{relative}::#{self.class.name}##{name}"
    end
  end
end
