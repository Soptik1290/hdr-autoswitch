import React, { useState, useEffect } from 'react';
import { HdrApp, HdrType, AppConfig, PickedGameInfo, QuarantinedApp, ScanGame, ScanResult } from '../types';
import type { MutationOrigin } from '../configState';
import { configClient } from '../useConfig';
import { captureLibraryRow, findLibraryApp, pathAfterExecutableEdit, primaryExecutable } from '../libraryState';
import { detectionKey, importSelectedDetections, selectDetections, selectedDetections, toggleDetection } from '../scanSelection';
import { launcherName } from '../catalogNotes';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  Search,
  Plus,
  Compass,
  Trash2,
  ScanSearch,
  Sparkles,
  Gamepad2,
  LayoutGrid,
  List as ListIcon,
  ShieldCheck,
  Zap,
  Check,
  X,
  FolderOpen,
  UploadCloud,
  AlertCircle,
  RefreshCw,
} from 'lucide-react';
import { GlitchButton } from './GlitchButton';
import { GlitchText } from './GlitchText';
import { useI18n } from '../i18n';

interface AppsManagerProps {
  config: AppConfig;
  quarantinedRows: QuarantinedApp[];
  onNavigateToCatalog: () => void;
  isDark: boolean;
}

export const AppsManager: React.FC<AppsManagerProps> = ({
  config,
  quarantinedRows,
  onNavigateToCatalog,
  isDark,
}) => {
  const { t, lang } = useI18n();
  const [search, setSearch] = useState('');
  const [viewMode, setViewMode] = useState<'grid' | 'list'>('grid');
  const [selectedLauncher, setSelectedLauncher] = useState<string>('all');
  const [isScanning, setIsScanning] = useState(false);
  const [scanMessage, setScanMessage] = useState<string | null>(null);

  // Scan modal
  const [showScanModal, setShowScanModal] = useState(false);
  const [scannedGames, setScannedGames] = useState<ScanGame[]>([]);
  const [scanOrigin, setScanOrigin] = useState<MutationOrigin | null>(null);
  const [selectedToImport, setSelectedToImport] = useState<Record<string, boolean>>({});
  const [pathStatus, setPathStatus] = useState<Record<string, boolean>>({});
  const isQuarantined = (app: HdrApp) =>
    quarantinedRows.some((row) => row.row_index === config.apps.indexOf(app)
      && row.exe_name === app.exe_name && row.path === (app.path ?? null));

  const handleRepairExecutable = async (app: HdrApp) => {
    try {
      const { row, origin } = captureLibraryRow(configClient, config.apps, app);
      const picked = await invoke<PickedGameInfo | null>('pick_game_exe', { language: lang });
      if (!picked) return;
      await configClient.mutate('repair_app_executable', {
        row, path: picked.path,
      }, origin);
    } catch (err) {
      configClient.reportError(err);
    }
  };

  // Verify paths of apps currently in the user's library
  useEffect(() => {
    const paths = config.apps
      .map((a) => a.path)
      .filter((p): p is string => !!p && p.trim().length > 0);

    if (paths.length > 0) {
      invoke<Record<string, boolean>>('verify_game_paths', { paths })
        .then((status) => setPathStatus(status))
        .catch((err) => console.error('Failed to verify app paths:', err));
    }
  }, [config.apps]);

  const findExistingApp = (item: ScanGame): HdrApp | undefined =>
    findLibraryApp(config.apps, item);

  const isPathDifferent = (existing: HdrApp, scanned: ScanGame): boolean => {
    if (existing.exe_name.toLowerCase() !== scanned.exe_name.toLowerCase()
        && !isQuarantined(existing)) return false;
    if (!scanned.path) return false;
    if (!existing.path) return true; // Path missing previously, now found on disk!
    const normOld = existing.path.replace(/\//g, '\\').toLowerCase().trim();
    const normNew = scanned.path.replace(/\//g, '\\').toLowerCase().trim();
    return normOld !== normNew;
  };

  // Manual Add Modal & File Picker
  const [showAddModal, setShowAddModal] = useState(false);
  const [newName, setNewName] = useState('');
  const [newExe, setNewExe] = useState('');
  const [newPath, setNewPath] = useState('');
  const [newType, setNewType] = useState<HdrType>('native');
  const [isHdrMatched, setIsHdrMatched] = useState(false);
  const [isDraggingOver, setIsDraggingOver] = useState(false);

  const handleToggleApp = async (app: HdrApp, enabled: boolean) => {
    try {
      const { row, origin } = captureLibraryRow(configClient, config.apps, app);
      await configClient.mutate('toggle_app', { row, enabled }, origin);
    } catch (err) {
      configClient.reportError(err);
    }
  };

  const handleDeleteApp = async (app: HdrApp) => {
    try {
      const { row, origin } = captureLibraryRow(configClient, config.apps, app);
      await configClient.mutate('remove_app', { row }, origin);
    } catch (err) {
      configClient.reportError(err);
    }
  };

  const handleStartScan = async () => {
    setIsScanning(true);
    setScanMessage(null);
    try {
      const origin = configClient.captureOrigin();
      const result = await invoke<ScanResult>('scan_installed_games', {
        expectedContext: origin.contextToken,
        expectedLibraryGeneration: origin.libraryGeneration,
      });
      const detected = result.games;
      setScanOrigin({
        contextToken: result.context_token,
        libraryGeneration: result.library_generation,
      });
      setScannedGames(detected);

      const initialSelected = selectDetections(detected, (item) => {
        const existing = findExistingApp(item);
        if (existing) {
          const pathChanged = isPathDifferent(existing, item);
          // If game is already tracked:
          // - If path moved or was newly discovered: PRE-SELECT to update path!
          // - If already up to date: uncheck by default
          return item.default_selected && pathChanged;
        } else {
          return item.default_selected;
        }
      });
      setSelectedToImport(initialSelected);
      setShowScanModal(true);
    } catch (err) {
      console.error('Failed to scan installed games:', err);
      configClient.reportError(err);
      setScanMessage(t.scanModalError);
    } finally {
      setIsScanning(false);
    }
  };

  const selectAllHdr = () => {
    setSelectedToImport(selectDetections(scannedGames, (item) => item.is_hdr_supported));
  };

  const selectAll = () => {
    setSelectedToImport(selectDetections(scannedGames, () => true));
  };

  const deselectAll = () => {
    setSelectedToImport({});
  };

  const handleBrowseExe = async () => {
    try {
      const picked: PickedGameInfo | null = await invoke('pick_game_exe', { language: lang });
      if (picked) {
        setNewName(picked.name);
        setNewExe(picked.exe_name);
        setNewPath(picked.path);
        setNewType(picked.hdr_type);
        setIsHdrMatched(picked.is_hdr_supported);
        setShowAddModal(true);
      }
    } catch (err) {
      console.error('Failed to pick game exe:', err);
      configClient.reportError(err);
    }
  };

  useEffect(() => {
    const unlistenPromise = listen<{ paths?: string[] }>('tauri://drag-drop', async (event) => {
      const paths = event.payload?.paths;
      if (paths && paths.length > 0) {
        const exePath = paths.find((p) => p.toLowerCase().endsWith('.exe'));
        if (exePath) {
          try {
            const picked: PickedGameInfo = await invoke('inspect_exe_path', { path: exePath });
            setNewName(picked.name);
            setNewExe(picked.exe_name);
            setNewPath(picked.path);
            setNewType(picked.hdr_type);
            setIsHdrMatched(picked.is_hdr_supported);
            setShowAddModal(true);
          } catch (err) {
            console.error('Failed to inspect dropped exe:', err);
            configClient.reportError(err);
          }
        }
      }
    });

    return () => {
      unlistenPromise.then((un) => un());
    };
  }, []);

  const handleConfirmImport = async () => {
    const toImport = selectedDetections(scannedGames, selectedToImport);

    if (toImport.length === 0) {
      setShowScanModal(false);
      return;
    }

    try {
      if (!scanOrigin) throw new Error(t.scanModalError);
      await importSelectedDetections(configClient, scannedGames, selectedToImport, scanOrigin);
      setShowScanModal(false);
      setScanMessage(t.scanModalSuccess(toImport.length));
      setTimeout(() => setScanMessage(null), 4500);
    } catch (err) {
      configClient.reportError(err);
    }
  };

  const handleAddCustomApp = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newName.trim() || !newExe.trim()) return;

    const cleanExe = primaryExecutable(newExe);

    const newApp: HdrApp = {
      name: newName.trim(),
      exe_name: cleanExe,
      enabled: true,
      hdr_type: newType,
      path: newPath || undefined,
    };

    try {
      await configClient.mutate('add_custom_app', { app: newApp });
      setShowAddModal(false);
      setNewName('');
      setNewExe('');
      setNewPath('');
      setIsHdrMatched(false);
      setNewType('native');
    } catch (err) {
      configClient.reportError(err);
    }
  };

  const filteredApps = config.apps.filter((app) => {
    const matchesSearch =
      app.name.toLowerCase().includes(search.toLowerCase()) ||
      app.exe_name.toLowerCase().includes(search.toLowerCase());

    const matchesLauncher =
      selectedLauncher === 'all'
        ? true
        : selectedLauncher === 'steam'
        ? !!app.steam_id || app.launcher?.toLowerCase() === 'steam'
        : app.launcher?.toLowerCase() === selectedLauncher.toLowerCase();

    return matchesSearch && matchesLauncher;
  });

  const getHdrBadge = (hdrType: HdrType) => {
    switch (hdrType) {
      case 'native':
        return (
          <span className="inline-flex items-center gap-1 text-[9px] font-mono px-1.5 py-0.5 bg-cyan-950/90 text-[#5accf5] border border-[#5accf5]/50 font-bold uppercase tracking-wider">
            <ShieldCheck className="w-3 h-3 text-[#5accf5]" /> {t.recentTierNative}
          </span>
        );
      case 'autohdr':
        return (
          <span className="inline-flex items-center gap-1 text-[9px] font-mono px-1.5 py-0.5 bg-purple-950/90 text-purple-300 border border-purple-500/50 font-bold uppercase tracking-wider">
            <Zap className="w-3 h-3 text-purple-400" /> {t.recentTierAutoHdr}
          </span>
        );
      case 'media':
        return (
          <span className="inline-flex items-center gap-1 text-[9px] font-mono px-1.5 py-0.5 bg-blue-950/90 text-blue-300 border border-blue-500/50 font-bold uppercase tracking-wider">
            {t.catalogTierMedia.toUpperCase()}
          </span>
        );
      default:
        return (
          <span className="inline-flex items-center gap-1 text-[9px] font-mono px-1.5 py-0.5 bg-amber-950/90 text-amber-300 border border-amber-500/50 font-bold uppercase tracking-wider">
            {t.recentTierMod}
          </span>
        );
    }
  };

  const steamCount = config.apps.filter((a) => a.steam_id || a.launcher === 'Steam').length;
  const activeCount = config.apps.filter((a) => a.enabled).length;

  return (
    <div className="space-y-5 font-mono">
      {/* Top Header */}
      <div className="flex flex-col md:flex-row md:items-center justify-between gap-4">
        <div>
          <div className="flex items-center gap-2.5">
            <h2 className="glitch-title-bar px-2.5 py-0.5 text-xs font-bold tracking-wider inline-block">
              {t.appsTitle}
            </h2>
            <span className={`text-xs px-2 py-0.5 border ${
              isDark
                ? 'border-[#5accf5]/40 text-[#5accf5] bg-[#140e10]'
                : 'border-sky-300 text-sky-800 bg-sky-50 shadow-xs'
            }`}>
              {t.appsCountSummary(config.apps.length, activeCount)}
            </span>
          </div>
          <p className={`text-xs mt-1 ${isDark ? 'text-[#8a7f81]' : 'text-slate-500'}`}>
            {t.appsSubtitle}
          </p>
        </div>

        {/* Top Action Buttons with CodePen Glitch styling */}
        <div className="flex items-center gap-2.5 flex-wrap">
          <GlitchButton
            label={isScanning ? t.appsScanningBtn : t.appsScanBtn}
            variant="primary"
            size="sm"
            disabled={isScanning}
            icon={<ScanSearch className={`w-3.5 h-3.5 ${isScanning ? 'animate-spin' : ''}`} />}
            onClick={handleStartScan}
          />

          <GlitchButton
            label={t.appsAddManualBtn}
            variant="outline"
            size="sm"
            icon={<Plus className={`w-3.5 h-3.5 ${isDark ? 'text-[#5accf5]' : 'text-[#e03e52]'}`} />}
            onClick={() => setShowAddModal(true)}
          />

          <GlitchButton
            label={t.appsCatalogBtn}
            variant="outline"
            size="sm"
            icon={<Compass className={`w-3.5 h-3.5 ${isDark ? 'text-[#f55a6b]' : 'text-[#e03e52]'}`} />}
            onClick={onNavigateToCatalog}
          />
        </div>
      </div>

      {scanMessage && (
        <div className={`p-3 border text-xs flex items-center gap-2.5 ${
          isDark
            ? 'border-[#5accf5]/40 bg-[#120e10] text-[#5accf5]'
            : 'border-sky-300 bg-sky-50 text-sky-800 shadow-xs'
        }`}>
          <Sparkles className={`w-4 h-4 shrink-0 ${isDark ? 'text-[#5accf5]' : 'text-sky-600'}`} />
          <span>&gt; {scanMessage}</span>
        </div>
      )}

      {/* Filter and View Mode Toolbar */}
      <div className="flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-3">
        {/* Search input */}
        <div className="relative flex-1">
          <Search className={`w-4 h-4 absolute left-3.5 top-1/2 -translate-y-1/2 ${isDark ? 'text-[#8a7f81]' : 'text-slate-400'}`} />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t.appsSearchPlaceholder}
            className={`w-full pl-9 pr-4 py-2 text-xs border focus:outline-none transition-all ${
              isDark
                ? 'border-[#f55a6b]/30 bg-[#120d0e] focus:border-[#f55a6b] text-white placeholder-[#8a7f81]'
                : 'border-slate-300 bg-white focus:border-[#f55a6b] text-slate-900 placeholder-slate-400 shadow-xs'
            }`}
          />
        </div>

        {/* View mode toggle (Grid vs List) */}
        <div className={`flex items-center gap-1 p-1 border ${
          isDark ? 'bg-[#120d0e] border-[#f55a6b]/30' : 'bg-white border-slate-300 shadow-xs'
        }`}>
          <button
            onClick={() => setViewMode('grid')}
            className={`p-1 text-xs cursor-pointer transition-all ${
              viewMode === 'grid'
                ? 'bg-[#f55a6b] text-white'
                : isDark ? 'text-[#8a7f81] hover:text-white' : 'text-slate-500 hover:text-slate-900'
            }`}
            title={t.appsGridView}
          >
            <LayoutGrid className="w-3.5 h-3.5" />
          </button>
          <button
            onClick={() => setViewMode('list')}
            className={`p-1 text-xs cursor-pointer transition-all ${
              viewMode === 'list'
                ? 'bg-[#f55a6b] text-white'
                : isDark ? 'text-[#8a7f81] hover:text-white' : 'text-slate-500 hover:text-slate-900'
            }`}
            title={t.appsListView}
          >
            <ListIcon className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      {/* Launcher Filter Tabs */}
      <div className="flex items-center gap-1.5 overflow-x-auto pb-1">
        {[
          { id: 'all', label: t.appsTabAll(config.apps.length) },
          { id: 'steam', label: t.appsTabSteam(steamCount) },
          { id: 'epic games', label: t.appsTabEpic },
          { id: 'windows', label: t.appsTabWindows },
        ].map((tab) => (
          <button
            key={tab.id}
            onClick={() => setSelectedLauncher(tab.id)}
            className={`px-3 py-1 text-xs uppercase font-bold cursor-pointer transition-all border ${
              selectedLauncher === tab.id
                ? isDark
                  ? 'bg-[#f55a6b] text-[#0f0b0b] border-[#f55a6b] neon-glow-coral'
                  : 'bg-[#f55a6b] text-white border-[#f55a6b] shadow-xs'
                : isDark
                ? 'bg-[#120d0e] text-[#8a7f81] border-[#f55a6b]/20 hover:border-[#f55a6b]/50 hover:text-white'
                : 'bg-white text-slate-700 border-slate-200 hover:border-slate-400 hover:text-slate-900 shadow-xs'
            }`}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* Main Content: Posters Grid or List */}
      {filteredApps.length === 0 ? (
        <div className={`p-16 text-center border space-y-4 ${
          isDark ? 'border-[#f55a6b]/20 bg-[#120d0e]' : 'border-slate-200 bg-white shadow-xs'
        }`}>
          <Gamepad2 className={`w-12 h-12 mx-auto stroke-1 ${isDark ? 'text-[#8a7f81]' : 'text-slate-400'}`} />
          <div className="space-y-1">
            <h3 className={`text-sm font-bold ${isDark ? 'text-white' : 'text-slate-900'}`}>
              {search ? t.appsNoGamesSearchTitle : t.appsNoGamesTitle}
            </h3>
            <p className={`text-xs max-w-md mx-auto ${isDark ? 'text-[#8a7f81]' : 'text-slate-500'}`}>
              {t.appsNoGamesSubtitle}
            </p>
          </div>
          {!search && (
            <div className="flex items-center justify-center gap-3 pt-2">
              <GlitchButton
                label={t.appsScanDisksBtn}
                variant="primary"
                size="sm"
                onClick={handleStartScan}
              />
              <GlitchButton
                label={t.appsBrowseCatalogBtn}
                variant="outline"
                size="sm"
                onClick={onNavigateToCatalog}
              />
            </div>
          )}
        </div>
      ) : viewMode === 'grid' ? (
        /* Poster Cards Grid (2:3 aspect ratio) */
        <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6 gap-3.5">
          {filteredApps.map((app) => {
            const steamCover = app.steam_id
              ? `https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/${app.steam_id}/library_600x900.jpg`
              : null;

            return (
              <div
                key={config.apps.indexOf(app)}
                className={`relative group border overflow-hidden flex flex-col justify-between transition-all duration-200 ${
                  app.enabled
                    ? isDark
                      ? 'bg-[#120d0e] border-[#f55a6b]/35 hover:border-[#f55a6b] hover:shadow-[0_0_15px_rgba(245,90,107,0.3)]'
                      : 'bg-white border-slate-200 hover:border-[#f55a6b] hover:shadow-md'
                    : isDark
                    ? 'bg-[#120d0e]/60 border-white/10 opacity-65'
                    : 'bg-slate-100/70 border-slate-200 opacity-65'
                }`}
                style={{ height: '240px' }}
              >
                {/* Poster Artwork with Scanlines */}
                <div className="absolute inset-0">
                  {steamCover ? (
                    <img
                      src={steamCover}
                      alt={app.name}
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

                {/* Top Badges */}
                <div className="relative z-10 p-2 flex items-center justify-between flex-wrap gap-1">
                  {getHdrBadge(app.hdr_type)}
                  <div className="flex items-center gap-1">
                    {app.path && pathStatus[app.path] === false && (
                      <span
                        className="px-1 py-0.2 text-[8px] font-mono text-rose-300 bg-rose-950/90 border border-rose-500/60 uppercase flex items-center gap-0.5"
                        title={t.appsPathMissingTooltip}
                      >
                        <AlertCircle className="w-2.5 h-2.5" />
                        {t.appsPathMissing}
                      </span>
                    )}
                    {app.launcher && (
                      <span className="px-1 py-0.2 text-[8px] font-mono text-[#b5a9ac] bg-black/60 border border-white/10 uppercase">
                        {launcherName(app.launcher, lang)}
                      </span>
                    )}
                  </div>
                </div>

                {/* Bottom Overlay & Controls */}
                <div className={`relative z-10 p-2.5 space-y-1.5 ${
                  isDark
                    ? 'bg-[#0f0b0b]/90 border-t border-[#f55a6b]/20'
                    : 'bg-white/95 border-t border-slate-200 shadow-xs'
                }`}>
                  <div className={`font-bold text-xs truncate ${isDark ? 'text-white' : 'text-slate-900'}`} title={app.name}>
                    <GlitchText text={app.name} scrambleOnHover={true} />
                  </div>

                  <div className={`text-[10px] font-mono truncate ${isDark ? 'text-[#5accf5]' : 'text-sky-700 font-semibold'}`}>
                    [{app.exe_name}]
                  </div>

                  <div className="flex items-center justify-between pt-0.5">
                    {/* Toggle button */}
                    <button
                      onClick={() => handleToggleApp(app, !app.enabled)}
                      disabled={isQuarantined(app)}
                      className={`px-2 py-0.5 text-[9px] font-bold uppercase tracking-wider cursor-pointer border transition-all ${
                        app.enabled
                          ? isDark
                            ? 'bg-[#f55a6b] text-[#0f0b0b] border-[#f55a6b]'
                            : 'bg-[#f55a6b] text-white border-[#f55a6b]'
                          : isDark
                          ? 'bg-[#120d0e] text-[#8a7f81] border-[#8a7f81]/30 hover:border-white'
                          : 'bg-slate-100 text-slate-600 border-slate-300 hover:border-slate-500'
                      }`}
                    >
                      {isQuarantined(app) ? t.appsQuarantined : app.enabled ? t.appsStatusTracked : t.appsStatusPaused}
                    </button>

                    {/* Delete button */}
                    {isQuarantined(app) && (
                      <button onClick={() => handleRepairExecutable(app)}
                        className={`p-1 cursor-pointer ${isDark ? 'text-amber-200' : 'text-amber-700'}`}
                        title={t.appsRepairExecutable} aria-label={t.appsRepairExecutable}>
                        <FolderOpen className="w-3.5 h-3.5" />
                      </button>
                    )}
                    <button
                      onClick={() => handleDeleteApp(app)}
                      className={`p-1 cursor-pointer transition-colors ${
                        isDark ? 'text-[#8a7f81] hover:text-[#f55a6b]' : 'text-slate-400 hover:text-rose-600'
                      }`}
                      title={t.appsRemoveFromLibrary}
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        /* List Mode View */
        <div className="space-y-2">
          {filteredApps.map((app) => {
            const steamCover = app.steam_id
              ? `https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/${app.steam_id}/capsule_sm_120.jpg`
              : null;

            return (
              <div
                key={config.apps.indexOf(app)}
                className={`p-3 border transition-all flex items-center justify-between gap-4 relative ${
                  app.enabled
                    ? isDark
                      ? 'bg-[#120d0e] border-[#f55a6b]/35 hover:border-[#f55a6b]'
                      : 'bg-white border-slate-200 hover:border-[#f55a6b] shadow-xs'
                    : isDark
                    ? 'bg-[#120d0e]/50 border-white/10 opacity-65'
                    : 'bg-slate-50 border-slate-200 opacity-65'
                }`}
              >
                <div className="flex items-center gap-3 min-w-0">
                  {/* Thumbnail */}
                  <div className={`w-16 h-10 border shrink-0 overflow-hidden relative ${
                    isDark ? 'border-[#f55a6b]/30 bg-black' : 'border-slate-300 bg-slate-100'
                  }`}>
                    {steamCover ? (
                      <img src={steamCover} alt={app.name} className="w-full h-full object-cover" />
                    ) : (
                      <div className={`w-full h-full flex items-center justify-center ${isDark ? 'bg-[#221314]' : 'bg-slate-200'}`}>
                        <Gamepad2 className={`w-4 h-4 ${isDark ? 'text-[#8a7f81]' : 'text-slate-400'}`} />
                      </div>
                    )}
                    {isDark && <div className="absolute inset-0 scanlines-overlay opacity-20" />}
                  </div>

                  <div className="space-y-0.5 min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className={`font-bold text-sm truncate ${isDark ? 'text-white' : 'text-slate-900'}`}>{app.name}</span>
                      {getHdrBadge(app.hdr_type)}
                      {app.launcher && (
                        <span className={`text-[9px] px-1.5 py-0.2 border uppercase ${
                          isDark ? 'bg-black border-white/15 text-[#b5a9ac]' : 'bg-slate-100 border-slate-300 text-slate-600'
                        }`}>
                          {launcherName(app.launcher, lang)}
                        </span>
                      )}
                      {app.path && pathStatus[app.path] === false && (
                        <span
                          className={`text-[9px] px-1.5 py-0.2 border font-mono flex items-center gap-1 ${
                            isDark ? 'bg-rose-950/70 border-rose-500/50 text-rose-300' : 'bg-rose-50 border-rose-300 text-rose-700'
                          }`}
                          title={t.appsPathMissingTooltip}
                        >
                          <AlertCircle className="w-3 h-3" />
                          {t.appsPathMissing}
                        </span>
                      )}
                    </div>
                    <div className={`text-xs font-mono ${isDark ? 'text-[#5accf5]' : 'text-sky-700'}`}>[{app.exe_name}]</div>
                  </div>
                </div>

                <div className="flex items-center gap-3 shrink-0">
                  <button
                    onClick={() => handleToggleApp(app, !app.enabled)}
                    disabled={isQuarantined(app)}
                    className={`px-2.5 py-1 text-xs font-bold uppercase tracking-wider cursor-pointer border transition-all ${
                      app.enabled
                        ? isDark
                          ? 'bg-[#f55a6b] text-[#0f0b0b] border-[#f55a6b]'
                          : 'bg-[#f55a6b] text-white border-[#f55a6b]'
                        : isDark
                        ? 'bg-[#120d0e] text-[#8a7f81] border-[#8a7f81]/30'
                        : 'bg-slate-100 text-slate-600 border-slate-300'
                    }`}
                  >
                    {isQuarantined(app) ? t.appsQuarantined : app.enabled ? t.appsStatusTracked : t.appsStatusPaused}
                  </button>

                  {isQuarantined(app) && (
                    <button onClick={() => handleRepairExecutable(app)}
                      className={`p-1.5 cursor-pointer ${isDark ? 'text-amber-200' : 'text-amber-700'}`}
                      title={t.appsRepairExecutable} aria-label={t.appsRepairExecutable}>
                      <FolderOpen className="w-4 h-4" />
                    </button>
                  )}
                  <button
                    onClick={() => handleDeleteApp(app)}
                    className={`p-1.5 cursor-pointer transition-colors ${
                      isDark ? 'text-[#8a7f81] hover:text-[#f55a6b]' : 'text-slate-400 hover:text-rose-600'
                    }`}
                    title={t.appsRemoveFromLibrary}
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Scan Modal */}
      {showScanModal && (() => {
        const hdrGames = scannedGames.filter((g) => g.is_hdr_supported);
        const unverifiedGames = scannedGames.filter((g) => !g.is_hdr_supported);
        const selectedCount = selectedDetections(scannedGames, selectedToImport).length;

        return (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-4">
            <div className={`border-2 max-w-2xl w-full p-6 space-y-4 relative max-h-[90vh] flex flex-col ${
              isDark
                ? 'bg-[#0f0b0b] border-[#f55a6b] shadow-[0_0_30px_rgba(245,90,107,0.4)]'
                : 'bg-white border-[#f55a6b] shadow-2xl text-slate-900'
            }`}>
              <div className={`flex items-center justify-between border-b pb-3 ${
                isDark ? 'border-[#f55a6b]/30' : 'border-slate-200'
              }`}>
                <div className="flex items-center gap-2">
                  <ScanSearch className={`w-5 h-5 ${isDark ? 'text-[#5accf5]' : 'text-sky-600'}`} />
                  <h3 className="glitch-title-bar px-2 py-0.5 text-xs font-bold uppercase">
                    {t.scanModalTitle(scannedGames.length)}
                  </h3>
                </div>
                <button
                  onClick={() => setShowScanModal(false)}
                  className={`cursor-pointer ${isDark ? 'text-[#8a7f81] hover:text-white' : 'text-slate-400 hover:text-slate-900'}`}
                >
                  <X className="w-5 h-5" />
                </button>
              </div>

              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 text-xs">
                <p className={isDark ? 'text-[#8a7f81]' : 'text-slate-500'}>
                  {t.scanModalSubtitle}
                  <span className="block mt-1">{t.appsHelperAliasCleanup}</span>
                </p>
                <div className="flex items-center gap-2 flex-shrink-0">
                  <button
                    type="button"
                    onClick={selectAllHdr}
                    className={`text-[10px] uppercase px-2 py-1 border cursor-pointer font-mono ${
                      isDark
                        ? 'bg-[#120d0e] border-[#5accf5]/50 text-[#5accf5] hover:bg-[#5accf5]/10'
                        : 'bg-sky-50 border-sky-300 text-sky-700 hover:bg-sky-100'
                    }`}
                  >
                    {t.scanModalSelectAllHdr}
                  </button>
                  <button
                    type="button"
                    onClick={selectAll}
                    className={`text-[10px] uppercase px-2 py-1 border cursor-pointer font-mono ${
                      isDark
                        ? 'bg-[#120d0e] border-white/20 text-white hover:bg-white/10'
                        : 'bg-slate-100 border-slate-300 text-slate-800 hover:bg-slate-200'
                    }`}
                  >
                    {t.scanModalSelectAll}
                  </button>
                  <button
                    type="button"
                    onClick={deselectAll}
                    className={`text-[10px] uppercase px-2 py-1 border cursor-pointer font-mono ${
                      isDark
                        ? 'bg-[#120d0e] border-white/10 text-[#8a7f81] hover:text-white'
                        : 'bg-slate-50 border-slate-200 text-slate-500 hover:text-slate-800'
                    }`}
                  >
                    {t.scanModalDeselectAll}
                  </button>
                </div>
              </div>

              <div className="flex-1 overflow-y-auto space-y-4 pr-1">
                {/* Verified support is independent of import selection. */}
                {hdrGames.length > 0 && (
                  <div className="space-y-2">
                    <div className={`flex items-center justify-between px-2 py-1.5 border-l-2 ${
                      isDark
                        ? 'bg-[#121c1f] border-[#5accf5] text-[#5accf5]'
                        : 'bg-sky-50 border-sky-500 text-sky-800'
                    }`}>
                      <div className="flex items-center gap-2 text-xs font-bold uppercase">
                        <Sparkles className={`w-4 h-4 ${isDark ? 'text-[#5accf5]' : 'text-sky-600'}`} />
                        <span>{t.scanModalSectionHdr(hdrGames.length)}</span>
                      </div>
                      <span className="text-[10px] font-mono opacity-80">[HDR]</span>
                    </div>

                    <div className="space-y-1.5">
                      {hdrGames.map((game) => {
                        const isSelected = !!selectedToImport[detectionKey(game)];
                        const existing = findExistingApp(game);
                        const pathChanged = existing ? isPathDifferent(existing, game) : false;
                        const isAlreadyInLib = !!existing && !pathChanged;

                        return (
                          <div
                            key={detectionKey(game)}
                            onClick={() =>
                              setSelectedToImport((prev) => toggleDetection(prev, game))
                            }
                            className={`p-2.5 border cursor-pointer flex items-center justify-between text-xs transition-all ${
                              isSelected
                                ? isDark
                                  ? 'bg-[#121c1f] border-[#5accf5] text-white shadow-[0_0_10px_rgba(90,204,245,0.15)]'
                                  : 'bg-sky-50 border-sky-500 text-slate-900 shadow-xs'
                                : isDark
                                ? 'bg-black/40 border-white/10 text-[#8a7f81] hover:border-white/30'
                                : 'bg-slate-50 border-slate-200 text-slate-600 hover:border-slate-400'
                            }`}
                          >
                            <div className="space-y-0.5 min-w-0 flex-1 pr-2">
                              <div className="font-bold flex items-center gap-2 flex-wrap">
                                <span className={!isDark && isSelected ? 'text-slate-900' : ''}>{game.name}</span>
                                {game.launcher && (
                                  <span className={`text-[9px] px-1 border ${
                                    isDark ? 'bg-black border-[#5accf5]/30 text-[#5accf5]' : 'bg-white border-sky-300 text-sky-700'
                                  }`}>
                                    {launcherName(game.launcher, lang)}
                                  </span>
                                )}
                                <span className={`text-[9px] px-1.5 py-0.2 border font-mono uppercase ${
                                  isDark ? 'bg-[#5accf5]/10 border-[#5accf5]/50 text-[#5accf5]' : 'bg-sky-100 border-sky-300 text-sky-800'
                                }`}>
                                  {game.hdr_type === 'autohdr' ? 'Auto HDR' : 'Native HDR'}
                                </span>

                                {/* Status Badges */}
                                {pathChanged && (
                                  <span className={`text-[9px] px-1.5 py-0.2 border font-mono font-bold uppercase tracking-wider flex items-center gap-1 ${
                                    isDark ? 'bg-amber-500/15 border-amber-500/60 text-amber-300' : 'bg-amber-50 border-amber-400 text-amber-800'
                                  }`}>
                                    <RefreshCw className="w-2.5 h-2.5" />
                                    {t.scanModalStatusPathUpdate}
                                  </span>
                                )}
                                {isAlreadyInLib && (
                                  <span className={`text-[9px] px-1.5 py-0.2 border font-mono uppercase ${
                                    isDark ? 'bg-emerald-500/10 border-emerald-500/30 text-emerald-400' : 'bg-emerald-50 border-emerald-400 text-emerald-800'
                                  }`}>
                                    ✓ {t.scanModalStatusInLibrary}
                                  </span>
                                )}
                                {!existing && (
                                  <span className={`text-[9px] px-1.5 py-0.2 border font-mono font-bold uppercase tracking-wider ${
                                    isDark ? 'bg-[#5accf5]/15 border-[#5accf5]/70 text-[#5accf5]' : 'bg-sky-100 border-sky-400 text-sky-800'
                                  }`}>
                                    ★ {t.scanModalStatusNew}
                                  </span>
                                )}
                              </div>
                              <div className={`text-[10px] font-mono truncate ${isDark ? 'text-[#5accf5]' : 'text-sky-700'}`}>[{game.exe_name}]</div>
                              {game.path && !pathChanged && (
                                <div className={`text-[9px] font-mono truncate ${isDark ? '' : 'text-slate-500'}`} title={game.path}>{game.path}</div>
                              )}
                              {pathChanged && game.path && (
                                <div className={`text-[9px] font-mono truncate ${isDark ? 'text-amber-300/80' : 'text-amber-700'}`} title={game.path}>
                                  ➔ {t.scanModalNewLocation}: {game.path}
                                </div>
                              )}
                            </div>

                            <div
                              className={`w-4 h-4 border flex items-center justify-center shrink-0 transition-colors ${
                                isSelected
                                  ? 'bg-[#5accf5] border-[#5accf5] text-black'
                                  : isDark ? 'border-[#8a7f81]' : 'border-slate-300'
                              }`}
                            >
                              {isSelected && <Check className="w-3 h-3 stroke-[3]" />}
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                )}

                {unverifiedGames.length > 0 && (
                  <div className="space-y-2">
                    <div className={`flex items-center justify-between px-2 py-1.5 border-l-2 ${
                      isDark
                        ? 'bg-[#120d0e] border-[#8a7f81] text-[#8a7f81]'
                        : 'bg-slate-100 border-slate-400 text-slate-700'
                    }`}>
                      <div className="flex items-center gap-2 text-xs font-bold uppercase">
                        <Gamepad2 className={`w-4 h-4 ${isDark ? 'text-[#8a7f81]' : 'text-slate-500'}`} />
                        <span>{t.scanModalSectionSdr(unverifiedGames.length)}</span>
                      </div>
                      <span className="text-[10px] font-mono opacity-80">[AUTO OFF]</span>
                    </div>

                    <div className="space-y-1.5">
                      {unverifiedGames.map((game) => {
                        const isSelected = !!selectedToImport[detectionKey(game)];
                        const existing = findExistingApp(game);
                        const pathChanged = existing ? isPathDifferent(existing, game) : false;
                        const isAlreadyInLib = !!existing && !pathChanged;

                        return (
                          <div
                            key={detectionKey(game)}
                            onClick={() =>
                              setSelectedToImport((prev) => toggleDetection(prev, game))
                            }
                            className={`p-2.5 border cursor-pointer flex items-center justify-between text-xs transition-all ${
                              isSelected
                                ? isDark
                                  ? 'bg-[#1c0f12] border-[#f55a6b] text-white'
                                  : 'bg-rose-50 border-[#f55a6b] text-slate-900 shadow-xs'
                                : isDark
                                ? 'bg-black/40 border-white/10 text-[#8a7f81] hover:border-white/30'
                                : 'bg-slate-50 border-slate-200 text-slate-600 hover:border-slate-400'
                            }`}
                          >
                            <div className="space-y-0.5 min-w-0 flex-1 pr-2">
                              <div className="font-bold flex items-center gap-2 flex-wrap">
                                <span className={!isDark && isSelected ? 'text-slate-900' : ''}>{game.name}</span>
                                {game.launcher && (
                                  <span className={`text-[9px] px-1 border ${
                                    isDark ? 'bg-black border-white/20 text-[#8a7f81]' : 'bg-white border-slate-300 text-slate-600'
                                  }`}>
                                    {launcherName(game.launcher, lang)}
                                  </span>
                                )}
                                <span className={`text-[9px] px-1.5 py-0.2 border font-mono uppercase ${
                                  isDark ? 'bg-white/5 border-white/10 text-[#8a7f81]' : 'bg-slate-100 border-slate-300 text-slate-600'
                                }`}>
                                  {t.scanModalSupportUnverified}
                                </span>

                                {pathChanged && (
                                  <span className={`text-[9px] px-1.5 py-0.2 border font-mono font-bold uppercase tracking-wider flex items-center gap-1 ${
                                    isDark ? 'bg-amber-500/15 border-amber-500/60 text-amber-300' : 'bg-amber-50 border-amber-400 text-amber-800'
                                  }`}>
                                    <RefreshCw className="w-2.5 h-2.5" />
                                    {t.scanModalStatusPathUpdate}
                                  </span>
                                )}
                                {isAlreadyInLib && (
                                  <span className={`text-[9px] px-1.5 py-0.2 border font-mono uppercase ${
                                    isDark ? 'bg-emerald-500/10 border-emerald-500/30 text-emerald-400' : 'bg-emerald-50 border-emerald-400 text-emerald-800'
                                  }`}>
                                    ✓ {t.scanModalStatusInLibrary}
                                  </span>
                                )}
                              </div>
                              <div className={`text-[10px] font-mono truncate ${isDark ? 'text-[#8a7f81]' : 'text-slate-500'}`}>[{game.exe_name}]</div>
                              {game.path && !pathChanged && (
                                <div className={`text-[9px] font-mono truncate ${isDark ? '' : 'text-slate-500'}`} title={game.path}>{game.path}</div>
                              )}
                              {pathChanged && game.path && (
                                <div className={`text-[9px] font-mono truncate ${isDark ? 'text-amber-300/80' : 'text-amber-700'}`} title={game.path}>
                                  ➔ {t.scanModalNewLocation}: {game.path}
                                </div>
                              )}
                            </div>

                            <div
                              className={`w-4 h-4 border flex items-center justify-center shrink-0 transition-colors ${
                                isSelected
                                  ? 'bg-[#f55a6b] border-[#f55a6b] text-black'
                                  : isDark ? 'border-[#8a7f81]' : 'border-slate-300'
                              }`}
                            >
                              {isSelected && <Check className="w-3 h-3 stroke-[3]" />}
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                )}
              </div>

              <div className={`flex items-center justify-between border-t pt-3 ${
                isDark ? 'border-[#f55a6b]/30' : 'border-slate-200'
              }`}>
                <div className={`text-xs ${isDark ? 'text-[#8a7f81]' : 'text-slate-500'}`}>
                  {t.scanModalSelected(selectedCount)}
                </div>
                <div className="flex items-center gap-3">
                  <GlitchButton
                    label={t.scanModalCancel}
                    variant="outline"
                    size="sm"
                    onClick={() => setShowScanModal(false)}
                  />
                  <GlitchButton
                    label={`${t.scanModalAddSelected} (${selectedCount})`}
                    variant="primary"
                    size="sm"
                    onClick={handleConfirmImport}
                  />
                </div>
              </div>
            </div>
          </div>
        );
      })()}

      {/* Manual Add Modal & File Picker */}
      {showAddModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-4">
          <div className={`border-2 max-w-lg w-full p-6 space-y-4 relative ${
            isDark
              ? 'bg-[#0f0b0b] border-[#f55a6b] shadow-[0_0_30px_rgba(245,90,107,0.4)]'
              : 'bg-white border-[#f55a6b] shadow-2xl text-slate-900'
          }`}>
            <div className={`flex items-center justify-between border-b pb-3 ${
              isDark ? 'border-[#f55a6b]/30' : 'border-slate-200'
            }`}>
              <h3 className="glitch-title-bar px-2 py-0.5 text-xs font-bold uppercase">
                {t.manualModalTitle}
              </h3>
              <button
                onClick={() => {
                  setShowAddModal(false);
                  setNewPath('');
                  setIsHdrMatched(false);
                }}
                className={`cursor-pointer ${isDark ? 'text-[#8a7f81] hover:text-white' : 'text-slate-400 hover:text-slate-900'}`}
              >
                <X className="w-5 h-5" />
              </button>
            </div>

            <p className={`text-[10px] ${isDark ? 'text-[#8a7f81]' : 'text-slate-500'}`}>
              {t.appsHelperAliasCleanup}
            </p>

            {/* Interactive File Dropzone & Browse Button */}
            <div
              onClick={handleBrowseExe}
              onDragOver={(e) => {
                e.preventDefault();
                setIsDraggingOver(true);
              }}
              onDragLeave={() => setIsDraggingOver(false)}
              onDrop={(e) => {
                e.preventDefault();
                setIsDraggingOver(false);
              }}
              className={`p-4 border-2 border-dashed cursor-pointer text-center transition-all ${
                isDraggingOver
                  ? isDark
                    ? 'border-[#5accf5] bg-[#5accf5]/15 text-white shadow-[0_0_15px_rgba(90,204,245,0.3)]'
                    : 'border-sky-500 bg-sky-50 text-sky-900 shadow-xs'
                  : isDark
                  ? 'border-[#f55a6b]/40 hover:border-[#f55a6b] bg-[#120d0e]/60 text-[#8a7f81] hover:text-white'
                  : 'border-slate-300 hover:border-[#f55a6b] bg-slate-50 text-slate-600 hover:text-slate-900'
              }`}
            >
              <UploadCloud className={`w-7 h-7 mx-auto mb-1.5 transition-colors ${
                isDraggingOver
                  ? isDark ? 'text-[#5accf5]' : 'text-sky-600'
                  : isDark ? 'text-[#f55a6b]' : 'text-[#e03e52]'
              }`} />
              <div className={`text-xs font-bold uppercase flex items-center justify-center gap-1.5 ${
                isDark ? 'text-white' : 'text-slate-900'
              }`}>
                <FolderOpen className={`w-3.5 h-3.5 ${isDark ? 'text-[#5accf5]' : 'text-sky-600'}`} />
                <span>{t.manualModalBrowseBtn}</span>
              </div>
              <p className={`text-[10px] mt-1 ${isDark ? 'text-[#8a7f81]' : 'text-slate-500'}`}>
                {t.manualModalDragDropHint}
              </p>
            </div>

            {/* Catalog Match Banner */}
            {isHdrMatched && (
              <div className={`p-2 border flex items-center gap-2 text-xs ${
                isDark
                  ? 'bg-[#121c1f] border-[#5accf5] text-[#5accf5]'
                  : 'bg-sky-50 border-sky-400 text-sky-800'
              }`}>
                <Sparkles className={`w-4 h-4 flex-shrink-0 ${isDark ? 'text-[#5accf5]' : 'text-sky-600'}`} />
                <span className="font-bold uppercase tracking-wider">{t.manualModalDetectedBadge}</span>
              </div>
            )}

            <form onSubmit={handleAddCustomApp} className="space-y-4">
              <div className="space-y-1">
                <label className={`text-xs uppercase ${isDark ? 'text-[#8a7f81]' : 'text-slate-600'}`}>{t.manualModalName}</label>
                <input
                  type="text"
                  required
                  placeholder={t.manualNamePlaceholder}
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  className={`w-full px-3 py-2 text-xs border focus:outline-none ${
                    isDark
                      ? 'border-[#f55a6b]/30 bg-[#120d0e] focus:border-[#f55a6b] text-white'
                      : 'border-slate-300 bg-white focus:border-[#f55a6b] text-slate-900 shadow-xs'
                  }`}
                />
              </div>

              <div className="space-y-1">
                <div className="flex items-center justify-between">
                  <label className={`text-xs uppercase ${isDark ? 'text-[#8a7f81]' : 'text-slate-600'}`}>{t.manualModalExe}</label>
                  <button
                    type="button"
                    onClick={handleBrowseExe}
                    className={`text-[10px] hover:underline flex items-center gap-1 cursor-pointer font-mono ${
                      isDark ? 'text-[#5accf5]' : 'text-sky-600'
                    }`}
                  >
                    <FolderOpen className="w-3 h-3" />
                    {t.manualModalBrowseBtn}
                  </button>
                </div>
                <div className="flex gap-2">
                  <input
                    type="text"
                    required
                    placeholder={t.manualExePlaceholder}
                    value={newExe}
                    onChange={(e) => {
                      setNewExe(e.target.value);
                      setNewPath((path) => pathAfterExecutableEdit(path, e.target.value));
                      setIsHdrMatched(false);
                    }}
                    className={`w-full px-3 py-2 text-xs border focus:outline-none font-mono ${
                      isDark
                        ? 'border-[#f55a6b]/30 bg-[#120d0e] focus:border-[#f55a6b] text-white'
                        : 'border-slate-300 bg-white focus:border-[#f55a6b] text-slate-900 shadow-xs'
                    }`}
                  />
                  <button
                    type="button"
                    onClick={handleBrowseExe}
                    className={`px-3 py-1.5 border text-xs font-bold flex items-center gap-1 cursor-pointer transition-colors ${
                      isDark
                        ? 'bg-[#1c0f12] border-[#f55a6b] text-[#f55a6b] hover:bg-[#f55a6b] hover:text-black'
                        : 'bg-rose-50 border-[#f55a6b] text-[#e03e52] hover:bg-[#f55a6b] hover:text-white'
                    }`}
                  >
                    <FolderOpen className="w-4 h-4" />
                  </button>
                </div>
                {newPath && (
                  <div className={`text-[9px] font-mono truncate pt-0.5 ${isDark ? 'text-[#5accf5]' : 'text-sky-700'}`} title={newPath}>
                    {t.manualModalPath}: {newPath}
                  </div>
                )}
              </div>

              <div className="space-y-1">
                <label className={`text-xs uppercase ${isDark ? 'text-[#8a7f81]' : 'text-slate-600'}`}>{t.manualModalType}</label>
                <select
                  value={newType}
                  onChange={(e) => setNewType(e.target.value as HdrType)}
                  className={`w-full px-3 py-2 text-xs border focus:outline-none ${
                    isDark
                      ? 'border-[#f55a6b]/30 bg-[#120d0e] focus:border-[#f55a6b] text-white'
                      : 'border-slate-300 bg-white focus:border-[#f55a6b] text-slate-900 shadow-xs'
                  }`}
                >
                  <option value="native">{t.catalogTierNative}</option>
                  <option value="autohdr">{t.catalogTierAutoHdr}</option>
                  <option value="custom">{t.catalogTierCustom}</option>
                  <option value="media">{t.catalogTierMedia}</option>
                </select>
              </div>

              <div className="flex items-center justify-end gap-3 pt-3">
                <GlitchButton
                  type="button"
                  label={t.manualModalCancel}
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    setShowAddModal(false);
                    setNewPath('');
                    setIsHdrMatched(false);
                  }}
                />
                <GlitchButton
                  type="submit"
                  label={t.manualModalSubmit}
                  variant="primary"
                  size="sm"
                />
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  );
};
