### Changed

- The window now waits until it has a drawn frame behind it before appearing,
  instead of appearing empty and then freezing. GPU adapter and device creation
  run on the event-loop thread, so a window mapped at creation time cannot
  answer a paint or a click until they finish -- measured at about 1.2 seconds
  on a machine whose D3D12 adapter enumeration is slow. What that bought was
  not an early UI but a white, non-responding rectangle. The app now appears
  already drawn.

- Work that does not have to finish before the first frame no longer holds it
  up. Time to that first frame dropped by roughly half a second on a
  7-workspace, 10-pane profile:
  - Git branch names resolve on a background thread and appear a moment after
    the sidebar does. Workspaces sharing a repository are probed once, not once
    each. A branch that has not resolved yet renders muted rather than as the
    red "no git" error it used to flash on every launch.
  - Panes that are not visible on the first frame reattach to the daemon in the
    background, taking the state lock one pane at a time so a long restore
    cannot stall the UI. The active pane still comes up eagerly, exactly as
    before.
  - Terminal cell metrics are measured against the embedded JetBrains Mono face
    instead of building a font database from every font installed on the
    machine. This is also a correctness fix: the old measurement asked the OS
    for `monospace` and got Consolas, whose advance width the renderer never
    uses.

### Fixed

- A background pane whose reattach failed now still refreshes the UI, so the
  spawn failure it recorded becomes visible instead of sitting in state until
  something else happens to trigger a rebuild.
