### Fixed

- **macOS build and process-based agent detection.** The 0.6.0 revision did
  not compile on macOS: the resource monitor's new agent detection read
  process command lines through a Windows-only probe. macOS now reads them
  with `sysctl(KERN_PROCARGS2)`, so a `codex` or `claude` typed into a plain
  terminal is filed under `agents` there as well, with the same
  agent-events telemetry.
