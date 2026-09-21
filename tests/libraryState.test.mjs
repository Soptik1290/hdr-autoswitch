import { test } from 'node:test';
import assert from 'node:assert/strict';
import { captureLibraryRow, findLibraryApp, findTrackedApp, libraryRowIdentity, pathAfterExecutableEdit, primaryExecutable } from '../src/libraryState.ts';
import { ConfigClient } from '../src/configState.ts';

const app = (overrides = {}) => ({
  name: 'Fixture game', exe_name: 'renderer.exe', enabled: true, hdr_type: 'native',
  path: 'D:\\Original\\renderer.exe', steam_id: '100', alternate_exes: [],
  ...overrides,
});

test('same title cannot match a different executable with a conflicting Steam ID', () => {
  const existing = app({ name: 'Shared title', exe_name: 'a.exe' });
  const incoming = app({ name: 'Shared title', exe_name: 'b.exe', steam_id: '200' });
  assert.equal(findLibraryApp([existing], incoming), undefined);
  assert.equal(findLibraryApp([incoming], existing), undefined);
  assert.equal(findTrackedApp([existing], incoming), undefined);
});

test('canonical library lookup is symmetric across primary and complete alias sets', () => {
  const existing = app({ name: 'User title', alternate_exes: ['other-renderer.exe'] });
  for (const incoming of [
    app({ name: 'Catalog title', exe_name: 'game.exe', alternate_exes: ['RENDERER.EXE'] }),
    app({ name: 'Catalog title', exe_name: 'game.exe', alternate_exes: ['OTHER-RENDERER.EXE'] }),
  ]) {
    assert.equal(findLibraryApp([existing], incoming), existing);
    assert.equal(findLibraryApp([incoming], existing), incoming);
    assert.equal(findTrackedApp([existing], incoming), existing);
  }
});

test('tracked state requires an enabled stored executable binding, not just a shared title', () => {
  const incoming = app({ exe_name: 'game.exe' });
  assert.equal(findTrackedApp([app()], incoming), undefined);
  assert.equal(findTrackedApp([app({ enabled: false, alternate_exes: ['game.exe'] })], incoming), undefined);
  const existing = app({ alternate_exes: ['GAME.EXE'] });
  assert.equal(findTrackedApp([existing], incoming), existing);
  assert.equal(findTrackedApp([existing], { ...incoming, steam_id: '200' }), undefined);
});

test('catalog add then remove uses the canonical primary returned by the committed library', async () => {
  const existing = app({ enabled: false });
  const catalog = app({ exe_name: 'game.exe', path: null });
  let current = {
    mode: 'ready', settings: { apps: [existing] }, store_id: 'fixture-store',
    revision: '1', context_token: 'fixture-context', library_generation: '1',
    control_epoch: '1', issue: null, controller_issue: null, candidates: [], config_path: 'fixture',
  };
  const requests = [];
  const client = new ConfigClient(async (command, args) => {
    if (command === 'get_config') return structuredClone(current);
    requests.push({ command, args });
    if (command === 'add_custom_app') {
      assert.equal(args.app.exe_name, 'game.exe');
      current.settings.apps = [{ ...existing, enabled: true, alternate_exes: ['game.exe'] }];
    } else if (command === 'remove_app') {
      assert.deepEqual(args.row, { index: 0, exe_name: 'renderer.exe', path: existing.path });
      assert.equal(args.expectedLibraryGeneration, current.library_generation);
      current.settings.apps.splice(args.row.index, 1);
    } else {
      throw new Error(`Unexpected mutation ${command}`);
    }
    current = { ...current, revision: String(BigInt(current.revision) + 1n),
      control_epoch: String(BigInt(current.control_epoch) + 1n),
      library_generation: String(BigInt(current.library_generation) + 1n) };
    return structuredClone(current);
  });
  await client.refresh();
  assert.equal(findTrackedApp(client.getView().snapshot.settings.apps, catalog), undefined);
  await client.mutate('add_custom_app', { app: catalog });
  const tracked = findTrackedApp(client.getView().snapshot.settings.apps, catalog);
  assert.equal(tracked.exe_name, 'renderer.exe');
  assert.equal(tracked.path, existing.path);
  assert.equal(tracked.steam_id, '100');
  await client.mutate('remove_app', {
    row: libraryRowIdentity(client.getView().snapshot.settings.apps, tracked),
  }, client.captureOrigin());
  assert.equal(client.getView().snapshot.settings.apps.length, 0);
  assert.deepEqual(requests.map((item) => item.command), ['add_custom_app', 'remove_app']);
});

