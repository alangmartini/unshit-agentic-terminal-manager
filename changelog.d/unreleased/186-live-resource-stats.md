### Fixed

- The status bar's `cpu` / `mem` / `↓ k/s` figures and the pane header's `pid · cpu` never showed what the terminals were actually costing: CPU and throughput were never sampled, memory only updated when the Sessions panel was opened (and then counted the shell alone), the clock stayed at `00:00` and every pane header read `pid 0 · 0.0%`. A background sampler now walks each session's whole process tree once a second — the shell, the agent it runs, that agent's node children and any `git` it spawns — and the status bar shows the machine-wide CPU share and working set of the UI, the daemon and every tree combined. Pane headers show the pane's own tree as `pid 1234 · 3.2% · 412 MiB`. Figures that are not known yet (first tick, daemon unreachable) read `--` instead of a misleading `0.0`. `↓ k/s` is PTY output received from the daemon across all sessions. The clock ticks.

### Added

- `resource-events.jsonl` in the profile config dir: `resource.monitor_started`, a one-per-minute `resource.summary` (tick cost, processes, unsampled processes, daemon list failures, current totals) and `resource.list_failed` / `resource.list_recovered` transitions, so "the numbers look wrong" can be diagnosed from telemetry alone.
- `scripts/resource-stats-shot.ps1`: puts a busy child under a pane's shell in an isolated profile and captures the live figures.
