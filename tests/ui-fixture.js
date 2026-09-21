import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import { manualScopeAvailable } from '../src/displayState.ts';
import { findLibraryApp, normalizeLibraryPath } from '../src/libraryState.ts';

const options = new URLSearchParams(location.search);
localStorage.setItem('hdr_lang', options.get('lang') === 'cs' ? 'cs' : 'en');
localStorage.removeItem('hdr_recent_games');
mockWindows('main');
const mode = options.get('mode') ?? 'ready';
const aliasMerge = options.get('aliasMerge') === '1';
const legacyHelperAlias = options.get('legacyHelperAlias') === '1';
const settings = {
  target_monitor: options.get('mixed') === '1' ? { kind: 'all' } : mode === 'import_available'
    ? { kind: 'needs_confirmation', legacy_runtime_id: 'old-runtime-id' }
    : { kind: 'monitor', device_path: 'missing-monitor', display_name: 'Saved gaming display' },
  alt_tab_delay_seconds: 2,
  exit_only_hdr: true,
  notifications_enabled: false,
  autostart: false,
  start_minimized: false,
  auto_detect_new_games: false,
  auto_sync_database: false,
  switch_method: mode === 'import_available' || options.get('consent') === 'pending' ? 'shortcut' : 'native',
  blacklist: [],
  apps: legacyHelperAlias ? [{
    name: 'Fixture game', exe_name: 'game.exe', enabled: false, hdr_type: 'custom',
    steam_id: '100', launcher: 'Steam', path: 'C:\\Old\\game.exe',
    alternate_exes: ['BsSndRpt64.exe', 'user.exe'],
  }] : aliasMerge ? [{
    name: 'Fixture game', exe_name: 'renderer.exe', enabled: false, hdr_type: 'native',
    steam_id: '100', alternate_exes: [], path: 'D:\\Fixture\\renderer.exe',
  }] : [],
};
let snapshot = {
  mode, settings, store_id: 'fixture-history', revision: '1', control_epoch: '1',
  context_token: 'fixture-context', library_generation: '1', issue: null,
  controller_issue: options.get('conflict') === '1' ? 'Fixture controller conflict' : null,
  config_path: 'C:\\ISOLATED-UI-FIXTURE\\config-v2.json',
  candidates: mode === 'recovery_required'
    ? [{ id: 'validated-checkpoint', label: 'Validated fixture checkpoint' }] : [],
};
const monitors = [{
  id: 'connected-monitor', device_path: 'connected-monitor', name: 'Connected test display',
  identity_status: 'ready', identity_error: null,
  adapter_id_low: 999, adapter_id_high: 0, target_id: 23,
  is_hdr_supported: true, is_hdr_enabled: false, is_primary: true,
  hdr_state_known: true, state_error: null,
}];
let inventoryRevision = 0n;
let inventoryFingerprint = null;
let inventoryFailure = null;
let statusRevision = 0n;
let statusFingerprint = null;
let manualRevision = 0n;
const manualResults = new Map();
const manualWarnings = new Map();
let nextManualFailure = null;

function manualScopeKey(scope) {
  return scope.kind === 'monitor' ? `monitor:${scope.device_path.toLowerCase()}`
    : scope.kind === 'all' ? 'all' : `legacy:${scope.legacy_runtime_id}`;
}

function recordManual(scope, request, verified, error) {
  ++manualRevision;
  for (const [key, entry] of manualResults) {
    if (manualScopeKey(entry.scope) === manualScopeKey(scope)) {
      manualResults.set(key, { ...entry, error: null });
    }
  }
  manualResults.set(JSON.stringify([manualScopeKey(scope), request.client_id]), {
    revision: String(manualRevision), scope: structuredClone(scope), request: structuredClone(request), verified, error,
  });
}

function observeInventory() {
  const fingerprint = JSON.stringify([
    monitors.map((monitor) => JSON.stringify(monitor)).sort(), inventoryFailure,
  ]);
  if (fingerprint !== inventoryFingerprint) {
    inventoryFingerprint = fingerprint;
    ++inventoryRevision;
  }
  return String(inventoryRevision);
}

