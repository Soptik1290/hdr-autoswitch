import React, { useRef, useState } from 'react';
import { MonitorInfo, HdrStatePayload, ActivityLogEntry, RecentGameSession, TargetMonitor, ManualSetResult, ManualRequestOrigin, ManualControlError } from '../types';
import { invoke } from '@tauri-apps/api/core';
import { activityMessage, describeHdrScope, telemetryTime } from '../telemetryText';
import { manualScopeAvailable, monitorMode, monitorReady, scopeVisuals } from '../displayState';
import { launcherName } from '../catalogNotes';
import {
  Monitor,
  ShieldCheck,
  RefreshCw,
  Terminal,
  ArrowRight,
  Sparkles,
  History,
  Zap,
  Gamepad2,
} from 'lucide-react';
import { HdrLogo } from './HdrLogo';
import { GlitchButton } from './GlitchButton';
import { GlitchText } from './GlitchText';
import { useI18n } from '../i18n';

interface DashboardProps {
  status: HdrStatePayload;
  monitors: MonitorInfo[];
  libraryCount: number | null;
  activityLogs: ActivityLogEntry[];
  recentGames: RecentGameSession[];
  onRefreshMonitors: () => void;
  onManualToggle: () => void;
  onNavigateToApps: () => void;
  controlAvailable: boolean;
  onControlError: (error: ManualControlError) => void;
  onManualResult: (result: ManualSetResult) => void;
  captureManualOrigin: (scope: TargetMonitor) => ManualRequestOrigin;
  isDark: boolean;
}

