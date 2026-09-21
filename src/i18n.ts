import { createContext, useContext } from 'react';

export type Language = 'cs' | 'en';

export interface Translations {
  // Brand & Header
  appTitle: string;
  hdrActive: string;
  hdrMixed: string;
  sdrStandby: string;
  themeToggle: string;
  langToggle: string;

  // Nav
  navOverview: string;
  navApps: string;
  navCatalog: string;
  navProcesses: string;
  navSettings: string;

  // Dashboard Hero
  heroHdrActiveTitle: string;
  heroSdrTitle: string;
  heroMixedTitle: string;
  heroHdrRec2020: string;
  heroSdrBt709: string;
  heroHookActive: string;
  heroDisplaysReady: (count: number) => string;
  heroActiveProcess: string;
  heroSdrSubtext: string;
  heroTurnOffHdr: string;
  heroTurnOnHdr: string;
  heroSwitching: string;

  // Displays
  displaysTitle: string;
  displaysRefresh: string;
  displaysPrimary: string;
  displaysTargetHdr: string;
  displaysHdrSupported: string;
  displaysSdrOnly: string;
  displaysStateUnknown: string;
  displaysTargetId: string;
  displaysHdrOn: string;
  displaysSdr: string;

  // Recent Games & Telemetry
  recentTitle: string;
  recentAllLibrary: (count: number) => string;
  recentHookActive: string;
  recentHookTriggered: string;
  recentTierNative: string;
  recentTierAutoHdr: string;
  recentTierMod: string;
  recentHdrOk: string;
  recentEmpty: string;
  recentYesterday: string;
  recentToday: string;

  // Activity Log
  activityTitle: string;
  activityInitSystem: string;
  activityInitDetect: string;
  activityHdrManualOn: string;
  activityHdrManualOff: string;
  activityHdrObserved: string;
  activityHdrMixed: string;
  activityHookWindowFocus: (app: string) => string;
  activityHookReturnSdr: string;

  // Apps Manager
  appsTitle: string;
  appsRepairExecutable: string;
  appsHelperAliasCleanup: string;
  appsQuarantined: string;
  quarantineWarning: (game: string, executable: string) => string;
  primaryPathWarning: (game: string, executable: string) => string;
  appsCountSummary: (total: number, active: number) => string;
  appsSubtitle: string;
  appsScanBtn: string;
  appsScanningBtn: string;
  appsAddManualBtn: string;
  appsCatalogBtn: string;
  appsSearchPlaceholder: string;
  appsTabAll: (count: number) => string;
  appsTabSteam: (count: number) => string;
  appsTabEpic: string;
  appsTabWindows: string;
  appsNoGamesTitle: string;
  appsNoGamesSearchTitle: string;
  appsNoGamesSubtitle: string;
  appsScanDisksBtn: string;
  appsBrowseCatalogBtn: string;
  appsStatusTracked: string;
  appsStatusPaused: string;
  appsRemoveFromLibrary: string;

  // Scan Modal
  scanModalTitle: (count: number) => string;
  scanModalSubtitle: string;
  scanModalSupportUnverified: string;
  scanModalCancel: string;
  scanModalAddSelected: string;
  scanModalSuccess: (count: number) => string;
  scanModalError: string;
  scanModalSectionHdr: (count: number) => string;
  scanModalSectionSdr: (count: number) => string;
  scanModalStatusNew: string;
  scanModalStatusInLibrary: string;
  scanModalStatusPathUpdate: string;
  scanModalNewLocation: string;
  scanModalSelectAllHdr: string;
  scanModalSelectAll: string;
  scanModalDeselectAll: string;
  appsPathMissing: string;
  appsPathMissingTooltip: string;
  appsGridView: string;
  appsListView: string;
  scanModalSelected: (count: number) => string;
  manualNamePlaceholder: string;
  manualExePlaceholder: string;

  // Manual Add Modal & Drag & Drop
  manualModalTitle: string;
  manualModalName: string;
  manualModalExe: string;
  manualModalType: string;
  manualModalCancel: string;
  manualModalSubmit: string;
  manualModalBrowseBtn: string;
  manualModalDragDropHint: string;
  manualModalDetectedBadge: string;
  manualModalPath: string;

  // Catalog Browser
  catalogTitle: string;
  catalogArchiveCount: (count: number) => string;
  catalogSubtitle: string;
  catalogSyncBtn: string;
  catalogSyncingBtn: string;
  catalogSyncSuccess: (count: number) => string;
  catalogSyncError: string;
  catalogSearchPlaceholder: string;
  catalogTabAll: (count: number) => string;
  catalogTabNative: (count: number) => string;
  catalogTabAutoHdr: (count: number) => string;
  catalogTabLimited: (count: number) => string;
  catalogTabMod: (count: number) => string;
  catalogTabAlwaysOn: (count: number) => string;
  catalogTierNative: string;
  catalogTierLimited: string;
  catalogTierAlwaysOn: string;
  catalogTierMod: string;
  catalogTierAutoHdr: string;
  catalogTierMedia: string;
  catalogTierCustom: string;
  catalogLoading: string;
  catalogEmpty: string;
  catalogAddBtn: string;
  catalogRemoveBtn: string;

  // Running Processes
  procTitle: string;
  procCountActive: (count: number) => string;
  procSubtitle: string;
  procRefreshBtn: string;
  procRefreshingBtn: string;
  procSearchPlaceholder: string;
  procLoading: string;
  procEmpty: string;
  procAlreadyTracked: string;
  procAddToHdr: string;
  procAdding: string;

