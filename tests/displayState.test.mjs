import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  DisplayObservationOrder, manualControlAvailable, manualScopeAvailable,
  monitorMode, monitorReady, scopeVisuals,
} from '../src/displayState.ts';
import { dictionaries } from '../src/i18n.ts';

const monitor = (overrides = {}) => ({
  id: 'display', device_path: 'display', name: 'Display',
  identity_status: 'ready', identity_error: null, is_selected: false,
  adapter_id_low: 1, adapter_id_high: 0, target_id: 1,
  is_hdr_supported: true, is_hdr_enabled: false, hdr_state_known: true,
  state_error: null, is_primary: true,
  ...overrides,
});

const status = (statusRevision, inventoryRevision, overrides = {}) => ({
  status_revision: String(statusRevision),
  inventory_revision: String(inventoryRevision),
  scope_hdr_state: 'sdr', any_hdr_active: false, inventory_stale: false, warning: null,
  ...overrides,
});

test('inventory-only changes refresh displays without aggregate HDR or activity changes', () => {
  const order = new DisplayObservationOrder();
  const initial = status(1, 1);
  assert.equal(order.acceptStatus(initial), true);
  assert.equal(order.needsInventory, true);
  assert.equal(order.acceptInventory({ inventory_revision: '1' }), true);
  assert.equal(order.needsInventory, false);
  for (const [index, monitors] of [
    [monitor(), monitor({ device_path: 'second' })],
    [monitor({ name: 'Renamed display' })],
    [monitor({ is_hdr_supported: false })],
    [],
  ].entries()) {
    const next = status(index + 2, index + 2);
    assert.equal(next.scope_hdr_state, initial.scope_hdr_state);
    assert.equal(next.any_hdr_active, initial.any_hdr_active);
    assert.equal(order.acceptStatus(next), true);
    assert.equal(order.needsInventory, true);
    assert.equal(order.acceptInventory({ inventory_revision: next.inventory_revision, monitors }), true);
    assert.equal(order.needsInventory, false);
    assert.equal(order.acceptStatus(next), true);
    assert.equal(order.needsInventory, false, 'identical observations must not refetch');
  }
});

test('newer events fence stale monitor responses, and newer monitor responses fence old status', () => {
  const order = new DisplayObservationOrder();
  assert.equal(order.acceptStatus(status(4, 2)), true);
  assert.equal(order.acceptInventory({ inventory_revision: '1' }), false);
  assert.equal(order.needsInventory, true);
  assert.equal(order.acceptInventory({ inventory_revision: '2' }), true);
  assert.equal(order.statusCurrent, true);
  assert.equal(order.acceptInventory({ inventory_revision: '3' }), true);
  assert.equal(order.statusCurrent, false, 'aggregate status is not yet current');
  assert.equal(order.acceptStatus(status(4, 2)), false);
  assert.equal(order.acceptStatus(status(5, 3)), true);
  assert.equal(order.statusCurrent, true);
  assert.equal(order.acceptInventory({ inventory_revision: '2' }), false);
});

test('inventory failure and recovery cannot be reversed by delayed responses', () => {
  const order = new DisplayObservationOrder();
  order.acceptStatus(status(1, 1));
  order.acceptInventory({ inventory_revision: '1' });
  const failed = status(2, 2, { inventory_stale: true, warning: 'Enumeration failed' });
  assert.equal(order.acceptStatus(failed), true);
  assert.equal(order.acceptInventory({ inventory_revision: '1' }), false);
  const recovered = status(3, 3);
  assert.equal(order.acceptStatus(recovered), true);
  assert.equal(order.needsInventory, true);
  assert.equal(order.acceptInventory({ inventory_revision: '3' }), true);
  assert.equal(order.acceptStatus(failed), false);
});

test('a failed inventory request can retry the same revision without losing ordering fences', () => {
  const order = new DisplayObservationOrder();
  order.acceptStatus(status(2, 2));
  order.acceptInventory({ inventory_revision: '2' });
  order.invalidateInventory();
  assert.equal(order.acceptStatus(status(2, 2)), true);
  assert.equal(order.needsInventory, true);
  assert.equal(order.acceptInventory({ inventory_revision: '1' }), false);
  assert.equal(order.acceptInventory({ inventory_revision: '2' }), true);
  assert.equal(order.needsInventory, false);
});

