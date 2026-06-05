// ============================================================
// command-palette.jsx — terminal.mgr universal palette
// Unified fuzzy search + typed modes (> actions, @ agents,
// : nav, / search) with a live preview pane.
// Exports: window.CommandPalette, window.CP_MODES
// ============================================================
const { useState, useEffect, useRef, useMemo, useCallback } = React;

// ---- Modes -------------------------------------------------
const CP_MODES = {
  unified: { prefix: "",  icon: "search",   chip: null,        label: "everything", ph: "search sessions, agents, commands…" },
  actions: { prefix: ">", icon: "gear",     chip: "m-actions", label: "actions",    ph: "run a command…" },
  agents:  { prefix: "@", icon: "agent",    chip: "m-agents",  label: "agents",     ph: "jump to an agent…" },
  nav:     { prefix: ":", icon: "worktree", chip: "m-nav",     label: "navigate",   ph: "workspace or worktree…" },
  search:  { prefix: "/", icon: "terminal", chip: "m-search",  label: "scrollback", ph: "search terminal output…" },
};
const MODE_ORDER = ["unified", "actions", "agents", "nav", "search"];
const PREFIX_TO_MODE = { ">": "actions", "@": "agents", ":": "nav", "/": "search" };
window.CP_MODES = CP_MODES;

// ---- Data --------------------------------------------------
const ACTIONS = [
  { id: "split-r", icon: "split-right", label: "Split pane right",       kbd: ["Ctrl", "D"],        desc: "Split the focused pane into two columns.", group: "actions" },
  { id: "split-d", icon: "split-down",  label: "Split pane down",        kbd: ["Ctrl", "⇧", "D"], desc: "Split the focused pane into two rows.", group: "actions" },
  { id: "newterm", icon: "plus",        label: "New terminal",           kbd: ["Ctrl", "T"],        desc: "Open a new shell in the active workspace.", group: "actions" },
  { id: "grid",    icon: "grid",        label: "Arrange grid 2×2",  kbd: null,                 desc: "Tile all panes into an even 2×2 grid.", group: "layout" },
  { id: "balance", icon: "balance",     label: "Balance panes",          kbd: ["Ctrl", "="],        desc: "Equalize the size of every pane in the tab.", group: "layout" },
  { id: "full",    icon: "fullscreen",  label: "Toggle pane fullscreen", kbd: ["Ctrl", "⏎"],   desc: "Zoom the focused pane to fill the grid.", group: "layout" },
  { id: "close",   icon: "close",       label: "Close pane",             kbd: ["Ctrl", "W"],        desc: "Close the focused pane and its session.", group: "actions" },
  { id: "kill",    icon: "terminal",    label: "Kill session",           kbd: ["Ctrl", "C"],        desc: "Send SIGINT to the process in the focused pane.", danger: true, group: "session" },
  { id: "restart", icon: "shell",       label: "Restart session",        kbd: null,                 desc: "Respawn the shell, keeping cwd and env.", group: "session" },
  { id: "clear",   icon: "terminal",    label: "Clear scrollback",       kbd: ["Ctrl", "L"],        desc: "Wipe the buffer of the focused terminal.", group: "session" },
  { id: "sidebar", icon: "sidebar",     label: "Toggle sidebar",         kbd: ["Ctrl", "B"],        desc: "Show or hide the workspaces sidebar.", group: "layout" },
  { id: "settings",icon: "gear",        label: "Open settings",          kbd: ["Ctrl", ","],        desc: "General, appearance, shell, keybinds, agents.", group: "app" },
  { id: "theme",   icon: "settings",    label: "Change theme…",     kbd: null,                 desc: "Switch CRT palette: amber · sage · azure · mono.", group: "app" },
  { id: "spawn",   icon: "agent",       label: "Spawn agent…",      kbd: null,                 desc: "Attach claude, amp, or codex to a terminal.", group: "session" },
  { id: "worktree",icon: "worktree",    label: "New worktree…",     kbd: null,                 desc: "Create a git worktree and open it in a tab.", group: "session" },
];

