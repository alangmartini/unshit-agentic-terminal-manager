// Tabs strip + pane grid (split layout) with terminal output.
const TabStrip = () => (
  <div className="tm-tabs">
    <button className="tm-tab active">
      <span className="dot status-running" /> dashboard-dev <span className="x">×</span>
    </button>
    <button className="tm-tab">
      <span className="dot status-agent" /> claude · refactor <span className="x">×</span>
    </button>
    <button className="tm-tab">
      <span className="dot status-error" /> tests <span className="x">×</span>
    </button>
    <button className="tm-tab">
      <span className="dot status-idle" /> psql · staging <span className="x">×</span>
    </button>
    <button className="tm-tab newtab"><svg width="11" height="11"><use href="#plus" /></svg></button>
    <div className="tab-actions">
      <button className="icon-btn" title="Split right"><svg width="12" height="12"><use href="#split-right" /></svg></button>
      <button className="icon-btn" title="Split down"><svg width="12" height="12"><use href="#split-down" /></svg></button>
      <button className="icon-btn" title="Grid 2x2"><svg width="12" height="12"><use href="#grid" /></svg></button>
      <button className="icon-btn" title="Fullscreen"><svg width="12" height="12"><use href="#fullscreen" /></svg></button>
    </div>
  </div>
);

const PaneHeader = ({ title, branch, agent, status = "running", active }) => (
  <div className={`pane-head ${active ? "active" : ""}`}>
    <span className={`dot status-${status}`} />
    {agent && <span className="badge violet sm">{agent}</span>}
    <span className="pane-title">{title}</span>
    {branch && <span className="pane-branch">({branch})</span>}
    <span className="pane-spacer" />
    <span className="pane-meta tnum">80×24</span>
    <button className="icon-btn xs"><svg width="10" height="10"><use href="#split-right" /></svg></button>
    <button className="icon-btn xs"><svg width="10" height="10"><use href="#close" /></svg></button>
  </div>
);

const Term = ({ children }) => <div className="term">{children}</div>;
const L = ({ kind = "out", children }) => <div className={`tline t-${kind}`}>{children}</div>;

const PaneA = () => (
  <div className="pane active">
    <PaneHeader title="~/code/main/dashboard" branch="main" status="running" active />
    <Term>
      <L kind="prompt"><span className="prompt">❯</span> <span className="path">~/code/main/dashboard</span> <span className="branch">(main)</span></L>
      <L kind="cmd"><span className="prompt">❯</span> npm run dev</L>
      <L kind="info">→ vite v5.4.0  ready in 312 ms</L>
      <L kind="info">→ local <span className="link">http://localhost:5173/</span></L>
      <L kind="success">✓ recompiled in 84ms</L>
      <L kind="prompt"><span className="prompt">❯</span> <span className="cur" /></L>
    </Term>
  </div>
);

const PaneB = () => (
  <div className="pane">
    <PaneHeader title="claude · refactor-userlist" agent="CLAUDE" status="agent" />
    <Term>
      <L kind="agent">▎ <span className="agent-tag">user</span> rename UserList → UserTable</L>
      <L kind="success">✓ found 12 import sites in 8 files</L>
      <L kind="info">→ writing <span className="path">src/components/UserTable.tsx</span></L>
      <L kind="agent">▎ <span className="agent-tag">claude</span> apply? <span className="kbd">y</span>/<span className="kbd">n</span></L>
      <L kind="prompt"><span className="prompt">❯</span> <span className="cur" /></L>
    </Term>
  </div>
);

const PaneC = () => (
  <div className="pane">
    <PaneHeader title="vitest --watch" status="error" />
    <Term>
      <L kind="success">✓ src/lib/format.test.ts (8)</L>
      <L kind="error">✗ src/components/UserTable.test.ts (2)</L>
      <L kind="dim">  expected 42 to be 41</L>
      <L kind="dim">─ 12 passed · 2 failed · 0.84s</L>
      <L kind="prompt"><span className="prompt">❯</span> <span className="cur" /></L>
    </Term>
  </div>
);

const PaneD = () => (
  <div className="pane">
    <PaneHeader title="psql staging" status="idle" />
    <Term>
      <L kind="info">staging=&gt; <span className="cmd">SELECT count(*) FROM users;</span></L>
      <L kind="cmd">  <span className="num">12,431</span></L>
      <L kind="dim">(1 row, 240 ms)</L>
      <L kind="info">staging=&gt; <span className="cur" /></L>
    </Term>
  </div>
);

const PaneGrid = () => (
  <div className="pane-grid">
    <div className="row">
      <PaneA />
      <div className="resizer v" />
      <PaneB />
    </div>
    <div className="resizer h" />
    <div className="row">
      <PaneC />
      <div className="resizer v" />
      <PaneD />
    </div>
  </div>
);

window.TabStrip = TabStrip;
window.PaneGrid = PaneGrid;
