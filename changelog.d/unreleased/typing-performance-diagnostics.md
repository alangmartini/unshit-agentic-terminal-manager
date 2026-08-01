### Added

- **Frame diagnostics now expose renderer work quantiles and the active display period.** Desktop performance tests can distinguish an overloaded renderer from a display refresh-rate limit while exercising real keyboard input.
- **A repeatable real-input typing performance gate is available through `cargo xtask typing-perf`.** It drives deterministic human-rate and stress-rate Win32 keyboard input against an isolated release build, captures input latency and frame cadence, and fails unless every run sustains the 120 Hz budget without dropped presentation slots.