const SESSIONS = [
  { id: "dashboard-dev", icon: "terminal", label: "dashboard-dev", cmd: "go run main.go --port 4040 --watch", path: "~/main/dashboard", branch: "main",        status: "running", pid: 82028, cpu: "7.0%", up: "14m",
    term: [["prompt","❯ ", "path","~/main/dashboard ", "branch","(main)"],["cmd","❯ go run main.go --port 4040 --watch"],["info","→ listening on http://localhost:4040"],["dim","[14:32:14] slow query: users.many  340ms"],["info","[14:32:16] GET /api/agents/list  200  11ms"]] },
  { id: "api.server", icon: "terminal", label: "api.server", cmd: "bun dev", path: "~/api", branch: "main", status: "running", pid: 74756, cpu: "0.6%", up: "52m",
    term: [["cmd","❯ bun run dev"],["success","✓ Ready in 842ms"],["info","- Local:   http://localhost:3000"],["success","✓ compiled /api/users in 142ms (42 modules)"],["info","GET /api/users   200  22ms"]] },
  { id: "tests", icon: "terminal", label: "tests", cmd: "vitest --watch", path: "~/tests", branch: "fix/pdf-export", status: "error", pid: 45866, cpu: "3.8%", up: "6m",
    term: [["cmd","❯ vitest --run"],["success","✓ src/parse.test.ts (12 tests) 142ms"],["error","✗ src/export.test.ts (3/4)"],["dim","  expected 42 to be 41"],["dim","Tests  37 passed | 1 failed (38)"]] },
  { id: "logs", icon: "terminal", label: "logs", cmd: "tail -f prod", path: "~/infra", branch: "staging", status: "running", pid: 95224, cpu: "2.5%", up: "3h",
    term: [["dim","— streaming logs from prod —"],["info","[info]  14:34 sync complete (412 files)"],["error","[error] 14:35 conn refused upstream"],["info","[info]  14:35 reconnected"],["dim","[info]  14:35 draining queue (42)"]] },
  { id: "psql", icon: "terminal", label: "psql · staging", cmd: "psql staging", path: "~/infra", branch: "staging", status: "idle", pid: 31002, cpu: "0.0%", up: "1h",
    term: [["info","staging=> SELECT count(*) FROM users;"],["cmd","  12,431"],["dim","(1 row, 240 ms)"],["info","staging=>"]] },
  { id: "scratch", icon: "terminal", label: "scratch", cmd: "bash", path: "~/scratch", branch: null, status: "stopped", pid: null, cpu: "—", up: "—",
    term: [["dim","[process exited — code 0]"],["dim","press ↵ to restart shell"]] },
];

const AGENTS = [
  { id: "claude", icon: "agent", label: "claude · refactor-userlist", status: "running", model: "claude-sonnet", turns: 18, ws: "main", last: "applying rename across 8 files…",
    term: [["agent","▎ user  rename UserList → UserTable"],["success","✓ found 12 import sites in 8 files"],["agent","▎ claude  apply? y/n"]] },
  { id: "amp", icon: "agent", label: "amp · verify readme docs", status: "stopped", model: "amp", turns: 4, ws: "api", last: "stopped · awaiting input",
    term: [["agent","▎ amp  checked 3 code blocks in README"],["dim","[stopped]"]] },
  { id: "codex", icon: "agent", label: "codex · review export.test", status: "waiting", model: "codex", turns: 9, ws: "infra", last: "needs review on 1 diff",
    term: [["agent","▎ codex  proposed fix for export.test.ts"],["dim","awaiting your review →"]] },
];

const WORKSPACES = [
  { id: "ws-main",   icon: "worktree", label: "main",    branch: "main",          terms: 4, agents: 2, path: "~/main",    group: "workspaces" },
  { id: "ws-api",    icon: "worktree", label: "api",     branch: "fix/pdf-export",terms: 2, agents: 1, path: "~/api",     group: "workspaces" },
  { id: "ws-infra",  icon: "worktree", label: "infra",   branch: "staging",       terms: 2, agents: 0, path: "~/infra",   group: "workspaces" },
  { id: "ws-scratch",icon: "worktree", label: "scratch", branch: null,            terms: 1, agents: 0, path: "~/scratch", group: "workspaces" },
  { id: "wt-pdf",    icon: "session",  label: "fix/pdf-export", branch: "fix/pdf-export", path: "~/.wt/pdf",   group: "worktrees", wt: true },
  { id: "wt-auth",   icon: "session",  label: "feat/auth-refresh", branch: "feat/auth-refresh", path: "~/.wt/auth", group: "worktrees", wt: true },
];

