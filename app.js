/* ============================================================
   TERMINAL MANAGER — DEMO APP
   Split pane grid, resizers, tabs, workspaces, settings
   ============================================================ */

/* ------------------------------------------------------------
   SAMPLE TERMINAL OUTPUT
   ------------------------------------------------------------ */
const SAMPLES = {
  dashboard: [
    { t: 'prompt', path: '~/main/dashboard', branch: 'main' },
    { t: 'cmd',    text: 'go mod tidy' },
    { t: 'dim',    text: 'go: finding module for package github.com/charmbracelet/bubbletea' },
    { t: 'dim',    text: 'go: downloading github.com/charmbracelet/lipgloss v1.1.0' },
    { t: 'success',text: '✓ resolved 23 dependencies in 1.42s' },
    { t: 'prompt', path: '~/main/dashboard', branch: 'main' },
    { t: 'cmd',    text: 'go run main.go --port 4040 --watch' },
    { t: 'info',   text: '→ listening on http://localhost:4040' },
    { t: 'dim',    text: '→ hot reload enabled (462 files tracked)' },
    { t: 'output', text: '[14:32:07] GET  /api/sessions        200  12ms' },
    { t: 'output', text: '[14:32:09] POST /api/spawn           200  45ms' },
    { t: 'output', text: '[14:32:11] GET  /api/sessions        200   8ms' },
    { t: 'warn',   text: '[14:32:14] slow query: users.many   340ms' },
    { t: 'output', text: '[14:32:16] GET  /api/agents/list     200  11ms' },
    { t: 'cursorline', prompt: '~/main/dashboard', branch: 'main' },
  ],
  server: [
    { t: 'prompt', path: '~/api', branch: 'main' },
    { t: 'cmd',    text: 'bun run dev' },
    { t: 'dim',    text: '$ next dev --turbo' },
    { t: 'success',text: '  ✓ Ready in 842ms' },
    { t: 'dim',    text: '  - Local:    http://localhost:3000' },
    { t: 'dim',    text: '  - Network:  http://192.168.1.42:3000' },
    { t: 'output', text: '  ○ compiling / ...' },
    { t: 'success',text: '  ✓ compiled / in 1.2s (923 modules)' },
    { t: 'output', text: '  ○ compiling /api/users ...' },
    { t: 'success',text: '  ✓ compiled /api/users in 142ms (42 modules)' },
    { t: 'info',   text: '  GET /              200  14ms' },
    { t: 'info',   text: '  GET /api/users     200  22ms' },
    { t: 'cursor' },
  ],
  tests: [
    { t: 'prompt', path: '~/tests', branch: 'fix/pdf' },
    { t: 'cmd',    text: 'vitest --run' },
    { t: 'dim',    text: ' RUN  v1.6.0  /home/alanm/tests' },
    { t: 'output', text: '' },
    { t: 'success',text: ' ✓ src/parse.test.ts (12 tests) 142ms' },
    { t: 'success',text: ' ✓ src/render.test.ts (8 tests) 89ms' },
    { t: 'success',text: ' ✓ src/pdf.test.ts (14 tests) 1.2s' },
    { t: 'error',  text: ' ✗ src/export.test.ts (3/4)' },
    { t: 'error',  text: '   × encodes tabular data correctly' },
    { t: 'dim',    text: '     expected 42 to be 41' },
    { t: 'dim',    text: '     at export.test.ts:14:5' },
    { t: 'output', text: '' },
    { t: 'output', text: ' Tests  37 passed | 1 failed (38)' },
    { t: 'output', text: ' Time   2.34s' },
    { t: 'cursor' },
  ],
  logs: [
    { t: 'dim',    text: '── streaming logs from prod ──' },
    { t: 'info',   text: '[info]  2026-04-09 14:32 session created' },
    { t: 'info',   text: '[info]  2026-04-09 14:32 agent spawned id=a82f' },
    { t: 'warn',   text: '[warn]  2026-04-09 14:33 rate limit near (82%)' },
    { t: 'info',   text: '[info]  2026-04-09 14:34 sync complete (412 files)' },
    { t: 'error',  text: '[error] 2026-04-09 14:35 conn refused upstream' },
    { t: 'info',   text: '[info]  2026-04-09 14:35 reconnecting in 2s' },
    { t: 'success',text: '[info]  2026-04-09 14:35 reconnected' },
    { t: 'info',   text: '[info]  2026-04-09 14:35 draining queue (42)' },
    { t: 'cursor' },
  ],
  git: [
    { t: 'prompt', path: '~/main', branch: 'main' },
    { t: 'cmd',    text: 'git status' },
    { t: 'dim',    text: 'On branch main' },
    { t: 'dim',    text: 'Your branch is up to date with \'origin/main\'.' },
    { t: 'output', text: '' },
    { t: 'output', text: 'Changes not staged for commit:' },
    { t: 'warn',   text: '  modified:   src/pane/split.ts' },
    { t: 'warn',   text: '  modified:   styles/theme.css' },
    { t: 'output', text: '' },
    { t: 'output', text: 'Untracked files:' },
    { t: 'dim',    text: '  docs/design-notes.md' },
    { t: 'cursor' },
  ],
  shell: [
    { t: 'prompt', path: '~', branch: null },
    { t: 'cmd',    text: 'neofetch' },
    { t: 'amber',  text: '       _nnnn_       alan@workstation' },
    { t: 'amber',  text: '      dGGGGMMb      ─────────────────' },
    { t: 'amber',  text: '     @p~qp~~qMb    OS: Arch Linux x86_64' },
    { t: 'amber',  text: '     M|@||@) M|    Kernel: 6.8.2-arch1' },
    { t: 'amber',  text: '     @,----.JM|    Shell: bash 5.2.26' },
    { t: 'amber',  text: '    JS^\\__/  qKL   Terminal: mgr v0.1.0' },
    { t: 'amber',  text: '   dZP        qKRb CPU: Ryzen 9 7950X' },
    { t: 'amber',  text: '  dZP          qKKb Memory: 14.2G / 64G' },
    { t: 'cursor' },
  ],
};

