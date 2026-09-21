import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import * as display from '../src/displayState.ts';

const all = { kind: 'all' };
const other = { kind: 'monitor', device_path: 'other', display_name: 'Other' };
const outcome = (revision, scope = all, verified = true) => ({
  revision: String(revision), scope, verified, error: null,
  request: { client_id: 'gui:test', sequence: String(revision) },
});
const status = (revision, results, warning = null) => ({
  status_revision: String(revision), inventory_revision: '1',
  manual_revision: String(revision), manual_results: results, warning,
});

function harness() {
  const feedback = new display.ManualFeedbackOrder('gui:test');
  const order = new display.DisplayObservationOrder();
  let current;
  return {
    feedback,
    accept(next) {
      feedback.acceptStatus(next);
      if (!order.acceptStatus(next)) return false;
      current = next;
      return true;
    },
    get warning() { return current?.warning; },
  };
}

test('late failure cannot resurrect an error after a correlated later same-client retry', () => {
  const h = harness();
  h.accept(status(0, []));
  const old = h.feedback.capture(all);
  h.feedback.acceptError({ ...old, message: 'tray failure' });
  assert.deepEqual(h.feedback.errors(), ['tray failure']);
  h.accept(status(2, [outcome(2)]));
  assert.deepEqual(h.feedback.errors(), []);
  assert.equal(h.feedback.acceptError({ ...old, message: 'late tray failure' }), false);
  assert.equal(h.accept(status(1, [outcome(1, all, false)], 'failed native On')), false);
  assert.equal(h.warning, null);
});

test('late GUI success cannot clear a newer tray failure or unrelated controller conditions', () => {
  const h = harness();
  h.accept(status(4, [outcome(4, all, false)], 'tray failed; unresolved controller conflict'));
  assert.equal(h.accept(status(3, [outcome(3)])), false);
  assert.equal(h.warning, 'tray failed; unresolved controller conflict');
  h.accept(status(5, [outcome(5)], 'unresolved controller conflict; HDR may have changed'));
  assert.equal(h.warning, 'unresolved controller conflict; HDR may have changed');
});

test('different-scope recovery retains transport errors and a full later snapshot retires only its scope', () => {
  const h = harness();
  h.accept(status(0, []));
  h.feedback.acceptError({ ...h.feedback.capture(all), message: 'All failed to submit' });
  h.feedback.acceptError({ ...h.feedback.capture(other), message: 'Other failed to submit' });
  h.accept(status(2, [outcome(2, other)]));
  assert.deepEqual(h.feedback.errors(), ['All failed to submit']);
  h.accept(status(4, [outcome(3), outcome(4, other, false)], 'Other native request failed'));
  assert.deepEqual(h.feedback.errors(), []);
  assert.equal(h.warning, 'Other native request failed');
});

test('same-scope newer failure replaces obsolete transport feedback without erasing the canonical failure', () => {
  const h = harness();
  h.accept(status(1, [outcome(1)]));
  const origin = h.feedback.capture(all);
  h.feedback.acceptError({ ...origin, message: 'reply unavailable' });
  h.accept(status(2, [outcome(2, all, false)], 'verified actor failure'));
  assert.deepEqual(h.feedback.errors(), []);
  assert.equal(h.warning, 'verified actor failure');
  assert.equal(h.feedback.acceptError({ ...origin, message: 'late reply unavailable' }), false);
});

test('duplicate reports and status event/reply delivery do not spam or clear a newer local error', () => {
  const h = harness();
  const next = status(8, [outcome(8)]);
  h.accept(next);
  const error = { ...h.feedback.capture(all), message: 'submission failed' };
  assert.equal(h.feedback.acceptError(error), true);
  assert.equal(h.feedback.acceptError(error), false);
  h.accept(next);
  assert.deepEqual(h.feedback.errors(), ['submission failed']);
  h.accept(status(9, [outcome(9)]));
  h.accept(status(9, [outcome(9)]));
  assert.deepEqual(h.feedback.errors(), []);
});