if (options.get('mixed') === '1') {
  monitors[0].is_hdr_enabled = true;
  monitors.push({
    ...monitors[0], id: 'second-monitor', device_path: 'second-monitor',
    name: 'Second test display', target_id: 24, is_hdr_enabled: false, is_primary: false,
  });
}
if (options.get('ambiguous') === '1') {
  monitors[0].identity_status = 'ambiguous';
  monitors[0].identity_error = 'Multiple endpoints claim this identity';
  monitors.push({ ...monitors[0], target_id: 24, is_primary: false });
}
if (options.get('unknown') === '1') {
  monitors[0].hdr_state_known = false;
  monitors[0].state_error = 'Fixture HDR state query failed';
  monitors[0].is_hdr_supported = false;
  monitors[0].is_hdr_enabled = false;
}
const games = [{
  name: 'Fixture game', exe_name: aliasMerge ? 'game.exe' : 'fixture.exe', hdr_type: 'native',
  is_hdr_supported: true, default_selected: false, evidence: { status: 'verified' },
  steam_id: aliasMerge ? '100' : null,
  alternate_exes: [], launcher: 'Média',
}];
const commands = [];
let history = 1;
let failSave = options.get('failSave') === '1';
let shuttingDown = options.get('shutdown') === '1';

const helper = (exe) => /^(gamelaunchhelper|bssndrpt|bssndrpt64|bugsplat|bugsplathd64)\.exe$/i.test(exe);
const repairReason = (app) => helper(app.exe_name) ? 'helper'
  : app.path && normalizeLibraryPath(app.path)?.split('\\').pop() !== app.exe_name.toLowerCase()
    ? 'primary_path_mismatch' : null;

function validateApp(app) {
  if (!app.name.trim() || [app.exe_name, ...(app.alternate_exes ?? [])].some((exe) =>
    helper(exe) || !/^[^\\/]+\.exe$/i.test(exe)) || repairReason(app)) {
    throw new Error('Select a consistent primary executable and path; helpers cannot be saved.');
  }
}

function overlaps(left, right) {
  return [left.exe_name, ...(left.alternate_exes ?? [])].some((exe) => !helper(exe)
    && [right.exe_name, ...(right.alternate_exes ?? [])].some((other) => {
      if (exe.toLowerCase() !== other.toLowerCase()) return false;
      if (exe.toLowerCase() === left.exe_name.toLowerCase() && other.toLowerCase() === right.exe_name.toLowerCase()) {
        const a = normalizeLibraryPath(left.path), b = normalizeLibraryPath(right.path);
        if (a && b && a !== b) return false;
      }
      return true;
    }));
}

function upsert(rows, incoming, importing = false) {
  validateApp(incoming);
  const existing = findLibraryApp(rows, incoming);
  if (!existing && rows.some((row) => findLibraryApp([row], incoming))) {
    throw new Error('Multiple library rows match. Remove duplicate rows before importing.');
  }
  const candidate = existing ? structuredClone(existing) : { ...incoming, alternate_exes: incoming.alternate_exes ?? [] };
  if (existing) {
    candidate.alternate_exes = (candidate.alternate_exes ?? []).filter((exe) => !helper(exe));
    if (repairReason(existing)) {
      candidate.exe_name = incoming.exe_name;
      candidate.path = incoming.path;
      candidate.alternate_exes = [];
    }
    candidate.alternate_exes ??= [];
    for (const exe of [incoming.exe_name, ...(incoming.alternate_exes ?? [])]) {
      if (exe.toLowerCase() !== candidate.exe_name.toLowerCase()
        && !candidate.alternate_exes.some((alias) => alias.toLowerCase() === exe.toLowerCase())) {
        candidate.alternate_exes.push(exe.toLowerCase());
      }
    }
    if (candidate.exe_name.toLowerCase() === incoming.exe_name.toLowerCase() && incoming.path) {
      candidate.path = incoming.path;
    }
    if (!importing) {
      candidate.name = incoming.name;
      candidate.hdr_type = incoming.hdr_type;
    }
    for (const field of ['steam_id', 'launcher']) {
      if (incoming[field] != null) candidate[field] = incoming[field];
    }
  }
  candidate.enabled = importing || incoming.enabled;
  validateApp(candidate);
  if (rows.some((row) => row !== existing && overlaps(row, candidate))) {
    throw new Error('Executable ownership conflicts with another row. No rows were merged.');
  }
  if (existing) rows[rows.indexOf(existing)] = candidate;
  else rows.push(candidate);
}

