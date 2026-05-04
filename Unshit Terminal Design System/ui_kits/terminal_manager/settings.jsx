// Settings modal — keybinds, agents, appearance preview.
const NavItem = ({ active, icon, children }) => (
  <button className={`set-nav ${active ? "active" : ""}`}>
    <svg width="11" height="11"><use href={`#${icon}`} /></svg>
    {children}
  </button>
);

const Field = ({ label, desc, children }) => (
  <div className="set-field">
    <div className="set-label-col">
      <div className="set-label">{label}</div>
      {desc && <div className="set-desc">{desc}</div>}
    </div>
    <div className="set-control-col">{children}</div>
  </div>
);

const Toggle = ({ on }) => <div className={`toggle ${on ? "on" : ""}`} />;

const SettingsModal = ({ onClose }) => (
  <div className="set-scrim" onClick={onClose}>
    <div className="set-modal" onClick={e => e.stopPropagation()}>
      <div className="set-head">
        <span className="set-mark">◆</span>
        <span className="set-title">settings</span>
        <span className="set-sub dim">terminal.mgr · v0.4.2</span>
        <span className="set-spacer" />
        <button className="icon-btn" onClick={onClose}><svg width="11" height="11"><use href="#close" /></svg></button>
      </div>
      <div className="set-body">
        <nav className="set-nav-rail">
          <NavItem icon="terminal">general</NavItem>
          <NavItem icon="grid">appearance</NavItem>
          <NavItem icon="shell">shell</NavItem>
          <NavItem active icon="kbd">keybinds</NavItem>
          <NavItem icon="agent">agents</NavItem>
          <NavItem icon="env">env</NavItem>
        </nav>
        <div className="set-content">
          <h3 className="set-section">global</h3>
          <Field label="Command palette" desc="Open the fuzzy command palette">
            <span className="kbd">Ctrl</span><span className="kbd">K</span>
          </Field>
          <Field label="New session" desc="Spawn a session in the active workspace">
            <span className="kbd">Ctrl</span><span className="kbd">T</span>
          </Field>
          <Field label="Kill session" desc="Send SIGINT to the active pane">
            <span className="kbd">Ctrl</span><span className="kbd">C</span>
          </Field>
          <h3 className="set-section">panes</h3>
          <Field label="Split right"><span className="kbd">Ctrl</span><span className="kbd">D</span></Field>
          <Field label="Split down"><span className="kbd">Shift</span><span className="kbd">Ctrl</span><span className="kbd">D</span></Field>
          <Field label="Cycle panes"><span className="kbd">Ctrl</span><span className="kbd">Enter</span></Field>
          <h3 className="set-section">agents</h3>
          <Field label="Confirm agent edits" desc="Require approval before applying file writes">
            <Toggle on />
          </Field>
          <Field label="Auto-attach claude" desc="Open agent panel when claude session starts">
            <Toggle />
          </Field>
        </div>
      </div>
      <div className="set-foot">
        <button className="ghost">reset to defaults</button>
        <span className="set-spacer" />
        <button className="ghost" onClick={onClose}>cancel</button>
        <button className="primary">save changes</button>
      </div>
    </div>
  </div>
);

window.SettingsModal = SettingsModal;
