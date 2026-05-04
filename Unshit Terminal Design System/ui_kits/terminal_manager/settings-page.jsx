// Standalone Settings page. Mirrors the modal nav, but full-screen with
// switchable sections, scrollable content, and a sticky save bar.

const { useState: useS } = React;

// --- Reusable controls ---
const SetCard = ({ name, meta, children }) => (
  <div className="set-card">
    <div className="set-card-head">
      <span className="name">{name}</span>
      {meta && <span className="name-meta">· {meta}</span>}
    </div>
    {children}
  </div>
);

const SetField = ({ label, desc, children }) => (
  <div className="set-field">
    <div>
      <div className="set-label">{label}</div>
      {desc && <div className="set-desc">{desc}</div>}
    </div>
    <div>{children}</div>
  </div>
);

const Segmented = ({ options, value, onChange }) => (
  <div className="input-segmented">
    {options.map(o => (
      <button key={o} className={value === o ? "active" : ""} onClick={() => onChange?.(o)}>{o}</button>
    ))}
  </div>
);

const Swatches = ({ colors, value }) => (
  <div className="color-swatches">
    {colors.map(c => (
      <div key={c.label} className={`sw ${c.label === value ? "active" : ""}`} style={{ background: c.color }} title={c.label} />
    ))}
  </div>
);

const Kbd = ({ keys }) => (
  <span className="kbd-binding">
    {keys.split("·").map((k, i) => <span className="kbd" key={i}>{k.trim()}</span>)}
  </span>
);

// --- Sections ---
const GeneralSection = () => (
  <>
    <SetCard name="startup">
      <SetField label="Restore last workspace" desc="Reopen the workspace and tabs that were active when terminal.mgr last quit.">
        <Toggle on />
      </SetField>
      <SetField label="Reattach running sessions" desc="Reconnect to ptyd-managed sessions that survived the previous launch.">
        <Toggle on />
      </SetField>
      <SetField label="Open settings on first launch">
        <Toggle />
      </SetField>
    </SetCard>

    <SetCard name="confirm">
      <SetField label="Confirm before closing tab" desc="Prompt when killing a tab with a running process.">
        <Toggle on />
      </SetField>
      <SetField label="Confirm quit" desc="Prompt when closing the window with active sessions.">
        <Toggle on />
      </SetField>
      <SetField label="Confirm kill all" desc="Prompt when sending SIGINT to every pane in the workspace.">
        <Toggle />
      </SetField>
    </SetCard>

    <SetCard name="updates" meta="v0.4.2">
      <SetField label="Update channel" desc="Stable releases ship every other Monday; nightly is built from main.">
        <Segmented options={["stable", "beta", "nightly"]} value="stable" />
      </SetField>
      <SetField label="Check for updates on launch">
        <Toggle on />
      </SetField>
    </SetCard>
  </>
);

const AppearanceSection = () => (
  <>
    <SetCard name="theme">
      <SetField label="Theme" desc="Walnut amber is the only ship-able theme today. Light theme is roadmapped.">
        <Segmented options={["walnut", "ember", "void"]} value="walnut" />
      </SetField>
      <SetField label="Accent" desc="Drives prompt arrows, focus rings, primary buttons, brand mark.">
        <Swatches value="amber" colors={[
          { label: "amber", color: "#d4a348" },
          { label: "ember", color: "#d06e2c" },
          { label: "sage", color: "#8ba85c" },
          { label: "azure", color: "#6aa2ad" },
          { label: "violet", color: "#a88bb8" },
        ]} />
      </SetField>
      <SetField label="Scanline overlay" desc="The CRT signature. Disable for accessibility or static screenshots.">
        <Segmented options={["off", "subtle", "default"]} value="subtle" />
      </SetField>
      <SetField label="Background grain">
        <Toggle on />
      </SetField>
    </SetCard>

    <SetCard name="density">
      <SetField label="Tab bar density">
        <Segmented options={["compact", "default", "comfy"]} value="default" />
      </SetField>
      <SetField label="Sidebar width">
        <input className="input-text input-num" defaultValue="252" /> <span style={{ color: "var(--fg-tertiary)", fontSize: 10, marginLeft: 6 }}>px</span>
      </SetField>
    </SetCard>

    <SetCard name="preview">
      <div className="preview-tile">
        <div><span className="prompt">❯</span> <span className="path">~/code/main/dashboard</span> <span className="branch">(main)</span></div>
        <div><span className="prompt">❯</span> <span style={{ color: "var(--fg-primary)" }}>npm run dev</span></div>
        <div style={{ color: "var(--azure)" }}>→ vite v5.4.0 ready in 312 ms</div>
        <div style={{ color: "var(--sage)" }}>✓ recompiled in 84ms</div>
        <div style={{ color: "var(--rust)" }}>✗ src/lib/format.test.ts (2)</div>
        <div style={{ color: "var(--fg-tertiary)" }}>  expected 42 to be 41</div>
        <div><span className="prompt">❯</span> <span className="cur" /></div>
      </div>
    </SetCard>
  </>
);

