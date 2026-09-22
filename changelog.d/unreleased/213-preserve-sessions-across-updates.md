### Changed

- **Updates keep terminal sessions running.** Installing a new version no
  longer stops the session daemon or the commands running inside it. The
  Windows installers place each daemon release in its own
  `daemons\<version>` folder, check that the running daemon is compatible
  before replacing the UI, and relaunch the UI so it reattaches to its
  existing sessions. A newer bundled daemon takes over on a later launch
  once the running one has no children and no other clients; older daemons
  keep running until they are stopped normally. Upgrading from a release
  before this one still stops the daemon once, because the old UI's updater
  does; to keep sessions across that first upgrade, close only the old UI
  and run the new installer manually.
