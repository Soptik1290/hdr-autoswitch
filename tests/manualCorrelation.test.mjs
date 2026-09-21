import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { ManualFeedbackOrder } from '../src/displayState.ts';

const all = { kind: 'all' };
const other = { kind: 'monitor', device_path: 'other', display_name: 'Other' };
const tray = (sequence) => ({ client_id: 'tray', sequence: String(sequence) });
const observed = (revision, scope, request, verified = true) => ({
  manual_revision: String(revision),
  manual_results: [{ revision: String(revision), scope, request, verified, error: null }],
});

function handler(feedback, invoke) {
  const source = readFileSync(new URL('../src/components/Dashboard.tsx', import.meta.url), 'utf8');
  const body = source.slice(source.indexOf('  const handleSet ='), source.indexOf('  const handleSetMonitor ='))
    .replace('scope: TargetMonitor, enable: boolean', 'scope, enable')
    .replace('invoke<ManualSetResult>', 'invoke');
  const env = {
    manualPending: { current: false }, captureManualOrigin: (scope) => feedback.capture(scope),
    controlAvailable: true, manualScopeAvailable: () => true, monitors: [],
    status: { manual_control: { status: 'available' } }, t: {}, setToggling() {}, invoke,
    onManualResult: (result) => feedback.acceptStatus(result.status),
    onControlError: (error) => feedback.acceptError(error), onManualToggle() {},
  };
  return new Function(...Object.keys(env), `${body}\nreturn handleSet;`)(...Object.values(env));
}

test('actual GUI pre-admission failure survives a missed older tray success', async () => {
  const feedback = new ManualFeedbackOrder('gui:test');
  await handler(feedback, async () => { throw new Error('New GUI Off never submitted'); })(all, false);
  feedback.acceptStatus(observed(1, all, tray(1)));
  assert.deepEqual(feedback.errors(), ['Error: New GUI Off never submitted']);
});

test('actual GUI failure survives an earlier in-flight tray completion', async () => {
  const feedback = new ManualFeedbackOrder('gui:test');
  let reject;
  const response = new Promise((_, no) => { reject = no; });
  const pending = handler(feedback, () => response)(all, false);
  reject(new Error('GUI submission failed'));
  await pending;
  feedback.acceptStatus(observed(1, all, tray(1)));
  feedback.acceptStatus(observed(2, other, tray(2)));
  assert.deepEqual(feedback.errors(), ['Error: GUI submission failed']);
});

test('two transport failures sharing an observed actor revision follow client request order', () => {
  const feedback = new ManualFeedbackOrder('gui:test');
  const first = feedback.capture(all);
  const second = feedback.capture(all);
  feedback.acceptError({ ...second, message: 'Newer submission failed' });
  feedback.acceptError({ ...first, message: 'Late older submission failed' });
  assert.deepEqual(feedback.errors(), ['Newer submission failed']);
});

test('actual GUI retry sends a new correlation identity and reconciles an unsubmitted request', async () => {
  const feedback = new ManualFeedbackOrder('gui:test');
  const requests = [];
  await handler(feedback, async (_, args) => {
    requests.push(args.request);
    throw new Error('First submission never arrived');
  })(all, false);
  assert.equal(feedback.errors().length, 1);
  await handler(feedback, async (_, args) => {
    requests.push(args.request);
    return { scope: args.scope, request: args.request, status: observed(1, args.scope, args.request) };
  })(all, false);
  assert.deepEqual(requests, [
    { client_id: 'gui:test', sequence: '1' },
    { client_id: 'gui:test', sequence: '2' },
  ]);
  assert.deepEqual(feedback.errors(), []);
});

test('matching GUI result retires only its request even when a newer unrelated status was seen first', () => {
  const feedback = new ManualFeedbackOrder('gui:test');
  const submitted = feedback.capture(all);
  feedback.acceptError({ ...submitted, message: 'Reply was lost' });
  feedback.acceptStatus(observed(9, other, tray(2)));
  assert.equal(feedback.errors().length, 1);
  assert.equal(feedback.acceptStatus(observed(8, all, submitted.request)), true);
  assert.deepEqual(feedback.errors(), []);
  assert.equal(feedback.acceptStatus(observed(8, all, submitted.request)), false);
  assert.equal(feedback.acceptError({ ...submitted, message: 'Late rejected reply' }), false);
});