const ShellSection = () => (
  <>
    <SetCard name="shell">
      <SetField label="Default shell" desc="Used when ptyd's protocol doesn't specify one. Falls back to the user's login shell.">
        <select className="input-select" defaultValue="/bin/zsh">
          <option>/bin/zsh</option>
          <option>/bin/bash</option>
          <option>/usr/bin/fish</option>
          <option>/opt/homebrew/bin/nu</option>
        </select>
      </SetField>
      <SetField label="Login shell" desc="Spawn shells with -l, sourcing .zprofile / .bash_profile.">
        <Toggle on />
      </SetField>
      <SetField label="Working directory" desc="New sessions inherit the workspace's repo root unless overridden.">
        <Segmented options={["workspace", "home", "last"]} value="workspace" />
      </SetField>
    </SetCard>

    <SetCard name="font">
      <SetField label="Font family" desc="Berkeley Mono is preferred when installed locally; JetBrains Mono is the public default.">
        <select className="input-select" defaultValue="JetBrains Mono">
          <option>JetBrains Mono</option>
          <option>Berkeley Mono</option>
          <option>SF Mono</option>
          <option>Menlo</option>
        </select>
      </SetField>
      <SetField label="Font size">
        <input className="input-text input-num" defaultValue="13" /> <span style={{ color: "var(--fg-tertiary)", fontSize: 10, marginLeft: 6 }}>px</span>
      </SetField>
      <SetField label="Line height">
        <input className="input-text input-num" defaultValue="1.55" />
      </SetField>
      <SetField label="Cursor style">
        <Segmented options={["block", "bar", "underline"]} value="block" />
      </SetField>
      <SetField label="Cursor blink">
        <Toggle on />
      </SetField>
    </SetCard>

    <SetCard name="scrollback">
      <SetField label="Lines retained" desc="Per-pane scrollback persists across UI restarts via ptyd.">
        <input className="input-text input-num" defaultValue="10000" />
      </SetField>
      <SetField label="Persist scrollback to disk">
        <Toggle on />
      </SetField>
    </SetCard>
  </>
);