const SCROLLBACK = [
  { sess: "dashboard-dev", file: "main.go:142",       text: "[14:32:14] slow query: users.many  340ms" },
  { sess: "dashboard-dev", file: "router.go:88",      text: "[14:32:16] GET /api/agents/list  200  11ms" },
  { sess: "api.server",    file: "users.ts:31",       text: "GET /api/users  200  22ms  (42 rows)" },
  { sess: "api.server",    file: "dev.log",           text: "✓ compiled /api/users in 142ms (42 modules)" },
  { sess: "tests",         file: "export.test.ts:14", text: "✗ encodes tabular data correctly: expected 42 to be 41" },
  { sess: "tests",         file: "export.test.ts:18", text: "Tests  37 passed | 1 failed (38)" },
  { sess: "logs",          file: "prod.log",          text: "[error] 14:35 conn refused upstream" },
  { sess: "logs",          file: "prod.log",          text: "[warn]  14:33 rate limit near (82%)" },
  { sess: "logs",          file: "prod.log",          text: "[info]  14:34 sync complete (412 files)" },
  { sess: "psql",          file: "query.sql",         text: "SELECT count(*) FROM users;  12,431" },
];

// ---- Fuzzy matcher ----------------------------------------
// Subsequence match; returns {score, idx:Set} or null. Lower score = better.
function fuzzy(q, text) {
  if (!q) return { score: 0, idx: null };
  const t = text.toLowerCase();
  const query = q.toLowerCase();
  let ti = 0, prev = -2, score = 0;
  const idx = [];
  for (let qi = 0; qi < query.length; qi++) {
    const c = query[qi];
    const found = t.indexOf(c, ti);
    if (found === -1) return null;
    idx.push(found);
    if (found === prev + 1) score += 1; else score += 6 + (found - ti); // reward contiguous, penalize gaps
    if (found === 0 || /[\s/._\-:]/.test(t[found - 1])) score -= 3;      // reward word-boundary
    prev = found; ti = found + 1;
  }
  return { score: score + (text.length - query.length) * 0.05, idx: new Set(idx) };
}

function Hi({ text, idx }) {
  if (!idx) return text;
  return [...text].map((ch, i) =>
    idx.has(i) ? <b className="hl" key={i}>{ch}</b> : <span key={i}>{ch}</span>
  );
}

// ---- Icon helper ------------------------------------------
const Ico = ({ name, w = 14 }) => <svg width={w} height={w}><use href={`#${name}`} /></svg>;
const Kbd = ({ keys }) => keys ? <span className="cp-kbd">{keys.map((k, i) => <span className="kbd" key={i}>{k}</span>)}</span> : null;

// ---- Result building --------------------------------------
function buildResults(mode, query) {
  const groups = []; // {title, items:[{...,score,idx}]}
  const add = (title, pool, makeItem) => {
    const scored = [];
    for (const it of pool) {
      const m = fuzzy(query, it._search || it.label);
      if (m) scored.push({ ...makeItem(it), _score: m.score, _idx: m.idx });
    }
    scored.sort((a, b) => a._score - b._score);
    if (scored.length) groups.push({ title, items: scored });
  };

  if (mode === "actions" || mode === "unified") {
    const byGroup = {};
    for (const a of ACTIONS) {
      const m = fuzzy(query, a.label);
      if (!m) continue;
      (byGroup[a.group] ||= []).push({ kind: "action", data: a, label: a.label, icon: a.icon, _score: m.score, _idx: m.idx });
    }
    const order = mode === "actions" ? ["actions", "layout", "session", "app"] : ["actions"];
    for (const g of order) {
      if (!byGroup[g]) continue;
      byGroup[g].sort((a, b) => a._score - b._score);
      groups.push({ title: g === "actions" ? "commands" : g, items: mode === "unified" ? byGroup[g].slice(0, 3) : byGroup[g] });
    }
  }
  if (mode === "agents" || mode === "unified") {
    add("agents", AGENTS, (a) => ({ kind: "agent", data: a, label: a.label, icon: a.icon }));
  }
  if (mode === "nav" || mode === "unified") {
    const ws = WORKSPACES.filter((w) => !w.wt);
    const wt = WORKSPACES.filter((w) => w.wt);
    add("workspaces", ws, (w) => ({ kind: "ws", data: w, label: w.label, icon: w.icon }));
    if (mode === "nav") add("worktrees", wt, (w) => ({ kind: "ws", data: w, label: w.label, icon: w.icon }));
  }
  if (mode === "unified") {
    add("sessions", SESSIONS, (s) => ({ kind: "session", data: s, label: s.label, icon: s.icon }));
  }
  if (mode === "search") {
    if (query) {
      const hits = SCROLLBACK
        .map((h) => ({ h, m: fuzzy(query, h.text) }))
        .filter((x) => x.m)
        .sort((a, b) => a.m.score - b.m.score)
        .map((x) => ({ kind: "hit", data: x.h, label: x.h.text, icon: "terminal", _score: x.m.score, _idx: x.m.idx }));
      if (hits.length) groups.push({ title: `${hits.length} matches in scrollback`, items: hits });
    }
  }
  return groups;
}