  // Settings
  settingsTitle: string;
  settingsSubtitle: string;
  settingsSavedMsg: string;
  settingsDisplayGroup: string;
  settingsTargetMonitor: string;
  settingsAllMonitors: string;
  settingsSwitchMethod: string;
  settingsMethodNative: string;
  settingsMethodShortcut: string;
  settingsSwitchingPolicyTitle: string;
  settingsSwitchingPolicyDesc: string;
  settingsPolicyExitOnly: string;
  settingsPolicyExitOnlyDesc: string;
  settingsPolicyAltTab: string;
  settingsPolicyAltTabDesc: string;
  settingsDebounceGroup: string;
  settingsDebounceLabel: string;
  settingsDebounceSeconds: (sec: number) => string;
  settingsDebounceDesc: string;
  settingsSystemGroup: string;
  settingsAutostartTitle: string;
  settingsAutostartDesc: string;
  settingsStartMinimizedTitle: string;
  settingsStartMinimizedDesc: string;
  settingsAutoDetectTitle: string;
  settingsAutoDetectDesc: string;
  settingsAutoSyncTitle: string;
  settingsAutoSyncDesc: string;
  settingsNotifTitle: string;
  settingsNotifDesc: string;
  settingsLanguageTitle: string;
  settingsLanguageDesc: string;
  settingsBlacklistGroup: string;
  settingsBlacklistDesc: string;
  settingsBlacklistPlaceholder: string;
  settingsBlacklistAddBtn: string;
  settingsBlacklistEmpty: string;
  settingsStateOn: string;
  settingsStateOff: string;
  configLoading: string;
  configFirstRun: string;
  configInitialize: string;
  configImportAvailable: string;
  configImport: string;
  configRecovery: string;
  configRestore: string;
  configReset: string;
  configResetConfirm: string;
  configUnsupported: string;
  configUnavailable: string;
  configRestartHint: string;
  configFile: string;
  configSaving: string;
  configError: string;
  configControlPaused: string;
  configConfirmTarget: string;
  configMissingTarget: string;
  configNativeConsent: string;
  configAcceptNative: string;
  configTargetHint: string;
  configSelectRecovery: string;
  configMonitorIdentityError: string;
  configManualPolicy: string;
  configManualUnavailable: string;
  manualRequestRetryHint: string;
  configAllOn: string;
  configAllOff: string;
  configStatusUnknown: string;
  configActiveTarget: string;
  configRecheck: string;
  settingsNoFlicker: string;
  settingsEnglish: string;
  settingsCzech: string;
}

