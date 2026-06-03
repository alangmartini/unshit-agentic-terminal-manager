# Restore Terminal Layout On Restart

## Fixed

- Persisted the full terminal layout (every workspace's tabs, pane splits,
  split ratios, and pane ids) so relaunching the app reattaches each
  surviving `unshit-ptyd` session to the pane it was in. Previously only
  workspace metadata was saved, so after "keep running" the relaunch
  rebuilt a single default terminal and every other session — including
  agent tabs like `dclaude` — was orphaned on the daemon and appeared lost.
- Made every close path (the "keep running" / "kill & quit" dialog buttons
  and the remembered silent-close preference) persist the layout, not just
  the cases where "remember my choice" was ticked.

## Notes

- On restart the active pane is brought up first (it must exist before the
  renderer can publish cell metrics); every other restored pane then
  reattaches via `attach_or_spawn`. A pane whose shell exited while the app
  was closed — or a config written before layout persistence — falls back
  to a fresh shell in that pane.
- `next_id` is restored above the largest persisted pane id so newly opened
  terminals never collide with a restored pane's daemon session key.
