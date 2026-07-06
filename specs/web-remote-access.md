# Web Remote Access (terminal in the browser, phone over cloudflared)

Status: proposed architecture, not yet implemented.

## Goal

Use the terminal manager's sessions from a phone (or any browser): view live
output, type, resize, switch sessions — over a Cloudflare tunnel, without
weakening the local-only security posture of the daemon.

## High-level shape

```
 Phone browser (xterm.js SPA, PWA)
        │  HTTPS / WSS (TLS at Cloudflare edge)
        ▼
 cloudflared tunnel  ──►  http://127.0.0.1:<port>
        ▼
 unshit-webd  (NEW crate: axum HTTP + WebSocket bridge, loopback-only)
        │  existing IPC (named pipe / unix socket, unshit_ptyd::client::Client)
        ▼
 unshit-ptyd daemon  (existing; one change: fan-out session output)
        ▼
 PTYs (portable-pty)
```

Three principles:

1. **The daemon stays local-only.** No network code lands in `unshit-ptyd`
   beyond the fan-out fix. The new attack surface is entirely in a separate,
   opt-in `unshit-webd` process that binds `127.0.0.1` only.
2. **The bridge is just another IPC client.** It uses
   `unshit_ptyd::client::Client::connect_with_events`
   (`crates/unshit-ptyd/src/client/mod.rs:78`) exactly like the GUI's
   `DaemonPty` shim (`src/pty.rs`). Same daemon, same sessions as the desktop.
3. **The browser runs a real terminal emulator (xterm.js).** The daemon
   already emits raw VT bytes (`ServerEvent::Output`) plus a structured
   `Snapshot` on attach — we replay the snapshot as VT sequences and then pipe
   raw bytes straight through. No custom grid renderer in JS.

## Prerequisite daemon change: fan-out attach

`Session` currently holds a single swappable `output_tx`
(`crates/unshit-ptyd/src/session/mod.rs:61`); `Session::attach()` replaces the
sender, so a phone attaching steals the live stream from the desktop.

Change `Session` to a fan-out:

- Replace the single `mpsc::Sender` with `tokio::sync::broadcast::Sender<Bytes>`
  (or a `Vec<mpsc::Sender>` pruned on send failure). `attach()` returns a new
  subscriber instead of displacing the previous one; `detach` drops only that
  subscriber.
- **Lag/backpressure policy:** with `broadcast`, a slow consumer gets
  `RecvError::Lagged`. On lag, the per-connection forwarder in
  `daemon/handler.rs` must *resync*: re-snapshot the session's `Terminal` and
  send a fresh `SessionAttached`-style repaint rather than resuming a stream
  with a hole in it. (A phone on flaky cellular will hit this; silent gaps
  corrupt the emulator state.)
- **Resize policy v1: last-resize-wins.** Any attached client's `Resize`
  applies to the PTY and daemon `Terminal` (as today). Document that a small
  phone resizing will reflow the desktop view (tmux behaves the same without
  aggressive-resize). A per-attachment "observe-only" flag (no resize, no
  write) is a cheap follow-up and doubles as a read-only share mode.

This change benefits the desktop app too (e.g. two windows on one session)
and is independently testable: attach two clients, assert both receive output,
kill one, assert the other still receives.

## New crate: `crates/unshit-webd`

A small axum-based binary (`tokio` + `axum` with its `ws` feature; both are
tokio-native and fit the existing async stack). Ships as lib + bin like
`unshit-ptyd`.

### Responsibilities

| Concern | Approach |
|---|---|
| Static UI | xterm.js SPA embedded in the binary (`rust-embed`/`include_dir`) — single file to deploy, works offline behind the tunnel. |
| Daemon connection | One `Client` per WebSocket connection (mirrors how the GUI holds one `Client`); socket path resolved exactly like the app: `TM_PTYD_SOCKET` env else `default_socket_path_for_instance(active_profile())`. |
| Sessions API | `GET /api/sessions` (list), `POST /api/sessions` (spawn), `DELETE /api/sessions/:id` (kill), `PATCH /api/sessions/:id` (rename) — thin JSON wrappers over the existing `Request` variants. |
| Terminal I/O | `GET /ws/session/:id` upgrades to WebSocket (protocol below). |
| Binding | `127.0.0.1` only, port from config/flag. Refuse to start with a non-loopback bind unless `--dangerously-bind` is passed. |

### WebSocket protocol (browser ⇄ webd)

Mirrors the daemon's own framing philosophy — JSON for control, binary for
bulk bytes:

- **Binary frames, server→client:** raw PTY output → `term.write(bytes)`.
- **Binary frames, client→server:** raw input bytes → `Request::Write`.
- **Text frames (JSON), both directions:** `{"kind": ...}` control messages:
  `attach`, `resize {cols, rows}`, `resync`, `ping`/`pong`, and server-side
  `error`, `session_exited`.
