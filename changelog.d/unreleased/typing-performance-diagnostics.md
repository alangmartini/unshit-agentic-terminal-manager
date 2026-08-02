### Added

- **Frame diagnostics now expose renderer work quantiles and the active display period.** Desktop performance tests can distinguish an overloaded renderer from a display refresh-rate limit while exercising real keyboard input.
- **A repeatable real-input typing performance gate is available through `cargo xtask typing-perf`.** It drives deterministic human-rate and stress-rate Win32 keyboard input against an isolated release build, captures input latency and frame cadence, and fails unless every run sustains the 120 Hz budget without dropped presentation slots.
- **Presentation telemetry now separates swapchain acquisition, intentional phase holding, and the platform present call.** Performance reports expose p50/p95/p99/max values for each wait source so driver pacing cannot masquerade as renderer work.
- **Cadence telemetry now separates compositor heartbeats from paint completion.** Native vblank-aligned intervals drive cadence acceptance on the Windows Mailbox path, while completion jitter stays independently queryable and every measured frame must return from its present call within one display period of the heartbeat.

### Changed

- **Windows now pairs D3D12's tear-free single-frame Mailbox queue with the native DirectComposition heartbeat, 4x MSAA, and a two-frame latency request.** The wgpu 30 renderer keeps the full decorative quad shader, DirectWrite text rasterization, gradients, masks, shadows, borders, and transforms while avoiding the rare double-refresh blocking acquire observed on FIFO drivers; unsupported systems retain the ordinary backend and tear-free FIFO fallback.
- **The Windows UI/render and compositor-clock threads register with the Multimedia Class Scheduler `Games` task.** This reduces scheduler preemption during active 120 Hz rendering while retaining safe fallbacks and structured lifecycle telemetry.

### Fixed

- **Timer-paced surfaces keep an absolute presentation phase instead of accumulating wake-up drift.** Optional render lead can absorb variable work while reporting its hold separately.
- **Input-latency capture now survives pacer rejections and empty animation frames.** Pending human input remains attached to the next frame that actually presents.
- **Desktop automation ignores winit's transparent thread-event helper window when locating the real app HWND.** Real-input runs persist bounded focus diagnostics and tolerate transient Win32 activation changes instead of misdirecting keystrokes.
- **Active rendering no longer performs synchronous monitor queries or native title rewrites once per second.** Refresh-rate changes are reconciled on startup, move, scale, and focus events, while live FPS remains available through frame metrics and the in-app overlay.