const cs: Translations = {
  appsRepairExecutable: 'Vybrat skutečný herní soubor',
  appsHelperAliasCleanup: 'Potvrzená aktualizace odstraní dříve uložené blokované pomocné soubory z alternativních spustitelných souborů. Ostatní alternativní soubory zůstanou zachovány.',
  appsQuarantined: 'Blokovaný soubor',
  quarantineWarning: (game, executable) => `${game}: soubor ${executable} je blokován. Automatické rozpoznávání této hry je blokováno; uložená nastavení zůstávají beze změny. V Moje hry vyberte skutečný herní soubor. Oprava odstraní staré alternativní soubory.`,
  primaryPathWarning: (game, executable) => `${game}: uložená cesta neodpovídá hlavnímu souboru ${executable}. Automatické rozpoznávání je blokováno; nastavení zůstávají beze změny. V Moje hry znovu vyberte skutečný herní soubor. Oprava odstraní staré alternativní soubory.`,
  appTitle: 'HDR AUTO-SWITCH',
  hdrActive: 'HDR AKTIVNÍ',
  hdrMixed: 'SMÍŠENÝ STAV HDR / SDR',
  sdrStandby: 'SDR STANDBY',
  themeToggle: 'Přepnout motiv',
  langToggle: 'Jazyk (Language)',

  navOverview: 'PŘEHLED',
  navApps: 'MOJE HRY',
  navCatalog: 'DATABÁZE HER',
  navProcesses: 'BĚŽÍCÍ OKNA',
  navSettings: 'NASTAVENÍ',

  heroHdrActiveTitle: 'VYBRANÉ DISPLEJE JSOU V REŽIMU HDR',
  heroSdrTitle: 'VYBRANÉ DISPLEJE JSOU V REŽIMU SDR',
  heroMixedTitle: 'VYBRANÉ DISPLEJE MAJÍ SMÍŠENÝ STAV HDR / SDR',
  heroHdrRec2020: 'HDR10 REC.2020 AKTIVNÍ',
  heroSdrBt709: 'SDR BT.709 STANDBY',
  heroHookActive: 'HOOK AKTIVOVÁN',
  heroDisplaysReady: (count) => `[${count} HDR ${count === 1 ? 'DISPLEJ PŘIPRAVEN' : count < 5 ? 'DISPLEJE PŘIPRAVENY' : 'DISPLEJŮ PŘIPRAVENO'}]`,
  heroActiveProcess: 'Aktivní HDR proces:',
  heroSdrSubtext: '> WinEventHook sleduje fokus; lehký watchdog zachytí i zmeškanou událost Windows.',
  heroTurnOffHdr: 'VYPNOUT HDR',
  heroTurnOnHdr: 'ZAPNOUT HDR RUČNĚ',
  heroSwitching: 'PŘEPÍNÁM...',

  displaysTitle: 'PŘIPOJENÉ DISPLEJE',
  displaysRefresh: 'OBNOVIT',
  displaysPrimary: 'PRIMÁRNÍ',
  displaysTargetHdr: 'CÍL HDR',
  displaysHdrSupported: 'HDR10 PODPOROVÁNO',
  displaysSdrOnly: 'POUZE SDR',
  displaysStateUnknown: 'STAV HDR NEZNÁMÝ / NEDOSTUPNÝ',
  displaysTargetId: 'TARGET ID',
  displaysHdrOn: 'HDR ZAPNUTO',
  displaysSdr: 'SDR',

  recentTitle: 'POSLEDNÍ SPUŠTĚNÉ HRY & HOOK TELEMETRIE',
  recentAllLibrary: (count) => `Všechny hry v knihovně (${count})`,
  recentHookActive: '● HOOK AKTIVNÍ (HDR ON)',
  recentHookTriggered: '✓ HOOK ZAFUNGOVAL',
  recentTierNative: 'NATIVNÍ HDR',
  recentTierAutoHdr: 'AUTO HDR',
  recentTierMod: 'HDR MOD/FIX',
  recentHdrOk: 'HDR10 OK',
  recentEmpty: 'Zatím nebyly spuštěny žádné HDR hry. Spusťte libovolnou HDR hru a telemetrie se zde automaticky zobrazí.',
  recentYesterday: 'Včera',
  recentToday: 'Dnes',

  activityTitle: 'ZÁZNAM AKTIVITY HOOKU',
  activityInitSystem: 'Rozhraní HDR Auto-Switch bylo inicializováno.',
  activityInitDetect: 'Čekání na ověřený stav ovládání HDR.',
  activityHdrManualOn: 'HDR zapnuto ručně přes ovládací panel.',
  activityHdrManualOff: 'HDR vypnuto ručně.',
  activityHdrObserved: 'Na vybraných displejích je aktivní HDR.',
  activityHdrMixed: 'Vybrané displeje mají smíšený stav HDR a SDR.',
  activityHookWindowFocus: (app) => `WinEventHook zachytil okno: ${app} -> HDR aktivováno`,
  activityHookReturnSdr: 'Vybrané displeje jsou v režimu SDR.',

  appsTitle: 'MOJE KNIHOVNA HER',
  appsCountSummary: (total, active) => `${total} CELKEM • ${active} SLEDOVÁNO`,
  appsSubtitle: 'Hry v tomto seznamu automaticky přepnou displej do HDR režimu při zaměření okna.',
  appsScanBtn: 'SKENOVAT HRY V PC',
  appsScanningBtn: 'PROHLEDÁVÁM...',
  appsAddManualBtn: 'PŘIDAT RUČNĚ',
  appsCatalogBtn: 'KATALOG',
  appsSearchPlaceholder: 'Hledat mezi nainstalovanými hrami...',
  appsTabAll: (count) => `VŠECHNY (${count})`,
  appsTabSteam: (count) => `STEAM (${count})`,
  appsTabEpic: 'EPIC GAMES',
  appsTabWindows: 'WINDOWS / OSTATNÍ',
  appsNoGamesTitle: '> Zatím zde nemáte žádné sledované hry.',
  appsNoGamesSearchTitle: '> Žádná hra neodpovídá hledání.',
  appsNoGamesSubtitle: 'Spusťte skenování disků pro automatické nalezení her nebo si vyberte z databáze PCGamingWiki.',
  appsScanDisksBtn: 'SKENOVAT DISKY',
  appsBrowseCatalogBtn: 'PROCHÁZET KATALOG',
  appsStatusTracked: 'SLEDOVÁNO',
  appsStatusPaused: 'POZASTAVENO',
  appsRemoveFromLibrary: 'Odebrat z knihovny',

  scanModalTitle: (count) => `NALEZENÉ HRY V PC (${count})`,
  scanModalSubtitle: 'Hry s ověřenou podporou HDR jsou nahoře. Předvýběr řídí nastavení automatického rozpoznávání; k importu vyberte libovolné hry:',
  scanModalSupportUnverified: 'HDR NEOVĚŘENO',
  scanModalCancel: 'ZRUŠIT',
  scanModalAddSelected: 'PŘIDAT VYBRANÉ',
  scanModalSuccess: (count) => `Úspěšně importováno nebo aktualizováno ${count} her.`,
  scanModalError: 'Chyba při prohledávání disků.',
  scanModalSectionHdr: (count) => `HRY S OVĚŘENOU PODPOROU HDR (${count})`,
  scanModalSectionSdr: (count) => `OSTATNÍ NAINSTALOVANÉ HRY (${count}) — PODPORA HDR NEOVĚŘENA`,
  scanModalStatusNew: 'NOVÁ HRA',
  scanModalStatusInLibrary: 'V KNIHOVNĚ',
  scanModalStatusPathUpdate: 'AKTUALIZOVAT CESTU',
  scanModalNewLocation: 'Nové umístění',
  scanModalSelectAllHdr: 'VYBRAT HDR',
  scanModalSelectAll: 'VYBRAT VŠE',
  scanModalDeselectAll: 'ODZNAČIT VŠE',
  appsPathMissing: 'SOUBOR NENALEZEN',
  appsPathMissingTooltip: 'Soubor nebyl nalezen na zadané cestě. Hra mohla být přesunuta na jiný disk nebo odinstalována. Spusťte Skenovat hry v PC pro aktualizaci.',
  appsGridView: 'Mřížka',
  appsListView: 'Seznam',
  scanModalSelected: (count) => `${count} vybráno`,
  manualNamePlaceholder: 'např. Silent Hill 2 / Cyberpunk 2077',
  manualExePlaceholder: 'např. SHProto-Win64-Shipping.exe',

  manualModalTitle: 'PŘIDAT HRU RUČNĚ',
  manualModalName: 'NÁZEV HRY',
  manualModalExe: 'NÁZEV EXEKUTIVY (.EXE)',
  manualModalType: 'TYP HDR PODPORY',
  manualModalCancel: 'ZRUŠIT',
  manualModalSubmit: 'PŘIDAT HRU',
  manualModalBrowseBtn: 'PROCHÁZET...',
  manualModalDragDropHint: 'Přetáhněte sem libovolný soubor .exe nebo klikněte na Procházet',
  manualModalDetectedBadge: 'ROZPOZNÁNO V KATALOGU HDR',
  manualModalPath: 'CESTA K SOUBORU',

  catalogTitle: 'DATABÁZE HDR HER',
  catalogArchiveCount: (count) => `${count} TITULŮ V ARCHIVU`,
  catalogSubtitle: 'Seznam her s nativní HDR podporou i oficiální databáze Windows Auto HDR (PCGamingWiki).',
  catalogSyncBtn: 'AKTUALIZOVAT Z WEBU',
  catalogSyncingBtn: 'SYNCHRONIZUJI...',
  catalogSyncSuccess: (count) => `Databáze byla úspěšně synchronizována z webu (${count} titulů).`,
  catalogSyncError: 'Chyba při stahování databáze z PCGamingWiki.',
  catalogSearchPlaceholder: 'Hledat hru podle názvu nebo .exe souboru...',
  catalogTabAll: (count) => `VŠECHNY (${count})`,
  catalogTabNative: (count) => `NATIVNÍ HDR (${count})`,
  catalogTabAutoHdr: (count) => `AUTO HDR (${count})`,
  catalogTabLimited: (count) => `OMEZENÉ (${count})`,
  catalogTabMod: (count) => `MOD / FIX (${count})`,
  catalogTabAlwaysOn: (count) => `ALWAYS-ON (${count})`,
  catalogTierNative: 'Nativní HDR',
  catalogTierLimited: 'Omezené',
  catalogTierAlwaysOn: 'Always-on',
  catalogTierMod: 'Vyžaduje mod',
  catalogTierAutoHdr: 'Windows Auto HDR',
  catalogTierMedia: 'Média / Video',
  catalogTierCustom: 'Vlastní',
  catalogLoading: '> Načítám katalog her...',
  catalogEmpty: '> Žádná hra neodpovídá zadanému filtru.',
  catalogAddBtn: '+ PŘIDAT DO MÝCH HER',
  catalogRemoveBtn: 'ODEBRAT',

  procTitle: 'BĚŽÍCÍ OKNA A PROCESY',
  procCountActive: (count) => `${count} AKTIVNÍCH OKEN`,
  procSubtitle: 'Aktuálně spuštěná okna na ploše. Kliknutím zařadíte libovolný běžící proces ihned do HDR sledování.',
  procRefreshBtn: 'OBNOVIT OKNA',
  procRefreshingBtn: 'SKENUJI...',
  procSearchPlaceholder: 'Filtrovat běžící okna podle názvu nebo .exe souboru...',
  procLoading: '> Skenuji běžící okna a procesy...',
  procEmpty: '> Žádný proces neodpovídá hledání.',
  procAlreadyTracked: 'JIŽ SLEDOVÁNO',
  procAddToHdr: '+ PŘIDAT DO HDR',
  procAdding: 'PŘIDÁVÁM...',

  settingsTitle: 'NASTAVENÍ APLIKACE',
  settingsSubtitle: 'Přizpůsobte si chování automatického přepínání, debounce prodlevu i spouštění se systémem.',
  settingsSavedMsg: 'Nastavení bylo úspěšně uloženo',
  settingsDisplayGroup: 'CÍLOVÝ DISPLEJ A METODA PŘEPÍNÁNÍ',
  settingsTargetMonitor: 'CÍLOVÝ MONITOR',
  settingsAllMonitors: 'Všechny HDR monitory současně',
  settingsSwitchMethod: 'METODA PŘEPÍNÁNÍ HDR',
  settingsMethodNative: 'Nativní Windows DisplayConfig API (Doporučeno)',
  settingsMethodShortcut: 'Virtuální Win + Alt + B zkratka',
  settingsSwitchingPolicyTitle: 'REŽIM VYPNUTÍ HDR (CHROVÁNÍ PŘI ALT+TAB)',
  settingsSwitchingPolicyDesc: 'Zvolte, jak má aplikace nakládat s vypínáním HDR:',
  settingsPolicyExitOnly: 'Vypnout až po ukončení hry (Doporučeno)',
  settingsPolicyExitOnlyDesc: 'Při Alt+Tab (např. kontrola Discordu či webu) zůstává HDR aktivní. Eliminuje zčernání monitoru, zpoždění a rozbití barev/swapchainu v běžících hrách. Po vypnutí hry se SDR obnoví okamžitě bez čekání.',
  settingsPolicyAltTab: 'Vypínat i při Alt+Tab (s prodlevou)',
  settingsPolicyAltTabDesc: 'Vrátí monitor do SDR po opuštění okna hry po uplynutí níže nastavených sekund. (Při úplném zavření hry se vypne okamžitě).',
  settingsDebounceGroup: 'PRODLEVA PŘI ALT+TAB (DEBOUNCE)',
  settingsDebounceLabel: 'Čas před návratem do SDR po opuštění okna hry:',
  settingsDebounceSeconds: (sec) => `${sec} SEKUND`,
  settingsDebounceDesc: 'Zabraňuje problikávání monitoru při rychlém přepínání oken. Uplatní se pouze pokud je zapnut režim Alt+Tab.',
  settingsSystemGroup: 'SYSTÉMOVÁ INTEGRACE',
  settingsAutostartTitle: 'SPOUŠTĚT AUTOMATICKY SE SYSTÉMEM',
  settingsAutostartDesc: 'Aplikace se tiše spustí na pozadí po startu Windows.',
  settingsStartMinimizedTitle: 'SPUSTIT MINIMALIZOVANĚ DO TRAY',
  settingsStartMinimizedDesc: 'Aplikace po spuštění zůstane skrytá v oznamovací oblasti lišty a neotevírá okno.',
  settingsAutoDetectTitle: 'AUTOMATICKÁ DETEKCE NOVÝCH HDR HER',
  settingsAutoDetectDesc: 'Při spuštění jakékoliv HDR hry z 949+ katalogu ji automaticky zařadí do sledování a zapne HDR bez nutnosti ručního přidávání.',
  settingsAutoSyncTitle: 'AUTOMATICKÁ SYNCHRONIZACE DATABÁZE (PCGAMINGWIKI)',
  settingsAutoSyncDesc: 'Jednou týdně na pozadí tiše aktualizuje katalog her z PCGamingWiki a prohledá disky pro spárování Steam ID.',
  settingsNotifTitle: 'WINDOWS NOTIFIKACE',
  settingsNotifDesc: 'Zobrazovat decentní systémové oznámení při každém přepnutí HDR režimu.',
  settingsLanguageTitle: 'JAZYK ROZHRANÍ (LANGUAGE)',
  settingsLanguageDesc: 'Zvolte jazyk aplikace nebo ponechte automatickou detekci podle systému Windows.',
  settingsBlacklistGroup: 'BLOKOVANÉ APLIKACE (BLACKLIST)',
  settingsBlacklistDesc: 'Procesy, které nikdy nesmí aktivovat HDR (např. prohlížeče, editory apod.):',
  settingsBlacklistPlaceholder: 'např. chrome.exe nebo obs64.exe',
  settingsBlacklistAddBtn: 'PŘIDAT',
  settingsBlacklistEmpty: '> Žádné blokované procesy.',
  settingsStateOn: 'ZAPNUTO',
  settingsStateOff: 'VYPNUTO',
  configLoading: 'Načítání nastavení. Přepínání HDR zatím není dostupné.',
  configFirstRun: 'Nebylo nalezeno nastavení. Vytvořte místní konfiguraci pro tento počítač.',
  configInitialize: 'VYTVOŘIT NASTAVENÍ',
  configImportAvailable: 'Bylo nalezeno starší nastavení. Import zachová knihovnu a předvolby; původní soubor zůstane beze změny. Konkrétní monitor je nutné znovu potvrdit.',
  configImport: 'IMPORTOVAT STARŠÍ NASTAVENÍ',
  configRecovery: 'Nastavení vyžaduje obnovu. Automatické HDR je pozastavené; původní soubory nebudou přepsány výchozími hodnotami.',
  configRestore: 'OBNOVIT VYBRANOU KOPII',
  configReset: 'VYTVOŘIT NOVÉ VÝCHOZÍ NASTAVENÍ',
  configResetConfirm: 'Chci začít s prázdnou knihovnou a výchozím nastavením. Původní soubory budou zachovány pro obnovu.',
  configUnsupported: 'Toto nastavení pochází z novější verze aplikace. Otevřete jej v novější verzi; obnova ani reset zde nejsou povoleny.',
  configUnavailable: 'Nastavení není dostupné. Změny a automatické HDR jsou pozastavené.',
  configRestartHint: 'Soubory nastavení upravujte pouze při vypnuté aplikaci. Po vyřešení problému aplikaci restartujte.',
  configFile: 'SOUBOR NASTAVENÍ',
  configSaving: 'UKLÁDÁNÍ...',
  configError: 'Akci se nepodařilo dokončit:',
  configControlPaused: 'OVLÁDÁNÍ HDR POZASTAVENO',
  configConfirmTarget: 'Staré ID monitoru není trvalé. V Nastavení znovu vyberte monitor; jiné displeje se mezitím nebudou přepínat.',
  configMissingTarget: 'Vybraný monitor není dostupný',
  configNativeConsent: 'Klávesová zkratka Win + Alt + B nemůže bezpečně ovládat konkrétní monitor. Automatické HDR čeká na váš souhlas s nativním ovládáním.',
  configAcceptNative: 'POVOLIT AUTOMATICKÉ NATIVNÍ HDR',
  configTargetHint: 'Výběr platí pro příští spuštění hry. Odpojený monitor zůstane vybraný; aplikace jej nenahradí jiným.',
  configSelectRecovery: 'Vyberte ověřenou kopii nastavení',
  configMonitorIdentityError: 'Trvalá identita displeje není dostupná; ovládání je zablokované.',
  configManualPolicy: 'Ruční nativní Zapnout/Vypnout platí jen pro výslovně zvolený displej nebo Vše. Funguje i při pozastavené automatizaci; nemění nastavení ani souhlas s automatickým HDR. Konflikt ovladače, ukončování nebo nedostupná autorita ovládání blokují.',
  configManualUnavailable: 'Ruční HDR není dostupné: ověřte autoritu ovládání, identitu a stav displeje.',
  manualRequestRetryHint: 'Doručení požadavku nebylo potvrzeno. Zopakujte stejný ovládací prvek v původním okně nebo nabídce tray. Výsledek jiného požadavku tuto chybu nesmaže.',
  configAllOn: 'ZAPNOUT HDR NA VŠECH DISPLEJÍCH',
  configAllOff: 'VYPNOUT HDR NA VŠECH DISPLEJÍCH',
  configStatusUnknown: 'STAV HDR NEZNÁMÝ / POZASTAVENO',
  configActiveTarget: 'CÍL AKTUÁLNÍ RELACE',
  configRecheck: 'ZNOVU OVĚŘIT OVLÁDÁNÍ HDR',
  settingsNoFlicker: 'BEZ PŘEPÍNÁNÍ PŘI ALT+TAB',
  settingsEnglish: 'Angličtina (EN)',
  settingsCzech: 'Čeština (CZ)',
};