// ---- Preview pane -----------------------------------------
function Preview({ item }) {
  if (!item) return <div className="cp-preview" />;
  const d = item.data;
  if (item.kind === "action") {
    return (
      <div className="cp-preview">
        <div className="cp-pv-head">
          <div className="cp-pv-ic" style={d.danger ? { color: "var(--rust)", borderColor: "rgba(201,85,58,.4)" } : null}><Ico name={d.icon} /></div>
          <div className="cp-pv-titles"><div className="cp-pv-title">{d.label}</div><div className="cp-pv-kicker">{d.group}</div></div>
        </div>
        <div className="cp-pv-body">
          <div className="cp-pv-desc">{d.desc}</div>
          <div className="cp-pv-rows">
            <div className="cp-pv-row"><span className="cp-pv-k">shortcut</span><span className="cp-pv-v">{d.kbd ? <Kbd keys={d.kbd} /> : "unbound"}</span></div>
            <div className="cp-pv-row"><span className="cp-pv-k">scope</span><span className="cp-pv-v">focused pane · dashboard-dev</span></div>
            <div className="cp-pv-row"><span className="cp-pv-k">reversible</span><span className="cp-pv-v">{d.danger ? "no" : "yes"}</span></div>
          </div>
        </div>
        <div className="cp-pv-run"><span>{d.danger ? "irreversible" : "ready"}</span><span className="spacer" /><span>run</span><span className="kbd">⏎</span></div>
      </div>
    );
  }
  if (item.kind === "session") {
    return (
      <div className="cp-preview">
        <div className="cp-pv-head">
          <div className="cp-pv-ic"><span className={`dot status-${d.status}`} style={{ width: 9, height: 9 }} /></div>
          <div className="cp-pv-titles"><div className="cp-pv-title">{d.label}</div><div className="cp-pv-kicker">{d.status} · {d.cmd}</div></div>
        </div>
        <div className="cp-pv-body">
          <div className="cp-pv-rows" style={{ marginBottom: 10 }}>
            <div className="cp-pv-row"><span className="cp-pv-k">path</span><span className="cp-pv-v path">{d.path}</span></div>
            <div className="cp-pv-row"><span className="cp-pv-k">branch</span><span className="cp-pv-v branch">{d.branch ? `(${d.branch})` : "—"}</span></div>
            <div className="cp-pv-row"><span className="cp-pv-k">pid · cpu</span><span className="cp-pv-v">{d.pid ?? "—"} · {d.cpu}</span></div>
            <div className="cp-pv-row"><span className="cp-pv-k">uptime</span><span className="cp-pv-v">{d.up}</span></div>
          </div>
          <div className="cp-pv-term">{d.term.map((ln, i) => <Termline key={i} parts={ln} />)}</div>
        </div>
        <div className="cp-pv-run"><span>jump to pane</span><span className="spacer" /><span className="kbd">⏎</span></div>
      </div>
    );
  }
  if (item.kind === "agent") {
    return (
      <div className="cp-preview">
        <div className="cp-pv-head">
          <div className="cp-pv-ic" style={{ color: "var(--violet)", borderColor: "rgba(168,139,184,.4)" }}><Ico name="agent" /></div>
          <div className="cp-pv-titles"><div className="cp-pv-title">{d.label}</div><div className="cp-pv-kicker">{d.status} · {d.model}</div></div>
        </div>
        <div className="cp-pv-body">
          <div className="cp-pv-rows" style={{ marginBottom: 10 }}>
            <div className="cp-pv-row"><span className="cp-pv-k">workspace</span><span className="cp-pv-v amber">{d.ws}</span></div>
            <div className="cp-pv-row"><span className="cp-pv-k">turns</span><span className="cp-pv-v">{d.turns}</span></div>
            <div className="cp-pv-row"><span className="cp-pv-k">last</span><span className="cp-pv-v">{d.last}</span></div>
          </div>
          <div className="cp-pv-term">{d.term.map((ln, i) => <Termline key={i} parts={ln} />)}</div>
        </div>
        <div className="cp-pv-run"><span>attach to agent</span><span className="spacer" /><span className="kbd">⏎</span></div>
      </div>
    );
  }
  if (item.kind === "ws") {
    return (
      <div className="cp-preview">
        <div className="cp-pv-head">
          <div className="cp-pv-ic" style={{ color: "var(--azure)", borderColor: "rgba(106,162,173,.4)" }}><Ico name={d.wt ? "session" : "worktree"} /></div>
          <div className="cp-pv-titles"><div className="cp-pv-title">{d.label}</div><div className="cp-pv-kicker">{d.wt ? "worktree" : "workspace"}</div></div>
        </div>
        <div className="cp-pv-body">
          <div className="cp-pv-rows">
            <div className="cp-pv-row"><span className="cp-pv-k">path</span><span className="cp-pv-v path">{d.path}</span></div>
            <div className="cp-pv-row"><span className="cp-pv-k">branch</span><span className="cp-pv-v branch">{d.branch ? `(${d.branch})` : "no branch"}</span></div>
            {!d.wt && <div className="cp-pv-row"><span className="cp-pv-k">terminals</span><span className="cp-pv-v">{d.terms}</span></div>}
            {!d.wt && <div className="cp-pv-row"><span className="cp-pv-k">agents</span><span className="cp-pv-v">{d.agents}</span></div>}
          </div>
        </div>
        <div className="cp-pv-run"><span>open workspace</span><span className="spacer" /><span className="kbd">⏎</span></div>
      </div>
    );
  }
  if (item.kind === "hit") {
    return (
      <div className="cp-preview">
        <div className="cp-pv-head">
          <div className="cp-pv-ic"><Ico name="terminal" /></div>
          <div className="cp-pv-titles"><div className="cp-pv-title">{d.sess}</div><div className="cp-pv-kicker">{d.file}</div></div>
        </div>
        <div className="cp-pv-body">
          <div className="cp-pv-term">
            <div className="tline t-dim">{d.file}</div>
            <div className="tline t-hit"><Hi text={d.text} idx={item._idx} /></div>
          </div>
        </div>
        <div className="cp-pv-run"><span>reveal in pane</span><span className="spacer" /><span className="kbd">⏎</span></div>
      </div>
    );
  }
  return <div className="cp-preview" />;
}

