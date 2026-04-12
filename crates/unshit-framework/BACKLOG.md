# unshit backlog

## Core improvements

- [ ] Reconciliation: diff new ElementTree against live Document instead of rebuilding every frame
- [ ] Scroll support: overflow:hidden/scroll with clip rects and scroll offset tracking
- [ ] Event handler dispatch: wire up onclick, onkeydown, onmouseenter/leave through the handler system
- [ ] Hot CSS reload: watch CSS files and re-parse on change without restarting
- [ ] view! macro string interpolation: support `"{state.count}"` syntax expanding to format!()