function pickRandomSample() {
  const keys = Object.keys(SAMPLES);
  return keys[Math.floor(Math.random() * keys.length)];
}

/* ------------------------------------------------------------
   GRID STATE MODEL
   grid = array of rows, each row = array of pane objects
   Supports up to 4 rows × 4 cols (16 panes)
   ------------------------------------------------------------ */
const MAX_ROWS = 4;
const MAX_COLS = 4;

let paneCounter = 0;
let grid = [];
let activeId = null;

function nextPaneId() {
  paneCounter++;
  return 'p' + paneCounter;
}

function makePane(sample, title, subtitle) {
  const id = nextPaneId();
  return {
    id,
    title: title || 'shell',
    subtitle: subtitle || 'bash',
    sample: sample || 'shell',
    pid: 10000 + Math.floor(Math.random() * 89999),
    cpu: (Math.random() * 8).toFixed(1),
  };
}

function seedGrid() {
  paneCounter = 0;
  grid = [
    [
      makePane('dashboard', 'dashboard', 'go run'),
      makePane('server',    'api.server', 'bun dev'),
    ],
    [
      makePane('tests',     'tests',      'vitest --watch'),
      makePane('logs',      'logs',       'tail -f prod'),
    ],
  ];
  activeId = grid[0][0].id;
}

function findPane(id) {
  for (let r = 0; r < grid.length; r++) {
    for (let c = 0; c < grid[r].length; c++) {
      if (grid[r][c].id === id) return [r, c];
    }
  }
  return [-1, -1];
}

function countPanes() {
  return grid.reduce((n, row) => n + row.length, 0);
}

function splitRight(id) {
  const [r, c] = findPane(id);
  if (r === -1) return;
  if (grid[r].length >= MAX_COLS) return;
  if (countPanes() >= MAX_ROWS * MAX_COLS) return;
  const np = makePane(pickRandomSample());
  grid[r].splice(c + 1, 0, np);
  activeId = np.id;
  render();
}

function splitDown(id) {
  const [r] = findPane(id);
  if (r === -1) return;
  if (grid.length >= MAX_ROWS) return;
  if (countPanes() >= MAX_ROWS * MAX_COLS) return;
  const np = makePane(pickRandomSample());
  grid.splice(r + 1, 0, [np]);
  activeId = np.id;
  render();
}

function closePane(id) {
  const [r, c] = findPane(id);
  if (r === -1) return;
  grid[r].splice(c, 1);
  if (grid[r].length === 0) grid.splice(r, 1);
  if (grid.length === 0) {
    const np = makePane('shell');
    grid.push([np]);
    activeId = np.id;
  } else if (activeId === id) {
    const fallback = grid[Math.min(r, grid.length - 1)][0];
    activeId = fallback.id;
  }
  render();
}

function balanceGrid() {
  // Reset all flex bases
  document.querySelectorAll('.pane-row').forEach(row => {
    row.style.flex = '1 1 0';
  });
  document.querySelectorAll('.pane').forEach(pane => {
    pane.style.flex = '1 1 0';
  });
}

