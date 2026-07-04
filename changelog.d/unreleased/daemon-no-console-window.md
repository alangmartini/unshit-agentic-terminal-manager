### Fixed

- The `unshit-ptyd` PTY daemon is now built as a Windows GUI-subsystem binary in
  release, so launching the installed app no longer pops a stray console window
  alongside it. Previously the daemon was a console-subsystem executable and,
  depending on how Windows honored the `CREATE_NO_WINDOW | DETACHED_PROCESS`
  spawn flags, could surface its own terminal window next to the app. Debug
  builds keep their console so `cargo run -p unshit-ptyd` still shows logs, and
  the `--status` / `--version` / `--help` / `--shutdown` subcommands still print
  when run from a terminal (via `attach_parent_console`, mirroring the UI binary).
