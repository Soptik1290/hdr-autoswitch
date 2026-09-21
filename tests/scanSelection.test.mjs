import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { ConfigClient } from '../src/configState.ts';
import * as scanSelection from '../src/scanSelection.ts';
const {
  detectionKey, importSelectedDetections, selectDetections, selectedDetections, toggleDetection,
} = scanSelection;

const game = (launcher, path, isHdrSupported = true, defaultSelected = isHdrSupported) => ({
  name: 'Shared game', exe_name: 'game.exe', path, launcher, hdr_type: isHdrSupported ? 'native' : 'custom',
  alternate_exes: [], steam_id: null,
  is_hdr_supported: isHdrSupported, default_selected: defaultSelected,
  evidence: isHdrSupported ? { status: 'verified' } : { status: 'unverified', reason: 'unresolved' },
});
const steam = game('Steam', 'C:\\SteamLibrary\\game.exe');
const gog = game('GOG', 'D:\\GOG\\game.exe');
const steamApp = {
  name: steam.name, exe_name: steam.exe_name, path: steam.path, launcher: steam.launcher,
  hdr_type: steam.hdr_type, alternate_exes: [], steam_id: null, enabled: true,
};
const gogApp = { ...steamApp, path: gog.path, launcher: gog.launcher };

test('same basename detections have independent provider/path identity, not vector-index identity', () => {
  assert.notEqual(detectionKey(steam), detectionKey(gog));
  assert.notEqual(detectionKey(steam), detectionKey({ ...steam, path: 'E:\\SteamLibrary\\game.exe' }));
  assert.notEqual(detectionKey(steam), detectionKey({ ...steam, launcher: 'GOG' }));
  assert.equal(detectionKey(steam), detectionKey({
    ...steam, exe_name: 'GAME.EXE', launcher: 'STEAM', path: 'c:/steamlibrary/GAME.exe',
  }));
  assert.equal(detectionKey(steam), detectionKey({ ...steam, name: 'A changed display title' }));
  assert.equal(detectionKey(steam), detectionKey({ ...steam, path: '\\\\?\\C:\\SteamLibrary\\game.exe' }));
});

test('actual scan selection handlers keep initial, HDR/all, individual toggle and submission in agreement', () => {
  const games = [steam, { ...gog, default_selected: false }];
  const initial = selectDetections(games, (item) => item.default_selected);
  assert.deepEqual(selectedDetections(games, initial), [steamApp]);
  const both = toggleDetection(initial, gog);
  assert.deepEqual(selectedDetections(games, both), [steamApp, gogApp]);
  const onlyGog = toggleDetection(both, steam);
  assert.deepEqual(selectedDetections(games, onlyGog), [gogApp]);
  assert.deepEqual(selectedDetections([...games].reverse(), onlyGog), [gogApp]);
  assert.deepEqual(selectedDetections(games, selectDetections(games, () => true)), [steamApp, gogApp]);
  assert.deepEqual(selectedDetections(games, {}), []);
});

test('auto-detect off changes default selection without losing verified HDR classification', () => {
  const verified = game('Steam', steam.path, true, false);
  const unverified = game('GOG', gog.path, false, false);
  const games = [verified, unverified];
  const before = structuredClone(games);
  assert.deepEqual(selectedDetections(games, selectDetections(games)), []);
  const hdrOnly = selectDetections(games, (item) => item.is_hdr_supported);
  assert.deepEqual(selectedDetections(games, hdrOnly), [steamApp]);
  const manual = toggleDetection(hdrOnly, unverified);
  assert.deepEqual(selectedDetections(games, manual), [steamApp, { ...gogApp, hdr_type: 'custom' }]);
  assert.deepEqual(games, before);
  assert.equal(verified.is_hdr_supported, true);
  assert.equal(verified.default_selected, false);
  assert.deepEqual(unverified.evidence, { status: 'unverified', reason: 'unresolved' });
});