/* ------------------------------------------------------------
   RENDERING
   ------------------------------------------------------------ */
function el(tag, cls, html) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (html !== undefined) e.innerHTML = html;
  return e;
}

function renderLine(line) {
  const div = document.createElement('div');
  div.className = 'term-line';

  switch (line.t) {
    case 'prompt': {
      const branch = line.branch ? ` <span class="term-branch">(${line.branch})</span>` : '';
      div.innerHTML = `<span class="term-dim">❯</span> <span class="term-path">${line.path}</span>${branch}`;
      return div;
    }
    case 'cursorline': {
      const branch = line.branch ? ` <span class="term-branch">(${line.branch})</span>` : '';
      div.innerHTML = `<span class="term-prompt">❯</span> <span class="term-path">${line.prompt}</span>${branch} <span class="term-cursor"></span>`;
      return div;
    }
    case 'cmd':
      div.innerHTML = `<span class="term-prompt">❯</span> <span class="term-cmd">${escapeHtml(line.text)}</span>`;
      return div;
    case 'cursor':
      div.innerHTML = `<span class="term-prompt">❯</span> <span class="term-cursor"></span>`;
      return div;
    default:
      div.classList.add('term-' + line.t);
      div.textContent = line.text;
      return div;
  }
}

function escapeHtml(s) {
  return s.replace(/[&<>"']/g, m => ({
    '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#39;'
  }[m]));
}

function createPaneEl(pane) {
  const isActive = pane.id === activeId;
  const wrap = el('div', 'pane' + (isActive ? ' active' : ''));
  wrap.dataset.paneId = pane.id;

  // header
  const header = el('div', 'pane-header');
  header.innerHTML = `
    <div class="pane-header-left">
      <span class="pane-status-dot"></span>
      <span class="pane-title">${escapeHtml(pane.title)}</span>
      <span class="pane-subtitle">· ${escapeHtml(pane.subtitle)}</span>
    </div>
    <div class="pane-meta">pid ${pane.pid} · ${pane.cpu}%</div>
    <div class="pane-header-right">
      <button class="pane-action" data-act="split-h" title="Split right">
        <svg viewBox="0 0 16 16" width="11" height="11" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2" y="3" width="12" height="10" rx="1"></rect><line x1="8" y1="3" x2="8" y2="13"></line></svg>
      </button>
      <button class="pane-action" data-act="split-v" title="Split down">
        <svg viewBox="0 0 16 16" width="11" height="11" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2" y="3" width="12" height="10" rx="1"></rect><line x1="2" y1="8" x2="14" y2="8"></line></svg>
      </button>
      <button class="pane-action danger" data-act="close" title="Close">
        <svg viewBox="0 0 16 16" width="11" height="11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M4 4l8 8M12 4l-8 8"></path></svg>
      </button>
    </div>
  `;

  // body
  const body = el('div', 'pane-body');
  const lines = SAMPLES[pane.sample] || SAMPLES.shell;
  lines.forEach(l => body.appendChild(renderLine(l)));

  wrap.appendChild(header);
  wrap.appendChild(body);

  // click = activate
  wrap.addEventListener('mousedown', (e) => {
    if (e.target.closest('.pane-action')) return;
    if (e.target.closest('.resizer')) return;
    if (activeId !== pane.id) {
      activeId = pane.id;
      document.querySelectorAll('.pane.active').forEach(p => p.classList.remove('active'));
      wrap.classList.add('active');
    }
  });

  // header action buttons
  header.querySelectorAll('.pane-action').forEach(btn => {
    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      const act = btn.dataset.act;
      if (act === 'split-h') splitRight(pane.id);
      else if (act === 'split-v') splitDown(pane.id);
      else if (act === 'close') closePane(pane.id);
    });
  });

  return wrap;
}

function createResizer(dir) {
  const r = document.createElement('div');
  r.className = 'resizer resizer-' + dir;
  return r;
}

function render() {
  const root = document.getElementById('terminal-grid');
  root.innerHTML = '';

  grid.forEach((row, rowIdx) => {
    const rowEl = el('div', 'pane-row');
    row.forEach((pane, colIdx) => {
      rowEl.appendChild(createPaneEl(pane));
      if (colIdx < row.length - 1) {
        rowEl.appendChild(createResizer('h'));
      }
    });
    root.appendChild(rowEl);
    if (rowIdx < grid.length - 1) {
      root.appendChild(createResizer('v'));
    }
  });

  setupResizers();
  updateStatusBar();
}

/* ------------------------------------------------------------
   RESIZER DRAG LOGIC
   ------------------------------------------------------------ */
