### Fixed

- Starting the app no longer flashes a series of black console windows on
  screen before the UI appears. The release binary owns no console, so every
  `git` subprocess it spawned made Windows allocate a fresh console window for
  the life of the child -- once per restored workspace. All `git` invocations
  now run with `CREATE_NO_WINDOW`, and a test rejects any new call site that
  does not.