test('verified manual recovery retires the warning even with unchanged inventory', () => {
  const order = new DisplayObservationOrder();
  let displayed = null;
  const accept = (next) => {
    if (order.acceptStatus(next)) displayed = next;
  };
  const failed = status(5, 3, { warning: 'The last manual HDR request failed: gate closed' });
  accept(failed);
  order.acceptInventory({ inventory_revision: '3' });
  assert.equal(displayed.warning, failed.warning);
  accept(status(6, 3));
  assert.equal(displayed.warning, null);
  assert.equal(order.needsInventory, false);
  accept(failed);
  assert.equal(displayed.warning, null, 'a delayed failure must not reintroduce a recovered warning');
  accept(status(7, 3, { warning: 'Unresolved controller conflict' }));
  assert.equal(displayed.warning, 'Unresolved controller conflict');
});

test('revision ordering preserves decimal precision and rejects malformed revisions', () => {
  const order = new DisplayObservationOrder();
  assert.equal(order.acceptStatus(status('9007199254740993', '9007199254740993')), true);
  assert.equal(order.acceptStatus(status('9007199254740992', '9007199254740993')), false);
  assert.equal(order.acceptInventory({ inventory_revision: '9007199254740992' }), false);
  for (const value of ['', '-1', '1.5', '01', 'unknown']) {
    assert.equal(order.acceptStatus(status(value, '9007199254740993')), false);
    assert.equal(order.acceptInventory({ inventory_revision: value }), false);
  }
});

test('manual admission follows backend authority, not automatic readiness or saved settings', () => {
  for (const target_status of ['automation_paused', 'needs_confirmation', 'disconnected', 'ready']) {
    assert.equal(manualControlAvailable({
      manual_control: { status: 'available' }, target_status, inventory_stale: false,
    }, true), true);
  }
  for (const reason of ['controller conflict', 'shutting down', 'authority unavailable']) {
    assert.equal(manualControlAvailable({
      manual_control: { status: 'blocked', reason }, inventory_stale: false,
    }, true), false);
  }
  assert.equal(manualControlAvailable({
    manual_control: { status: 'available' }, inventory_stale: true,
  }, true), false);
  assert.equal(manualControlAvailable({
    manual_control: { status: 'available' }, inventory_stale: false,
  }, false), false);
});

test('target options and manual scopes share native identity and known-state readiness', () => {
  const scope = { kind: 'monitor', device_path: 'DISPLAY', display_name: 'Display' };
  assert.equal(manualScopeAvailable(scope, [monitor()]), true);
  for (const invalid of [
    { identity_status: 'ambiguous', identity_error: null },
    { identity_status: 'identity_unavailable' },
    { identity_error: 'duplicate path' },
    { hdr_state_known: false },
    { state_error: 'read failed' },
    { is_hdr_supported: false },
    { device_path: null },
    { device_path: '' },
  ]) {
    const unavailable = monitor(invalid);
    assert.equal(monitorReady(unavailable), false);
    assert.equal(manualScopeAvailable(scope, [unavailable]), false);
    assert.equal(manualScopeAvailable({ kind: 'all' }, [unavailable]), false);
  }
  assert.equal(manualScopeAvailable({ kind: 'needs_confirmation', legacy_runtime_id: 'old' }, [monitor()]), false);
  assert.equal(manualScopeAvailable({ kind: 'all' }, []), false);
});

test('unknown display state cannot be presented as confirmed SDR or HDR', () => {
  for (const enabled of [false, true]) {
    assert.equal(monitorMode(monitor({ is_hdr_enabled: enabled, hdr_state_known: false })), 'unknown');
    assert.equal(monitorMode(monitor({ is_hdr_enabled: enabled, state_error: 'query failed' })), 'unknown');
  }
  assert.equal(monitorMode(monitor()), 'sdr');
  assert.equal(monitorMode(monitor({ is_hdr_enabled: true })), 'hdr');
});

test('all four scope modes have distinct hero, dial, badge, and indicator styles', () => {
  for (const field of ['panel', 'dial', 'badge', 'dot']) {
    assert.equal(new Set(Object.values(scopeVisuals).map((style) => style[field])).size, 4);
  }
  assert.match(scopeVisuals.mixed.panel, /amber/);
  assert.match(scopeVisuals.unknown.panel, /dashed/);
});

test('English and Czech explain manual versus automatic consent and unknown display state', () => {
  assert.match(dictionaries.en.configManualPolicy, /never changes settings or automatic HDR consent/);
  assert.match(dictionaries.cs.configManualPolicy, /nemění nastavení ani souhlas/);
  assert.match(dictionaries.en.configAcceptNative, /AUTOMATIC/);
  assert.match(dictionaries.cs.configAcceptNative, /AUTOMATICKÉ/);
  for (const language of ['en', 'cs']) {
    assert.notEqual(dictionaries[language].displaysStateUnknown, dictionaries[language].displaysSdrOnly);
  }
});
