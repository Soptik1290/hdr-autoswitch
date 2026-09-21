export type HdrType = 'native' | 'autohdr' | 'media' | 'custom';

export type SupportTier =
  | 'native'
  | 'limited'
  | 'always_on'
  | 'manual_fix'
  | 'autohdr'
  | 'media'
  | 'custom';

export interface HdrApp {
  name: string;
  exe_name: string;
  enabled: boolean;
  hdr_type: HdrType;
  path?: string | null;
  alternate_exes?: string[];
  steam_id?: string | null;
  launcher?: string | null;
}

export interface AppRowIdentity {
  index: number;
  exe_name: string;
  path: string | null;
}

export interface QuarantinedApp {
  row_index: number;
  name: string;
  exe_name: string;
  path: string | null;
  reason: 'helper' | 'primary_path_mismatch';
}

export type ScanEvidence =
  | { status: 'verified' }
  | { status: 'unverified'; reason: 'unresolved' };

export interface ScanGame extends Omit<HdrApp, 'enabled'> {
  is_hdr_supported: boolean;
  default_selected: boolean;
  evidence: ScanEvidence;
}

export interface PickedGameInfo {
  name: string;
  exe_name: string;
  path: string;
  hdr_type: HdrType;
  is_hdr_supported: boolean;
  notes?: string | null;
  launcher?: string | null;
}

export interface ActivityLogEntry {
  id: string;
  timestamp: string;
  message:
    | { kind: 'init_system' | 'init_detect' | 'hdr_active' | 'sdr' | 'mixed' }
    | { kind: 'game_hdr'; appName: string };
  type: 'info' | 'hdr_on' | 'hdr_off' | 'game' | 'system';
}

export interface CatalogEntry {
  name: string;
  name_aliases?: string[];
  exe_name: string;
  hdr_type: HdrType;
  support_tier: SupportTier;
  notes?: string | null;
  steam_id?: string | null;
  alternate_exes?: string[];
}

export interface MonitorInfo {
  id: string;
  device_path: string | null;
  identity_status: TargetStatus;
  identity_error: string | null;
  is_selected: boolean;
  name: string;
  adapter_id_low: number;
  adapter_id_high: number;
  target_id: number;
  is_hdr_supported: boolean;
  is_hdr_enabled: boolean;
  hdr_state_known: boolean;
  state_error: string | null;
  is_primary: boolean;
}

export interface MonitorInventorySnapshot {
  inventory_revision: string;
  monitors: MonitorInfo[];
}

export type SwitchMethod = 'native' | 'shortcut';

export type TargetMonitor =
  | { kind: 'all' }
  | { kind: 'monitor'; device_path: string; display_name: string }
  | { kind: 'needs_confirmation'; legacy_runtime_id: string };

export interface AppConfig {
  target_monitor: TargetMonitor;
  alt_tab_delay_seconds: number;
  notifications_enabled: boolean;
  autostart: boolean;
  start_minimized: boolean;
  auto_detect_new_games: boolean;
  auto_sync_database: boolean;
  last_sync_timestamp?: number | null;
  exit_only_hdr: boolean;
  switch_method: SwitchMethod;
  blacklist: string[];
  apps: HdrApp[];
}

export type SettingsPatch = Partial<Omit<AppConfig, 'apps' | 'last_sync_timestamp'>>;

export type ConfigMode =
  | 'ready'
  | 'first_run'
  | 'import_available'
  | 'recovery_required'
  | 'unsupported_schema'
  | 'unavailable';

export interface ConfigSnapshot {
  settings: AppConfig;
  mode: ConfigMode;
  store_id: string | null;
  revision: string;
  context_token: string;
  library_generation: string;
  control_epoch: string;
  issue: string | null;
  controller_issue: string | null;
  candidates: { id: string; label: string }[];
  config_path: string;
}

export interface ScanResult {
  context_token: string;
  library_generation: string;
  games: ScanGame[];
}

export interface RunningProcessInfo {
  pid: number;
  name: string;
  exe_name: string;
  title: string;
  path: string;
  tracked_primary: string | null;
}

export interface HdrStatePayload {
  status_revision: string;
  inventory_revision: string;
  manual_revision: string;
  manual_results: ManualScopeResult[];
  is_hdr_active: boolean;
  scope_hdr_state: 'hdr' | 'sdr' | 'mixed' | 'unknown';
  manual_control: { status: 'available' } | { status: 'blocked'; reason: string };
  current_app_name?: string | null;
  current_exe?: string | null;
  switched_by_app: boolean;
  steam_id?: string | null;
  launcher?: string | null;
  hdr_type?: string | null;
  warning: string | null;
  quarantined_apps: QuarantinedApp[];
  target_status: TargetStatus;
  active_target: TargetMonitor | null;
  target_deferred: boolean;
  any_hdr_active: boolean;
  inventory_stale: boolean;
  uncertain_targets: string[];
  operation_outcomes: MonitorOutcome[];
}

export type TargetStatus =
  | 'ready' | 'disconnected' | 'not_hdr_capable' | 'needs_confirmation'
  | 'identity_unavailable' | 'ambiguous' | 'enumeration_failed' | 'state_unavailable'
  | 'automation_paused' | 'controller_conflict' | 'outcome_unknown';

export interface MonitorOutcome {
  device_path: string | null;
  display_name: string | null;
  requested_hdr: boolean;
  outcome: 'already_in_desired_state' | 'changed' | 'outcome_unknown' | 'failed';
  failure: 'disconnected' | 'not_hdr_capable' | 'needs_confirmation'
    | 'identity_unavailable' | 'ambiguous' | 'enumeration_failed' | 'state_unavailable'
    | 'identity_changed' | 'native_rejected' | 'external_change' | 'authority_denied'
    | 'attempt_budget_exhausted' | null;
  message: string | null;
  previous_hdr: boolean | null;
  observed_hdr: boolean | null;
  previous_hdr_user_enabled: boolean | null;
  observed_hdr_user_enabled: boolean | null;
  attempts: number;
}

export interface ManualSetResult {
  scope: TargetMonitor;
  request: ManualRequestIdentity;
  outcomes: MonitorOutcome[];
  partial: boolean;
  status: HdrStatePayload;
}

export interface ManualScopeResult {
  revision: string;
  scope: TargetMonitor;
  request: ManualRequestIdentity;
  verified: boolean;
  error: string | null;
}

export interface ManualRequestOrigin {
  scope: TargetMonitor;
  request: ManualRequestIdentity;
}

export interface ManualRequestIdentity {
  client_id: string;
  sequence: string;
}

export interface ManualControlError extends ManualRequestOrigin {
  message: string;
}

export interface RecentGameSession {
  exe: string;
  name: string;
  steam_id?: string;
  launcher?: string;
  hdr_type: 'native' | 'autohdr' | 'media' | 'custom' | string;
  hdr_tier_label?: string;
  last_switched_at: string;
  hook_status: 'active' | 'switched_on' | 'switched_off';
  hook_message?: string;
}
