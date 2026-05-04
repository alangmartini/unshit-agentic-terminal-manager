// Sidebar — workspaces, sessions, and agent activity feed.
const SectionHead = ({ children, count }) => (
  <div className="sb-section-head">
    <span>{children}</span>
    {count != null && <span className="count">{count}</span>}
  </div>
);

const SBRow = ({ icon, label, meta, status, active, agent, branch, indent = 0 }) => (
  <div className={`sb-row ${active ? "active" : ""}`} style={{ paddingLeft: 8 + indent * 12 }}>
    {status && <span className={`sb-dot status-${status}`} />}
    {icon && !status && <svg width="11" height="11" className="sb-icon"><use href={`#${icon}`} /></svg>}
    <span className="sb-label">{label}</span>
    {agent && <span className="sb-agent">{agent}</span>}
    {branch && <span className="sb-branch">({branch})</span>}
    {meta && <span className="sb-meta">{meta}</span>}
  </div>
);

const Sidebar = () => {
  return (
    <aside className="tm-sidebar">
      <div className="sb-tabs">
        <button className="sb-tab active"><svg width="11" height="11"><use href="#terminal" /></svg>sessions</button>
        <button className="sb-tab"><svg width="11" height="11"><use href="#agent" /></svg>agents</button>
        <button className="sb-tab"><svg width="11" height="11"><use href="#worktree" /></svg>worktrees</button>
        <button className="sb-tab"><svg width="11" height="11"><use href="#env" /></svg>env</button>
      </div>

      <div className="sb-scroll">
        <SectionHead count={3}>workspaces</SectionHead>
        <div className="sb-ws active">
          <span className="chev">▾</span><span>main</span><span className="ws-meta">4</span>
        </div>
        <div className="sb-tree">
          <SBRow status="running" label="dashboard-dev" branch="main" meta="Ctrl 1" indent={0} active />
          <SBRow status="running" label="api-server" branch="feat/auth" meta="Ctrl 2" indent={0} />
          <SBRow status="agent" label="refactor-userlist" agent="claude" meta="Ctrl 3" indent={0} />
          <SBRow status="error" label="test-watcher" meta="exit 1" indent={0} />
        </div>
        <div className="sb-ws">
          <span className="chev">▸</span><span>infra</span><span className="ws-meta">2</span>
        </div>
        <div className="sb-ws">
          <span className="chev">▸</span><span>scratch</span><span className="ws-meta">1</span>
        </div>

        <SectionHead count={2}>agent activity</SectionHead>
        <div className="agent-card">
          <div className="agent-head">
            <span className="badge violet">CLAUDE</span>
            <span className="agent-name">refactor-userlist</span>
            <span className="agent-time tnum">2m</span>
          </div>
          <div className="agent-line dim">  18 turns · 4.2k tok</div>
        </div>
        <div className="agent-card dim">
          <div className="agent-head">
            <span className="badge muted">CODEX</span>
            <span className="agent-name">spec-writer</span>
            <span className="agent-time dim tnum">stopped</span>
          </div>
        </div>
      </div>

      <div className="sb-foot">
        <span className="dot status-running" />
        <span>ptyd</span>
        <span className="dim">· 4 sess · 1.42 GB</span>
      </div>
    </aside>
  );
};

window.Sidebar = Sidebar;
