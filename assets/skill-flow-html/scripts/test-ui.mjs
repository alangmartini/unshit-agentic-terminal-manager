// Run: node scripts/test-ui.mjs [path/to/playwright/index.mjs]
// Requires Playwright and its Chromium browser; never runs the modeled skill.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, resolve, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const here = dirname(fileURLToPath(import.meta.url));
const { chromium } = await import(process.argv[2] ? pathToFileURL(resolve(process.argv[2])).href : 'playwright');
const temp = mkdtempSync(join(tmpdir(), 'skill-flow-ui-'));
const flow = JSON.parse(readFileSync(resolve(here, '../assets/example/create-worktree.flow.json'), 'utf8'));
flow.repo_root = resolve(here, '../assets/example/create-worktree');
writeFileSync(join(temp, 'example.flow.json'), JSON.stringify(flow));
execFileSync(process.execPath, [join(here, 'render.mjs'), join(temp, 'example.flow.json'), '--strict']);
const browser = await chromium.launch();
try {
  for (const width of [1440, 1100]) {
    const page = await browser.newPage({ viewport: { width, height: 900 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.goto(pathToFileURL(join(temp, 'example.flow.html')).href);
    await page.evaluate(() => document.fonts.ready);
    assert.equal(await page.locator('.flow-stage-card').count(), 5);
    await page.locator('[data-node-id="SKILL.md::step-2-slug"]').click();
    assert.equal(await page.locator('.flow-panes').count(), 0);
    assert.equal(await page.locator('[data-node-id="SKILL.md::step-3-worktree-add"]').count(), 0);
    assert.equal(await page.locator('.flow-graph-node').evaluateAll(ns => ns.some(n => n.scrollHeight > n.clientHeight + 3)), false);
    await page.locator('[data-node-id="decision.name-supplied"]').click();
    assert.equal(await page.locator('.flow-stage-card').count(), 2);
    await page.keyboard.press('Escape');
    await page.locator('[data-node-id="SKILL.md::step-2-slug"]').click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Source', exact: true }).click();
    await page.locator('dialog[open] .flow-snippet').waitFor();
    await page.keyboard.press('Escape');
    assert.equal(await page.locator('dialog[open]').count(), 0);
    const scroll = page.locator('.flow-graph-scroll');
    const previous = await scroll.evaluate(e => e.scrollTop);
    await page.locator('.flow-connections summary').click();
    assert.equal(await scroll.evaluate(e => e.scrollTop), previous);
    await page.getByRole('button', { name: 'panes', exact: true }).click();
    assert.equal(await page.locator('.flow-panes .flow-snippet').count(), 0);
    await page.getByRole('button', { name: 'Whole graph', exact: true }).click();
    assert.equal(await page.locator('.flow-graph-canvas').evaluate(e => getComputedStyle(e).transform), 'matrix(1, 0, 0, 1, 0, 0)');
    await page.getByRole('button', { name: 'Fit width', exact: true }).click();
    assert.equal(await scroll.evaluate(e => e.scrollWidth > e.clientWidth + 2), false);
    assert.deepEqual(errors, []);
    await page.close();
  }
  console.log('Flow UI regression checks passed at 1440px and 1100px.');
} finally {
  await browser.close();
  rmSync(temp, { recursive: true, force: true });
}
