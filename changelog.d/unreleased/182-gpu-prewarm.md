### Changed

- GPU bring-up now starts at process entry instead of after the window exists,
  on Windows. Adapter and device creation need no window -- verified by
  measurement, an adapter requested with no compatible surface costs the same
  as one requested with it -- but they are the single longest stretch of
  startup, and they run on the event-loop thread. Starting them first means
  config load, state seeding, the daemon handshake, the event loop and the
  window all happen alongside that wait rather than in front of it. Measured
  200-320ms off the time to first frame, and the window is idle for that much
  less of it.

  The prewarmed adapter is only used if it can actually present to the window
  that was ultimately created; on a machine where it cannot -- a second GPU
  driving the display the window landed on -- it is discarded and the original
  path runs unchanged. Backend selection goes through the same resolution the
  real request uses, environment overrides included, so `UNSHIT_RENDER_BACKEND`
  still decides and the compositor-clock D3D12/Mailbox pacing is unaffected.