const en: Translations = {
  appsRepairExecutable: 'Select actual game executable',
  appsHelperAliasCleanup: 'Confirming an update removes previously saved blocked helper aliases. Other executable aliases are preserved.',
  appsQuarantined: 'Blocked executable',
  quarantineWarning: (game, executable) => `${game}: ${executable} is blocked. Automatic matching for this game is blocked; saved settings remain unchanged. Select the actual game executable in My Games. Repair removes historical executable aliases.`,
  primaryPathWarning: (game, executable) => `${game}: the saved path does not match primary executable ${executable}. Automatic matching is blocked; saved settings remain unchanged. Select the actual game executable again in My Games. Repair removes historical executable aliases.`,
  appTitle: 'HDR AUTO-SWITCH',
  hdrActive: 'HDR ACTIVE',
  hdrMixed: 'MIXED HDR / SDR',
  sdrStandby: 'SDR STANDBY',
  themeToggle: 'Toggle theme',
  langToggle: 'Language',

  navOverview: 'OVERVIEW',
  navApps: 'MY GAMES',
  navCatalog: 'GAME CATALOG',
  navProcesses: 'RUNNING WINDOWS',
  navSettings: 'SETTINGS',

  heroHdrActiveTitle: 'SELECTED DISPLAYS ARE IN HDR MODE',
  heroSdrTitle: 'SELECTED DISPLAYS ARE IN SDR MODE',
  heroMixedTitle: 'SELECTED DISPLAYS HAVE MIXED HDR / SDR STATES',
  heroHdrRec2020: 'HDR10 REC.2020 ACTIVE',
  heroSdrBt709: 'SDR BT.709 STANDBY',
  heroHookActive: 'HOOK TRIGGERED',
  heroDisplaysReady: (count) => `[${count} HDR ${count === 1 ? 'DISPLAY READY' : 'DISPLAYS READY'}]`,
  heroActiveProcess: 'Active HDR process:',
  heroSdrSubtext: '> WinEventHook monitors focus; a lightweight watchdog recovers missed Windows events.',
  heroTurnOffHdr: 'TURN OFF HDR',
  heroTurnOnHdr: 'TURN ON HDR MANUALLY',
  heroSwitching: 'SWITCHING...',

  displaysTitle: 'CONNECTED DISPLAYS',
  displaysRefresh: 'REFRESH',
  displaysPrimary: 'PRIMARY',
  displaysTargetHdr: 'HDR TARGET',
  displaysHdrSupported: 'HDR10 SUPPORTED',
  displaysSdrOnly: 'SDR ONLY',
  displaysStateUnknown: 'HDR STATE UNKNOWN / UNAVAILABLE',
  displaysTargetId: 'TARGET ID',
  displaysHdrOn: 'HDR ENABLED',
  displaysSdr: 'SDR',

  recentTitle: 'RECENTLY PLAYED & HOOK TELEMETRY',
  recentAllLibrary: (count) => `All games in library (${count})`,
  recentHookActive: '● HOOK ACTIVE (HDR ON)',
  recentHookTriggered: '✓ HOOK TRIGGERED',
  recentTierNative: 'NATIVE HDR',
  recentTierAutoHdr: 'AUTO HDR',
  recentTierMod: 'HDR MOD/FIX',
  recentHdrOk: 'HDR10 OK',
  recentEmpty: 'No HDR games played yet. Launch any HDR-supported game and telemetry will automatically appear here.',
  recentYesterday: 'Yesterday',
  recentToday: 'Today',

  activityTitle: 'HOOK ACTIVITY LOG',
  activityInitSystem: 'HDR Auto-Switch interface initialized.',
  activityInitDetect: 'Waiting for a verified HDR controller status.',
  activityHdrManualOn: 'HDR enabled manually via dashboard control.',
  activityHdrManualOff: 'HDR disabled manually.',
  activityHdrObserved: 'HDR is active on the selected displays.',
  activityHdrMixed: 'The selected displays have mixed HDR and SDR states.',
  activityHookWindowFocus: (app) => `WinEventHook foreground focus: ${app} -> HDR enabled`,
  activityHookReturnSdr: 'The selected displays are in SDR mode.',

  appsTitle: 'MY GAME LIBRARY',
  appsCountSummary: (total, active) => `${total} TOTAL • ${active} TRACKED`,
  appsSubtitle: 'Games in this library automatically switch your display into HDR10 mode upon window focus.',
  appsScanBtn: 'SCAN PC FOR GAMES',
  appsScanningBtn: 'SCANNING DRIVES...',
  appsAddManualBtn: 'ADD MANUALLY',
  appsCatalogBtn: 'CATALOG',
  appsSearchPlaceholder: 'Search tracked games...',
  appsTabAll: (count) => `ALL (${count})`,
  appsTabSteam: (count) => `STEAM (${count})`,
  appsTabEpic: 'EPIC GAMES',
  appsTabWindows: 'WINDOWS / OTHER',
  appsNoGamesTitle: '> No tracked games found in your library.',
  appsNoGamesSearchTitle: '> No games matched your search query.',
  appsNoGamesSubtitle: 'Scan your local drives for installed games or browse the PCGamingWiki database.',
  appsScanDisksBtn: 'SCAN DRIVES',
  appsBrowseCatalogBtn: 'BROWSE CATALOG',
  appsStatusTracked: 'TRACKED',
  appsStatusPaused: 'PAUSED',
  appsRemoveFromLibrary: 'Remove from library',

  scanModalTitle: (count) => `FOUND GAMES ON PC (${count})`,
  scanModalSubtitle: 'Verified HDR games are grouped first. Automatic detection controls initial selection; select any games to import:',
  scanModalSupportUnverified: 'HDR UNVERIFIED',
  scanModalCancel: 'CANCEL',
  scanModalAddSelected: 'ADD SELECTED',
  scanModalSuccess: (count) => `Successfully imported or updated ${count} games.`,
  scanModalError: 'Failed to scan storage drives.',
  scanModalSectionHdr: (count) => `VERIFIED HDR SUPPORTED GAMES (${count})`,
  scanModalSectionSdr: (count) => `OTHER INSTALLED GAMES (${count}) — HDR SUPPORT UNVERIFIED`,
  scanModalStatusNew: 'NEW',
  scanModalStatusInLibrary: 'IN LIBRARY',
  scanModalStatusPathUpdate: 'UPDATE PATH',
  scanModalNewLocation: 'New location',
  scanModalSelectAllHdr: 'SELECT HDR',
  scanModalSelectAll: 'SELECT ALL',
  scanModalDeselectAll: 'DESELECT ALL',
  appsPathMissing: 'FILE NOT FOUND',
  appsPathMissingTooltip: 'File not found at specified path. Game may have been moved to another drive or uninstalled. Run Scan PC for games to update.',
  appsGridView: 'Grid view',
  appsListView: 'List view',
  scanModalSelected: (count) => `${count} selected`,
  manualNamePlaceholder: 'e.g. Silent Hill 2 / Cyberpunk 2077',
  manualExePlaceholder: 'e.g. SHProto-Win64-Shipping.exe',

  manualModalTitle: 'ADD GAME MANUALLY',
  manualModalName: 'GAME TITLE',
  manualModalExe: 'EXECUTABLE NAME (.EXE)',
  manualModalType: 'HDR SUPPORT TYPE',
  manualModalCancel: 'CANCEL',
  manualModalSubmit: 'ADD GAME',
  manualModalBrowseBtn: 'BROWSE...',
  manualModalDragDropHint: 'Drag and drop any .exe file here or click Browse',
  manualModalDetectedBadge: 'MATCHED IN HDR DATABASE',
  manualModalPath: 'EXECUTABLE PATH',

  catalogTitle: 'HDR GAME DATABASE',
  catalogArchiveCount: (count) => `${count} TITLES IN ARCHIVE`,
  catalogSubtitle: 'Curated database of Native HDR games and official Windows Auto HDR titles (PCGamingWiki).',
  catalogSyncBtn: 'UPDATE FROM WEB',
  catalogSyncingBtn: 'SYNCING...',
  catalogSyncSuccess: (count) => `Database synchronized successfully from web (${count} titles).`,
  catalogSyncError: 'Failed to fetch catalog from PCGamingWiki.',
  catalogSearchPlaceholder: 'Search database by title or .exe filename...',
  catalogTabAll: (count) => `ALL (${count})`,
  catalogTabNative: (count) => `NATIVE HDR (${count})`,
  catalogTabAutoHdr: (count) => `AUTO HDR (${count})`,
  catalogTabLimited: (count) => `LIMITED (${count})`,
  catalogTabMod: (count) => `MOD / FIX (${count})`,
  catalogTabAlwaysOn: (count) => `ALWAYS-ON (${count})`,
  catalogTierNative: 'Native HDR',
  catalogTierLimited: 'Limited',
  catalogTierAlwaysOn: 'Always-on',
  catalogTierMod: 'Requires Mod',
  catalogTierAutoHdr: 'Windows Auto HDR',
  catalogTierMedia: 'Media / Video',
  catalogTierCustom: 'Custom',
  catalogLoading: '> Loading game database...',
  catalogEmpty: '> No games matched your selected filter.',
  catalogAddBtn: '+ ADD TO MY GAMES',
  catalogRemoveBtn: 'REMOVE',

  procTitle: 'RUNNING WINDOWS & PROCESSES',
  procCountActive: (count) => `${count} ACTIVE WINDOWS`,
  procSubtitle: 'Currently running desktop windows. Click to instantly add any active game to HDR tracking.',
  procRefreshBtn: 'REFRESH WINDOWS',
  procRefreshingBtn: 'SCANNING...',
  procSearchPlaceholder: 'Filter active processes by window title or .exe name...',
  procLoading: '> Scanning active desktop windows...',
  procEmpty: '> No running processes matched your search query.',
  procAlreadyTracked: 'ALREADY TRACKED',
  procAddToHdr: '+ ADD TO HDR',
  procAdding: 'ADDING...',

  settingsTitle: 'APPLICATION SETTINGS',
  settingsSubtitle: 'Configure automatic switching behavior, Alt+Tab debounce delay, and Windows startup.',
  settingsSavedMsg: 'Settings saved successfully',
  settingsDisplayGroup: 'TARGET DISPLAY & SWITCHING METHOD',
  settingsTargetMonitor: 'TARGET DISPLAY',
  settingsAllMonitors: 'All connected HDR displays simultaneously',
  settingsSwitchMethod: 'HDR SWITCHING METHOD',
  settingsMethodNative: 'Native Windows DisplayConfig API (Recommended)',
  settingsMethodShortcut: 'Simulated Win + Alt + B Keyboard Shortcut',
  settingsSwitchingPolicyTitle: 'HDR DEACTIVATION POLICY (ALT+TAB BEHAVIOR)',
  settingsSwitchingPolicyDesc: 'Choose when the application should restore SDR mode on your displays:',
  settingsPolicyExitOnly: 'Only when the game exits (Recommended)',
  settingsPolicyExitOnlyDesc: 'Keeps HDR active during Alt+Tab (checking Discord, web browser, etc.). Completely eliminates monitor blackouts, renegotiation lag, and DirectX swapchain desync. Switches back to SDR immediately when the game closes.',
  settingsPolicyAltTab: 'Deactivate on Alt+Tab (with debounce delay)',
  settingsPolicyAltTabDesc: 'Restores SDR mode after the debounce delay below when you switch focus away from the game. (When the game completely closes, SDR is restored immediately).',
  settingsDebounceGroup: 'ALT+TAB DEBOUNCE DELAY',
  settingsDebounceLabel: 'Delay before returning to SDR after leaving game window:',
  settingsDebounceSeconds: (sec) => `${sec} SECONDS`,
  settingsDebounceDesc: 'Prevents display flicker during rapid window switching. Only applies when Alt+Tab mode is selected.',
  settingsSystemGroup: 'SYSTEM INTEGRATION',
  settingsAutostartTitle: 'START WITH WINDOWS',
  settingsAutostartDesc: 'Silently launch upon Windows boot.',
  settingsStartMinimizedTitle: 'START MINIMIZED TO TRAY',
  settingsStartMinimizedDesc: 'Application starts hidden in the Windows notification tray without opening the window.',
  settingsAutoDetectTitle: 'AUTO-DETECT & ENROLL NEW HDR GAMES',
  settingsAutoDetectDesc: 'Automatically detects when you launch any HDR game from the 949+ catalog, adds it to library and activates HDR instantly.',
  settingsAutoSyncTitle: 'AUTOMATIC DATABASE SYNC (PCGAMINGWIKI)',
  settingsAutoSyncDesc: 'Periodically synchronizes newly released HDR game titles from PCGamingWiki and matches Steam IDs.',
  settingsNotifTitle: 'WINDOWS NOTIFICATIONS',
  settingsNotifDesc: 'Display subtle native notifications whenever display mode switches.',
  settingsLanguageTitle: 'USER INTERFACE LANGUAGE',
  settingsLanguageDesc: 'Choose your preferred language or allow automatic Windows system detection.',
  settingsBlacklistGroup: 'BLOCKED APPLICATIONS (BLACKLIST)',
  settingsBlacklistDesc: 'Processes that are explicitly prohibited from triggering HDR (e.g. browsers, editors):',
  settingsBlacklistPlaceholder: 'e.g. chrome.exe or obs64.exe',
  settingsBlacklistAddBtn: 'ADD',
  settingsBlacklistEmpty: '> No blocked applications defined.',
  settingsStateOn: 'ENABLED',
  settingsStateOff: 'DISABLED',
  configLoading: 'Loading settings. HDR controls are not available yet.',
  configFirstRun: 'No settings were found. Create a machine-local configuration to get started.',
  configInitialize: 'CREATE SETTINGS',
  configImportAvailable: 'Older settings were found. Import preserves your library and preferences and leaves the original file untouched. A specific monitor must be confirmed again.',
  configImport: 'IMPORT OLDER SETTINGS',
  configRecovery: 'Settings need recovery. Automatic HDR is paused; the original files will not be overwritten with defaults.',
  configRestore: 'RESTORE SELECTED COPY',
  configReset: 'CREATE NEW DEFAULT SETTINGS',
  configResetConfirm: 'Start with an empty library and default settings. Preserve the original files as recovery evidence.',
  configUnsupported: 'These settings belong to a newer application version. Open them with that version; restore and reset are disabled here.',
  configUnavailable: 'Settings are unavailable. Changes and automatic HDR are paused.',
  configRestartHint: 'Edit settings files only while the app is closed. Restart the app after resolving the problem.',
  configFile: 'SETTINGS FILE',
  configSaving: 'SAVING...',
  configError: 'The action could not be completed:',
  configControlPaused: 'HDR CONTROL PAUSED',
  configConfirmTarget: 'The old monitor ID is not persistent. Choose your monitor again in Settings; other displays will not be substituted.',
  configMissingTarget: 'Selected monitor is unavailable',
  configNativeConsent: 'Win + Alt + B cannot safely control a specific monitor. Automatic HDR is paused until you consent to native display control.',
  configAcceptNative: 'ENABLE AUTOMATIC NATIVE HDR',
  configTargetHint: 'Selection changes apply to the next game activation. A disconnected display stays selected; no other display is substituted.',
  configSelectRecovery: 'Choose a validated settings copy',
  configMonitorIdentityError: 'A persistent display identity is unavailable; control is blocked.',
  configManualPolicy: 'Manual native On/Off applies only to the explicitly chosen display or All. It remains available while automation is paused and never changes settings or automatic HDR consent. Controller conflicts, shutdown, or unavailable authority block manual control.',
  configManualUnavailable: 'Manual HDR is unavailable: check controller authority, display identity, and state.',
  manualRequestRetryHint: 'Request delivery was not confirmed. Retry the same control from the original window or tray menu. An unrelated result will not dismiss this error.',
  configAllOn: 'TURN HDR ON FOR ALL DISPLAYS',
  configAllOff: 'TURN HDR OFF FOR ALL DISPLAYS',
  configStatusUnknown: 'HDR STATE UNKNOWN / PAUSED',
  configActiveTarget: 'CURRENT SESSION TARGET',
  configRecheck: 'RECHECK HDR CONTROLLER',
  settingsNoFlicker: 'NO SWITCHING ON ALT+TAB',
  settingsEnglish: 'English (EN)',
  settingsCzech: 'Czech (CZ)',
};

export const dictionaries: Record<Language, Translations> = { cs, en };

export function detectDefaultLanguage(): Language {
  try {
    const saved = localStorage.getItem('hdr_lang');
    if (saved === 'cs' || saved === 'en') return saved;
    const navLang = navigator.language || '';
    if (navLang.toLowerCase().startsWith('cs') || navLang.toLowerCase().startsWith('sk')) {
      return 'cs';
    }
  } catch (e) {
    console.error(e);
  }
  return 'en';
}

interface I18nContextType {
  lang: Language;
  setLang: (l: Language) => void;
  t: Translations;
}

export const I18nContext = createContext<I18nContextType>({
  lang: 'en',
  setLang: () => {},
  t: en,
});

export const useI18n = () => useContext(I18nContext);