const KeybindsSection = () => {
  const binds = [
    ["global", [
      ["Open command palette", "Open the fuzzy command palette", "Ctrl · K"],
      ["Open settings", "Show this settings page", "Ctrl · ,"],
      ["Quick switcher", "Jump to any session by name", "Ctrl · P"],
      ["Toggle sidebar", "Show / hide workspace sidebar", "Ctrl · B"],
    ]],
    ["sessions", [
      ["New session", "Spawn a session in active workspace", "Ctrl · T"],
      ["Close tab", "Close the active tab", "Ctrl · W"],
      ["Kill process", "Send SIGINT to active pane", "Ctrl · C"],
      ["Restart session", "Re-spawn process with same args", "Ctrl · Shift · R"],
      ["Rename tab", "Inline-rename the active tab", "Ctrl · E"],
    ]],
    ["panes", [
      ["Split right", "Split active pane to the right", "Ctrl · D"],
      ["Split down", "Split active pane downward", "Ctrl · Shift · D"],
      ["Cycle panes", "Move focus to next pane", "Ctrl · Enter"],
      ["Maximize pane", "Zoom active pane to fill workspace", "Ctrl · M"],
      ["Balance panes", "Equalize sizes in current grid", "Ctrl · ="],
    ]],
    ["agents", [
      ["Attach claude", "Open agent panel for active session", "Ctrl · Shift · C"],
      ["Approve edit", "Apply pending agent edits", "Ctrl · Enter"],
      ["Reject edit", "Discard pending agent edits", "Ctrl · Backspace"],
    ]],
  ];
  return (
    <>
      <SetCard name="search">
        <div style={{ padding: "12px 14px" }}>
          <input className="input-text" style={{ width: "100%", minWidth: 0 }} placeholder="filter by command, key, or chord…" />
        </div>
      </SetCard>
      {binds.map(([group, rows]) => (
        <SetCard key={group} name={group} meta={`${rows.length} bindings`}>
          <table className="kbd-table">
            <thead>
              <tr><th style={{ width: "60%" }}>command</th><th>shortcut</th></tr>
            </thead>
            <tbody>
              {rows.map(([name, desc, keys]) => (
                <tr key={name}>
                  <td>
                    <div className="cmd-name">{name}</div>
                    <div className="cmd-desc">{desc}</div>
                  </td>
                  <td><Kbd keys={keys} /></td>
                </tr>
              ))}
            </tbody>
          </table>
        </SetCard>
      ))}
    </>
  );
};

const AgentsSection = () => (
  <>
    <SetCard name="defaults">
      <SetField label="Confirm agent edits" desc="Require approval before applying file writes from any agent.">
        <Toggle on />
      </SetField>
      <SetField label="Auto-attach panel" desc="Open the agent panel when a claude / amp / codex session starts.">
        <Toggle on />
      </SetField>
      <SetField label="Stream tokens to status bar" desc="Live token-rate readout for the active agent session.">
        <Toggle />
      </SetField>
    </SetCard>

    <SetCard name="claude">
      <SetField label="Model">
        <select className="input-select" defaultValue="claude-sonnet-4.5">
          <option>claude-sonnet-4.5</option>
          <option>claude-opus-4.1</option>
          <option>claude-haiku-4.5</option>
        </select>
      </SetField>
      <SetField label="Max turns" desc="Hard ceiling per agent session before it auto-stops.">
        <input className="input-text input-num" defaultValue="50" />
      </SetField>
      <SetField label="API key" desc="Read from $ANTHROPIC_API_KEY if blank.">
        <input className="input-text" defaultValue="sk-ant-…7f3a" />
      </SetField>
    </SetCard>

    <SetCard name="amp">
      <SetField label="Enabled"><Toggle on /></SetField>
      <SetField label="Auto-confirm low-risk edits" desc="Skip confirm for whitespace / import-only changes.">
        <Toggle />
      </SetField>
    </SetCard>

    <SetCard name="codex">
      <SetField label="Enabled"><Toggle /></SetField>
    </SetCard>
  </>
);