export const Dashboard: React.FC<DashboardProps> = ({
  status,
  monitors,
  libraryCount,
  activityLogs,
  recentGames,
  onRefreshMonitors,
  onManualToggle,
  onNavigateToApps,
  controlAvailable,
  onControlError,
  onManualResult,
  captureManualOrigin,
  isDark,
}) => {
  const { t, lang } = useI18n();
  const [toggling, setToggling] = useState(false);
  const manualPending = useRef(false);

  const handleSet = async (scope: TargetMonitor, enable: boolean) => {
    if (manualPending.current) return;
    const origin = captureManualOrigin(scope);
    if (!controlAvailable || !manualScopeAvailable(scope, monitors)) {
      onControlError({ ...origin, message: status.manual_control.status === 'blocked'
        ? status.manual_control.reason : t.configManualUnavailable });
      return;
    }
    manualPending.current = true;
    setToggling(true);
    try {
      const result = await invoke<ManualSetResult>('set_hdr', { scope, enable, request: origin.request });
      onManualResult(result);
    } catch (err) {
      onControlError({ ...origin, message: String(err) });
    } finally {
      onManualToggle();
      manualPending.current = false;
      setToggling(false);
    }
  };

  const handleSetMonitor = (monitor: MonitorInfo, enable: boolean) => {
    if (!monitorReady(monitor)) {
      onControlError({
        ...captureManualOrigin(monitor.device_path
          ? { kind: 'monitor', device_path: monitor.device_path, display_name: monitor.name }
          : { kind: 'needs_confirmation', legacy_runtime_id: monitor.id }),
        message: t.configMonitorIdentityError,
      });
      return;
    }
    void handleSet({ kind: 'monitor', device_path: monitor.device_path, display_name: monitor.name }, enable);
  };

  const hdrSupportedMonitors = monitors.filter(monitorReady);
  const scope = describeHdrScope(status, t);
  const visuals = scopeVisuals[scope.mode];
  const allAvailable = controlAvailable && manualScopeAvailable({ kind: 'all' }, monitors);

  return (
    <div className="space-y-6 font-mono">
      {/* Hero Display Control Center (Retro Glitch Terminal) */}
      <div
        data-hdr-scope={scope.mode}
        className={`relative overflow-hidden border p-6 transition-all duration-300 ${
          isDark
            ? visuals.panel
            : scope.mode === 'hdr'
            ? 'bg-gradient-to-r from-rose-50 via-white to-white border-[#f55a6b] shadow-md shadow-rose-100/60'
            : scope.mode === 'sdr'
            ? 'bg-gradient-to-r from-sky-50/60 via-white to-white border-[#5accf5]/50 shadow-xs'
            : scope.mode === 'mixed'
            ? 'bg-gradient-to-r from-amber-50/60 via-white to-white border-amber-400 shadow-xs'
            : 'bg-white border-slate-400 border-dashed shadow-xs'
        }`}
      >
        {/* Subtle scanline background texture */}
        {isDark && <div className="absolute inset-0 scanlines-overlay opacity-30 pointer-events-none" />}

        <div className="flex flex-col lg:flex-row lg:items-center justify-between gap-6 relative z-10">
          <div className="flex items-center gap-5">
            {/* Ambient Aperture Dial with Glitch Border */}
            <div
              className={`p-3 border shrink-0 flex items-center justify-center aspect-square transition-all duration-300 ${
                isDark
                  ? visuals.dial
                  : scope.mode === 'hdr'
                  ? 'bg-white border-[#f55a6b] shadow-[0_0_20px_rgba(245,90,107,0.3)] scale-105'
                  : scope.mode === 'sdr'
                  ? 'bg-white border-[#5accf5]/50 shadow-xs'
                  : scope.mode === 'mixed'
                  ? 'bg-white border-amber-400 shadow-xs'
                  : 'bg-white border-slate-400 border-dashed shadow-xs'
              }`}
            >
              <HdrLogo size={52} mode={scope.mode} />
            </div>

            <div className="space-y-1.5">
              <div className="flex items-center gap-2.5 flex-wrap">
                <span
                  className={`inline-flex items-center gap-1.5 px-2.5 py-0.5 text-xs font-bold uppercase tracking-wider border ${
                    isDark
                      ? visuals.badge
                      : scope.mode === 'hdr'
                      ? 'bg-[#f55a6b] text-white border-[#f55a6b]'
                      : scope.mode === 'sdr'
                      ? 'bg-sky-100 text-sky-900 border-sky-300'
                      : scope.mode === 'mixed'
                      ? 'bg-amber-100 text-amber-900 border-amber-400'
                      : 'bg-slate-100 text-slate-800 border-slate-400 border-dashed'
                  }`}
                >
                  <span
                    className={`w-2 h-2 ${
                      isDark
                        ? visuals.dot
                        : scope.mode === 'hdr'
                        ? 'bg-white animate-status-pulse'
                        : scope.mode === 'sdr'
                        ? 'bg-sky-600'
                        : scope.mode === 'mixed'
                        ? 'bg-amber-600'
                        : 'border border-slate-500'
                    }`}
                  />
                  {scope.mode === 'hdr' ? t.heroHdrRec2020 : scope.mode === 'sdr' ? t.heroSdrBt709 : scope.badge}
                </span>

                {status.switched_by_app && (
                  <span className={`text-xs px-2 py-0.5 border ${
                    isDark
                      ? 'bg-[#5accf5]/15 text-[#5accf5] border-[#5accf5]/40'
                      : 'bg-sky-50 text-sky-800 border-sky-300'
                  }`}>
                    {t.heroHookActive}
                  </span>
                )}

                <span className={`text-xs ${isDark ? 'text-[#8a7f81]' : 'text-slate-500'}`}>
                  {t.heroDisplaysReady(hdrSupportedMonitors.length)}
                </span>
              </div>

              <h2 className={`text-2xl font-bold tracking-tight flex items-center gap-2 ${
                isDark ? 'text-white' : 'text-slate-900'
              }`}>
                <GlitchText
                  text={scope.title}
                  scrambleOnHover={true}
                />
              </h2>

              <p className={`text-xs ${isDark ? 'text-[#b5a9ac]' : 'text-slate-600'}`}>
                {status.current_app_name ? (
                  <span className={`flex items-center gap-2 ${isDark ? 'text-[#f55a6b]' : 'text-[#e03e52]'}`}>
                    <Sparkles className={`w-4 h-4 shrink-0 ${isDark ? 'text-[#5accf5]' : 'text-sky-600'}`} />
                    <span>{t.heroActiveProcess}</span>
                    <strong className={`font-bold tracking-wide ${isDark ? 'text-white' : 'text-slate-900'}`}>
                      {status.current_app_name}
                    </strong>
                    {status.current_exe && (
                      <span className={isDark ? 'text-[#5accf5]' : 'text-sky-600'}>[{status.current_exe}]</span>
                    )}
                  </span>
                ) : (
                  <span>{scope.badge}</span>
                )}
              </p>
            </div>
          </div>

          <div className="shrink-0 flex flex-col gap-2">
            <GlitchButton
              label={toggling ? t.heroSwitching : t.configAllOn}
              variant="primary"
              icon={<Zap className="w-4 h-4 fill-current" />}
              size="lg"
              disabled={toggling || !allAvailable}
              onClick={() => handleSet({ kind: 'all' }, true)}
            />
            <GlitchButton
              label={toggling ? t.heroSwitching : t.configAllOff}
              variant="outline"
              size="lg"
              disabled={toggling || !allAvailable}
              onClick={() => handleSet({ kind: 'all' }, false)}
            />
          </div>
        </div>
        <p className={`relative z-10 mt-4 text-xs ${isDark ? 'text-[#b5a9ac]' : 'text-slate-500'}`}>{t.configManualPolicy}</p>
      </div>

      {/* Connected Displays & Monitor Controls */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Monitor className={`w-4 h-4 ${isDark ? 'text-[#5accf5]' : 'text-sky-600'}`} />
            <h3 className={`font-bold text-xs uppercase tracking-wider ${isDark ? 'text-[#f55a6b]' : 'text-[#e03e52]'}`}>
              {t.displaysTitle}
            </h3>
            <span className={`text-xs ${isDark ? 'text-[#8a7f81]' : 'text-slate-500'}`}>({monitors.length})</span>
          </div>

          <GlitchButton
            label={t.displaysRefresh}
            variant="outline"
            size="sm"
            icon={<RefreshCw className={`w-3 h-3 ${isDark ? 'text-[#5accf5]' : 'text-[#e03e52]'}`} />}
            onClick={onRefreshMonitors}
          />
        </div>

        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          {monitors.map((m) => {
            const isTarget = m.is_selected;
            const mode = monitorMode(m);

            return (
              <div
                key={`${m.adapter_id_low}:${m.adapter_id_high}:${m.target_id}`}
                data-monitor-state={mode}
                className={`p-4 border transition-all relative ${
                  isDark
                    ? scopeVisuals[mode].panel
                    : mode === 'hdr'
                    ? 'bg-linear-to-r from-rose-50/80 via-white to-white border-[#f55a6b] shadow-xs'
                    : mode === 'sdr'
                    ? 'bg-white border-slate-200 shadow-xs'
                    : mode === 'mixed'
                    ? 'bg-amber-50/50 border-amber-400/80 shadow-xs'
                    : 'bg-slate-50 border-slate-300 border-dashed shadow-xs'
                }`}
              >
                {isDark && <div className="absolute inset-0 scanlines-overlay opacity-20 pointer-events-none" />}

                <div className="flex flex-wrap items-start justify-between gap-3 relative z-10">
                  <div className="space-y-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <h4 className={`font-bold text-sm truncate max-w-[220px] ${isDark ? 'text-white' : 'text-slate-900'}`} title={m.name}>
                        {m.name}
                      </h4>
                      {m.is_primary && (
                        <span className={`text-[10px] px-1.5 py-0.5 border font-bold ${
                          isDark
                            ? 'bg-[#5accf5]/15 text-[#5accf5] border-[#5accf5]/40'
                            : 'bg-sky-50 text-sky-800 border-sky-300'
                        }`}>
                          {t.displaysPrimary}
                        </span>
                      )}
                      {isTarget && (
                        <span className={`text-[10px] px-1.5 py-0.5 border font-bold ${
                          isDark
                            ? 'bg-[#f55a6b]/15 text-[#f55a6b] border-[#f55a6b]/40'
                            : 'bg-rose-50 text-rose-800 border-rose-300'
                        }`}>
                          {t.displaysTargetHdr}
                        </span>
                      )}
                    </div>

                    <div className="flex items-center gap-2 text-xs">
                      {mode === 'unknown' ? (
                        <span className={isDark ? 'text-amber-300' : 'text-amber-700 font-semibold'}>{t.displaysStateUnknown}</span>
                      ) : m.is_hdr_supported ? (
                        <span className={`flex items-center gap-1 font-semibold ${isDark ? 'text-emerald-400' : 'text-emerald-700'}`}>
                          <ShieldCheck className="w-3.5 h-3.5" />
                          {t.displaysHdrSupported}
                        </span>
                      ) : (
                        <span className={isDark ? 'text-[#8a7f81]' : 'text-slate-500'}>{t.displaysSdrOnly}</span>
                      )}
                      <span className={isDark ? 'text-[#8a7f81]' : 'text-slate-400'}>•</span>
                      <span className={`text-xs font-mono ${isDark ? 'text-[#5accf5]' : 'text-sky-700'}`}>
                        {t.displaysTargetId}: {m.target_id}
                      </span>
                    </div>
                  </div>

                  <div className="flex items-center gap-2">
                    {m.is_hdr_supported && <>
                      <GlitchButton
                        label={t.heroTurnOnHdr}
                        variant={mode === 'hdr' ? 'primary' : 'outline'}
                        size="sm"
                        disabled={toggling || !controlAvailable || !monitorReady(m)}
                        onClick={() => handleSetMonitor(m, true)}
                      />
                      <GlitchButton
                        label={t.heroTurnOffHdr}
                        variant={mode === 'sdr' ? 'primary' : 'outline'}
                        size="sm"
                        disabled={toggling || !controlAvailable || !monitorReady(m)}
                        onClick={() => handleSetMonitor(m, false)}
                      />
                    </>}
                  </div>
                  {m.identity_error && <p className={`mt-2 text-xs ${isDark ? 'text-amber-300' : 'text-amber-700'}`} role="alert">{m.identity_error}</p>}
                  {m.state_error && <p className={`mt-2 text-xs ${isDark ? 'text-amber-300' : 'text-amber-700'}`} role="alert">{m.state_error}</p>}
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* POSLEDNÍ HRY & HOOK TELEMETRIE */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <History className={`w-4 h-4 ${isDark ? 'text-[#f55a6b]' : 'text-[#e03e52]'}`} />
            <h3 className={`font-bold text-xs uppercase tracking-wider ${isDark ? 'text-[#f55a6b]' : 'text-[#e03e52]'}`}>
              {t.recentTitle}
            </h3>
            <span className={`text-xs ${isDark ? 'text-[#8a7f81]' : 'text-slate-500'}`}>({recentGames.length})</span>
          </div>

          {libraryCount !== null && <button
            onClick={onNavigateToApps}
            className={`text-xs flex items-center gap-1 font-bold cursor-pointer uppercase transition-colors ${
              isDark ? 'text-[#5accf5] hover:text-[#70d6f7]' : 'text-sky-600 hover:text-sky-800'
            }`}
          >
            <span>{t.recentAllLibrary(libraryCount)}</span>
            <ArrowRight className="w-3.5 h-3.5" />
          </button>}
        </div>

        {recentGames.length === 0 ? (
          <div className={`p-8 border flex flex-col sm:flex-row items-center justify-center gap-3 text-center sm:text-left relative overflow-hidden ${
            isDark ? 'border-[#f55a6b]/20 bg-[#120d0e]/60' : 'border-slate-200 bg-white shadow-xs'
          }`}>
            {isDark && <div className="absolute inset-0 scanlines-overlay opacity-15 pointer-events-none" />}
            <div className={`p-3 border shrink-0 ${
              isDark ? 'border-[#f55a6b]/30 bg-[#1a0e10] text-[#f55a6b]' : 'border-rose-200 bg-rose-50 text-[#e03e52]'
            }`}>
              <Gamepad2 className="w-6 h-6" />
            </div>
            <div className="space-y-0.5">
              <p className={`text-xs font-bold font-mono ${isDark ? 'text-[#e5e0e1]' : 'text-slate-800'}`}>
                {t.recentEmpty}
              </p>
              <p className={`text-[11px] font-mono ${isDark ? 'text-[#8a7f81]' : 'text-slate-500'}`}>
                {t.heroSdrSubtext}
              </p>
            </div>
          </div>
        ) : (
          <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-3.5">
            {recentGames.slice(0, 6).map((game) => {
            const steamCover = game.steam_id
              ? `https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/${game.steam_id}/library_600x900.jpg`
              : null;

            const isHookActive = game.hook_status === 'active';

            // Support Tier label colors
            const getTierBadge = () => {
              if (game.hdr_type === 'autohdr') {
                return (
                  <span className="px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider bg-purple-950/80 text-purple-300 border border-purple-500/40">
                    {t.recentTierAutoHdr}
                  </span>
                );
              }
              if (game.hdr_type === 'mod' || game.hdr_type === 'custom') {
                return (
                  <span className="px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider bg-amber-950/80 text-amber-300 border border-amber-500/40">
                    {t.recentTierMod}
                  </span>
                );
              }
              return (
                <span className="px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider bg-cyan-950/80 text-[#5accf5] border border-[#5accf5]/50">
                  {t.recentTierNative}
                </span>
              );
            };

            return (
              <div
                key={game.exe}
                className={`relative group border overflow-hidden flex flex-col justify-between transition-all duration-200 ${
                  isHookActive
                    ? isDark
                      ? 'bg-[#1c0f12] border-[#f55a6b] neon-glow-coral'
                      : 'bg-rose-50/50 border-[#f55a6b] shadow-md shadow-rose-100'
                    : isDark
                    ? 'bg-[#120d0e] border-[#f55a6b]/30 hover:border-[#f55a6b] hover:shadow-[0_0_15px_rgba(245,90,107,0.3)]'
                    : 'bg-white border-slate-200 hover:border-[#f55a6b] hover:shadow-md'
                }`}
                style={{ height: '230px' }}
              >
                {/* Poster Artwork with Scanlines */}
                <div className="absolute inset-0">
                  {steamCover ? (
                    <img
                      src={steamCover}
                      alt={game.name}
                      className="w-full h-full object-cover transition-transform duration-300 group-hover:scale-105"
                      onError={(e) => {
                        (e.target as HTMLElement).style.display = 'none';
                      }}
                    />
                  ) : (
                    <div className={`w-full h-full ${isDark ? 'bg-gradient-to-b from-[#221314] to-[#0f0b0b]' : 'bg-gradient-to-b from-slate-100 to-slate-200'}`} />
                  )}
                  {/* CRT Scanline overlay on image in dark mode */}
                  {isDark && <div className="absolute inset-0 scanlines-overlay opacity-35 pointer-events-none" />}
                  <div className={`absolute inset-0 pointer-events-none ${
                    isDark
                      ? 'bg-gradient-to-t from-[#0f0b0b] via-[#0f0b0b]/60 to-transparent'
                      : 'bg-gradient-to-t from-black/60 via-transparent to-transparent'
                  }`} />
                </div>

                {/* Top Badge: HDR Support Type */}
                <div className="relative z-10 p-2 flex items-center justify-between">
                  {getTierBadge()}
                  {game.launcher && (
                    <span className="px-1 py-0.2 text-[8px] font-mono text-[#b5a9ac] bg-black/60 border border-white/10 uppercase">
                      {launcherName(game.launcher, lang)}
                    </span>
                  )}
                </div>

                {/* Bottom Overlay: Title & Hook Telemetry Status */}
                <div className={`relative z-10 p-2.5 space-y-1.5 ${
                  isDark
                    ? 'bg-[#0f0b0b]/90 border-t border-[#f55a6b]/20'
                    : 'bg-white/95 border-t border-slate-200 shadow-xs'
                }`}>
                  <div className={`font-bold text-xs truncate ${isDark ? 'text-white' : 'text-slate-900'}`}>
                    <GlitchText text={game.name} scrambleOnHover={true} />
                  </div>

                  {/* Hook Verification Badge */}
                  <div className="space-y-0.5">
                    <div className="flex items-center gap-1.5 text-[10px]">
                      <span
                        className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                          isHookActive
                            ? isDark ? 'bg-[#5accf5] animate-status-pulse' : 'bg-sky-500 animate-status-pulse'
                            : isDark ? 'bg-emerald-400' : 'bg-emerald-600'
                        }`}
                      />
                      <span
                        className={`truncate font-semibold ${
                          isHookActive
                            ? isDark ? 'text-[#5accf5]' : 'text-sky-700'
                            : isDark ? 'text-emerald-300' : 'text-emerald-700'
                        }`}
                      >
                        {isHookActive ? t.recentHookActive : t.recentHookTriggered}
                      </span>
                    </div>

                    <div className={`flex items-center justify-between text-[9px] ${isDark ? 'text-[#8a7f81]' : 'text-slate-500'}`}>
                      <span>{telemetryTime(game.last_switched_at, lang, t)}</span>
                      <span className={isDark ? 'text-[#5accf5]' : 'text-sky-600 font-bold'}>{t.recentHdrOk}</span>
                    </div>
                  </div>
                </div>
              </div>
            );
          })}
          </div>
        )}
      </div>

      {/* Activity Log (Real-time CRT System Event Feed) */}
      <div className="space-y-3">
        <div className="flex items-center gap-2">
          <Terminal className={`w-4 h-4 ${isDark ? 'text-[#5accf5]' : 'text-sky-600'}`} />
          <h3 className={`font-bold text-xs uppercase tracking-wider ${isDark ? 'text-[#f55a6b]' : 'text-[#e03e52]'}`}>
            {t.activityTitle}
          </h3>
        </div>

        <div className={`p-3.5 border relative space-y-2 max-h-[160px] overflow-y-auto ${
          isDark
            ? 'border-[#f55a6b]/30 bg-[#0f0b0b]'
            : 'border-slate-200 bg-white shadow-xs'
        }`}>
          {isDark && <div className="absolute inset-0 scanlines-overlay opacity-20 pointer-events-none" />}

          {activityLogs.map((log) => (
            <div
              key={log.id}
              className={`flex items-center gap-2 text-xs font-mono transition-colors relative z-10 ${
                isDark ? 'hover:text-white' : 'hover:text-slate-900'
              }`}
            >
              <span className={`shrink-0 ${isDark ? 'text-[#8a7f81]' : 'text-slate-400'}`}>[{telemetryTime(log.timestamp, lang, t)}]</span>
              <span
                className={`w-1.5 h-1.5 shrink-0 ${
                  log.type === 'hdr_on'
                    ? 'bg-[#f55a6b]'
                    : log.type === 'hdr_off'
                    ? isDark ? 'bg-amber-400' : 'bg-amber-500'
                    : log.type === 'game'
                    ? isDark ? 'bg-[#5accf5]' : 'bg-sky-500'
                    : isDark ? 'bg-emerald-400' : 'bg-emerald-600'
                }`}
              />
              <span className={`truncate ${isDark ? 'text-[#d8cfd1]' : 'text-slate-700'}`}>&gt; {activityMessage(log.message, t)}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
};
