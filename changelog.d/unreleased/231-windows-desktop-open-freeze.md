### Fixed

- **Opening a file from Explorer no longer freezes a running window.** On
  Windows, "Open with Terminal Manager" on a document while the app was
  already open could hang the UI ("Not Responding") before the file
  appeared, because matching the file's folder to a workspace touched every
  saved workspace path on disk and stalled on unreachable network or WSL
  paths. Workspace and editor matching now compares paths without filesystem
  access, and opened paths no longer carry Windows' `\\?\` prefix.
