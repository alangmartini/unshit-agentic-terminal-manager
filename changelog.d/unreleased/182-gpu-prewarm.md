### Changed

- GPU bring-up now starts at process entry instead of after the window exists,
  on Windows. Adapter and device creation need no window -- verified by
  measurement, an adapter requested with no compatible surface costs the same
  as one requested with it -- but they are the single longest stretch of
  startup, and they run on the event-loop thread. Starting them first means
  config load, state seeding, the daemon handshake, the event loop and the
  window all happen alongside that wait rather than in front of it. Measured
  over five cold starts, 160-480ms of GPU bring-up now happens underneath other
  startup work, median about 300ms.

  That figure is read from a single run rather than by comparing two: adapter
  creation on this hardware varies by hundreds of milliseconds launch to
  launch, so a before/after difference would be mostly drift. The app reports
  how long bring-up took and how long the event-loop thread waited for it, and
  the gap between them is what the overlap saved.

  The prewarmed adapter is only used if it can actually present to the window
  that was ultimately created; on a machine where it cannot -- a second GPU
  driving the display the window landed on -- it is discarded and the original
  path runs unchanged. Backend selection goes through the same resolution the
  real request uses, environment overrides included, so `UNSHIT_RENDER_BACKEND`
  still decides and the compositor-clock D3D12/Mailbox pacing is unaffected.
