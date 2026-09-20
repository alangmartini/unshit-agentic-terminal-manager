#!/usr/bin/env node
// Validate a flow document, embed source excerpts, and inject it into the
// standalone Flow Explorer template.
//
//   node render.mjs <flow.json> [out.html] [--check] [--strict] [--template <file>] [--no-excerpts]
//
// Exit codes: 0 rendered (or --check passed), 1 warnings under --strict,
// 2 validation errors, 3 usage / I/O errors. The last stdout line is one
// JSON event (`flow.render` / `flow.check` / `flow.invalid`) so a caller
// can parse the outcome without scraping.

import { readFileSync, writeFileSync, statSync } from 'node:fs';
import { dirname, resolve, isAbsolute, join, extname, basename } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCHEMA_VERSION = 1;
const CONTEXT_LINES = 3;
const MAX_EXCERPT_LINES = 80;
const MAX_SOURCE_BYTES = 256 * 1024;
const MAX_FLOW_BYTES = 8 * 1024 * 1024;
const CARRIERS = new Set(['ui', 'ipc', 'rpc', 'http', 'fs', 'process', 'network', 'in_memory']);
const KINDS = new Set(['function', 'event', 'state']);
const EDGE_KINDS = new Set(['calls', 'handled_by', 'resolves']);
const STATUSES = new Set(['same', 'added', 'removed', 'modified']);
const LANGUAGES = {
  '.md': 'markdown', '.markdown': 'markdown', '.txt': 'text',
  '.js': 'javascript', '.mjs': 'javascript', '.cjs': 'javascript', '.ts': 'typescript', '.tsx': 'typescript', '.jsx': 'javascript',
  '.py': 'python', '.sh': 'shell', '.bash': 'shell', '.zsh': 'shell', '.ps1': 'powershell', '.psm1': 'powershell',
  '.rs': 'rust', '.go': 'go', '.rb': 'ruby', '.json': 'json', '.yaml': 'yaml', '.yml': 'yaml', '.toml': 'toml',
  '.html': 'html', '.css': 'css', '.cmd': 'shell', '.bat': 'shell',
};

function usage(msg) {
  if (msg) console.error(msg);
  console.error('usage: node render.mjs <flow.json> [out.html] [--check] [--strict] [--template <file>] [--no-excerpts]');
  process.exit(3);
}

const args = process.argv.slice(2);
const opts = { check: false, strict: false, excerpts: true, template: null, positional: [] };
for (let i = 0; i < args.length; i++) {
  const a = args[i];
  if (a === '--check') opts.check = true;
  else if (a === '--strict') opts.strict = true;
  else if (a === '--no-excerpts') opts.excerpts = false;
  else if (a === '--template') opts.template = args[++i];
  else if (a.startsWith('--')) usage(`unknown option ${a}`);
  else opts.positional.push(a);
}
if (opts.positional.length < 1) usage();
const flowPath = resolve(opts.positional[0]);
const outPath = resolve(opts.positional[1] || flowPath.replace(/(\.flow)?\.json$/i, '') + '.flow.html');
const here = dirname(fileURLToPath(import.meta.url));
const templatePath = opts.template ? resolve(opts.template) : join(here, '..', 'assets', 'template.html');

const emit = (event, fields) => console.log(JSON.stringify({ event, flow: basename(flowPath), ...fields }));

let flow;
try {
  if (statSync(flowPath).size > MAX_FLOW_BYTES) throw new Error(`flow file is over ${MAX_FLOW_BYTES} bytes`);
  flow = JSON.parse(readFileSync(flowPath, 'utf8'));
} catch (err) {
  emit('flow.invalid', { reason: 'invalid_json', detail: err.message });
  process.exit(2);
}

// ---- Validation (mirrors the Flow Explorer's model validation) --------------
const errors = [];
const warnings = [];
const validId = (id) => typeof id === 'string' && id.trim() !== '' && !/[;\r\n]/.test(id);
const safePath = (file) => {
  if (typeof file !== 'string' || file === '' || file.includes('\0') || file.startsWith('/')) return false;
  if (file[1] === ':') return false;
  return !file.split('/').includes('..');
};