function setupResizers() {
  document.querySelectorAll('.resizer').forEach(r => {
    r.addEventListener('mousedown', startResize);
  });
}

function startResize(e) {
  e.preventDefault();
  const resizer = e.currentTarget;
  const isH = resizer.classList.contains('resizer-h');
  const prev = resizer.previousElementSibling;
  const next = resizer.nextElementSibling;
  if (!prev || !next) return;

  resizer.classList.add('dragging');
  document.body.classList.add('dragging-pane');
  document.body.style.cursor = isH ? 'col-resize' : 'row-resize';

  const prevRect = prev.getBoundingClientRect();
  const nextRect = next.getBoundingClientRect();
  const total = isH
    ? prevRect.width + nextRect.width
    : prevRect.height + nextRect.height;
  const startPos = isH ? e.clientX : e.clientY;
  const startPrev = isH ? prevRect.width : prevRect.height;
  const MIN = 120;

  function onMove(ev) {
    const pos = isH ? ev.clientX : ev.clientY;
    let newPrev = startPrev + (pos - startPos);
    if (newPrev < MIN) newPrev = MIN;
    if (newPrev > total - MIN) newPrev = total - MIN;
    const pct = (newPrev / total) * 100;
    prev.style.flex = `${pct} 1 0`;
    next.style.flex = `${100 - pct} 1 0`;
  }

  function onUp() {
    resizer.classList.remove('dragging');
    document.body.classList.remove('dragging-pane');
    document.body.style.cursor = '';
    document.removeEventListener('mousemove', onMove);
    document.removeEventListener('mouseup', onUp);
  }

  document.addEventListener('mousemove', onMove);
  document.addEventListener('mouseup', onUp);
}

/* ------------------------------------------------------------
   TAB BAR
   ------------------------------------------------------------ */
function setupTabs() {
  const tabs = document.getElementById('tabs');
  tabs.addEventListener('click', (e) => {
    const close = e.target.closest('.tab-close');
    const tab = e.target.closest('.tab');
    if (close && tab) {
      e.stopPropagation();
      if (tab.classList.contains('active')) {
        const next = tab.nextElementSibling?.classList.contains('tab')
          ? tab.nextElementSibling
          : tab.previousElementSibling?.classList.contains('tab')
          ? tab.previousElementSibling
          : null;
        tab.remove();
        next && next.classList.add('active');
      } else {
        tab.remove();
      }
      return;
    }
    if (tab) {
      document.querySelectorAll('.tab.active').forEach(t => t.classList.remove('active'));
      tab.classList.add('active');
    }
  });
}

/* ------------------------------------------------------------
   WORKSPACE / SUBTAB
   ------------------------------------------------------------ */
function setupWorkspaces() {
  document.querySelectorAll('.workspace-head').forEach(head => {
    head.addEventListener('click', () => {
      const ws = head.closest('.workspace');
      // toggling collapse
      ws.classList.toggle('collapsed');
    });

    head.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        head.click();
      }
    });
  });

  document.querySelectorAll('.subtab').forEach(sub => {
    sub.addEventListener('click', (e) => {
      e.stopPropagation();
      // de-activate all workspaces + subtabs
      document.querySelectorAll('.workspace.active').forEach(w => w.classList.remove('active'));
      document.querySelectorAll('.subtab.active').forEach(t => t.classList.remove('active'));
      sub.classList.add('active');
      sub.closest('.workspace').classList.add('active');
    });
  });
}

/* ------------------------------------------------------------
   TAB BAR ACTIONS  (split/balance/settings buttons)
   ------------------------------------------------------------ */
function setupTabbarActions() {
  document.getElementById('btn-split-h').addEventListener('click', () => {
    if (activeId) splitRight(activeId);
  });
  document.getElementById('btn-split-v').addEventListener('click', () => {
    if (activeId) splitDown(activeId);
  });
  document.getElementById('btn-grid').addEventListener('click', () => {
    seedGrid();
    render();
  });
  document.getElementById('btn-balance').addEventListener('click', () => {
    balanceGrid();
  });
  document.querySelector('.tab-add').addEventListener('click', () => {
    const bar = document.getElementById('tabs');
    const n = document.querySelectorAll('.tab').length + 1;
    const tab = document.createElement('button');
    tab.className = 'tab';
    tab.dataset.tabId = 't' + n;
    tab.innerHTML = `
      <span class="tab-status idle"></span>
      <span class="tab-name">shell-${n}</span>
      <span class="tab-subtitle">bash</span>
      <span class="tab-close" aria-label="Close tab">×</span>
    `;
    bar.insertBefore(tab, bar.querySelector('.tab-add'));
    document.querySelectorAll('.tab.active').forEach(t => t.classList.remove('active'));
    tab.classList.add('active');
  });
}

