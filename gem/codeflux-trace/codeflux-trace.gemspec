Gem::Specification.new do |s|
  s.name        = "codeflux-trace"
  s.version     = "0.1.0"
  s.summary     = "TracePoint-based method tracing for test impact analysis"
  s.description = "Hooks into Minitest to record which methods each test invokes. Writes .cft trace files for ingestion by the CodeFlux CLI."
  s.authors     = ["Sai Mageshvar"]
  s.files       = Dir["lib/**/*.rb"]
  s.require_paths = ["lib"]

  s.required_ruby_version = ">= 3.0"
end
