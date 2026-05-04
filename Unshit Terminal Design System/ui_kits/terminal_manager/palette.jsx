// Command palette overlay (Ctrl K).
const PalItem = ({ icon, label, hint, kbd, active }) => (
  <div className={`pal-item ${active ? "active" : ""}`}>
    <svg width="11" height="11" className="pal-icon"><use href={`#${icon}`} /></svg>
    <span className="pal-label">{label}</span>
    {hint && <span className="pal-hint">{hint}</span>}
    {kbd && <span className="pal-kbd">{kbd.split("").map((k, i) => <span className="kbd" key={i}>{k}</span>)}</span>}
  </div>
);

const Palette = ({ onClose }) => (
  <div className="pal-scrim" onClick={onClose}>
    <div className="pal" onClick={e => e.stopPropagation()}>
      <div className="pal-input">
        <span className="prompt">❯</span>
        <input defaultValue="split" autoFocus />
        <span className="kbd">esc</span>
      </div>
      <div className="pal-group">
        <div className="pal-group-title">actions</div>
        <PalItem active icon="split-right" label="Split pane right" kbd="Ctrl D" />
        <PalItem icon="split-down" label="Split pane down" kbd="Ctrl Shift D" />
        <PalItem icon="grid" label="Arrange grid 2×2" hint="layout" />
      </div>
      <div className="pal-group">
        <div className="pal-group-title">recent</div>
        <PalItem icon="terminal" label="dashboard-dev" hint="~/code/main/dashboard" />
        <PalItem icon="agent" label="claude · refactor-userlist" hint="18 turns" />
      </div>
    </div>
  </div>
);

window.Palette = Palette;