if (!flow || typeof flow !== 'object') { emit('flow.invalid', { reason: 'not_an_object' }); process.exit(2); }
if (flow.schema_version !== SCHEMA_VERSION) errors.push(`unsupported schema_version ${flow.schema_version} (expected ${SCHEMA_VERSION})`);
if (typeof flow.error === 'string' && flow.error.trim()) errors.push(`producer error: ${flow.error}`);
if (typeof flow.title !== 'string' || !flow.title.trim()) errors.push('title is missing');
if (typeof flow.repo_root !== 'string' || !flow.repo_root) errors.push('repo_root is missing');
if (flow.mode != null && flow.mode !== 'explain' && flow.mode !== 'review') errors.push(`mode must be explain or review, got ${JSON.stringify(flow.mode)}`);
for (const key of ['processes', 'nodes', 'edges', 'entries']) if (!Array.isArray(flow[key])) errors.push(`${key} must be an array`);
if (errors.length) { for (const e of errors) console.error(`error: ${e}`); emit('flow.invalid', { reason: 'validation', errors: errors.length }); process.exit(2); }

const processIds = new Set();
for (const p of flow.processes) {
  if (!validId(p.id)) { errors.push(`invalid process id ${JSON.stringify(p.id)}`); continue; }
  if (processIds.has(p.id)) errors.push(`duplicate process id ${p.id}`);
  processIds.add(p.id);
  if (typeof p.label !== 'string' || !p.label) warnings.push(`process ${p.id} has no label`);
}
const nodeIds = new Set();
for (const n of flow.nodes) {
  if (!validId(n.id)) { errors.push(`invalid node id ${JSON.stringify(n.id)}`); continue; }
  if (nodeIds.has(n.id)) errors.push(`duplicate node id ${n.id}`);
  nodeIds.add(n.id);
  if (typeof n.name !== 'string' || !n.name.trim()) errors.push(`node ${n.id} has no name`);
  if (!KINDS.has(n.kind)) errors.push(`node ${n.id} has kind ${JSON.stringify(n.kind)}; expected function, event or state`);
  if (n.process != null && !processIds.has(n.process)) errors.push(`node ${n.id} references unknown process ${n.process}`);
  if (n.carrier != null && !CARRIERS.has(n.carrier)) errors.push(`node ${n.id} has carrier ${JSON.stringify(n.carrier)}; expected one of ${[...CARRIERS].join(', ')}`);
  if (n.status != null && !STATUSES.has(n.status)) errors.push(`node ${n.id} has status ${JSON.stringify(n.status)}`);
  if (n.location) {
    if (typeof n.location.file === 'string') n.location.file = n.location.file.replace(/\\/g, '/');
    if (!safePath(n.location.file)) errors.push(`node ${n.id} has an unsafe location path ${JSON.stringify(n.location.file)}`);
    if (!Number.isInteger(n.location.line) || n.location.line < 1) errors.push(`node ${n.id} location.line must be a 1-based integer`);
    if (n.location.end_line != null && (!Number.isInteger(n.location.end_line) || n.location.end_line < n.location.line)) errors.push(`node ${n.id} location.end_line must be >= line`);
  }
  if (n.kind === 'event' && !n.carrier) warnings.push(`event ${n.id} has no carrier`);
  if (n.kind === 'function' && n.process == null) warnings.push(`function ${n.id} has no process (it will land in the synthetic "outside" lane)`);
  if (!n.description || !String(n.description).trim()) warnings.push(`node ${n.id} has no description`);
}
for (const e of flow.edges) {
  for (const id of [e.from, e.to]) if (!nodeIds.has(id)) errors.push(`edge references unknown node ${id}`);
  if (!EDGE_KINDS.has(e.kind)) errors.push(`edge ${e.from} -> ${e.to} has kind ${JSON.stringify(e.kind)}; expected calls, handled_by or resolves`);
}
if (!flow.entries.length) errors.push('entries is empty');
for (const id of flow.entries) if (!nodeIds.has(id)) errors.push(`entries references unknown node ${id}`);

if (!errors.length) {
  // Reachability: a node no entry reaches never appears in any view.
  const reach = new Set();
  const stack = [...flow.entries];
  while (stack.length) {
    const id = stack.pop();
    if (reach.has(id)) continue;
    reach.add(id);
    for (const e of flow.edges) if (e.from === id) stack.push(e.to);
  }
  for (const n of flow.nodes) if (!reach.has(n.id)) warnings.push(`node ${n.id} is unreachable from entries (it will not be shown)`);
}