test('all manual result surfaces route actor status instead of assigning an unversioned error', () => {
  const source = (file) => readFileSync(new URL(`../${file}`, import.meta.url), 'utf8');
  const dashboard = source('src/components/Dashboard.tsx');
  assert.match(dashboard, /onManualResult\(result\)/);
  assert.doesNotMatch(dashboard, /onControlError\(null\)|onControlError\(String\(err\)\)/);
  const app = source('src/App.tsx');
  assert.match(app, /manualFeedback\.current\.acceptStatus\(next\)/);
  assert.doesNotMatch(app, /listen<string \| null>\('controller-error'/);
  assert.match(app, /listen<ManualSetResult>\('manual-control-result'/);
  assert.match(source('src-tauri/src/tray.rs'), /handle\.emit\("manual-control-result", result\)/);
});

function actualDashboardHandler(h, invoke) {
  const ref = process.env.MANUAL_FEEDBACK_DASHBOARD_REF;
  const source = ref
    ? execFileSync('git', ['show', `${ref}:src/components/Dashboard.tsx`], { encoding: 'utf8' })
    : readFileSync(new URL('../src/components/Dashboard.tsx', import.meta.url), 'utf8');
  const handler = source.slice(source.indexOf('  const handleSet ='), source.indexOf('  const handleSetMonitor ='))
    .replace('scope: TargetMonitor, enable: boolean', 'scope, enable')
    .replace('invoke<ManualSetResult>', 'invoke');
  const environment = {
    manualPending: { current: false },
    captureManualOrigin: (scope) => h.feedback.capture(scope),
    controlAvailable: true, manualScopeAvailable: () => true, monitors: [],
    status: { manual_control: { status: 'available' } },
    t: { configManualUnavailable: 'Unavailable' }, setToggling() {}, invoke,
    onManualResult: (result) => h.accept(result.status),
    onControlError: (error) => {
      // Exercise the previous unversioned callback contract during baseline-red runs.
      if (error === null || typeof error === 'string') h.legacyError = error;
      else h.feedback.acceptError(error);
    },
    onManualToggle() {},
  };
  return new Function(...Object.keys(environment), `${handler}\nreturn handleSet;`)(...Object.values(environment));
}

test('actual GUI handler rejects a late transport error after its matching result was observed', async () => {
  const h = harness();
  h.accept(status(0, []));
  let rejectReply;
  const reply = new Promise((_, reject) => { rejectReply = reject; });
  const pending = actualDashboardHandler(h, () => reply)(all, true);
  h.accept(status(2, [outcome(2)]));
  rejectReply(new Error('Late GUI reply failed'));
  await pending;
  assert.deepEqual(h.feedback.errors(), []);
  assert.equal(h.legacyError ?? null, null);
  assert.equal(h.warning, null);
});

test('actual GUI handler cannot clear newer tray failure with an older successful reply', async () => {
  const h = harness();
  h.accept(status(0, []));
  let resolveReply;
  const reply = new Promise((resolve) => { resolveReply = resolve; });
  const pending = actualDashboardHandler(h, () => reply)(all, false);
  h.accept(status(2, [outcome(2, all, false)], 'Tray On failed'));
  resolveReply({ scope: all, outcomes: [], partial: false, status: status(1, [outcome(1)]) });
  await pending;
  assert.equal(h.warning, 'Tray On failed');
  assert.deepEqual(h.feedback.errors(), []);
});

test('actual GUI handler routes late partial failure through status ordering, not the local error channel', async () => {
  const h = harness();
  h.accept(status(0, []));
  let resolveReply;
  const reply = new Promise((resolve) => { resolveReply = resolve; });
  const pending = actualDashboardHandler(h, () => reply)(all, true);
  h.accept(status(2, [outcome(2)]));
  resolveReply({
    scope: all, outcomes: [{ outcome: 'failed', message: 'Late native failure' }], partial: true,
    status: status(1, [outcome(1, all, false)], 'Late native failure'),
  });
  await pending;
  assert.equal(h.warning, null);
  assert.deepEqual(h.feedback.errors(), []);
  assert.equal(h.legacyError ?? null, null);
});

test('manual feedback preserves large revisions and normalized monitor identity', () => {
  const h = harness();
  h.accept(status('9007199254740993', [outcome('9007199254740993', other)]));
  assert.equal(h.feedback.acceptError({
    scope: { ...other, device_path: 'OTHER', display_name: 'Renamed' },
    request: { client_id: 'gui:test', sequence: '9007199254740992' }, message: 'Old failure',
  }), false);
  const origin = h.feedback.capture(other);
  assert.equal(origin.request.sequence, '9007199254740994');
  for (const invalid of ['', '-1', '1.5', '01']) {
    assert.equal(h.feedback.acceptError({ ...origin, request: { ...origin.request, sequence: invalid }, message: 'Invalid' }), false);
  }
});
