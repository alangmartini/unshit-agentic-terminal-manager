### Changed

- The window now appears in about 90ms on a cold start instead of about 610ms
  (measured over 7 cold launches against a 7-workspace, 10-pane profile).
  Everything that does not have to finish before the window exists was moved
  off that path:
  - Git branch names resolve on a background thread and appear a moment after
    the sidebar does. Workspaces sharing a repository are probed once, not once
    each. A branch that has not resolved yet renders muted rather than as the
    red "no git" error it used to flash on every launch.
  - Panes that are not visible on the first frame reattach to the daemon in the
    background, taking the state lock one pane at a time so a long restore
    cannot stall the UI. The active pane still comes up eagerly, before the
    window, exactly as before.
  - Terminal cell metrics are measured against the embedded JetBrains Mono face
    instead of building a font database from every font installed on the
    machine. This is also a correctness fix: the old measurement asked the OS
    for `monospace` and got Consolas, whose advance width the renderer never
    uses.
