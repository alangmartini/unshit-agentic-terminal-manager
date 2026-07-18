### Added

- Add a default-off **Start at Windows sign-in** control under Settings > Sessions. When combined with the separate Automatic agent resume opt-in, Terminal Manager can reopen saved Claude Code and Codex conversations after a PC restart without waiting for the app to be launched manually.

### Changed

- Register login startup directly in the current user's Windows `Run` key with profile-isolated values, bounded quoted executable paths, redacted durable telemetry, failure-safe UI state, and uninstall cleanup for the installed app value.