if (errors.length) {
  for (const e of errors) console.error(`error: ${e}`);
  emit('flow.invalid', { reason: 'validation', errors: errors.length, warnings: warnings.length });
  process.exit(2);
}

// ---- Excerpts ---------------------------------------------------------------
const repoRoot = isAbsolute(flow.repo_root) || /^[\\/]/.test(flow.repo_root) ? flow.repo_root : resolve(dirname(flowPath), flow.repo_root);
let excerpts = 0;
const sourceCache = new Map();
function readSource(file) {
  if (sourceCache.has(file)) return sourceCache.get(file);
  let result;
  try {
    const full = join(repoRoot, file);
    if (statSync(full).size > MAX_SOURCE_BYTES) result = { error: 'source over 256 KiB' };
    else result = { lines: readFileSync(full, 'utf8').split(/\r?\n/) };
  } catch (err) { result = { error: err.code === 'ENOENT' ? 'source not found' : err.message }; }
  sourceCache.set(file, result);
  return result;
}
if (opts.excerpts) {
  for (const n of flow.nodes) {
    if (!n.location) continue;
    const src = readSource(n.location.file);
    if (src.error) { warnings.push(`node ${n.id}: ${src.error} (${n.location.file})`); n.excerpt = null; continue; }
    const total = src.lines.length;
    const start = n.location.line;
    const end = Math.max(start, n.location.end_line || start);
    if (start > total) { warnings.push(`node ${n.id}: location.line ${start} is past the end of ${n.location.file} (${total} lines)`); n.excerpt = null; continue; }
    if (end > total) warnings.push(`node ${n.id}: location.end_line ${end} is past the end of ${n.location.file} (${total} lines)`);
    if (!src.lines[start - 1].trim()) warnings.push(`node ${n.id}: located line ${start} of ${n.location.file} is blank; the line number is probably off`);
    const first = Math.max(1, start - CONTEXT_LINES);
    let last = Math.min(total, end + CONTEXT_LINES);
    if (last - first + 1 > MAX_EXCERPT_LINES) { last = first + MAX_EXCERPT_LINES - 1; warnings.push(`node ${n.id}: excerpt truncated to ${MAX_EXCERPT_LINES} lines`); }
    n.excerpt = {
      file: n.location.file,
      language: LANGUAGES[extname(n.location.file).toLowerCase()] || 'text',
      first_line: first,
      lines: src.lines.slice(first - 1, last),
      highlight: [start, Math.min(end, last)],
    };
    excerpts++;
  }
}

for (const w of warnings) console.error(`warning: ${w}`);
const counts = { nodes: flow.nodes.length, edges: flow.edges.length, entries: flow.entries.length, processes: flow.processes.length, excerpts, warnings: warnings.length };

if (opts.check) {
  emit('flow.check', { ok: true, ...counts });
  process.exit(opts.strict && warnings.length ? 1 : 0);
}

// ---- Inject -----------------------------------------------------------------
let template;
try { template = readFileSync(templatePath, 'utf8'); } catch (err) { emit('flow.invalid', { reason: 'template_missing', detail: templatePath }); process.exit(3); }
if (!template.includes('__FLOW_JSON__')) { emit('flow.invalid', { reason: 'template_has_no_placeholder', detail: templatePath }); process.exit(3); }
const escapeHtml = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
// Inside <script type="application/json">, `</` must never appear verbatim;
// escaping every < > & in the JSON string keeps the document safe as HTML.
const json = JSON.stringify(flow).replace(/</g, '\\u003c').replace(/>/g, '\\u003e').replace(/&/g, '\\u0026').replace(/\u2028/g, '\\u2028').replace(/\u2029/g, '\\u2029');
const html = template.replace('__FLOW_TITLE__', escapeHtml(flow.title)).replace('__FLOW_JSON__', () => json);
try { writeFileSync(outPath, html, 'utf8'); } catch (err) { emit('flow.invalid', { reason: 'write_failed', detail: err.message }); process.exit(3); }
emit('flow.render', { ok: true, ...counts, bytes: Buffer.byteLength(html, 'utf8'), out: outPath });
process.exit(opts.strict && warnings.length ? 1 : 0);