const Termline = ({ parts }) => {
  // parts: [kindClassForFirst, text, ...] -> support pair runs: [k1,t1,k2,t2,...]
  const kind = parts[0];
  const runs = [];
  for (let i = 0; i < parts.length; i += 2) runs.push([parts[i], parts[i + 1]]);
  return <div className={`tline t-${kind}`}>{runs.map(([k, t], i) => <span key={i} className={k}>{t}</span>)}</div>;
};

// ---- Right-side cell per row type -------------------------
function RightCell({ item }) {
  const d = item.data;
  if (item.kind === "action") return d.kbd ? <Kbd keys={d.kbd} /> : <span className="cp-meta">{d.group}</span>;
  if (item.kind === "session") return <span className={`cp-tag ${d.status}`}>{d.status}</span>;
  if (item.kind === "agent")   return <span className={`cp-tag ${d.status === "stopped" ? "stopped" : d.status}`}>{d.status}</span>;
  if (item.kind === "ws")      return d.branch ? <span className="cp-sub"><span className="branch">({d.branch})</span></span> : <span className="cp-meta">no branch</span>;
  if (item.kind === "hit")     return <span className="cp-meta">{d.sess}</span>;
  return null;
}
function SubLine({ item }) {
  const d = item.data;
  if (item.kind === "session") return <div className="cp-sub"><span className="path">{d.path}</span> · {d.cmd}</div>;
  if (item.kind === "agent")   return <div className="cp-sub">{d.model} · {d.turns} turns · ws {d.ws}</div>;
  if (item.kind === "ws")      return <div className="cp-sub"><span className="path">{d.path}</span></div>;
  if (item.kind === "action")  return <div className="cp-sub">{d.desc}</div>;
  if (item.kind === "hit")     return <div className="cp-sub">{d.file}</div>;
  return null;
}
function RowIcon({ item }) {
  const d = item.data;
  if (item.kind === "session") return <span className={`cp-ic ${d.status === "running" ? "run" : ""}`}><span className={`dot status-${d.status}`} /></span>;
  if (item.kind === "agent")   return <span className={`cp-ic ${d.status === "running" ? "run" : ""}`}><span className="dot status-agent" /></span>;
  return <span className="cp-ic"><Ico name={item.icon} w={13} /></span>;
}

