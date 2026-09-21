import React, { useState, useEffect } from 'react';
import { CatalogEntry, AppConfig, HdrApp, SupportTier } from '../types';
import { invoke } from '@tauri-apps/api/core';
import { configClient } from '../useConfig';
import { captureLibraryRow, findTrackedApp } from '../libraryState';
import { catalogNotes } from '../catalogNotes';
import {
  Search,
  CheckCircle2,
  Plus,
  RefreshCw,
  Check,
  Sparkles,
  Lock,
  Wrench,
  Film,
  Zap,
} from 'lucide-react';
import { GlitchButton } from './GlitchButton';
import { GlitchText } from './GlitchText';
import { useI18n } from '../i18n';

interface CatalogBrowserProps {
  config: AppConfig;
  isDark: boolean;
}

export const CatalogBrowser: React.FC<CatalogBrowserProps> = ({
  config,
  isDark,
}) => {
  const { t, lang } = useI18n();
  const [catalog, setCatalog] = useState<CatalogEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState('');
  const [selectedTier, setSelectedTier] = useState<string>('all');
  const [syncing, setSyncing] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const fetchCatalog = async () => {
    setLoading(true);
    try {
      const entries: CatalogEntry[] = await invoke('get_catalog');
      setCatalog(entries);
    } catch (err) {
      console.error('Failed to get catalog:', err);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchCatalog();
  }, []);

  const handleAddGame = async (entry: CatalogEntry) => {
    const newApp: HdrApp = {
      name: entry.name,
      exe_name: entry.exe_name.toLowerCase(),
      enabled: true,
      hdr_type: entry.hdr_type,
      steam_id: entry.steam_id,
      alternate_exes: entry.alternate_exes,
    };

    try {
      await configClient.mutate('add_custom_app', { app: newApp });
    } catch (err) {
      configClient.reportError(err);
    }
  };

  const handleRemoveGame = async (app: HdrApp) => {
    try {
      const { row, origin } = captureLibraryRow(configClient, config.apps, app);
      await configClient.mutate('remove_app', { row }, origin);
    } catch (err) {
      configClient.reportError(err);
    }
  };

  const handleSync = async () => {
    setSyncing(true);
    try {
      const count: number = await invoke('sync_database');
      await fetchCatalog();
      setMessage(t.catalogSyncSuccess(count));
      setTimeout(() => setMessage(null), 4000);
    } catch (err) {
      console.error('Sync failed:', err);
      configClient.reportError(err);
      setMessage(t.catalogSyncError);
      setTimeout(() => setMessage(null), 4000);
    } finally {
      setSyncing(false);
    }
  };

  const filtered = catalog.filter((item) => {
    const matchesSearch =
      item.name.toLowerCase().includes(search.toLowerCase()) ||
      item.exe_name.toLowerCase().includes(search.toLowerCase());

    const matchesTier =
      selectedTier === 'all' ? true : item.support_tier === selectedTier;

    return matchesSearch && matchesTier;
  });

  const getTierBadge = (tier: SupportTier) => {
    switch (tier) {
      case 'native':
        return (
          <span className={`inline-flex items-center gap-1 text-[10px] font-mono px-2 py-0.5 font-bold uppercase tracking-wider border ${
            isDark ? 'bg-cyan-950/80 text-[#5accf5] border-[#5accf5]/40' : 'bg-cyan-50 text-cyan-800 border-cyan-300'
          }`}>
            <CheckCircle2 className={`w-3 h-3 ${isDark ? 'text-[#5accf5]' : 'text-cyan-600'}`} /> {t.catalogTierNative}
          </span>
        );
      case 'limited':
        return (
          <span className={`inline-flex items-center gap-1 text-[10px] font-mono px-2 py-0.5 font-bold uppercase tracking-wider border ${
            isDark ? 'bg-teal-950/80 text-teal-300 border-teal-500/40' : 'bg-teal-50 text-teal-800 border-teal-300'
          }`}>
            <Sparkles className={`w-3 h-3 ${isDark ? 'text-teal-400' : 'text-teal-600'}`} /> {t.catalogTierLimited}
          </span>
        );
      case 'always_on':
        return (
          <span className={`inline-flex items-center gap-1 text-[10px] font-mono px-2 py-0.5 font-bold uppercase tracking-wider border ${
            isDark ? 'bg-purple-950/80 text-purple-300 border-purple-500/40' : 'bg-purple-50 text-purple-800 border-purple-300'
          }`}>
            <Lock className={`w-3 h-3 ${isDark ? 'text-purple-400' : 'text-purple-600'}`} /> {t.catalogTierAlwaysOn}
          </span>
        );
      case 'manual_fix':
        return (
          <span className={`inline-flex items-center gap-1 text-[10px] font-mono px-2 py-0.5 font-bold uppercase tracking-wider border ${
            isDark ? 'bg-amber-950/80 text-amber-300 border-amber-500/40' : 'bg-amber-50 text-amber-800 border-amber-300'
          }`}>
            <Wrench className={`w-3 h-3 ${isDark ? 'text-amber-400' : 'text-amber-600'}`} /> {t.catalogTierMod}
          </span>
        );
      case 'autohdr':
        return (
          <span className={`inline-flex items-center gap-1 text-[10px] font-mono px-2 py-0.5 font-bold uppercase tracking-wider border ${
            isDark ? 'bg-rose-950/80 text-[#f55a6b] border-[#f55a6b]/40' : 'bg-rose-50 text-rose-700 border-rose-300'
          }`}>
            <Zap className={`w-3 h-3 ${isDark ? 'text-[#f55a6b]' : 'text-rose-600'}`} /> {t.catalogTierAutoHdr}
          </span>
        );
      case 'media':
        return (
          <span className={`inline-flex items-center gap-1 text-[10px] font-mono px-2 py-0.5 font-bold uppercase tracking-wider border ${
            isDark ? 'bg-blue-950/80 text-blue-300 border-blue-500/40' : 'bg-blue-50 text-blue-800 border-blue-300'
          }`}>
            <Film className={`w-3 h-3 ${isDark ? 'text-blue-400' : 'text-blue-600'}`} /> {t.catalogTierMedia}
          </span>
        );
      default:
        return (
          <span className={`inline-flex items-center gap-1 text-[10px] font-mono px-2 py-0.5 font-bold uppercase tracking-wider border ${
            isDark ? 'bg-slate-900 text-slate-300 border-slate-700' : 'bg-slate-100 text-slate-700 border-slate-300'
          }`}>
            {t.catalogTierCustom}
          </span>
        );
    }
  };

  const countForTier = (tier: string) => {
    if (tier === 'all') return catalog.length;
    return catalog.filter((i) => i.support_tier === tier).length;
  };

  return (
    <div className="space-y-5 font-mono">
      {/* Top Header */}
      <div className="flex flex-col md:flex-row md:items-center justify-between gap-4">
        <div>
          <div className="flex items-center gap-2.5">
            <h2 className="glitch-title-bar px-2.5 py-0.5 text-xs font-bold tracking-wider inline-block">
              {t.catalogTitle}
            </h2>
            <span className={`text-xs px-2 py-0.5 border ${
              isDark ? 'border-[#5accf5]/40 text-[#5accf5] bg-[#140e10]' : 'border-sky-300 text-sky-700 bg-sky-50 font-semibold'
            }`}>
              {t.catalogArchiveCount(catalog.length)}
            </span>
          </div>
          <p className={`text-xs mt-1 ${isDark ? 'text-[#8a7f81]' : 'text-slate-600'}`}>
            {t.catalogSubtitle}
          </p>
          <p className={`text-[10px] mt-1 ${isDark ? 'text-[#8a7f81]' : 'text-slate-600'}`}>
            {t.appsHelperAliasCleanup}
          </p>
        </div>

        <GlitchButton
          label={syncing ? t.catalogSyncingBtn : t.catalogSyncBtn}
          variant="outline"
          size="sm"
          disabled={syncing}
          isDark={isDark}
          icon={<RefreshCw className={`w-3.5 h-3.5 ${isDark ? 'text-[#5accf5]' : 'text-sky-600'} ${syncing ? 'animate-spin' : ''}`} />}
          onClick={handleSync}
        />
      </div>

      {message && (
        <div className={`p-3 border text-xs flex items-center gap-2.5 ${
          isDark ? 'border-[#5accf5]/40 bg-[#120e10] text-[#5accf5]' : 'border-sky-300 bg-sky-50 text-sky-800'
        }`}>
          <Sparkles className={`w-4 h-4 shrink-0 ${isDark ? 'text-[#5accf5]' : 'text-sky-600'}`} />
          <span>&gt; {message}</span>
        </div>
      )}

      {/* Filter and Search Bar */}
      <div className="space-y-3">
        <div className="relative">
          <Search className={`w-4 h-4 absolute left-3.5 top-1/2 -translate-y-1/2 ${isDark ? 'text-[#8a7f81]' : 'text-slate-400'}`} />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t.catalogSearchPlaceholder}
            className={`w-full pl-9 pr-4 py-2 text-xs border focus:border-[#f55a6b] focus:outline-none transition-all ${
              isDark
                ? 'border-[#f55a6b]/30 bg-[#120d0e] text-white placeholder-[#8a7f81]'
                : 'border-slate-300 bg-white text-slate-900 placeholder-slate-400 shadow-2xs'
            }`}
          />
        </div>

        {/* Tier Filter Tabs */}
        <div className="flex items-center gap-1.5 overflow-x-auto pb-1">
          {[
            { id: 'all', label: t.catalogTabAll(countForTier('all')) },
            { id: 'native', label: t.catalogTabNative(countForTier('native')) },
            { id: 'autohdr', label: t.catalogTabAutoHdr(countForTier('autohdr')) },
            { id: 'limited', label: t.catalogTabLimited(countForTier('limited')) },
            { id: 'manual_fix', label: t.catalogTabMod(countForTier('manual_fix')) },
            { id: 'always_on', label: t.catalogTabAlwaysOn(countForTier('always_on')) },
          ].map((tab) => (
            <button
              key={tab.id}
              onClick={() => setSelectedTier(tab.id)}
              className={`px-3 py-1 text-xs uppercase font-bold cursor-pointer transition-all border ${
                selectedTier === tab.id
                  ? 'bg-[#f55a6b] text-[#0f0b0b] border-[#f55a6b] neon-glow-coral'
                  : isDark
                    ? 'bg-[#120d0e] text-[#8a7f81] border-[#f55a6b]/20 hover:border-[#f55a6b]/50 hover:text-white'
                    : 'bg-white text-slate-700 border-slate-200 hover:border-slate-400 hover:text-slate-900 shadow-2xs'
              }`}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </div>

      {/* Games Catalog List */}
      {loading ? (
        <div className={`p-12 text-center border text-xs ${
          isDark ? 'border-[#f55a6b]/20 bg-[#120d0e] text-[#5accf5]' : 'border-slate-200 bg-white text-sky-700 shadow-2xs'
        }`}>
          {t.catalogLoading}
        </div>
      ) : filtered.length === 0 ? (
        <div className={`p-12 text-center border text-xs ${
          isDark ? 'border-[#f55a6b]/20 bg-[#120d0e] text-[#8a7f81]' : 'border-slate-200 bg-white text-slate-600 shadow-2xs'
        }`}>
          {t.catalogEmpty}
        </div>
      ) : (
        <div className="space-y-2">
          {filtered.map((item) => {
            const tracked = findTrackedApp(config.apps, item);

            return (
              <div
                key={item.exe_name}
                className={`p-3 border transition-all flex items-center justify-between gap-4 relative ${
                  isDark
                    ? tracked
                      ? 'bg-[#180e10] border-[#f55a6b]/50'
                      : 'bg-[#120d0e] border-[#f55a6b]/20 hover:border-[#f55a6b]/60'
                    : tracked
                      ? 'bg-rose-50/50 border-[#f55a6b]/50 shadow-2xs'
                      : 'bg-white border-slate-200 hover:border-[#f55a6b] shadow-2xs'
                }`}
              >
                {isDark && <div className="absolute inset-0 scanlines-overlay opacity-10 pointer-events-none" />}

                <div className="space-y-1 min-w-0 relative z-10">
                  <div className="flex items-center gap-2.5 flex-wrap">
                    <span className={`font-bold text-sm truncate ${isDark ? 'text-white' : 'text-slate-900'}`}>
                      <GlitchText text={item.name} scrambleOnHover={true} />
                    </span>
                    {getTierBadge(item.support_tier)}
                  </div>

                  <div className={`flex items-center gap-2 text-xs ${isDark ? 'text-[#8a7f81]' : 'text-slate-500'}`}>
                    <span className={`font-mono ${isDark ? 'text-[#5accf5]' : 'text-sky-700 font-semibold'}`}>[{item.exe_name}]</span>
                    {item.notes && (
                      <>
                        <span>•</span>
                        <span className="truncate max-w-[400px]">{catalogNotes(item.notes, lang)}</span>
                      </>
                    )}
                  </div>
                </div>

                <div className="shrink-0 relative z-10">
                  {tracked ? (
                    <GlitchButton
                      label={t.catalogRemoveBtn}
                      variant="outline"
                      size="sm"
                      isDark={isDark}
                      icon={<Check className={`w-3.5 h-3.5 ${isDark ? 'text-emerald-400' : 'text-emerald-600'}`} />}
                      onClick={() => handleRemoveGame(tracked)}
                    />
                  ) : (
                    <GlitchButton
                      label={t.catalogAddBtn}
                      variant="primary"
                      size="sm"
                      isDark={isDark}
                      icon={<Plus className="w-3.5 h-3.5 fill-current" />}
                      onClick={() => handleAddGame(item)}
                    />
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