test('nonauthoritative catalog lookup resolves an enabled alias to its canonical library row', () => {
  const existing = app({ alternate_exes: ['game.exe'] });
  const process = { exe_name: 'GAME.EXE', name: 'Different window title' };
  assert.equal(findTrackedApp([existing], process), existing);
  assert.equal(findTrackedApp([{ ...existing, enabled: false }], process), undefined);
});

test('title association is a manual library suggestion, not executable tracking authority', () => {
  const existing = app({ name: 'User title', exe_name: 'chosen.exe' });
  const suggestion = app({ name: 'User title', exe_name: 'different.exe' });
  assert.equal(findLibraryApp([existing], suggestion), existing);
  assert.equal(findTrackedApp([existing], suggestion), undefined);
  assert.equal(findTrackedApp([existing], { exe_name: 'chosen_dx12.exe' }), undefined);
});

test('exact-primary matching preserves the established canonical executable key', () => {
  const existing = app();
  const incoming = app({ exe_name: 'RENDERER.EXE', steam_id: '200' });
  assert.equal(findLibraryApp([existing], incoming), existing);
  assert.equal(findTrackedApp([existing], incoming), existing);
});

test('duplicate primary rows use an exact path or stay ambiguous, never first-match', () => {
  const first = app();
  const second = app({ path: 'E:\\Other\\renderer.exe' });
  assert.equal(findLibraryApp([first, second], second), second);
  assert.equal(findLibraryApp([second, first], first), first);
  assert.equal(findLibraryApp([first, second], { exe_name: 'renderer.exe' }), undefined);
  assert.equal(findTrackedApp([first, second], { exe_name: 'renderer.exe' }), undefined);
  assert.equal(findLibraryApp([first, { ...first }], first), undefined);
});

test('browse-edit-submit cannot retain a path bound to the previously picked executable', () => {
  const picked = { exe_name: 'game.exe', path: 'D:\\Games\\game.exe' };
  const edited = 'renderer';
  const submitted = { ...picked, exe_name: primaryExecutable(edited),
    path: pathAfterExecutableEdit(picked.path, edited) || undefined };
  assert.equal(submitted.exe_name, 'renderer.exe');
  assert.equal(submitted.path, undefined);
  assert.equal(pathAfterExecutableEdit(picked.path, 'GAME.EXE'), picked.path);
  assert.equal(pathAfterExecutableEdit('\\\\?\\D:\\Games\\.\\game.exe', 'game'), '\\\\?\\D:\\Games\\.\\game.exe');
  assert.equal(pathAfterExecutableEdit(picked.path, ''), '');
});

test('row command identity distinguishes same-path duplicates without persisted IDs', () => {
  const first = app();
  const second = { ...first };
  const rows = [first, second];
  assert.equal(libraryRowIdentity(rows, first).index, 0);
  assert.equal(libraryRowIdentity(rows, second).index, 1);
  assert.throws(() => libraryRowIdentity(rows, { ...first }), /row changed/);
});

test('rendered row identity cannot borrow the generation of a newer not-yet-rendered library', async () => {
  const rows = [app(), app()];
  const snapshot = { mode: 'ready', context_token: 'ctx', library_generation: '4',
    revision: '4', control_epoch: '4', settings: { apps: rows } };
  const client = new ConfigClient(async () => snapshot);
  await client.refresh();
  const action = captureLibraryRow(client, rows, rows[1]);
  assert.equal(action.row.index, 1);
  assert.equal(action.origin.libraryGeneration, '4');
  client.acceptEvent({ ...snapshot, revision: '5', control_epoch: '5', library_generation: '5',
    settings: { apps: [rows[1]] } });
  assert.throws(() => captureLibraryRow(client, rows, rows[0]), /library view changed/);
});