test('explicit scan conversion writes only library fields and never persists scan state', () => {
  assert.equal(typeof scanSelection.fromScanGame, 'function');
  const detected = {
    ...steam, alternate_exes: ['game_dx12.exe'], default_selected: false, enabled: false,
  };
  const before = structuredClone(detected);
  const imported = scanSelection.fromScanGame(detected);
  assert.deepEqual(imported, { ...steamApp, alternate_exes: ['game_dx12.exe'] });
  imported.alternate_exes.push('changed.exe');
  assert.deepEqual(detected, before);
  assert.equal(Object.hasOwn(imported, 'evidence'), false);
  assert.equal(Object.hasOwn(imported, 'is_hdr_supported'), false);
  assert.equal(Object.hasOwn(imported, 'default_selected'), false);
});

test('real import handler submits only the individually selected installation with original scan fences', async () => {
  let snapshot = {
    mode: 'ready', context_token: 'ctx', library_generation: '7', revision: '2', control_epoch: '2',
    store_id: 'store', settings: { apps: [] },
  };
  const requests = [];
  const client = new ConfigClient(async (command, args) => {
    if (command === 'get_config') return structuredClone(snapshot);
    requests.push({ command, args });
    snapshot = { ...snapshot, revision: '3', control_epoch: '3', library_generation: '8',
      settings: { apps: args.detected } };
    return structuredClone(snapshot);
  });
  await client.refresh();
  const origin = client.captureOrigin();
  const selected = toggleDetection({}, gog);
  assert.equal(await importSelectedDetections(client, [steam, gog], selected, origin), 1);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].command, 'import_detected_games');
  assert.deepEqual(requests[0].args.detected, [gogApp]);
  assert.equal(requests[0].args.expectedContext, 'ctx');
  assert.equal(requests[0].args.expectedLibraryGeneration, '7');
  assert.deepEqual(client.getView().snapshot.settings.apps, [gogApp]);
});

test('auto-detect off does not import anything until an explicit selection is made', async () => {
  const games = [{ ...steam, default_selected: false }, { ...gog, default_selected: false }];
  const requests = [];
  const client = { mutate: async (...args) => requests.push(args) };
  const origin = { context_token: 'ctx', library_generation: '7' };
  const defaults = selectDetections(games);
  assert.equal(await importSelectedDetections(client, games, defaults, origin), 0);
  assert.deepEqual(requests, []);
  assert.equal(await importSelectedDetections(
    client, games, toggleDetection(defaults, gog), origin,
  ), 1);
  assert.deepEqual(requests, [['import_detected_games', { detected: [gogApp] }, origin]]);
  assert.equal(games[1].default_selected, false);
  assert.equal(Object.hasOwn(games[1], 'enabled'), false);
});

test('real import handler leaves selection and settings intact when a conflicting batch is rejected', async () => {
  const snapshot = { mode: 'ready', context_token: 'ctx', library_generation: '7', revision: '2',
    control_epoch: '2', store_id: 'store', settings: { apps: [steamApp] } };
  let sent;
  const client = new ConfigClient(async (command, args) => {
    if (command === 'get_config') return structuredClone(snapshot);
    sent = args.detected;
    throw new Error('Select only one installation for game.exe.');
  });
  await client.refresh();
  const selected = selectDetections([steam, gog], () => true);
  await assert.rejects(importSelectedDetections(
    client, [steam, gog], selected, client.captureOrigin(),
  ), /Select only one installation/);
  assert.deepEqual(sent, [steamApp, gogApp]);
  assert.deepEqual(selectedDetections([steam, gog], selected), [steamApp, gogApp]);
  assert.deepEqual(client.getView().snapshot.settings.apps, [steamApp]);
});

test('both rendered scan sections and confirmation use the tested detection identity handlers', () => {
  const component = readFileSync(new URL('../src/components/AppsManager.tsx', import.meta.url), 'utf8');
  assert.equal((component.match(/key=\{detectionKey\(game\)\}/g) ?? []).length, 2);
  assert.equal((component.match(/toggleDetection\(prev, game\)/g) ?? []).length, 2);
  assert.doesNotMatch(component, /selectedToImport\[(?:game|g)\.exe_name\]/);
  assert.match(component, /importSelectedDetections\(configClient, scannedGames, selectedToImport, scanOrigin\)/);
});
