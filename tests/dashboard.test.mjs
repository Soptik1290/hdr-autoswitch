import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

let server;
let Dashboard;
let I18nContext;
let dictionaries;

before(async () => {
  server = await createServer({
    server: { middlewareMode: true, hmr: false, watch: null },
    appType: 'custom',
  });
  ({ Dashboard } = await server.ssrLoadModule('/src/components/Dashboard.tsx'));
  ({ I18nContext, dictionaries } = await server.ssrLoadModule('/src/i18n.ts'));
});
after(async () => { await server?.close(); });

const monitor = (overrides = {}) => ({
  id: 'chosen', device_path: 'chosen', name: 'Fixture display',
  identity_status: 'ready', identity_error: null, is_selected: false,
  adapter_id_low: 1, adapter_id_high: 0, target_id: 1,
  is_hdr_supported: true, is_hdr_enabled: false, hdr_state_known: true,
  state_error: null, is_primary: true, ...overrides,
});
const status = (mode = 'sdr') => ({
  status_revision: '1', inventory_revision: '1', manual_revision: '0', manual_results: [],
  scope_hdr_state: mode, is_hdr_active: mode === 'hdr',
  manual_control: { status: 'available' }, switched_by_app: false,
  target_status: 'ready', warning: null, target_deferred: false,
  active_target: null, any_hdr_active: mode !== 'sdr', inventory_stale: false,
  uncertain_targets: [], operation_outcomes: [],
});
function render(overrides = {}, language = 'en') {
  return renderToStaticMarkup(React.createElement(I18nContext.Provider, {
    value: { lang: language, setLang() {}, t: dictionaries[language] },
  }, React.createElement(Dashboard, {
    status: status(), monitors: [monitor()], libraryCount: null,
    activityLogs: [], recentGames: [], onRefreshMonitors() {}, onManualToggle() {},
    onNavigateToApps() {}, onControlError() {}, onManualResult() {},
    captureManualOrigin(scope) { return { scope, request: { client_id: 'gui:test', sequence: '1' } }; },
    controlAvailable: true, isDark: true,
    ...overrides,
  })));
}
function button(html, label) {
  const found = html.match(/<button\b[\s\S]*?<\/button>/g)?.find((item) => item.includes(label));
  assert.ok(found, `Missing ${label}`);
  return found;
}

test('read-only dashboard renders manual controls without writable settings in both languages', () => {
  for (const language of ['en', 'cs']) {
    const t = dictionaries[language];
    const html = render({ status: { ...status(), target_status: 'automation_paused' } }, language);
    assert.doesNotMatch(button(html, t.configAllOn), /\bdisabled=/);
    assert.doesNotMatch(button(html, t.configAllOff), /\bdisabled=/);
    assert.ok(html.includes(t.configManualPolicy));
    assert.ok(!html.includes(t.recentAllLibrary(0)));
    assert.ok(!html.includes(t.displaysTargetHdr));
  }
});

test('blocked manual admission disables controls rather than hiding the dashboard', () => {
  const html = render({
    status: { ...status(), manual_control: { status: 'blocked', reason: 'Controller conflict' } },
    controlAvailable: false,
  });
  assert.match(button(html, dictionaries.en.configAllOn), /\bdisabled=/);
  assert.match(button(html, dictionaries.en.configAllOff), /\bdisabled=/);
  assert.match(html, /Fixture display/);
});

test('mixed and unknown scope render their own hero and logo states', () => {
  for (const mode of ['hdr', 'sdr', 'mixed', 'unknown']) {
    const html = render({ status: status(mode) });
    assert.ok(html.includes(`data-hdr-scope="${mode}"`));
    assert.ok(html.includes(`data-hdr-logo="${mode}"`));
    if (mode === 'mixed') assert.match(html, /border-amber-400/);
    if (mode === 'unknown') assert.match(html, /border-dashed/);
  }
});

test('unknown HDR query renders unavailable, never SDR-only or confirmed-off visuals', () => {
  const html = render({ monitors: [monitor({
    hdr_state_known: false, is_hdr_supported: false, state_error: 'Fixture query failure',
  })] });
  assert.ok(html.includes(dictionaries.en.displaysStateUnknown));
  assert.ok(!html.includes(dictionaries.en.displaysSdrOnly));
  assert.match(html, /data-monitor-state="unknown"/);
  assert.match(html, /Fixture query failure/);
  assert.match(button(html, dictionaries.en.configAllOn), /\bdisabled=/);
});

test('ambiguous identities disable both per-display actions even without an error string', () => {
  const html = render({ monitors: [monitor({ identity_status: 'ambiguous' })] });
  assert.match(button(html, dictionaries.en.heroTurnOnHdr), /\bdisabled=/);
  assert.match(button(html, dictionaries.en.heroTurnOffHdr), /\bdisabled=/);
});
