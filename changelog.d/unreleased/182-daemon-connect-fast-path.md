### Fixed

- Launching the app no longer waits a fixed 25ms before its first attempt to
  reach the `unshit-ptyd` daemon, and no longer retries at all when the daemon
  socket does not exist yet. The pause was paid on every single launch,
  including the common case where the daemon was already running and answered
  immediately. Connecting now starts at 0.5ms of backoff and only retries when
  the endpoint exists but is momentarily busy -- which is the one case a retry
  can actually help.
