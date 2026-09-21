import { useState, useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import gsap from 'gsap';
import {
  MonitorInfo,
  MonitorInventorySnapshot,
  ConfigSnapshot,
  HdrStatePayload,
  ManualControlError,
  ManualSetResult,
  ActivityLogEntry,
  RecentGameSession,
} from './types';
import { Dashboard } from './components/Dashboard';
import { AppsManager } from './components/AppsManager';
import { CatalogBrowser } from './components/CatalogBrowser';
import { RunningProcesses } from './components/RunningProcesses';
import { Settings } from './components/Settings';
import { ConfigNotice } from './components/ConfigNotice';
import { configClient, useConfig } from './useConfig';
import { describeHdrScope, statusWarnings } from './telemetryText';
import { DisplayObservationOrder, ManualFeedbackOrder, manualControlAvailable, scopeVisuals } from './displayState';
import { HdrLogo } from './components/HdrLogo';
import { GlitchNavItem } from './components/GlitchNavItem';
import { Sun, Moon, Globe } from 'lucide-react';
import { I18nContext, Language, dictionaries, detectDefaultLanguage } from './i18n';
import { ThemeContext } from './theme';
import './App.css';

type Tab = 'dashboard' | 'apps' | 'catalog' | 'processes' | 'settings';

const DEFAULT_RECENT_GAMES: RecentGameSession[] = [];

const IGNORED_SYSTEM_EXES = new Set([
  'snippingtool.exe',
  'screenclippinghost.exe',
  'explorer.exe',
  'chrome.exe',
  'msedge.exe',
  'firefox.exe',
  'applicationframehost.exe',
  'gamingservicesui.exe',
  'searchhost.exe',
  'shellexperiencehost.exe',
  'startmenuexperiencehost.exe',
  'taskmgr.exe',
  'systemsettings.exe',
  'hdr-autoswitch.exe',
  'tauri-app.exe',
]);

const isMockSession = (item: RecentGameSession): boolean => {
  return (
    item.hook_message === 'WinEventHook: HDR zapnuto -> SDR obnoveno' ||
    item.name === 'Battlefield 6' ||
    item.name === 'Forza Horizon 6' ||
    (item.name === 'Bodycam' && item.last_switched_at === '14:27') ||
    (item.name === 'Assetto Corsa' && item.last_switched_at === '13:45') ||
    (item.name === 'BeamNG.drive' && item.last_switched_at === '11:05') ||
    (item.name === 'Enshrouded' && item.last_switched_at === 'Včera')
  );
};

const normalizeKey = (str?: string | null) => {
  if (!str) return '';
  return str
    .toLowerCase()
    .replace('.exe', '')
    .replace(/-win64-shipping|_win64_shipping|-shipping|_shipping|_dx12|_dx11|_vk/g, '')
    .replace(/[^a-z0-9]/g, '');
};

const isSameGame = (
  a: { exe?: string | null; name?: string | null; steam_id?: string | null },
  b: { exe?: string | null; name?: string | null; steam_id?: string | null }
): boolean => {
  if (a.steam_id && b.steam_id && a.steam_id === b.steam_id) return true;
  if (a.exe && b.exe && a.exe.toLowerCase() === b.exe.toLowerCase()) return true;

  const normNameA = normalizeKey(a.name);
  const normNameB = normalizeKey(b.name);
  if (normNameA.length > 2 && normNameB.length > 2 && normNameA === normNameB) return true;

  const normExeA = normalizeKey(a.exe);
  const normExeB = normalizeKey(b.exe);
  if (normExeA.length > 2 && normExeB.length > 2 && normExeA === normExeB) return true;

  if (normExeA.length > 2 && normNameB.length > 2 && normExeA === normNameB) return true;
  if (normNameA.length > 2 && normExeB.length > 2 && normNameA === normExeB) return true;

  return false;
};

function dedupeRecentGameList(list: RecentGameSession[]): RecentGameSession[] {
  const result: RecentGameSession[] = [];
  for (const item of list) {
    if (!item.exe || IGNORED_SYSTEM_EXES.has(item.exe.toLowerCase())) continue;
    if (isMockSession(item)) continue; // Purge prototype mock data
    const existingIndex = result.findIndex((r) => isSameGame(r, item));
    if (existingIndex >= 0) {
      const existing = result[existingIndex];
      result[existingIndex] = {
        ...existing,
        name: existing.name || item.name,
        steam_id: existing.steam_id || item.steam_id,
        launcher: existing.launcher || item.launcher,
        hdr_tier_label: existing.hdr_tier_label || item.hdr_tier_label,
        hdr_type: existing.hdr_type || item.hdr_type,
      };
    } else {
      result.push(item);
    }
  }
  return result;
}

export default function App() {
  const [activeTab, setActiveTab] = useState<Tab>('dashboard');
  const [isDark, setIsDark] = useState(true);
  const [lang, setLang] = useState<Language>(detectDefaultLanguage);

  const handleSetLang = (newLang: Language) => {
    setLang(newLang);
    localStorage.setItem('hdr_lang', newLang);
  };

  const t = dictionaries[lang];

  useEffect(() => {
    document.documentElement.lang = lang;
    invoke('set_ui_language', { language: lang }).catch(configClient.reportError);
  }, [lang]);

  const [monitors, setMonitors] = useState<MonitorInfo[]>([]);
  const { snapshot, pending } = useConfig();
  const config = snapshot?.mode === 'ready' ? snapshot.settings : null;
  const monitorRequest = useRef(0);
  const statusRequest = useRef(0);
  const displayOrder = useRef(new DisplayObservationOrder());
  const monitorRefreshRevision = useRef<string | null>(null);
  const [statusLoaded, setStatusLoaded] = useState(false);
  const manualFeedback = useRef(new ManualFeedbackOrder());
  const [controlErrors, setControlErrors] = useState<string[]>([]);
  const [monitorError, setMonitorError] = useState<string | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const loggedObservation = useRef<string | null>(null);

  const [status, setStatus] = useState<HdrStatePayload>({
    status_revision: '0',
    inventory_revision: '0',
    manual_revision: '0',
    manual_results: [],
    is_hdr_active: false,
    scope_hdr_state: 'unknown',
    manual_control: { status: 'blocked', reason: 'HDR controller starting' },
    current_app_name: null,
    current_exe: null,
    switched_by_app: false,
    steam_id: null,
    launcher: null,
    hdr_type: null,
    warning: null,
    quarantined_apps: [],
    target_status: 'automation_paused',
    active_target: null,
    target_deferred: false,
    any_hdr_active: false,
    inventory_stale: true,
    uncertain_targets: [],
    operation_outcomes: [],
  });
  const scopePresentation = describeHdrScope(status, t);
  const headerMode = statusLoaded ? scopePresentation.mode : 'unknown';
  const headerVisuals = scopeVisuals[headerMode];

  const [recentGames, setRecentGames] = useState<RecentGameSession[]>(() => {
    try {
      const saved = localStorage.getItem('hdr_recent_games');
      if (saved) {
        const parsed = JSON.parse(saved);
        if (Array.isArray(parsed)) {
          const sanitized = dedupeRecentGameList(parsed);
          localStorage.setItem('hdr_recent_games', JSON.stringify(sanitized));
          return sanitized;
        }
      }
    } catch (e) {
      console.error('Error loading recent games:', e);
    }
    return DEFAULT_RECENT_GAMES;
  });

  const [activityLogs, setActivityLogs] = useState<ActivityLogEntry[]>([
    {
      id: '1',
      timestamp: new Date().toISOString(),
      message: { kind: 'init_system' },
      type: 'system',
    },
    {
      id: '2',
      timestamp: new Date().toISOString(),
      message: { kind: 'init_detect' },
      type: 'info',
    },
  ]);

  const addLog = (message: ActivityLogEntry['message'], type: ActivityLogEntry['type']) => {
    const entry: ActivityLogEntry = {
      id: Math.random().toString(36).substring(2, 9),
      timestamp: new Date().toISOString(),
      message,
      type,
    };
    setActivityLogs((prev) => [entry, ...prev.slice(0, 19)]);
  };

  const refreshMonitors = async () => {
    const request = ++monitorRequest.current;
    try {
      const inventory: MonitorInventorySnapshot = await invoke('get_monitors');
      if (request === monitorRequest.current && displayOrder.current.acceptInventory(inventory)) {
        setMonitors(inventory.monitors);
        setMonitorError(null);
        if (!displayOrder.current.statusCurrent) {
          setStatusLoaded(false);
          void refreshStatus();
        }
      }
    } catch (err) {
      if (request === monitorRequest.current) {
        displayOrder.current.invalidateInventory();
        setMonitors([]);
        setMonitorError(String(err));
      }
    } finally {
      if (request === monitorRequest.current) monitorRefreshRevision.current = null;
    }
  };

  const acceptStatus = (next: HdrStatePayload): boolean => {
    if (manualFeedback.current.acceptStatus(next)) {
      setControlErrors(manualFeedback.current.errors());
    }
    if (!displayOrder.current.acceptStatus(next)) return false;
    setStatus(next);
    setStatusLoaded(true);
    setStatusError(null);
    if (next.inventory_stale) {
      ++monitorRequest.current;
      monitorRefreshRevision.current = null;
      displayOrder.current.invalidateInventory();
      setMonitors([]);
    } else if (displayOrder.current.needsInventory) {
      setMonitors([]);
      if (monitorRefreshRevision.current !== next.inventory_revision) {
        monitorRefreshRevision.current = next.inventory_revision;
        void refreshMonitors();
      }
    }
    return true;
  };

  const acceptManualResult = (result: ManualSetResult) => {
    if (acceptStatus(result.status)) ++statusRequest.current;
  };

  const reportControlError = (error: ManualControlError) => {
    if (manualFeedback.current.acceptError(error)) {
      setControlErrors(manualFeedback.current.errors());
    }
  };

  const refreshStatus = async () => {
    const request = ++statusRequest.current;
    try {
      const stat: HdrStatePayload = await invoke('get_current_status');
      if (request === statusRequest.current) {
        acceptStatus(stat);
      }
    } catch (err) {
      if (request === statusRequest.current) {
        setStatusLoaded(false);
        setStatusError(String(err));
      }
    }
  };

  useEffect(() => {
    refreshMonitors();
    let active = true;

    // Listen for live HDR status changes from Rust WinEventHook
    const unlistenPromise = listen<HdrStatePayload>('hdr-status-changed', (event) => {
      const newStatus = event.payload;
      if (!active || !acceptStatus(newStatus)) return;
      ++statusRequest.current;

      if (describeHdrScope(newStatus, dictionaries.en).mode === 'unknown') return;
      const observation = JSON.stringify([
        newStatus.scope_hdr_state, newStatus.current_exe, newStatus.switched_by_app,
      ]);
      if (loggedObservation.current === observation) return;
      loggedObservation.current = observation;
      const currentTime = new Date().toISOString();

      if (newStatus.scope_hdr_state === 'mixed') {
        addLog({ kind: 'mixed' }, 'info');
      } else if (newStatus.scope_hdr_state === 'hdr') {
        addLog(
          newStatus.current_app_name && newStatus.switched_by_app
            ? { kind: 'game_hdr', appName: newStatus.current_app_name }
            : { kind: 'hdr_active' },
          'hdr_on'
        );
      } else {
        addLog({ kind: 'sdr' }, 'hdr_off');
      }

      // Update Recent Games telemetry (only for actual games detected by hook)
      if (
        newStatus.switched_by_app &&
        newStatus.current_exe &&
        !IGNORED_SYSTEM_EXES.has(newStatus.current_exe.toLowerCase())
      ) {
        setRecentGames((prev) => {
          const candidate = {
            exe: newStatus.current_exe,
            name: newStatus.current_app_name,
            steam_id: newStatus.steam_id,
          };

          const existing = prev.find((g) => isSameGame(g, candidate));

          const resolvedSteamId =
            newStatus.steam_id ||
            existing?.steam_id ||
            undefined;

          const resolvedLauncher =
            newStatus.launcher ||
            existing?.launcher ||
            (resolvedSteamId ? 'Steam' : undefined);

          const resolvedHdrType =
            newStatus.hdr_type ||
            existing?.hdr_type ||
            'native';

          const updatedSession: RecentGameSession = {
            exe: newStatus.current_exe!,
            name: newStatus.current_app_name || existing?.name || newStatus.current_exe!,
            steam_id: resolvedSteamId,
            launcher: resolvedLauncher,
            hdr_type: resolvedHdrType,
            last_switched_at: currentTime,
            hook_status: 'active',
          };

          const filtered = prev.filter(
            (g) =>
              !isSameGame(g, updatedSession) &&
              !IGNORED_SYSTEM_EXES.has(g.exe.toLowerCase())
          );

          const newList = [updatedSession, ...filtered].slice(0, 10);
          try {
            localStorage.setItem('hdr_recent_games', JSON.stringify(newList));
          } catch (e) {
            console.error(e);
          }
          return newList;
        });
      } else if (!newStatus.switched_by_app) {
        // This records released automatic control, not every display becoming SDR.
        setRecentGames((prev) => {
          const newList = prev.map((g, idx) =>
            idx === 0 && g.hook_status === 'active'
              ? {
                ...g,
                hook_status: 'switched_off' as const,
                hook_message: undefined,
              }
              : g
          );
          try {
            localStorage.setItem('hdr_recent_games', JSON.stringify(newList));
          } catch (e) {
            console.error(e);
          }
          return newList;
        });
      }
    });

    const unlistenConfigPromise = listen<ConfigSnapshot>('config-changed', (event) => {
      configClient.acceptEvent(event.payload);
      void refreshMonitors();
    });
    const unlistenControlPromise = listen<ManualControlError>('controller-error', (event) => {
      if (active) reportControlError(event.payload);
    });
    const unlistenManualPromise = listen<ManualSetResult>('manual-control-result', (event) => {
      if (active) acceptManualResult(event.payload);
    });
    const unlistenNavigationPromise = listen('navigate-settings', () => setActiveTab('settings'));
    unlistenControlPromise.catch(configClient.reportError);
    unlistenManualPromise.catch(configClient.reportError);
    unlistenNavigationPromise.catch(configClient.reportError);
    unlistenConfigPromise.then(() => {
      if (active) void configClient.refresh();
    }).catch(configClient.reportError);
    unlistenPromise.then(() => {
      if (active) void refreshStatus();
    }).catch(configClient.reportError);

    return () => {
      active = false;
      ++monitorRequest.current;
      ++statusRequest.current;
      unlistenPromise.then((unlisten) => unlisten()).catch(configClient.reportError);
      unlistenConfigPromise.then((unlisten) => unlisten()).catch(configClient.reportError);
      unlistenControlPromise.then((unlisten) => unlisten()).catch(configClient.reportError);
      unlistenManualPromise.then((unlisten) => unlisten()).catch(configClient.reportError);
      unlistenNavigationPromise.then((unlisten) => unlisten()).catch(configClient.reportError);
    };
  }, []);

  // Zero-overhead GUI Hibernation when minimized or hidden to system tray:
  useEffect(() => {
    const handleVisibilityChange = () => {
      if (document.hidden) {
        // App is hidden in system tray or minimized: pause GSAP ticker & animation loops for 0.0% CPU overhead
        gsap.ticker.sleep();
      } else {
        // Window restored / focused from tray: wake up GSAP ticker and refresh state
        gsap.ticker.wake();
        refreshMonitors();
        refreshStatus();
        void configClient.refresh();
      }
    };

    document.addEventListener('visibilitychange', handleVisibilityChange);
    return () => {
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  }, []);

  return (
    <I18nContext.Provider value={{ lang, setLang: handleSetLang, t }}>
      <ThemeContext.Provider value={{ isDark }}>
        <div
          className={`min-h-screen flex flex-col transition-colors duration-200 font-mono ${
            isDark ? 'bg-retro-dark text-[#e5e0e1]' : 'bg-retro-light text-slate-900'
          }`}
        >
          {/* Top Header Bar - CodePen Retro Glitch Aesthetic */}
          <header
            className={`sticky top-0 z-30 px-6 py-2.5 border-b transition-colors ${
              isDark
                ? 'bg-[#0f0b0b]/95 border-[#f55a6b]/30'
                : 'bg-white/95 border-[#f55a6b]/30 shadow-xs'
            }`}
          >
            <div className="max-w-7xl mx-auto flex flex-wrap items-center justify-between gap-4">
              {/* Brand Logo & Name with Solid Glitch Title Bar */}
              <div className="flex items-center gap-3">
                <div
                  className={`p-1.5 border flex items-center justify-center shrink-0 aspect-square ${
                    isDark ? 'border-[#f55a6b]/40 bg-[#180e10]' : 'border-[#f55a6b]/30 bg-rose-50/50'
                  }`}
                >
                  <HdrLogo size={28} mode={headerMode} />
                </div>

                <div className="flex items-center gap-2.5">
                  <h1 className="glitch-title-bar px-2 py-0.5 text-xs font-bold tracking-wider inline-block">
                    {t.appTitle}
                  </h1>

                  {/* Status Pill Badge */}
                  <div
                    className={`inline-flex items-center gap-1.5 px-2 py-0.5 text-[11px] font-bold tracking-wider border uppercase transition-all ${
                      isDark
                        ? headerVisuals.badge
                        : headerMode === 'hdr'
                        ? 'bg-[#f55a6b] text-white border-[#f55a6b]'
                        : headerMode === 'sdr'
                        ? 'bg-sky-100 text-sky-900 border-sky-300'
                        : headerMode === 'mixed'
                        ? 'bg-amber-100 text-amber-900 border-amber-400'
                        : 'bg-slate-100 text-slate-800 border-slate-400 border-dashed'
                    }`}
                  >
                    <span
                      className={`w-1.5 h-1.5 ${
                        isDark
                          ? headerVisuals.dot
                          : headerMode === 'hdr'
                          ? 'bg-white animate-status-pulse'
                          : headerMode === 'sdr'
                          ? 'bg-sky-600'
                          : headerMode === 'mixed'
                          ? 'bg-amber-600'
                          : 'border border-slate-500'
                      }`}
                    />
                    <span>{!statusLoaded
                      ? t.configStatusUnknown
                      : scopePresentation.badge}</span>
                  </div>
                </div>
              </div>

              {/* Glitch Navigation Bar (GSAP SVG Displacement from CodePen) */}
              <nav className="flex items-center gap-2">
                <GlitchNavItem
                  label={t.navOverview}
                  isActive={activeTab === 'dashboard'}
                  onClick={() => setActiveTab('dashboard')}
                  width={lang === 'en' ? 125 : 125}
                  height={36}
                  isDark={isDark}
                />
                <GlitchNavItem
                  label={t.navApps}
                  count={config?.apps.length ?? 0}
                  isActive={activeTab === 'apps'}
                  onClick={() => setActiveTab('apps')}
                  width={lang === 'en' ? 140 : 145}
                  height={36}
                  isDark={isDark}
                />
                <GlitchNavItem
                  label={t.navCatalog}
                  isActive={activeTab === 'catalog'}
                  onClick={() => setActiveTab('catalog')}
                  width={lang === 'en' ? 140 : 140}
                  height={36}
                  isDark={isDark}
                />
                <GlitchNavItem
                  label={t.navProcesses}
                  isActive={activeTab === 'processes'}
                  onClick={() => setActiveTab('processes')}
                  width={lang === 'en' ? 150 : 135}
                  height={36}
                  isDark={isDark}
                />
                <GlitchNavItem
                  label={t.navSettings}
                  isActive={activeTab === 'settings'}
                  onClick={() => setActiveTab('settings')}
                  width={lang === 'en' ? 125 : 125}
                  height={36}
                  isDark={isDark}
                />
              </nav>

              {/* Right Controls: Language & Theme Switch */}
              <div className="flex items-center gap-2">
                {/* Language Switch Button */}
                <button
                  onClick={() => handleSetLang(lang === 'cs' ? 'en' : 'cs')}
                  className={`px-2 py-1 border text-xs font-bold transition-all cursor-pointer flex items-center gap-1.5 ${
                    isDark
                      ? 'border-[#f55a6b]/30 bg-[#180e10] text-[#5accf5] hover:border-[#f55a6b] hover:shadow-[0_0_10px_rgba(245,90,107,0.3)]'
                      : 'border-[#f55a6b]/40 bg-white text-[#e03e52] hover:bg-slate-50 shadow-xs'
                  }`}
                  title={t.langToggle}
                >
                  <Globe className="w-3.5 h-3.5" />
                  <span>{lang.toUpperCase()}</span>
                </button>

                <button
                  onClick={() => setIsDark(!isDark)}
                  className={`p-1.5 border transition-all cursor-pointer ${
                    isDark
                      ? 'border-[#f55a6b]/30 bg-[#180e10] text-[#5accf5] hover:border-[#f55a6b] hover:shadow-[0_0_10px_rgba(245,90,107,0.4)]'
                      : 'border-[#f55a6b]/40 bg-white text-[#e03e52] hover:bg-slate-50 shadow-xs'
                  }`}
                  title={t.themeToggle}
                >
                  {isDark ? <Sun className="w-3.5 h-3.5" /> : <Moon className="w-3.5 h-3.5" />}
                </button>
              </div>
            </div>
          </header>

        {/* Main Content Body */}
        <main className="flex-1 max-w-7xl w-full mx-auto p-6">
          <ConfigNotice onSettings={() => setActiveTab('settings')} />
          {[...controlErrors, monitorError, statusError].filter(Boolean).map((error, index) =>
            <p key={index} role="alert" className={`mb-4 p-3 border text-xs ${
              isDark ? 'border-amber-400/50 text-amber-200 bg-amber-950/20' : 'border-amber-500/40 text-amber-900 bg-amber-50 shadow-xs'
            }`}>{t.configError} {error}</p>
          )}
          {controlErrors.length > 0 && <p className={`mb-4 text-xs ${isDark ? 'text-amber-200' : 'text-amber-900'}`}>
            {t.manualRequestRetryHint}
          </p>}
          {statusWarnings(status, t).map((warning) => (
            <p key={warning} role="alert" className={`mb-4 p-3 border text-xs ${
              isDark ? 'border-amber-400/50 text-amber-200 bg-amber-950/20' : 'border-amber-500/40 text-amber-900 bg-amber-50 shadow-xs'
            }`}>{warning}</p>
          ))}
          {status.target_deferred && status.active_target && <p className={`mb-4 text-xs ${isDark ? 'text-[#5accf5]' : 'text-sky-700 font-semibold'}`}>
            {t.configActiveTarget}: {status.active_target.kind === 'all'
              ? t.settingsAllMonitors
              : status.active_target.kind === 'monitor' ? status.active_target.display_name : t.configConfirmTarget}.
            {' '}{t.configTargetHint}
          </p>}
          {activeTab === 'dashboard' && (
            <Dashboard
              status={statusLoaded ? status : { ...status, inventory_stale: true }}
              monitors={monitors}
              libraryCount={config?.apps.length ?? null}
              activityLogs={activityLogs}
              recentGames={recentGames}
              onRefreshMonitors={() => {
                void refreshMonitors();
                void refreshStatus();
              }}
              onManualToggle={() => {
                void refreshStatus();
                void refreshMonitors();
              }}
              onNavigateToApps={() => setActiveTab('apps')}
              onControlError={reportControlError}
              onManualResult={acceptManualResult}
              captureManualOrigin={(scope) => manualFeedback.current.capture(scope)}
              controlAvailable={manualControlAvailable(status, statusLoaded)}
              isDark={isDark}
            />
          )}

          <fieldset disabled={pending} className={`min-w-0 ${pending ? 'pointer-events-none opacity-70' : ''}`}>
          {config && activeTab === 'apps' && (
            <AppsManager
              quarantinedRows={status.quarantined_apps ?? []}
              config={config}
              isDark={isDark}
              onNavigateToCatalog={() => setActiveTab('catalog')}
            />
          )}

          {config && activeTab === 'catalog' && (
            <CatalogBrowser
              config={config}
              isDark={isDark}
            />
          )}

          {config && activeTab === 'processes' && (
            <RunningProcesses
              config={config}
              isDark={isDark}
            />
          )}

          {config && activeTab === 'settings' && (
            <Settings
              config={config}
              monitors={monitors}
              isDark={isDark}
            />
          )}
          </fieldset>
        </main>
      </div>
    </ThemeContext.Provider>
  </I18nContext.Provider>
  );
}
