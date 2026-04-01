require "fileutils"

module Codeflux
  class Writer
    def initialize(output_dir)
      @output_dir = output_dir
      FileUtils.mkdir_p(@output_dir)
    end

    # Write a .cft file for a single test.
    def write(test_id:, commit_sha:, methods:)
      filename = safe_filename(test_id)
      path = File.join(@output_dir, "#{filename}.cft")

      File.open(path, "w") do |f|
        f.puts "T #{test_id}"
        f.puts "C #{commit_sha}"
        methods.each do |m|
          f.puts "M #{m}"
        end
      end
    end

    private

    def safe_filename(test_id)
      # "test/unit/models/user_test.rb::UserTest#test_deactivate"
      # → "test-unit-models-user_test__UserTest-test_deactivate"
      file_part, method_part = test_id.split("::", 2)
      file_safe = file_part.tr("/", "-").sub(/\.rb$/, "")
      method_safe = (method_part || "unknown").tr("#", "-")
      "#{file_safe}__#{method_safe}"
    end
  end
end
