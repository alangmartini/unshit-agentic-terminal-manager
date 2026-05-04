const Statusbar = () => (
  <div className="tm-statusbar">
    <span className="sb-cell"><span className="dot status-running" /> ptyd up</span>
    <span className="sb-cell sage">main</span>
    <span className="sb-cell">4 sess · 9 panes</span>
    <span className="sb-spacer" />
    <span className="sb-cell dim tnum">80×24</span>
    <span className="sb-cell amber">Ctrl K</span>
  </div>
);
window.Statusbar = Statusbar;
