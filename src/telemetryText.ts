import type { ActivityLogEntry, HdrStatePayload } from './types.ts';
import type { Language, Translations } from './i18n.ts';

export function statusWarnings(
  status: Pick<HdrStatePayload, 'warning' | 'quarantined_apps'>,
  t: Translations,
): string[] {
  return [
    ...(status.warning ? [status.warning] : []),
    ...(status.quarantined_apps ?? []).map((row) => row.reason === 'primary_path_mismatch'
      ? t.primaryPathWarning(row.name, row.exe_name) : t.quarantineWarning(row.name, row.exe_name)),
  ];
}

export function activityMessage(message: ActivityLogEntry['message'], t: Translations): string {
  switch (message.kind) {
    case 'init_system': return t.activityInitSystem;
    case 'init_detect': return t.activityInitDetect;
    case 'hdr_active': return t.activityHdrObserved;
    case 'game_hdr': return t.activityHookWindowFocus(message.appName);
    case 'sdr': return t.activityHookReturnSdr;
    case 'mixed': return t.activityHdrMixed;
  }
}

export function describeHdrScope(
  status: Pick<HdrStatePayload, 'scope_hdr_state' | 'target_status' | 'inventory_stale'>
    & Partial<Pick<HdrStatePayload, 'target_deferred' | 'active_target' | 'uncertain_targets'>>,
  t: Translations,
) {
  const unavailableNextTarget = status.target_deferred && status.active_target
    && status.uncertain_targets?.length === 0
    && ['disconnected', 'not_hdr_capable', 'needs_confirmation', 'identity_unavailable', 'ambiguous', 'state_unavailable']
      .includes(status.target_status);
  const mode = status.inventory_stale || (status.target_status !== 'ready' && !unavailableNextTarget)
    ? 'unknown' : status.scope_hdr_state;
  switch (mode) {
    case 'hdr': return { mode, badge: t.hdrActive, title: t.heroHdrActiveTitle };
    case 'sdr': return { mode, badge: t.sdrStandby, title: t.heroSdrTitle };
    case 'mixed': return { mode, badge: t.hdrMixed, title: t.heroMixedTitle };
    case 'unknown': return { mode, badge: t.configStatusUnknown, title: t.configStatusUnknown };
  }
}

export function telemetryTime(value: string, language: Language, t: Translations): string {
  if (value.toLowerCase() === 'včera') return t.recentYesterday;
  if (value.toLowerCase() === 'dnes') return t.recentToday;
  if (!/^\d{4}-\d{2}-\d{2}T/.test(value)) return value;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(language === 'cs' ? 'cs-CZ' : 'en', {
    hour: '2-digit',
    minute: '2-digit',
  }).format(date);
}
