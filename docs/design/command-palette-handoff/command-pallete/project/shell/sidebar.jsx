// Sidebar — workspaces tree. Numbered workspaces > terminals group > session leaves.
const Chev = ({ open }) => (
  <span className="chev" aria-hidden="true">{open ? "▾" : "▸"}</span>
);

const SBLeaf = ({ label, branch, isLast }) => (
  <div className={`sb-leaf ${isLast ? "last" : ""}`}>
    <span className="leaf-rail" aria-hidden="true">└</span>
    <svg width="11" height="11" className="sb-leaf-icon" aria-hidden="true"><use href="#terminal" /></svg>
    <span className="sb-leaf-label">{label}</span>
    {branch && <span className="sb-branch-pill">{branch}</span>}
  </div>
);

const SBTerminalsGroup = ({ open = true, count, children }) => (
  <div className={`sb-group ${open ? "open" : ""}`}>
    <div className="sb-group-head">
      <Chev open={open} />
      <svg width="11" height="11" className="sb-group-icon" aria-hidden="true"><use href="#terminal" /></svg>
      <span className="sb-group-label">terminals</span>
      <span className="sb-group-count tnum">{count}</span>
    </div>
    {open && <div className="sb-group-body">{children}</div>}
  </div>
);

const SBWorkspace = ({ index, name, open = true, active, children }) => (
  <div className={`sb-ws-block ${active ? "active" : ""} ${open ? "open" : ""}`}>
    <div className="sb-ws-head">
      <Chev open={open} />
      <span className="sb-ws-index tnum">{index}</span>
      <span className="sb-ws-name">{name}</span>
    </div>
    {open && <div className="sb-ws-body">{children}</div>}
  </div>
);

const Sidebar = () => {
  return (
    <aside className="tm-sidebar">
      <div className="sb-tabs">
        <button className="sb-tab active"><svg width="11" height="11"><use href="#terminal" /></svg>sessions</button>
        <button className="sb-tab"><svg width="11" height="11"><use href="#worktree" /></svg>worktrees</button>
        <button className="sb-tab"><svg width="11" height="11"><use href="#env" /></svg>env</button>
      </div>

      <div className="sb-scroll">
        <SBWorkspace index={1} name="terminal-manager" open={true}>
          <SBTerminalsGroup open={false} count={0} />
        </SBWorkspace>

        <SBWorkspace index={2} name="Fiscale_note" open={true} active>
          <SBTerminalsGroup open={true} count={1}>
            <SBLeaf label="shell" branch="feat/local-sync-runner" isLast />
          </SBTerminalsGroup>
        </SBWorkspace>
      </div>

      <div className="sb-foot">
        <span className="dot status-running" />
        <span>ptyd</span>
        <span className="dim">· 1 sess · 312 MB</span>
      </div>
    </aside>
  );
};

window.Sidebar = Sidebar;