- **On attach:** webd calls `AttachSession`, receives
  `Response::SessionAttached { snapshot }`, converts the `Snapshot` to a VT
  repaint (clear screen, replay scrollback tail + grid with SGR attributes,
  position cursor) and sends it as the first binary frame. New module
  `snapshot_to_vt` — best placed in `unshit-terminal-core` next to
  `Snapshot`, since that crate owns the grid model and both sides already
  depend on it.
- **On broadcast lag or WS reconnect:** client sends `resync`, server
  re-attaches and replays a fresh snapshot. This makes flaky mobile
  connections self-healing.

### Auth

The tunnel makes this reachable from the public internet, so two layers:

1. **App-level token (mandatory, defense in depth).** Modeled on the existing
   diagnostics-pipe pattern (`src/diagnostics/config.rs`, token check in
   `server.rs`): webd generates a random 256-bit token on first run, stores it
   in the profile's config dir. Browser presents it once at `/login`
   (constant-time compare, rate-limited), gets an HttpOnly `Secure` session
   cookie; the WS upgrade and all `/api` routes require the cookie. Origin
   header checked on upgrade (must match the tunnel hostname) to block
   cross-site WebSocket hijacking.
2. **Cloudflare Access (recommended for a persistent tunnel).** A named
   tunnel + Access policy (email/device based) puts Cloudflare's auth in
   front before a single byte reaches webd. For ad-hoc use,
   `cloudflared tunnel --url http://127.0.0.1:<port>` quick tunnels work with
   the app token alone.

Pairing UX: `unshit-webd --print-login` (and later the GUI settings page)
prints the login URL with the token embedded as a one-time query param and
renders it as a QR code in the terminal — scan with the phone, cookie is set,
token param is consumed.

### Observability

Structured JSON logs via `tracing` + `tracing-subscriber` (json), written to
the profile's data dir (reachable later, not just stdout): stable event names
(`webd.ws_connect`, `webd.auth_fail`, `webd.session_attach`,
`webd.broadcast_lag_resync`, `webd.ws_close`) with a per-connection
`conn_id` on every event. Never log the token or session output bytes; counts
and lengths only.

## Browser client

- **xterm.js + fit addon**, dark theme matching the app. No framework needed;
  one page, vanilla TS compiled to a single JS bundle checked in or built by
  `xtask`.
- **Mobile-specific chrome:** session list drawer, and an on-screen key bar
  (Esc, Tab, Ctrl, arrows, `/`, `-`) — soft keyboards can't send these, and
  it's the difference between "demo" and "usable for vim/Claude Code on a
  phone".
- **PWA manifest** so it installs to the home screen and runs fullscreen.
- Reconnect loop with exponential backoff sending `resync` on reopen.

## Instance isolation

webd resolves the profile exactly like the app (`src/profile.rs::active_profile`)
so it attaches to the *same* daemon and session set as the running GUI —
never spawns its own daemon under a different profile. Dev testing follows
the existing `TM_PROFILE`/`tm-isolation.ps1` rules; webd under `TM_PROFILE=dev`
talks to the dev daemon only.

## Cloudflared integration

cloudflared stays external to the codebase (no vendoring). Convenience only:

- `unshit-webd --tunnel quick` shells out to `cloudflared tunnel --url ...`,
  parses the assigned `*.trycloudflare.com` URL, and prints it + the QR login
  link. Warns that quick tunnels have no Access policy.
- Docs cover the persistent path: named tunnel + `config.yml` ingress to
  `http://127.0.0.1:<port>` + Cloudflare Access policy.

## Delivery plan

1. **Daemon fan-out** — broadcast-based multi-attach in `Session` +
   handler resync path + registry tests. Ships alone; desktop unaffected.
2. **webd MVP** — axum server, token/cookie auth, WS bridge,
   `snapshot_to_vt`, minimal xterm.js page. Verified end-to-end on LAN
   (phone on same Wi-Fi hitting the QR login) before any tunnel.
3. **Mobile UX** — key bar, session drawer, PWA manifest, reconnect/resync.
4. **Tunnel convenience + hardening** — `--tunnel quick`, docs for named
   tunnel + Access, rate limiting, observe-only attachments, GUI settings
   toggle ("Enable web access") that starts/stops webd and shows the QR.

## Decisions & tradeoffs (summary)

- **Separate `unshit-webd` process, not in the daemon:** crash isolation,
  opt-in attack surface, daemon stays pure-local. Cost: one more binary to
  locate/spawn (same pattern as `unshit-ptyd` already solves in
  `src/daemon.rs::locate_daemon_binary`).
- **xterm.js over custom grid rendering:** the raw-byte stream + snapshot
  replay is already what the daemon produces; a JSON-grid protocol to a
  custom canvas renderer would duplicate `unshit-terminal-core` in JS for no
  gain. Cost: xterm.js's emulation may differ in corners from the app's
  (acceptable — the daemon's `Terminal` remains the source of truth for
  snapshots).
- **Last-resize-wins:** simplest correct v1; observe-only flag later if the
  phone reflowing the desktop annoys.
- **TLS at the Cloudflare edge only:** the localhost hop is plaintext but
  loopback-bound; anyone who can read loopback traffic already owns the
  machine.