const EnvSection = () => (
  <>
    <SetCard name="environment" meta="14 vars · 3 secrets">
      <table className="kbd-table">
        <thead><tr><th>name</th><th>value</th></tr></thead>
        <tbody>
          <tr><td className="cmd-name">PATH</td><td style={{ color: "var(--azure)" }}>/usr/local/bin:/usr/bin:/bin</td></tr>
          <tr><td className="cmd-name">EDITOR</td><td style={{ color: "var(--fg-primary)" }}>nvim</td></tr>
          <tr><td className="cmd-name">SHELL</td><td style={{ color: "var(--fg-primary)" }}>/bin/zsh</td></tr>
          <tr><td className="cmd-name">ANTHROPIC_API_KEY</td><td style={{ color: "var(--fg-tertiary)" }}>•••• read from keychain</td></tr>
          <tr><td className="cmd-name">DATABASE_URL</td><td style={{ color: "var(--fg-tertiary)" }}>•••• read from .env.local</td></tr>
          <tr><td className="cmd-name">NODE_ENV</td><td style={{ color: "var(--fg-primary)" }}>development</td></tr>
        </tbody>
      </table>
    </SetCard>

    <SetCard name="dotfiles">
      <SetField label="Source on shell start" desc="Sourced after the user's shell rc, so terminal.mgr overrides take precedence.">
        <Toggle on />
      </SetField>
      <SetField label="File">
        <input className="input-text" defaultValue="~/.config/terminal-mgr/init.sh" style={{ minWidth: 280 }} />
      </SetField>
    </SetCard>
  </>
);

const SECTIONS = [
  { id: "general",    icon: "terminal", label: "general",    component: GeneralSection,    desc: "Startup behavior, confirms, update channel." },
  { id: "appearance", icon: "grid",     label: "appearance", component: AppearanceSection, desc: "Theme, accent, density, preview." },
  { id: "shell",      icon: "shell",    label: "shell",      component: ShellSection,      desc: "Default shell, font, scrollback." },
  { id: "keybinds",   icon: "kbd",      label: "keybinds",   component: KeybindsSection,   desc: "Every binding, grouped." },
  { id: "agents",     icon: "agent",    label: "agents",     component: AgentsSection,     desc: "Per-provider config and approvals." },
  { id: "env",        icon: "env",      label: "env",        component: EnvSection,        desc: "Environment variables, dotfiles." },
];

const SettingsPage = () => {
  const [active, setActive] = useS("appearance");
  const [dirty, setDirty] = useS(true);
  const Section = SECTIONS.find(s => s.id === active).component;
  const meta = SECTIONS.find(s => s.id === active);

  return (
    <div className="settings-page">
      <aside className="set-page-rail">
        <div className="set-page-rail-head">
          <span className="title">settings</span>
          <span className="sub">v0.4.2</span>
        </div>
        <div className="set-page-search">
          <svg width="11" height="11"><use href="#search" /></svg>
          <input placeholder="find a setting…" />
          <span className="kbd">Ctrl F</span>
        </div>
        <nav className="set-page-nav">
          <div className="group">workspace</div>
          {SECTIONS.slice(0, 3).map(s => (
            <button key={s.id} className={`set-page-nav-item ${active === s.id ? "active" : ""}`} onClick={() => setActive(s.id)}>
              <svg width="11" height="11"><use href={`#${s.icon}`} /></svg>
              {s.label}
            </button>
          ))}
          <div className="group">automation</div>
          {SECTIONS.slice(3).map(s => (
            <button key={s.id} className={`set-page-nav-item ${active === s.id ? "active" : ""}`} onClick={() => setActive(s.id)}>
              <svg width="11" height="11"><use href={`#${s.icon}`} /></svg>
              {s.label}
            </button>
          ))}
        </nav>
        <div className="set-page-foot">
          <span className="dot status-running" />
          <span>ptyd up</span>
          <span style={{ marginLeft: "auto", color: "var(--fg-muted)" }}>build 4a2f1c</span>
        </div>
      </aside>

      <div className="set-page-content">
        <div className="set-page-header">
          <div className="crumb">settings · {meta.label}</div>
          <h1>{meta.label}</h1>
          <div className="blurb">{meta.desc}</div>
        </div>
        <div className="set-page-body">
          <Section />
          <div className="set-page-savebar">
            {dirty
              ? <span className="dirty">● 2 unsaved changes</span>
              : <span>all changes saved</span>
            }
            <span className="spacer" />
            <button className="ghost" onClick={() => setDirty(false)}>discard</button>
            <button className="ghost">reset to defaults</button>
            <button className="primary" onClick={() => setDirty(false)}>save changes</button>
          </div>
        </div>
      </div>
    </div>
  );
};

window.SettingsPage = SettingsPage;
