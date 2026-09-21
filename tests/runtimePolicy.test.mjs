import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dictionaries } from '../src/i18n.ts';
import { statusWarnings } from '../src/telemetryText.ts';
import { ConfigClient } from '../src/configState.ts';

const source = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

test('quarantine warning identifies the row, preserves settings, and explains explicit repair in both languages', () => {
  const status = { warning: 'Display diagnostic', quarantined_apps: [
    { name: 'My Age of Empires IV', exe_name: 'BsSndRpt64.exe' },
  ] };
  for (const language of ['en', 'cs']) {
    const warnings = statusWarnings(status, dictionaries[language]);
    assert.equal(warnings.length, 2);
    assert.equal(warnings[0], 'Display diagnostic');
    assert.match(warnings[1], /My Age of Empires IV/);
    assert.match(warnings[1], /BsSndRpt64\.exe/);
    assert.deepEqual(statusWarnings(status, dictionaries[language]), warnings);
  }
  const english = statusWarnings(status, dictionaries.en)[1];
  assert.match(english, /Automatic matching.*blocked/);
  assert.match(english, /settings remain unchanged/);
  assert.match(english, /actual game executable/);
  assert.match(english, /removes historical executable aliases/);
  assert.notEqual(english, statusWarnings(status, dictionaries.cs)[1]);
  assert.deepEqual(statusWarnings({ ...status, quarantined_apps: [] }, dictionaries.en), ['Display diagnostic']);
});

test('status and tray consume derived quarantine without persisted schema or a new dashboard', () => {
  const app = source('src/App.tsx');
  assert.match(app, /statusWarnings\(status, t\)\.map/);
  assert.match(app, /role="alert"/);
  const runtime = source('src-tauri/src/runtime_policy.rs');
  assert.match(runtime, /pub fn quarantined_apps\(config: &AppConfig\)/);
  assert.doesNotMatch(runtime, /mutate|config_committed|emit\(/);
  const config = source('src-tauri/src/config.rs');
  const persisted = config.slice(config.indexOf('pub struct HdrApp'), config.indexOf('impl Default for AppConfig'));
  assert.doesNotMatch(persisted, /quarantin/);
  assert.match(source('src-tauri/src/tray.rs'), /!status.quarantined_apps.is_empty\(\)/);
});

test('primary/path repair warning is distinct, actionable and identifies the affected row in both languages', () => {
  const status = { warning: null, quarantined_apps: [{
    row_index: 2, name: 'Custom game', exe_name: 'game.exe', path: 'D:\\Game\\renderer.exe',
    reason: 'primary_path_mismatch',
  }] };
  for (const language of ['en', 'cs']) {
    const warnings = statusWarnings(status, dictionaries[language]);
    assert.equal(warnings.length, 1);
    assert.match(warnings[0], /Custom game/);
    assert.match(warnings[0], /game.exe/);
    assert.notEqual(warnings[0], dictionaries[language].quarantineWarning('Custom game', 'game.exe'));
  }
  assert.match(statusWarnings(status, dictionaries.en)[0], /path does not match/);
  assert.match(statusWarnings(status, dictionaries.en)[0], /settings remain unchanged/);
});

test('library mutation IPC requires a generation fence and never mutates by executable alone', () => {
  const commands = source('src-tauri/src/commands.rs');
  for (const name of ['remove_app', 'toggle_app', 'repair_app_executable']) {
    const body = commands.slice(commands.indexOf(`pub fn ${name}(`)).split('#[tauri::command]')[0];
    assert.match(body, /row: library::AppRowIdentity/);
    assert.match(body, /expected_library_generation: String/);
    assert.match(body, /Some\(&expected_library_generation\)/);
    assert.doesNotMatch(body, /\.retain\(|\.find\(/);
  }
});

test('manual catalog synchronization respects the native safe-test and shutdown gates', () => {
  const commands = source('src-tauri/src/commands.rs');
  const body = commands.slice(commands.indexOf('pub async fn sync_database(')).split('#[tauri::command]')[0];
  assert.match(body, /state.ensure_admission\(\)\?/);
  assert.match(body, /if state.safe_test_mode[\s\S]*?return Err/);
  assert.ok(body.indexOf('if state.safe_test_mode') < body.indexOf('database::fetch_online_database()'));
});

test('manual repair reuses picker, reports helper selection errors and fences library generation', async () => {
  const component = source('src/components/AppsManager.tsx');
  const repair = component.slice(component.indexOf('const handleRepairExecutable'), component.indexOf('// Verify paths'));
  assert.match(repair, /captureLibraryRow\(configClient, config.apps, app\)/);
  assert.match(repair, /'pick_game_exe'/);
  assert.match(repair, /'repair_app_executable'/);
  assert.match(repair, /configClient.reportError\(err\)/);
  assert.match(component, /Failed to pick game exe:[\s\S]*?configClient.reportError\(err\)/);
  assert.match(component, /Failed to inspect dropped exe:[\s\S]*?configClient.reportError\(err\)/);
  assert.match(component, /existing.exe_name.toLowerCase\(\) !== scanned.exe_name.toLowerCase\(\)[\s\S]*?&& !isQuarantined\(existing\)\) return false/);
  let snapshot = { mode: 'ready', context_token: 'context', library_generation: '5',
    revision: '2', control_epoch: '2', store_id: 'store', settings: { apps: [] } };
  let received;
  const client = new ConfigClient(async (command, args) => {
    if (command === 'get_config') return snapshot;
    received = { command, args };
    snapshot = { ...snapshot, revision: '3', control_epoch: '3', library_generation: '6' };
    return snapshot;
  });
  await client.refresh();
  const origin = client.captureOrigin();
  await client.mutate('repair_app_executable', { exeName: 'BsSndRpt.exe', path: 'D:\\Game\\chosen.exe' }, origin);
  assert.equal(received.args.expectedLibraryGeneration, '5');
  assert.equal(received.args.expectedContext, 'context');
  const commands = source('src-tauri/src/commands.rs');
  assert.match(commands, /Some\(&expected_library_generation\), true,[\s\S]*?library::repair_executable/);
});

test('running process badges and activation share backend exact path authorization', () => {
  const component = source('src/components/RunningProcesses.tsx');
  assert.match(component, /proc.tracked_primary != null/);
  assert.doesNotMatch(component, /findTrackedApp/);
  const monitor = source('src-tauri/src/monitor_hook.rs');
  assert.match(monitor, /Some\(&process.path\)/);
  assert.doesNotMatch(monitor, /fn matches_app|\.find\(\|app\|/);
  const config = source('src-tauri/src/config.rs');
  assert.doesNotMatch(config, /fn executable_stem|fn alphanumeric/);
});

test('README describes bounded provider support and does not claim live verification', () => {
  const readme = source('README.md');
  for (const phrase of ['provider-specific', 'MicrosoftGame.config', 'unresolved rather than guessed',
    'not a universal storefront mapping', 'quarantined at runtime',
    'not a claim of live AOE3 HDR verification', 'preserves other choices']) {
    assert.ok(readme.includes(phrase), phrase);
  }
  assert.doesNotMatch(readme, /100% accurate game identification|in under \*\*2\.5 seconds/);
});