/* ------------------------------------------------------------
   SETTINGS MODAL
   ------------------------------------------------------------ */
function setupSettings() {
  const btn = document.getElementById('btn-settings');
  const modal = document.getElementById('settings-modal');
  const close = document.getElementById('settings-close');
  const cancel = document.getElementById('settings-cancel');

  btn.addEventListener('click', () => openModal());
  close.addEventListener('click', () => closeModal());
  cancel.addEventListener('click', () => closeModal());

  modal.addEventListener('click', (e) => {
    if (e.target === modal) closeModal();
  });

  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && modal.classList.contains('open')) {
      closeModal();
    }
    if ((e.metaKey || e.ctrlKey) && e.key === ',') {
      e.preventDefault();
      openModal();
    }
  });

  // modal nav tabs
  document.querySelectorAll('.modal-nav-item').forEach(item => {
    item.addEventListener('click', () => {
      document.querySelectorAll('.modal-nav-item.active').forEach(i => i.classList.remove('active'));
      item.classList.add('active');
    });
  });

  // toggles
  document.querySelectorAll('.toggle').forEach(t => {
    t.addEventListener('click', () => {
      t.classList.toggle('on');
      t.setAttribute('aria-pressed', t.classList.contains('on'));
    });
  });

  // theme chips
  document.querySelectorAll('.theme-chip').forEach(chip => {
    chip.addEventListener('click', () => {
      document.querySelectorAll('.theme-chip.active').forEach(c => c.classList.remove('active'));
      chip.classList.add('active');
    });
  });

  // stepper
  document.querySelectorAll('.stepper').forEach(s => {
    const val = s.querySelector('.stepper-val');
    const btns = s.querySelectorAll('.stepper-btn');
    btns[0].addEventListener('click', () => {
      val.textContent = Math.max(8, parseInt(val.textContent) - 1);
    });
    btns[1].addEventListener('click', () => {
      val.textContent = Math.min(32, parseInt(val.textContent) + 1);
    });
  });
}

function openModal() {
  document.getElementById('settings-modal').classList.add('open');
}
function closeModal() {
  document.getElementById('settings-modal').classList.remove('open');
}

/* ------------------------------------------------------------
   KEYBOARD SHORTCUTS
   ------------------------------------------------------------ */
function setupKeyboard() {
  document.addEventListener('keydown', (e) => {
    const mod = e.metaKey || e.ctrlKey;
    if (!mod) return;

    if (e.key === 'd' && !e.shiftKey) {
      e.preventDefault();
      if (activeId) splitRight(activeId);
    } else if (e.key === 'D' && e.shiftKey) {
      e.preventDefault();
      if (activeId) splitDown(activeId);
    } else if (e.key === 'w') {
      e.preventDefault();
      if (activeId) closePane(activeId);
    }
  });
}

/* ------------------------------------------------------------
   STATUS BAR (live updates)
   ------------------------------------------------------------ */
function updateStatusBar() {
  const n = countPanes();
  const active = document.querySelector('.statusbar-left .status-item:nth-child(2) span:last-child');
  if (active) active.textContent = `${n} active`;
}

function tickStatusBar() {
  // animate small changes so the bar feels alive
  const cpuEl = document.querySelector('#status-cpu .tnum');
  const memEl = document.querySelector('#status-mem .tnum');
  const netEl = document.querySelector('#status-net .tnum');
  if (cpuEl) cpuEl.textContent = (8 + Math.random() * 8).toFixed(1);
  if (memEl) memEl.textContent = (1.2 + Math.random() * 0.5).toFixed(2);
  if (netEl) netEl.textContent = (Math.random() * 3).toFixed(1);
}

function tickClock() {
  const el = document.querySelector('#status-clock .tnum');
  if (!el) return;
  const d = new Date();
  el.textContent = `${String(d.getHours()).padStart(2,'0')}:${String(d.getMinutes()).padStart(2,'0')}`;
}

/* ------------------------------------------------------------
   BOOT
   ------------------------------------------------------------ */
document.addEventListener('DOMContentLoaded', () => {
  seedGrid();
  render();
  setupTabs();
  setupWorkspaces();
  setupTabbarActions();
  setupSettings();
  setupKeyboard();
  tickStatusBar();
  tickClock();
  setInterval(tickStatusBar, 2500);
  setInterval(tickClock, 30000);
});
