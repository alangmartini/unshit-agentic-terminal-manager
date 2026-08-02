### Added

- **Frame diagnostics now expose renderer work quantiles and the active display period.** Desktop performance tests can distinguish an overloaded renderer from a display refresh-rate limit while exercising real keyboard input.
- **A repeatable real-input typing performance gate is available through `cargo xtask typing-perf`.** It drives deterministic human-rate and stress-rate Win32 keyboard input against an isolated release build, captures input latency and frame cadence, and fails unless every run sustains the 120 Hz budget without dropped presentation slots.
- **Presentation telemetry now separates swapchain acquisition, intentional phase holding, and the platform present call.** Performance reports expose p50/p95/p99/max values for each wait source so driver pacing cannot masquerade as renderer work.

### Changed

- **Windows now prefers the native D3D12 renderer with tear-free FIFO presentation, 4x MSAA, and a bounded four-image surface queue.** The full quad shader packs constant decorative varyings into a 59-component layout, preserving gradients, masks, shadows, borders, and transforms within D3D12's 60-component limit; the spare surfaces absorb rare driver-release stalls without adding measured input latency.
- **The Windows UI/render thread registers with the Multimedia Class Scheduler `Games` task.** This reduces scheduler preemption during active 120 Hz rendering while retaining a safe fallback and structured lifecycle telemetry.

### Fixed

- **Timer-paced surfaces keep an absolute presentation phase instead of accumulating wake-up drift.** Optional render lead can absorb variable work while reporting its hold separately.
- **Input-latency capture now survives pacer rejections and empty animation frames.** Pending human input remains attached to the next frame that actually presents.
- **Desktop automation ignores winit's transparent thread-event helper window when locating the real app HWND.** Real-input runs persist bounded focus diagnostics and tolerate transient Win32 activation changes instead of misdirecting keystrokes.