// ---- Palette (commands only) ------------------------------
function CommandPalette({ onClose, onExecute, tweaks = {} }) {
  const { width = 560, density = "comfortable", accent = "amber", glow = 0.55, preview = true, footer = true } = tweaks;
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef(null);
  const listRef = useRef(null);
  const activeRef = useRef(null);

  // Always searches the command set, grouped commands · layout · session · app.
  const groups = useMemo(() => buildResults("actions", query), [query]);
  const flat = useMemo(() => groups.flatMap((g) => g.items), [groups]);
  const total = flat.length;

  useEffect(() => { setActive(0); }, [query]);
  useEffect(() => { inputRef.current && inputRef.current.focus(); }, []);

  // keep active row in view (no scrollIntoView)
  useEffect(() => {
    const el = activeRef.current, list = listRef.current;
    if (!el || !list) return;
    const top = el.offsetTop, bottom = top + el.offsetHeight;
    if (top < list.scrollTop) list.scrollTop = top - 28;
    else if (bottom > list.scrollTop + list.clientHeight) list.scrollTop = bottom - list.clientHeight + 8;
  }, [active, query]);

  const onKeyDown = (e) => {
    if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) { e.preventDefault(); if (total) setActive((a) => (a + 1) % total); }
    else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) { e.preventDefault(); if (total) setActive((a) => (a - 1 + total) % total); }
    else if (e.key === "Enter") { e.preventDefault(); if (flat[active]) { onExecute && onExecute(flat[active]); onClose(); } }
    else if (e.key === "Escape") { e.preventDefault(); if (query) setQuery(""); else onClose(); }
  };

  let flatIdx = -1;
  const activeItem = flat[active];

  const scrimStyle = {
    "--cp-w": width + "px",
    "--cp-glow": glow,
    "--cp-accent": `var(--${accent === "amber" ? "amber-300" : accent})`,
    "--cp-accent-bright": `var(--${accent === "amber" ? "amber-100" : accent})`,
    "--cp-accent-soft": ACCENT_SOFT[accent] || ACCENT_SOFT.amber,
  };

  return (
    <div className="cp-scrim" style={scrimStyle} onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className={`cp ${density === "compact" ? "compact" : ""} ${preview ? "has-preview" : ""}`} role="dialog" aria-label="Command palette">
        <div className="cp-input">
          <span className="cp-prompt">❯</span>
          <input ref={inputRef} value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={onKeyDown} placeholder="run a command…" spellCheck="false" autoComplete="off" />
          <span className="cp-esc">esc</span>
        </div>

        <div className="cp-body">
          <div className="cp-list" ref={listRef}>
            {total === 0 && (
              <div className="cp-empty">
                <div className="big">no command matches “{query}”</div>
                <div className="dim">try a verb — split · kill · theme · settings</div>
              </div>
            )}
            {groups.map((g, gi) => (
              <div key={gi}>
                <div className="cp-group-title"><span>{g.title}</span><span className="rule" /><span className="gcount">{g.items.length}</span></div>
                {g.items.map((it) => {
                  flatIdx++;
                  const isActive = flatIdx === active;
                  const myIdx = flatIdx;
                  const mm = fuzzy(query, it.label);
                  return (
                    <div
                      key={it.data.id || myIdx}
                      ref={isActive ? activeRef : null}
                      className={`cp-item ${isActive ? "active" : ""}`}
                      onMouseMove={() => setActive(myIdx)}
                      onClick={() => { onExecute && onExecute(it); onClose(); }}
                    >
                      <RowIcon item={it} />
                      <div className="cp-main">
                        <div className="cp-label"><Hi text={it.label} idx={mm ? mm.idx : null} /></div>
                        <SubLine item={it} />
                      </div>
                      <div className="cp-right"><RightCell item={it} /></div>
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
          {preview && <Preview item={activeItem} />}
        </div>

        {footer && (
          <div className="cp-foot">
            <span className="fh"><span className="kbd">↑</span><span className="kbd">↓</span> move</span>
            <span className="fh"><span className="kbd">⏎</span> run</span>
            <span className="fh"><span className="kbd">esc</span> close</span>
            <span className="spacer" />
            <span className="count"><b>{total}</b> {total === 1 ? "command" : "commands"}</span>
          </div>
        )}
      </div>
    </div>
  );
}

const ACCENT_SOFT = {
  amber:  "rgba(212,163,72,.12)",
  sage:   "rgba(139,168,92,.14)",
  azure:  "rgba(106,162,173,.14)",
  violet: "rgba(168,139,184,.14)",
};

window.CommandPalette = CommandPalette;
