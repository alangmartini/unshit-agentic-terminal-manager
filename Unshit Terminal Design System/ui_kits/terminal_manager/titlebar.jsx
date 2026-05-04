// Titlebar — top chrome with brand mark, workspace breadcrumb, search, window controls.
const Titlebar = ({ workspace = "main", branch = "feat/auth-refresh" }) => {
  return (
    <div className="tm-titlebar">
      <div className="tm-brand">
        <svg width="13" height="13"><use href="#brand" /></svg>
        <span>terminal<span className="dot">.</span>mgr</span>
      </div>
      <div className="tm-breadcrumb">
        <span className="crumb">workspaces</span>
        <span className="sep">/</span>
        <span className="crumb amber">{workspace}</span>
        <span className="sep">·</span>
        <span className="crumb sage">({branch})</span>
      </div>
      <div className="tm-search">
        <svg width="11" height="11"><use href="#search" /></svg>
        <span>find session, agent, command</span>
        <span className="kbd">Ctrl K</span>
      </div>
      <div className="tm-tb-right">
        <button className="icon-btn" title="Toggle sidebar"><svg width="13" height="13"><use href="#sidebar" /></svg></button>
        <button className="icon-btn" title="Settings"><svg width="13" height="13"><use href="#gear" /></svg></button>
      </div>
      <div className="tm-win-controls" aria-label="Window controls">
        <button className="win-btn" title="Minimize" aria-label="minimize">
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5h10" stroke="currentColor" strokeWidth="1" fill="none"/></svg>
        </button>
        <button className="win-btn" title="Maximize" aria-label="maximize">
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><rect x="0.5" y="0.5" width="9" height="9" stroke="currentColor" strokeWidth="1" fill="none"/></svg>
        </button>
        <button className="win-btn win-close" title="Close" aria-label="close">
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 0l10 10M10 0l-10 10" stroke="currentColor" strokeWidth="1" fill="none"/></svg>
        </button>
      </div>
    </div>
  );
};

window.Titlebar = Titlebar;