function targetedRow(args) {
  if (args.expectedLibraryGeneration !== snapshot.library_generation) throw new Error('Stale fixture library');
  const row = snapshot.settings.apps[args.row.index];
  if (!row || row.exe_name !== args.row.exe_name || (row.path ?? null) !== args.row.path) {
    throw new Error('Fixture row changed. Refresh before editing.');
  }
  return row;
}

function manualControl() {
  const reason = shuttingDown ? 'Fixture controller shutting down'
    : snapshot.controller_issue ?? (snapshot.mode === 'unavailable' ? 'Fixture authority unavailable' : null);
  return reason ? { status: 'blocked', reason } : { status: 'available' };
}

function trackedPrimary(exe, path) {
  const rows = snapshot.settings.apps;
  const exact = rows.filter((row) => row.exe_name.toLowerCase() === exe.toLowerCase()
    && normalizeLibraryPath(row.path) === normalizeLibraryPath(path));
  if (exact.length) {
    return exact.length === 1 && exact[0].enabled && !repairReason(exact[0]) ? exact[0].exe_name : null;
  }
  const unscoped = rows.filter((row) => (!row.path && row.exe_name.toLowerCase() === exe.toLowerCase())
    || row.alternate_exes?.some((alias) => alias.toLowerCase() === exe.toLowerCase()
      && alias.toLowerCase() !== row.exe_name.toLowerCase()));
  return unscoped.length === 1 && unscoped[0].enabled && !repairReason(unscoped[0]) ? unscoped[0].exe_name : null;
}

function status() {
  const target = snapshot.settings.target_monitor;
  const target_status = snapshot.controller_issue ? 'controller_conflict'
    : inventoryFailure ? 'enumeration_failed'
    : snapshot.mode !== 'ready' || snapshot.settings.switch_method !== 'native'
    ? 'automation_paused'
    : target.kind === 'needs_confirmation' ? 'needs_confirmation'
    : target.kind === 'monitor' && !monitors.some((monitor) => monitor.device_path === target.device_path)
      ? 'disconnected' : 'ready';
  const selected = monitors.filter((monitor) => target.kind === 'all' || target.device_path === monitor.device_path);
  const scope_hdr_state = target_status !== 'ready' || selected.length === 0 ? 'unknown'
    : selected.every((monitor) => monitor.is_hdr_enabled) ? 'hdr'
    : selected.some((monitor) => monitor.is_hdr_enabled) ? 'mixed' : 'sdr';
  const payload = {
    inventory_revision: observeInventory(),
    manual_revision: String(manualRevision), manual_results: structuredClone([...manualResults.values()]),
    is_hdr_active: scope_hdr_state === 'hdr', scope_hdr_state,
    manual_control: manualControl(),
    current_app_name: null, current_exe: null, switched_by_app: false,
    steam_id: null, launcher: null, hdr_type: null, target_status,
    warning: [...new Set([
      snapshot.controller_issue, inventoryFailure,
      target_status === 'disconnected' ? 'The saved display is disconnected. No other display is substituted.' : null,
      ...[...manualResults.values()].map((result) => result.error),
      ...manualWarnings.values(),
    ].filter(Boolean))].join('\n') || null,
    active_target: null, target_deferred: false, inventory_stale: inventoryFailure !== null,
    uncertain_targets: [], operation_outcomes: [],
    quarantined_apps: snapshot.settings.apps.flatMap((app, row_index) => {
      const reason = repairReason(app);
      return reason ? [{ row_index, name: app.name, exe_name: app.exe_name, path: app.path ?? null, reason }] : [];
    }),
    any_hdr_active: monitors.some((monitor) => monitor.is_hdr_enabled),
  };
  const fingerprint = JSON.stringify(payload);
  if (fingerprint !== statusFingerprint) {
    statusFingerprint = fingerprint;
    ++statusRevision;
  }
  return { ...payload, status_revision: String(statusRevision) };
}

