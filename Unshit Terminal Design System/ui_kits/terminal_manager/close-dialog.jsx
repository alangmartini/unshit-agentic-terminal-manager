// Close confirmation prompt — shown when the user attempts to quit with running sessions.
const CloseDialog = ({ runningCount = 3, agentCount = 1, onClose }) => {
  const [remember, setRemember] = React.useState(false);
  const [busy, setBusy] = React.useState(null);
  const click = (action) => {
    setBusy(action);
    setTimeout(() => { setBusy(null); onClose?.(action); }, 600);
  };
  return (
    <div className="cd-scrim" onClick={() => onClose?.("cancel")}>
      <div className="cd-panel" role="alertdialog" aria-labelledby="cd-title" onClick={(e) => e.stopPropagation()}>
        <div className="cd-head">
          <span className="cd-mark">◆</span>
          <span className="cd-title" id="cd-title">close terminal.mgr?</span>
          <span className="cd-spacer" />
          <button className="icon-btn" onClick={() => onClose?.("cancel")} aria-label="cancel">
            <svg width="11" height="11"><use href="#close" /></svg>
          </button>
        </div>

        <div className="cd-body">
          <p className="cd-blurb">
            <span className="amber tnum">{runningCount}</span> running sessions
            {agentCount > 0 && <> · <span className="violet tnum">{agentCount}</span> attached agent</>}
            <span className="dim">. ptyd will keep them alive in the background unless you kill them.</span>
          </p>

          <ul className="cd-list">
            <li><span className="dot status-running" /><span className="cd-label">dashboard-dev</span><span className="cd-meta path">~/main/dashboard</span><span className="cd-meta dim tnum">2.4k lines</span></li>
            <li><span className="dot status-agent" /><span className="cd-label">refactor-userlist</span><span className="cd-meta"><span className="badge violet">CLAUDE</span></span><span className="cd-meta dim tnum">18 turns</span></li>
            <li><span className="dot status-running" /><span className="cd-label">api-server</span><span className="cd-meta path">:4040</span><span className="cd-meta dim tnum">1h 12m</span></li>
          </ul>

          <label className="cd-check">
            <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
            <span className="cd-box">{remember && "✓"}</span>
            <span>remember choice for this workspace</span>
          </label>
        </div>

        <div className="cd-foot">
          <button className="ghost" onClick={() => click("cancel")} disabled={busy !== null}>cancel</button>
          <span className="cd-spacer" />
          <button className="secondary" onClick={() => click("detach")} disabled={busy !== null}>
            {busy === "detach" ? "detaching…" : "keep running"}
            <span className="kbd">Enter</span>
          </button>
          <button className="danger" onClick={() => click("kill")} disabled={busy !== null}>
            {busy === "kill" ? "killing…" : `kill ${runningCount} & quit`}
          </button>
        </div>
      </div>
    </div>
  );
};
window.CloseDialog = CloseDialog;