test('older matching success cannot erase a newer unadmitted GUI failure', () => {
  const feedback = new ManualFeedbackOrder('gui:test');
  const older = feedback.capture(all);
  const newer = feedback.capture(all);
  feedback.acceptError({ ...newer, message: 'Newer failed before admission' });
  feedback.acceptStatus(observed(1, all, older.request));
  assert.deepEqual(feedback.errors(), ['Newer failed before admission']);
  feedback.acceptError({ ...older, message: 'Older failure delivered late' });
  assert.deepEqual(feedback.errors(), ['Newer failed before admission']);
});

test('same-client different-scope completion and different-client same-scope completion stay independent', () => {
  const feedback = new ManualFeedbackOrder('gui:test');
  const failed = feedback.capture(all);
  feedback.acceptError({ ...failed, message: 'GUI All was not admitted' });
  feedback.acceptError({ scope: all, request: tray(1), message: 'Tray All was not admitted' });
  const otherRequest = feedback.capture(other);
  feedback.acceptStatus(observed(1, other, otherRequest.request));
  assert.equal(feedback.errors().length, 2);
  feedback.acceptStatus(observed(2, all, tray(2)));
  assert.deepEqual(feedback.errors(), ['GUI All was not admitted']);
  const retry = feedback.capture(all);
  feedback.acceptStatus(observed(3, all, retry.request));
  assert.deepEqual(feedback.errors(), []);
});

test('missed GUI completion remains reconcilable from a later multi-origin snapshot', () => {
  const feedback = new ManualFeedbackOrder('gui:test');
  const submitted = feedback.capture(all);
  feedback.acceptError({ ...submitted, message: 'Lost GUI reply' });
  const guiResult = observed(1, all, submitted.request).manual_results[0];
  const trayResult = observed(2, all, tray(1)).manual_results[0];
  assert.equal(feedback.acceptStatus({
    manual_revision: '2', manual_results: [guiResult, trayResult],
  }), true);
  assert.deepEqual(feedback.errors(), []);
});

test('malformed or uncorrelated feedback cannot clear or overwrite a real submission failure', () => {
  const feedback = new ManualFeedbackOrder('gui:test');
  const origin = feedback.capture(all);
  feedback.acceptError({ ...origin, message: 'Still unconfirmed' });
  feedback.acceptStatus(observed(99, all, undefined));
  for (const sequence of ['0', '01', '-1', '18446744073709551616']) {
    assert.equal(feedback.acceptError({
      ...origin, request: { ...origin.request, sequence }, message: 'Malformed',
    }), false);
  }
  for (const request of [
    { sequence: '2' }, { client_id: undefined, sequence: '2' },
    { client_id: 'gui:test', sequence: 2 }, { client_id: 'gui:bad\nid', sequence: '2' },
  ]) {
    assert.equal(feedback.acceptError({ scope: all, request, message: 'Malformed' }), false);
  }
  assert.deepEqual(feedback.errors(), ['Still unconfirmed']);
});

test('production GUI, actor and tray paths transmit correlation rather than observed-revision inference', () => {
  const source = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');
  assert.match(source('src/components/Dashboard.tsx'), /request: origin\.request/);
  const app = source('src/App.tsx');
  assert.ok(app.indexOf('manualFeedback.current.acceptStatus(next)') < app.indexOf('displayOrder.current.acceptStatus(next)'));
  const actor = source('src-tauri/src/monitor_hook.rs');
  assert.match(actor, /Command::ManualSet\(scope, enable, request, reply\)/);
  const execution = actor.slice(actor.indexOf('    fn manual_set('), actor.indexOf('    fn execute_manual_set('));
  assert.ok(execution.indexOf('manual_observations.admit') < execution.indexOf('execute_manual_set'));
  assert.match(source('src-tauri/src/tray.rs'), /client_id: "tray".into\(\), sequence: sequence.to_string\(\)/);
  for (const path of ['src/displayState.ts', 'src/types.ts', 'src-tauri/src/tray.rs']) {
    assert.doesNotMatch(source(path), /after_revision/);
  }
});