async function commit(changesLibrary = false, newHistory = false) {
  snapshot.control_epoch = String(BigInt(snapshot.control_epoch) + 1n);
  snapshot.revision = newHistory ? '1' : String(BigInt(snapshot.revision) + 1n);
  if (newHistory) {
    ++history;
    snapshot.store_id = `fixture-history-${history}`;
    snapshot.context_token = `fixture-context-${history}`;
    snapshot.mode = 'ready';
  }
  if (changesLibrary) snapshot.library_generation = String(BigInt(snapshot.library_generation) + 1n);
  await emit('config-changed', structuredClone(snapshot));
  await emit('hdr-status-changed', status());
  return structuredClone(snapshot);
}

mockIPC(async (command, args = {}) => {
  commands.push({ command, args: structuredClone(args) });
  if (args.expectedContext && args.expectedContext !== snapshot.context_token) throw new Error('Retired fixture history');
  switch (command) {
    case 'set_ui_language': return null;
    case 'get_config': return structuredClone(snapshot);
    case 'get_current_status': return status();
    case 'get_monitors':
      if (inventoryFailure !== null) throw new Error(inventoryFailure);
      return {
        inventory_revision: observeInventory(),
        monitors: monitors.map((monitor) => ({
          ...monitor,
          is_selected: snapshot.mode === 'ready' && monitor.identity_status === 'ready'
            && (snapshot.settings.target_monitor.kind === 'all'
              || snapshot.settings.target_monitor.device_path === monitor.device_path),
        })),
      };
    case 'patch_settings':
      if (failSave) throw new Error('Simulated save failure; previous settings preserved');
      Object.assign(snapshot.settings, args.patch);
      return commit();
    case 'initialize_config':
    case 'reset_config':
      snapshot.settings = { ...settings, target_monitor: { kind: 'all' }, apps: [] };
      return commit(true, true);
    case 'restore_config':
    case 'import_legacy_config': return commit(true, true);
    case 'recheck_controller': return commit();
    case 'get_catalog': return games.map(({ name, exe_name, hdr_type, steam_id, alternate_exes }) => ({
      name, exe_name, hdr_type, steam_id, alternate_exes,
      support_tier: 'native', notes: 'Nativní HDR podpora (PCGamingWiki)',
    }));
    case 'scan_installed_games': return {
      context_token: snapshot.context_token, library_generation: snapshot.library_generation,
      games: games.map((game) => ({ ...game, default_selected: game.is_hdr_supported && snapshot.settings.auto_detect_new_games })),
    };
    case 'add_custom_app': {
      const rows = structuredClone(snapshot.settings.apps);
      upsert(rows, args.app);
      snapshot.settings.apps = rows;
      return commit(true);
    }
    case 'import_detected_games': {
      if (args.expectedLibraryGeneration !== snapshot.library_generation) throw new Error('Stale fixture scan');
      const primaries = new Map();
      for (const app of args.detected) {
        const key = app.exe_name.toLowerCase();
        const previous = primaries.get(key);
        if (previous && (normalizeLibraryPath(previous.path) !== normalizeLibraryPath(app.path)
          || previous.launcher?.toLowerCase() !== app.launcher?.toLowerCase()
          || (previous.steam_id && app.steam_id && previous.steam_id !== app.steam_id))) {
          throw new Error(`Select only one installation for ${app.exe_name}.`);
        }
        primaries.set(key, { ...app, steam_id: app.steam_id ?? previous?.steam_id });
      }
      const rows = structuredClone(snapshot.settings.apps);
      for (const app of args.detected) upsert(rows, app, true);
      snapshot.settings.apps = rows;
      return commit(true);
    }
    case 'remove_app':
      targetedRow(args);
      snapshot.settings.apps.splice(args.row.index, 1);
      return commit(true);
    case 'toggle_app': {
      const row = targetedRow(args);
      if (args.enabled && repairReason(row)) throw new Error('Select the actual game executable first.');
      row.enabled = args.enabled;
      return commit(true);
    }
    case 'repair_app_executable': {
      const row = targetedRow(args);
      if (!repairReason(row)) throw new Error('This row is no longer quarantined.');
      const candidate = { ...row, exe_name: normalizeLibraryPath(args.path)?.split('\\').pop() ?? '',
        path: args.path, alternate_exes: [] };
      validateApp(candidate);
      if (snapshot.settings.apps.some((other) => other !== row && overlaps(other, candidate))) {
        throw new Error('Executable ownership conflicts with another row. No rows were merged.');
      }
      snapshot.settings.apps[args.row.index] = candidate;
      return commit(true);
    }
    case 'verify_game_paths': return {};
    case 'get_running_processes': return aliasMerge ? [{
      pid: 123, name: 'Fixture game', exe_name: 'game.exe',
      title: 'Fixture renderer', path: 'D:\\Fixture\\game.exe',
      tracked_primary: trackedPrimary('game.exe', 'D:\\Fixture\\game.exe'),
    }] : [];
    case 'pick_game_exe': return null;
    case 'sync_database': return games.length;
    case 'set_hdr': {
      const request = args.request;
      if (!request?.client_id?.startsWith('gui:') || !/^[1-9]\d*$/.test(request.sequence)
        || BigInt(request.sequence) > 18446744073709551615n) throw new Error('Invalid GUI correlation identity');
      const previous = manualResults.get(JSON.stringify([manualScopeKey(args.scope), request.client_id]));
      if (previous && BigInt(previous.request.sequence) >= BigInt(request.sequence)) {
        throw new Error('Manual request superseded or already completed');
      }
      const admission = manualControl();
      const rejection = admission.status === 'blocked' ? admission.reason
        : !manualScopeAvailable(args.scope, monitors) ? 'Fixture scope unavailable' : null;
      if (rejection) {
        recordManual(args.scope, request, false, rejection);
        await emit('hdr-status-changed', status());
        throw new Error(rejection);
      }
      const failure = nextManualFailure;
      nextManualFailure = null;
      const selected = monitors.filter((monitor) => args.scope.kind === 'all' || args.scope.device_path === monitor.device_path);
      const outcomes = selected.map((monitor) => {
        const previous = monitor.is_hdr_enabled;
        if (!failure) monitor.is_hdr_enabled = args.enable;
        if (failure) manualWarnings.set(monitor.device_path, failure);
        else manualWarnings.delete(monitor.device_path);
        return {
          device_path: monitor.device_path, display_name: monitor.name,
          requested_hdr: args.enable, outcome: failure ? 'failed'
            : previous === args.enable ? 'already_in_desired_state' : 'changed',
          failure: failure ? 'native_rejected' : null, message: failure,
          previous_hdr: previous, observed_hdr: monitor.is_hdr_enabled, attempts: 1,
          previous_hdr_user_enabled: previous, observed_hdr_user_enabled: monitor.is_hdr_enabled,
        };
      });
      recordManual(args.scope, request, !failure, null);
      await emit('hdr-status-changed', status());
      return { scope: structuredClone(args.scope), request: structuredClone(request), outcomes, partial: !!failure, status: status() };
    }
    default: throw new Error(`Unexpected fixture command: ${command}`);
  }
}, { shouldMockEvents: true });

// The installed SDK mock expects `id`, while the real event API sends `eventId`.
const mockInvoke = window.__TAURI_INTERNALS__.invoke;
window.__TAURI_INTERNALS__.invoke = (command, args, invokeOptions) => mockInvoke(
  command,
  command === 'plugin:event|unlisten' ? { ...args, id: args.eventId } : args,
  invokeOptions,
);

window.__hdrFixture = {
  commands,
  get snapshot() { return structuredClone(snapshot); },
  permitSaves() { failSave = false; },
  failNextManual(message) { nextManualFailure = message; },
  emitStatus: (payload = status()) => emit('hdr-status-changed', payload),
  get status() { return status(); },
  get monitors() { return structuredClone(monitors); },
  async replaceMonitors(next) {
    monitors.splice(0, monitors.length, ...structuredClone(next));
    await emit('hdr-status-changed', status());
  },
  async failInventory(message) {
    inventoryFailure = message;
    await emit('hdr-status-changed', status());
  },
  async stopController() {
    shuttingDown = true;
    await emit('hdr-status-changed', status());
  },
};
await import('../src/main.tsx');
