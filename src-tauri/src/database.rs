use crate::config::{HdrApp, HdrType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorefrontProvider {
    Steam,
    Xbox,
}

/// Authored catalog data only; no installation, package identity, or runtime evidence is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorefrontBinding {
    pub provider: StorefrontProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_id: Option<String>,
    #[serde(default)]
    pub game_executables: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_executables: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorefrontLookupError {
    AmbiguousBindings,
    InvalidProductId,
    InvalidExecutable,
    DuplicateExecutable,
    ExcludedGameExecutable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutableAuthority {
    canonical_exe: String,
    steam_id: Option<String>,
    nominations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    /// Explicit equivalent product names, authored only in the embedded catalog.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_aliases: Vec<String>,
    pub exe_name: String,
    pub hdr_type: HdrType,
    pub support_tier: String, // "native", "limited", "always_on", "manual_fix", "autohdr", "media"
    pub notes: Option<String>,
    #[serde(default)]
    pub steam_id: Option<String>,
    #[serde(default)]
    pub alternate_exes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub storefronts: Vec<StorefrontBinding>,
    // Source authority is never restored from serialized display/suggestion fields.
    #[serde(skip)]
    storefront_authority: Option<Vec<StorefrontBinding>>,
    #[serde(skip)]
    executable_authority: Option<ExecutableAuthority>,
}

impl CatalogEntry {
    fn legacy_steam_binding(&self) -> Option<StorefrontBinding> {
        self.steam_id.as_ref().map(|product_id| StorefrontBinding {
            provider: StorefrontProvider::Steam,
            product_id: Some(product_id.clone()),
            game_executables: vec![self.exe_name.clone()],
            excluded_executables: Vec::new(),
        })
    }

    fn authoritative_bindings(&self) -> Vec<StorefrontBinding> {
        self.storefront_authority.clone().unwrap_or_default()
    }

    fn capture_executable_authority(&mut self) {
        self.executable_authority = Some(ExecutableAuthority {
            canonical_exe: self.exe_name.clone(),
            steam_id: self.steam_id.clone(),
            nominations: std::iter::once(self.exe_name.clone())
                .chain(self.alternate_exes.iter().cloned()).collect(),
        });
    }

    pub(crate) fn authoritative_primary(&self) -> Option<&str> {
        self.executable_authority.as_ref().map(|source| source.canonical_exe.as_str())
    }

    pub(crate) fn authoritative_steam_id(&self) -> Option<&str> {
        self.executable_authority.as_ref().and_then(|source| source.steam_id.as_deref())
    }

    pub(crate) fn authorizes_declared_executable(&self, basename: &str) -> bool {
        self.executable_authority.as_ref().is_some_and(|source| {
            source.nominations.iter().any(|exe| exe.eq_ignore_ascii_case(basename))
        })
    }

    /// The source snapshot applies the legacy Steam adapter only at the embedded boundary.
    /// Deserialization and mutable display fields cannot supply automatic authority.
    pub fn storefront_binding(
        &self,
        provider: StorefrontProvider,
        product_id: Option<&str>,
    ) -> Result<Option<StorefrontBinding>, StorefrontLookupError> {
        find_storefront_binding(std::slice::from_ref(self), provider, product_id)
            .map(|found| found.map(|(_, binding)| binding))
    }
}

#[cfg(test)]
pub(crate) fn authored_test_catalog(mut entries: Vec<CatalogEntry>) -> Vec<CatalogEntry> {
    for entry in &mut entries {
        let mut bindings = entry.storefronts.clone();
        if !bindings.iter().any(|binding| binding.provider == StorefrontProvider::Steam) {
            bindings.extend(entry.legacy_steam_binding());
        }
        entry.storefront_authority = Some(bindings);
        entry.capture_executable_authority();
    }
    entries
}

fn normalized_executables(names: &[String]) -> Result<Vec<String>, StorefrontLookupError> {
    let mut normalized: Vec<String> = names.iter().map(|name| name.trim().to_lowercase()).collect();
    if normalized.iter().any(|name| {
        name.len() <= 4 || !name.ends_with(".exe") || name.contains(['\\', '/', ':'])
    }) {
        return Err(StorefrontLookupError::InvalidExecutable);
    }
    normalized.sort();
    if normalized.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(StorefrontLookupError::DuplicateExecutable);
    }
    Ok(normalized)
}

impl StorefrontBinding {
    fn normalized(mut self) -> Result<Self, StorefrontLookupError> {
        if let Some(product_id) = &mut self.product_id {
            *product_id = product_id.trim().to_string();
            if product_id.is_empty() {
                return Err(StorefrontLookupError::InvalidProductId);
            }
        }
        self.game_executables = normalized_executables(&self.game_executables)?;
        self.excluded_executables = normalized_executables(&self.excluded_executables)?;
        if self.game_executables.iter().any(|name| self.excluded_executables.contains(name)) {
            return Err(StorefrontLookupError::ExcludedGameExecutable);
        }
        Ok(self)
    }
}

/// Pure provider/product lookup. None leaves the product unconstrained; multiple candidates
/// (including identical duplicate rows) are ambiguous, never resolved by list order.
/// Names are normalized on returned copies only. Product IDs remain provider-local and case-sensitive.
pub fn find_storefront_binding<'a>(
    catalog: &'a [CatalogEntry],
    provider: StorefrontProvider,
    product_id: Option<&str>,
) -> Result<Option<(&'a CatalogEntry, StorefrontBinding)>, StorefrontLookupError> {
    let product_id = product_id.map(str::trim);
    if product_id == Some("") {
        return Err(StorefrontLookupError::InvalidProductId);
    }
    let mut candidates = catalog.iter().flat_map(|entry| {
        entry.authoritative_bindings().into_iter().filter_map(move |binding| {
            (binding.provider == provider
                && product_id.is_none_or(|id| binding.product_id.as_deref().map(str::trim) == Some(id)))
                .then_some((entry, binding))
        })
    });
    let Some((entry, binding)) = candidates.next() else {
        return Ok(None);
    };
    if candidates.next().is_some() {
        return Err(StorefrontLookupError::AmbiguousBindings);
    }
    binding.normalized().map(|binding| Some((entry, binding)))
}

static EMBEDDED_CATALOG_JSON: &str = include_str!("../catalog.json");

static CACHED_CATALOG: CatalogCache = CatalogCache::new();

struct CatalogCacheState {
    entries: Option<Vec<CatalogEntry>>,
    revision: u64,
}

struct CatalogCache {
    state: RwLock<CatalogCacheState>,
    syncing: AtomicBool,
}

struct CatalogSync<'a>(&'a AtomicBool);

impl Drop for CatalogSync<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl CatalogCache {
    const fn new() -> Self {
        Self {
            state: RwLock::new(CatalogCacheState { entries: None, revision: 0 }),
            syncing: AtomicBool::new(false),
        }
    }

    fn begin_sync(&self) -> Result<CatalogSync<'_>, String> {
        self.syncing.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Catalog synchronization is already in progress.".to_string())?;
        Ok(CatalogSync(&self.syncing))
    }

    fn snapshot(&self, path: &Path) -> Result<(Vec<CatalogEntry>, u64), String> {
        self.snapshot_with(path, read_cached_catalog)
    }

    fn snapshot_with(
        &self,
        path: &Path,
        load: impl FnOnce(&Path) -> Vec<CatalogEntry>,
    ) -> Result<(Vec<CatalogEntry>, u64), String> {
        {
            let state = self.state.read().map_err(|_| "Catalog cache lock is poisoned.")?;
            if let Some(entries) = &state.entries {
                return Ok((entries.clone(), state.revision));
            }
        }
        let mut state = self.state.write().map_err(|_| "Catalog cache lock is poisoned.")?;
        // A publisher or another cold reader may have populated the cache while we waited.
        if state.entries.is_none() {
            state.entries = Some(merge_catalog(embedded_catalog(), load(path)));
        }
        Ok((state.entries.as_ref().unwrap().clone(), state.revision))
    }

    fn publish(
        &self,
        path: &Path,
        entries: &[CatalogEntry],
        expected_revision: Option<u64>,
    ) -> Result<Vec<CatalogEntry>, String> {
        self.publish_with(path, entries, expected_revision, write_cache_atomically)
    }

    fn publish_with(
        &self,
        path: &Path,
        entries: &[CatalogEntry],
        expected_revision: Option<u64>,
        write: impl FnOnce(&Path, &[u8]) -> Result<(), String>,
    ) -> Result<Vec<CatalogEntry>, String> {
        let mut state = self.state.write().map_err(|_| "Catalog cache lock is poisoned.")?;
        if expected_revision.is_some_and(|expected| expected != state.revision) {
            return Err("The catalog changed during synchronization. Retry the sync.".into());
        }
        let revision = state.revision.checked_add(1).ok_or("Catalog revision exhausted.")?;
        let entries = with_embedded_authority(entries.to_vec());
        let json = serde_json::to_vec_pretty(&entries).map_err(|error| error.to_string())?;
        // Disk replacement and memory publication are one serialized transaction. Failed writes
        // leave both the last complete disk document and the in-memory snapshot unchanged.
        write(path, &json)?;
        state.entries = Some(entries.clone());
        state.revision = revision;
        Ok(entries)
    }
}

fn get_cache_path() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        Err("Catalog I/O requires an explicitly injected path in tests.".into())
    }
    #[cfg(not(test))]
    {
        let app_data = std::env::var_os("APPDATA").filter(|value| !value.is_empty())
            .ok_or("APPDATA is unavailable; the catalog cache cannot be located.")?;
        let app_data = PathBuf::from(app_data);
        if !app_data.is_absolute() {
            return Err("APPDATA must identify an absolute catalog cache location.".into());
        }
        Ok(app_data.join("HDRAutoSwitch").join("catalog_cache.json"))
    }
}

pub fn get_full_catalog() -> Vec<CatalogEntry> {
    #[cfg(test)]
    {
        merge_catalog(embedded_catalog(), Vec::new())
    }
    #[cfg(not(test))]
    {
        match get_cache_path().and_then(|path| CACHED_CATALOG.snapshot(&path)) {
            Ok((entries, _)) => entries,
            Err(error) => {
                eprintln!("{error} Using embedded catalog.");
                merge_catalog(embedded_catalog(), Vec::new())
            }
        }
    }
}

fn read_cached_catalog(path: &Path) -> Vec<CatalogEntry> {
    match fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<Vec<CatalogEntry>>(&content) {
            Ok(entries) => entries,
            Err(error) => {
                eprintln!("Cannot parse catalog cache; using embedded catalog: {error}");
                Vec::new()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            eprintln!("Cannot read catalog cache; using embedded catalog: {error}");
            Vec::new()
        }
    }
}

fn embedded_catalog() -> Vec<CatalogEntry> {
    serde_json::from_str(EMBEDDED_CATALOG_JSON).expect("Embedded catalog must be valid")
}

#[derive(Clone, PartialEq, Eq)]
struct DisplayMetadata {
    support_tier: String,
    notes: Option<String>,
}

struct DisplayOverlay {
    canonical: bool,
    metadata: Option<DisplayMetadata>,
}

fn merge_catalog(mut embedded: Vec<CatalogEntry>, cached: Vec<CatalogEntry>) -> Vec<CatalogEntry> {
    let mut identities: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, entry) in embedded.iter_mut().enumerate() {
        let mut bindings = entry.storefronts.clone();
        if !bindings.iter().any(|binding| binding.provider == StorefrontProvider::Steam) {
            bindings.extend(entry.legacy_steam_binding());
        }
        entry.storefront_authority = Some(bindings);
        entry.capture_executable_authority();
        for name in std::iter::once(&entry.name).chain(&entry.name_aliases) {
            let indices = identities.entry(clean_key(name)).or_default();
            if !indices.contains(&index) {
                indices.push(index);
            }
        }
    }

    // Never merge authored rows by executable, product ID, or fuzzy name. Conflicting source
    // rows stay separate and ambiguous. Only explicit names can target a display-only overlay.
    let mut overlays: HashMap<usize, DisplayOverlay> = HashMap::new();
    let mut suggestions: HashMap<String, Option<CatalogEntry>> = HashMap::new();
    for mut entry in cached {
        if !valid_catalog_entry(&entry) {
            continue;
        }
        let key = clean_key(&entry.name);
        if let Some(indices) = identities.get(&key) {
            if let [index] = indices.as_slice() {
                let canonical = key == clean_key(&embedded[*index].name);
                let metadata = DisplayMetadata { support_tier: entry.support_tier, notes: entry.notes };
                overlays.entry(*index).and_modify(|overlay| {
                    if canonical && !overlay.canonical {
                        overlay.canonical = true;
                        overlay.metadata = Some(metadata.clone());
                    } else if canonical == overlay.canonical && overlay.metadata.as_ref() != Some(&metadata) {
                        overlay.metadata = None;
                    }
                }).or_insert(DisplayOverlay { canonical, metadata: Some(metadata) });
            }
            continue;
        }
        restrict_storefront_authority(&mut entry, None);
        suggestions.entry(key).and_modify(|existing| {
            if existing.as_ref() != Some(&entry) {
                *existing = None;
            }
        }).or_insert(Some(entry));
    }
    for (index, overlay) in overlays {
        if let Some(metadata) = overlay.metadata {
            embedded[index].support_tier = metadata.support_tier;
            embedded[index].notes = metadata.notes;
        }
    }

    embedded.extend(suggestions.into_values().flatten());
    embedded.sort_by(|a, b| a.name.cmp(&b.name));
    embedded
}

fn restrict_storefront_authority(entry: &mut CatalogEntry, embedded: Option<&CatalogEntry>) {
    if let Some(source) = embedded {
        let support_tier = entry.support_tier.clone();
        let notes = entry.notes.clone();
        *entry = source.clone();
        entry.support_tier = support_tier;
        entry.notes = notes;
    } else {
        entry.name_aliases.clear();
        entry.storefronts.clear();
        entry.storefront_authority = Some(Vec::new());
        entry.executable_authority = None;
    }
}

// Online results, disk reloads, and explicit saves use exactly the same merge policy.
fn with_embedded_authority(entries: Vec<CatalogEntry>) -> Vec<CatalogEntry> {
    merge_catalog(embedded_catalog(), entries)
}

pub fn save_to_cache(entries: &[CatalogEntry]) -> Result<(), String> {
    CACHED_CATALOG.publish(&get_cache_path()?, entries, None).map(|_| ())
}

struct CacheStage(PathBuf);

impl Drop for CacheStage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn write_cache_atomically(path: &Path, json: &[u8]) -> Result<(), String> {
    write_cache_atomically_with(path, json, |_| Ok(()))
}

fn write_cache_atomically_with(
    path: &Path,
    json: &[u8],
    before_replace: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("Cannot create catalog cache: {error}"))?;
    }
    let stage_path = path.with_file_name(format!(".catalog-cache-{}.stage", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&stage_path)
        .map_err(|error| format!("Cannot stage catalog cache: {error}"))?;
    let stage = CacheStage(stage_path);
    let flushed = file.write_all(json).and_then(|_| file.sync_all());
    drop(file);
    flushed.map_err(|error| format!("Cannot flush catalog cache: {error}"))?;
    before_replace(&stage.0)?;
    replace_cache_file(&stage.0, path)
}

#[cfg(windows)]
fn replace_cache_file(stage: &Path, path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let wide = |path: &Path| -> Result<Vec<u16>, String> {
        let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err("Catalog cache path contains a NUL character.".into());
        }
        value.push(0);
        Ok(value)
    };
    let from = wide(stage)?;
    let to = wide(path)?;
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }.map_err(|error| format!("Cannot replace catalog cache: {error}"))
}

#[cfg(not(windows))]
fn replace_cache_file(stage: &Path, path: &Path) -> Result<(), String> {
    fs::rename(stage, path).map_err(|error| format!("Cannot replace catalog cache: {error}"))
}

#[allow(dead_code)]
pub fn get_default_catalog() -> Vec<HdrApp> {
    get_full_catalog()
        .into_iter()
        .map(|entry| HdrApp {
            name: entry.name,
            exe_name: entry.exe_name,
            enabled: true,
            hdr_type: entry.hdr_type,
            path: None,
            alternate_exes: entry.alternate_exes,
            steam_id: entry.steam_id,
            launcher: None,
        })
        .collect()
}


pub fn find_in_catalog(exe_name: &str) -> Option<CatalogEntry> {
    let catalog = get_full_catalog();
    find_catalog_suggestion(&catalog, exe_name)
}

fn unique_catalog_match(
    catalog: &[CatalogEntry],
    predicate: impl Fn(&CatalogEntry) -> bool,
) -> Result<Option<&CatalogEntry>, ()> {
    let mut matches = catalog.iter().filter(|entry| predicate(entry));
    let found = matches.next();
    if matches.next().is_some() { Err(()) } else { Ok(found) }
}

fn find_catalog_suggestion(catalog: &[CatalogEntry], exe_name: &str) -> Option<CatalogEntry> {
    let exe_clean = exe_name.to_lowercase();
    let exe_stem = exe_clean.trim_end_matches(".exe");

    // 1. Direct match with entry.exe_name or entry.alternate_exes
    if let Some(entry) = unique_catalog_match(catalog, |c| {
        c.exe_name.eq_ignore_ascii_case(&exe_clean)
            || c.alternate_exes.iter().any(|alt| alt.eq_ignore_ascii_case(&exe_clean))
    }).ok()? {
        return Some(entry.clone());
    }

    // 2. Direct match with clean stem against entry.name (e.g. "forzahorizon5" == clean_key("Forza Horizon 5"))
    let clean_exe_alphanumeric: String = exe_stem.chars().filter(|c| c.is_alphanumeric()).collect();
    if clean_exe_alphanumeric.len() >= 3 {
        if let Some(entry) = unique_catalog_match(catalog, |c| {
            let cat_clean = clean_key(&c.name);
            cat_clean == clean_exe_alphanumeric
        }).ok()? {
            return Some(entry.clone());
        }
    }

    // 3. Unreal Engine & shipping prefixes/suffixes: "game-win64-shipping", "game_dx12", "game_vk"
    let stripped_stem = exe_stem
        .replace("-win64-shipping", "")
        .replace("_win64_shipping", "")
        .replace("-shipping", "")
        .replace("_dx12", "")
        .replace("_dx11", "")
        .replace("_vk", "");
    let clean_stripped: String = stripped_stem.chars().filter(|c| c.is_alphanumeric()).collect();
    if clean_stripped.len() >= 3 && clean_stripped != clean_exe_alphanumeric {
        if let Some(entry) = unique_catalog_match(catalog, |c| {
            let cat_exe_clean = c.exe_name.to_lowercase();
            let cat_stem = cat_exe_clean.trim_end_matches(".exe");
            let cat_clean = clean_key(&c.name);
            cat_stem == stripped_stem
                || cat_clean == clean_stripped
                || c.alternate_exes.iter().any(|alt| {
                    let alt_stem = alt.to_lowercase();
                    alt_stem.trim_end_matches(".exe") == stripped_stem
                })
        }).ok()? {
            return Some(entry.clone());
        }
    }

    None
}


pub async fn fetch_online_database() -> Result<Vec<CatalogEntry>, String> {
    // Do not hold a blocking lock across network awaits. The guard is cancellation-safe and
    // rejects overlapping startup/manual fetches before either captures a stale snapshot.
    let cache_path = get_cache_path()?;
    let _sync = CACHED_CATALOG.begin_sync()?;
    let (current_catalog, revision) = CACHED_CATALOG.snapshot(&cache_path)?;
    let mut catalog_map: HashMap<String, CatalogEntry> = current_catalog
        .into_iter()
        .map(|entry| (catalog_key(&entry.name), entry))
        .collect();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|e| e.to_string())?;
    let mut fetched = false;

    // 1. Try PCGamingWiki MediaWiki Cargo Query for HDR games
    let mut pcgw_fetched = Vec::new();
    for offset in [0, 500] {
        let cargo_query = format!(
            "{{{{#cargo_query:tables=Game,Video|join on=Game._pageID=Video._pageID|where=Video.HDR='true' OR Video.HDR='hackable' OR Video.HDR='always on' OR Video.HDR='limited'|fields=Game._pageName=Name,Video.HDR=Supported|limit=500|offset={}|format=table}}}}",
            offset
        );

        let params = [
            ("action", "parse"),
            ("text", &cargo_query),
            ("contentmodel", "wikitext"),
            ("format", "json"),
        ];

        if let Ok(res) = client
            .post("https://www.pcgamingwiki.com/w/api.php")
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
            )
            .form(&params)
            .send()
            .await
        {
            if res.status().is_success() {
                if let Ok(json_data) = res.json::<serde_json::Value>().await {
                    if let Some(html) = json_data.pointer("/parse/text/*").and_then(|v| v.as_str()) {
                        parse_pcgw_table_html(html, &mut pcgw_fetched);
                    }
                }
            }
        }
    }

    if !pcgw_fetched.is_empty() {
        fetched = true;
        for (name, supported) in pcgw_fetched {
            let key = catalog_key(&name);
            let (tier, hdr_type, notes) = match supported.as_str() {
                "hackable" => ("manual_fix", HdrType::Custom, "Vyžaduje úpravu / mod / Special K (PCGamingWiki)"),
                "limited" => ("limited", HdrType::Native, "Omezená nativní podpora HDR (PCGamingWiki)"),
                "always on" => ("always_on", HdrType::Native, "Trvale aktivní v enginu (PCGamingWiki)"),
                _ => ("native", HdrType::Native, "Nativní HDR podpora (PCGamingWiki)"),
            };

            catalog_map
                .entry(key)
                .and_modify(|existing| {
                    existing.support_tier = tier.to_string();
                    existing.notes = Some(notes.to_string());
                })
                .or_insert_with(|| {
                    let clean_exe = name
                        .to_lowercase()
                        .chars()
                        .filter(|c| c.is_alphanumeric())
                        .collect::<String>();
                    CatalogEntry {
                        name,
                        name_aliases: Vec::new(),
                        exe_name: format!("{}.exe", clean_exe),
                        hdr_type,
                        support_tier: tier.to_string(),
                        notes: Some(notes.to_string()),
                        steam_id: None,
                        alternate_exes: Vec::new(),
                        storefronts: Vec::new(),
                        storefront_authority: None,
                        executable_authority: None,
                    }
                });
        }
    }


    // 2. Fetch PCGamingWiki Windows Auto HDR games page
    let autohdr_params = [
        ("action", "parse"),
        ("page", "List_of_games_that_support_Auto_HDR"),
        ("prop", "wikitext"),
        ("format", "json"),
    ];

    if let Ok(res) = client
        .post("https://www.pcgamingwiki.com/w/api.php")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
        )
        .form(&autohdr_params)
        .send()
        .await
    {
        if res.status().is_success() {
            if let Ok(json_data) = res.json::<serde_json::Value>().await {
                if let Some(wikitext) = json_data.pointer("/parse/wikitext/*").and_then(|v| v.as_str()) {
                    fetched |= parse_pcgw_autohdr_wikitext(wikitext, &mut catalog_map) > 0;
                }
            }
        }
    }

    // 3. Fallback: If both failed, try GitHub repository raw JSON
    if !fetched {
        let gh_url = "https://raw.githubusercontent.com/Soptik1290/hdr-autoswitch/main/database/hdr_games.json";
        if let Ok(res) = client
            .get(gh_url)
            .header("User-Agent", "HDR-AutoSwitch-App")
            .send()
            .await
        {
            if res.status().is_success() {
                if let Ok(entries) = res.json::<Vec<CatalogEntry>>().await {
                    for entry in entries.into_iter().filter(valid_catalog_entry) {
                        fetched = true;
                        catalog_map.insert(catalog_key(&entry.name), entry);
                    }
                }
            }
        }
    }

    let result = synced_catalog(fetched, catalog_map)?;
    CACHED_CATALOG.publish(&cache_path, &result, Some(revision))
}

fn valid_catalog_entry(entry: &CatalogEntry) -> bool {
    !clean_key(&entry.name).is_empty()
        && !entry.exe_name.contains(['\\', '/'])
        && entry.exe_name.to_ascii_lowercase().ends_with(".exe")
        && entry.exe_name.len() > 4
}

fn synced_catalog(
    fetched: bool,
    catalog_map: HashMap<String, CatalogEntry>,
) -> Result<Vec<CatalogEntry>, String> {
    if !fetched {
        return Err("No online catalog source succeeded. The existing catalog was not replaced.".into());
    }

    let mut result = with_embedded_authority(catalog_map.into_values().collect());
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

fn clean_key(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn catalog_key(name: &str) -> String {
    static ALIASES: OnceLock<HashMap<String, String>> = OnceLock::new();
    let aliases = ALIASES.get_or_init(|| {
        let mut names: HashMap<String, Vec<String>> = HashMap::new();
        for entry in embedded_catalog() {
            let canonical = clean_key(&entry.name);
            for name in std::iter::once(&entry.name).chain(&entry.name_aliases) {
                names.entry(clean_key(name)).or_default().push(canonical.clone());
            }
        }
        names.into_iter().filter_map(|(name, canonical)| {
            (canonical.len() == 1).then(|| (name, canonical[0].clone()))
        }).collect()
    });
    let key = clean_key(name);
    aliases.get(&key).cloned().unwrap_or(key)
}

fn parse_pcgw_table_html(html: &str, out: &mut Vec<(String, String)>) {
    let name_tag = "<td class=\"field_Name\">";
    let supp_tag = "<td class=\"field_Supported\">";

    let mut rest = html;
    while let Some(name_pos) = rest.find(name_tag) {
        let after_name = &rest[name_pos + name_tag.len()..];
        if let Some(a_end) = after_name.find("</a>") {
            let before_a_end = &after_name[..a_end];
            let game_name = if let Some(last_gt) = before_a_end.rfind('>') {
                &before_a_end[last_gt + 1..]
            } else {
                before_a_end
            }
            .trim();

            if let Some(supp_pos) = after_name.find(supp_tag) {
                let after_supp = &after_name[supp_pos + supp_tag.len()..];
                if let Some(td_end) = after_supp.find("</td>") {
                    let supported_val = after_supp[..td_end].trim();
                    if !clean_key(game_name).is_empty()
                        && matches!(supported_val, "true" | "hackable" | "limited" | "always on")
                    {
                        out.push((game_name.to_string(), supported_val.to_string()));
                    }
                    rest = &after_supp[td_end..];
                    continue;
                }
            }
        }
        rest = after_name;
    }
}

fn parse_pcgw_autohdr_wikitext(wikitext: &str, map: &mut HashMap<String, CatalogEntry>) -> usize {
    let mut recognized = 0;
    for line in wikitext.lines() {
        let Some(row) = line.trim().strip_prefix('|') else {
            continue;
        };
        let Some(link) = row.trim().strip_prefix("[[") else {
            continue;
        };
        let Some((inside, _)) = link.split_once("]]") else {
            continue;
        };
        let (page, label) = inside.split_once('|').unwrap_or((inside, inside));
        let game_name = label.trim();
        if clean_key(page).is_empty()
            || clean_key(game_name).is_empty()
            || ["file:", "image:", "category:", "template:", "help:"]
                .iter()
                .any(|prefix| page.trim().to_ascii_lowercase().starts_with(prefix))
        {
            continue;
        }
        recognized += 1;
        let key = catalog_key(game_name);
        map.entry(key.clone()).or_insert_with(|| CatalogEntry {
            name: game_name.to_string(),
            name_aliases: Vec::new(),
            exe_name: format!("{key}.exe"),
            hdr_type: HdrType::AutoHdr,
            support_tier: "autohdr".to_string(),
            notes: Some("Podporuje Microsoft Windows Auto HDR".to_string()),
            steam_id: None,
            alternate_exes: Vec::new(),
            storefronts: Vec::new(),
            storefront_authority: None,
            executable_authority: None,
        });
    }
    recognized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_entry() -> CatalogEntry {
        authored_test_catalog(vec![serde_json::from_str(r#"{
            "name": "Example game", "exe_name": "Legacy.EXE", "hdr_type": "native",
            "support_tier": "native", "notes": null, "steam_id": "123",
            "alternate_exes": ["global.exe"]
        }"#).unwrap()]).remove(0)
    }

    fn binding(
        provider: StorefrontProvider,
        product_id: Option<&str>,
        games: &[&str],
        excluded: &[&str],
    ) -> StorefrontBinding {
        StorefrontBinding {
            provider,
            product_id: product_id.map(str::to_string),
            game_executables: games.iter().map(|value| value.to_string()).collect(),
            excluded_executables: excluded.iter().map(|value| value.to_string()).collect(),
        }
    }

    #[test]
    fn legacy_catalog_decodes_and_serializes_without_new_fields() {
        let entry = legacy_entry();
        assert!(entry.storefronts.is_empty());
        assert_eq!(entry.name, "Example game");
        assert_eq!(entry.hdr_type, HdrType::Native);
        assert_eq!(entry.alternate_exes, ["global.exe"]);
        let value = serde_json::to_value(&entry).unwrap();
        assert_eq!(value, serde_json::json!({
            "name": "Example game", "exe_name": "Legacy.EXE", "hdr_type": "native",
            "support_tier": "native", "notes": null, "steam_id": "123",
            "alternate_exes": ["global.exe"]
        }));
        assert_eq!(
            entry.storefront_binding(StorefrontProvider::Steam, Some("123")).unwrap(),
            Some(binding(StorefrontProvider::Steam, Some("123"), &["legacy.exe"], &[])),
        );
        assert_eq!(entry.storefront_binding(StorefrontProvider::Xbox, None), Ok(None));

        let entry: CatalogEntry = serde_json::from_str(r#"{
            "name": "Old", "exe_name": "old.exe", "hdr_type": "native", "support_tier": "native"
        }"#).unwrap();
        assert!(entry.steam_id.is_none());
        assert!(entry.notes.is_none());
        assert!(entry.alternate_exes.is_empty());
        assert!(entry.storefronts.is_empty());
        assert_eq!(entry.storefront_binding(StorefrontProvider::Steam, None), Ok(None));
    }

    #[test]
    fn explicit_steam_overrides_legacy_without_borrowing_xbox_or_global_aliases() {
        let mut entry = legacy_entry();
        let steam = binding(StorefrontProvider::Steam, Some("456"), &["steam.exe"], &[]);
        let xbox = binding(StorefrontProvider::Xbox, None, &["xbox.exe"], &["helper.exe"]);
        entry.storefronts = vec![steam.clone(), xbox.clone()];
        entry = authored_test_catalog(vec![entry]).remove(0);
        assert_eq!(entry.storefront_binding(StorefrontProvider::Steam, None), Ok(Some(steam)));
        assert_eq!(entry.storefront_binding(StorefrontProvider::Steam, Some("123")), Ok(None));
        assert_eq!(entry.storefront_binding(StorefrontProvider::Xbox, None), Ok(Some(xbox)));
        assert_eq!(entry.alternate_exes, ["global.exe"]);
        assert_eq!(entry.exe_name, "Legacy.EXE");

        entry.storefronts[0].game_executables.clear();
        entry = authored_test_catalog(vec![entry]).remove(0);
        assert!(entry.storefront_binding(StorefrontProvider::Steam, None).unwrap().unwrap()
            .game_executables.is_empty());
        entry.storefronts.remove(0);
        entry = authored_test_catalog(vec![entry]).remove(0);
        assert_eq!(entry.storefront_binding(StorefrontProvider::Steam, None).unwrap().unwrap()
            .game_executables, ["legacy.exe"]);
    }

    #[test]
    fn binding_defaults_and_normalization_do_not_rewrite_authored_json() {
        let minimal: StorefrontBinding = serde_json::from_str(r#"{"provider":"xbox"}"#).unwrap();
        assert_eq!(minimal, binding(StorefrontProvider::Xbox, None, &[], &[]));
        assert_eq!(serde_json::to_value(minimal).unwrap(), serde_json::json!({
            "provider": "xbox", "game_executables": []
        }));
        assert!(serde_json::from_str::<StorefrontBinding>(r#"{"provider":"unknown"}"#).is_err());

        let mut entry = legacy_entry();
        entry.storefronts = vec![binding(
            StorefrontProvider::Xbox, Some(" local-id "), &[" Z.EXE ", "a.exe"], &["Helper.EXE"],
        )];
        entry = authored_test_catalog(vec![entry]).remove(0);
        let before = serde_json::to_value(&entry).unwrap();
        let expected = binding(StorefrontProvider::Xbox, Some("local-id"), &["a.exe", "z.exe"], &["helper.exe"]);
        assert_eq!(entry.storefront_binding(StorefrontProvider::Xbox, Some("local-id")), Ok(Some(expected.clone())));
        entry.storefronts[0].game_executables.reverse();
        entry = authored_test_catalog(vec![entry]).remove(0);
        assert_eq!(entry.storefront_binding(StorefrontProvider::Xbox, Some(" local-id ")), Ok(Some(expected)));
        entry.storefronts[0].game_executables.reverse();
        assert_eq!(serde_json::to_value(&entry).unwrap(), before);
        let round_trip: CatalogEntry = serde_json::from_value(before.clone()).unwrap();
        assert_eq!(serde_json::to_value(round_trip).unwrap(), before);
    }

    #[test]
    fn duplicate_basenames_and_positive_exclusion_conflicts_fail_closed() {
        let mut entry = legacy_entry();
        for (games, excluded, error) in [
            (vec!["GAME.exe", "game.EXE"], vec![], StorefrontLookupError::DuplicateExecutable),
            (vec!["game.exe"], vec!["HELPER.exe", "helper.EXE"], StorefrontLookupError::DuplicateExecutable),
            (vec!["GAME.exe"], vec!["game.EXE"], StorefrontLookupError::ExcludedGameExecutable),
        ] {
            entry.storefronts = vec![binding(StorefrontProvider::Xbox, None, &games, &excluded)];
            entry = authored_test_catalog(vec![entry]).remove(0);
            assert_eq!(entry.storefront_binding(StorefrontProvider::Xbox, None), Err(error));
            entry.storefronts[0].game_executables.reverse();
            entry.storefronts[0].excluded_executables.reverse();
            assert_eq!(entry.storefront_binding(StorefrontProvider::Xbox, None), Err(error));
        }
        for invalid in ["", ".exe", "game.dll", r"C:\game.exe", "dir/game.exe", "game:stream.exe"] {
            entry.storefronts = vec![binding(StorefrontProvider::Xbox, None, &[invalid], &[])];
            entry = authored_test_catalog(vec![entry]).remove(0);
            assert_eq!(entry.storefront_binding(StorefrontProvider::Xbox, None), Err(StorefrontLookupError::InvalidExecutable));
        }
        entry.storefronts = vec![binding(StorefrontProvider::Steam, Some(" "), &["game.exe"], &[])];
        entry = authored_test_catalog(vec![entry]).remove(0);
        assert_eq!(entry.storefront_binding(StorefrontProvider::Steam, None), Err(StorefrontLookupError::InvalidProductId));
        assert_eq!(entry.storefront_binding(StorefrontProvider::Steam, Some(" ")), Err(StorefrontLookupError::InvalidProductId));
    }

    #[test]
    fn duplicate_provider_products_are_ambiguous_in_either_order() {
        let mut entry = legacy_entry();
        for second_game in ["one.exe", "two.exe"] {
            entry.storefronts = vec![
                binding(StorefrontProvider::Steam, Some("123"), &["one.exe"], &[]),
                binding(StorefrontProvider::Steam, Some(" 123 "), &[second_game], &[]),
            ];
            entry = authored_test_catalog(vec![entry]).remove(0);
            for _ in 0..2 {
                assert_eq!(entry.storefront_binding(StorefrontProvider::Steam, Some("123")), Err(StorefrontLookupError::AmbiguousBindings));
                assert_eq!(entry.storefront_binding(StorefrontProvider::Steam, None), Err(StorefrontLookupError::AmbiguousBindings));
                entry.storefronts.reverse();
            }
        }
        entry.storefronts = vec![
            binding(StorefrontProvider::Xbox, None, &["one.exe"], &[]),
            binding(StorefrontProvider::Xbox, None, &["two.exe"], &[]),
        ];
        entry = authored_test_catalog(vec![entry]).remove(0);
        assert_eq!(entry.storefront_binding(StorefrontProvider::Xbox, None), Err(StorefrontLookupError::AmbiguousBindings));
    }

    #[test]
    fn product_lookup_disambiguates_products_but_not_duplicate_catalog_rows() {
        let mut entry = legacy_entry();
        entry.storefronts = vec![
            binding(StorefrontProvider::Steam, Some("123"), &["game.exe"], &[]),
            binding(StorefrontProvider::Steam, Some("456"), &["game.exe"], &[]),
        ];
        entry = authored_test_catalog(vec![entry]).remove(0);
        assert_eq!(entry.storefront_binding(StorefrontProvider::Steam, None), Err(StorefrontLookupError::AmbiguousBindings));
        assert_eq!(entry.storefront_binding(StorefrontProvider::Steam, Some("456")).unwrap().unwrap()
            .product_id.as_deref(), Some("456"));
        assert_eq!(entry.storefront_binding(StorefrontProvider::Steam, Some("789")), Ok(None));
        let mut duplicate = entry.clone();
        duplicate.name = "Different title".into();
        let mut catalog = vec![entry, duplicate];
        for _ in 0..2 {
            assert_eq!(find_storefront_binding(&catalog, StorefrontProvider::Steam, Some("123")).unwrap_err(),
                StorefrontLookupError::AmbiguousBindings);
            catalog.reverse();
        }
    }

    #[test]
    fn duplicate_embedded_titles_remain_separate_and_ambiguous() {
        for explicit in [false, true] {
            let mut first = legacy_entry();
            if explicit {
                first.storefronts = vec![binding(StorefrontProvider::Steam, Some("123"), &["one.exe"], &[])];
            }
            let mut second = first.clone();
            second.name = "Example GAME!".into();
            second.exe_name = "two.exe".into();
            if explicit {
                second.storefronts[0].game_executables = vec!["two.exe".into()];
            }
            for embedded in [vec![first.clone(), second.clone()], vec![second, first]] {
                let merged = merge_catalog(embedded, Vec::new());
                assert_eq!(merged.len(), 2);
                assert_eq!(find_storefront_binding(&merged, StorefrontProvider::Steam, Some("123")),
                    Err(StorefrontLookupError::AmbiguousBindings));
            }
        }
    }

    #[test]
    fn equal_titles_do_not_inherit_another_rows_executables_or_products() {
        let legacy = legacy_entry();
        let mut suggestion = legacy.clone();
        suggestion.steam_id = None;
        suggestion.exe_name = "suggested.exe".into();
        let mut explicit = suggestion.clone();
        explicit.storefronts = vec![binding(StorefrontProvider::Steam, Some("456"), &[], &[])];
        for embedded in [vec![suggestion.clone(), legacy.clone()], vec![legacy.clone(), suggestion]] {
            let merged = merge_catalog(embedded, Vec::new());
            assert_eq!(merged.len(), 2);
            let (_, binding) = find_storefront_binding(&merged, StorefrontProvider::Steam, Some("123"))
                .unwrap().unwrap();
            assert_eq!(binding.game_executables, ["legacy.exe"]);
            let suggestion = merged.iter().find(|entry| entry.steam_id.is_none()).unwrap();
            assert_eq!(suggestion.storefront_binding(StorefrontProvider::Steam, None), Ok(None));
            assert!(!suggestion.authorizes_declared_executable("legacy.exe"));
        }
        for embedded in [vec![legacy.clone(), explicit.clone()], vec![explicit, legacy]] {
            let merged = merge_catalog(embedded, Vec::new());
            assert_eq!(merged.len(), 2);
            assert_eq!(find_storefront_binding(&merged, StorefrontProvider::Steam, None),
                Err(StorefrontLookupError::AmbiguousBindings));
            assert_eq!(find_storefront_binding(&merged, StorefrontProvider::Steam, Some("123"))
                .unwrap().unwrap().1.game_executables, ["legacy.exe"]);
            assert!(find_storefront_binding(&merged, StorefrontProvider::Steam, Some("456"))
                .unwrap().unwrap().1.game_executables.is_empty());
        }
    }

    #[test]
    fn cached_records_cannot_override_or_add_explicit_or_implicit_authority() {
        let mut embedded = legacy_entry();
        embedded.storefronts = vec![binding(StorefrontProvider::Xbox, None, &["embedded.exe"], &["helper.exe"])];
        embedded = authored_test_catalog(vec![embedded]).remove(0);
        let mut cached = embedded.clone();
        cached.storefronts = vec![
            binding(StorefrontProvider::Xbox, None, &["cached.exe"], &[]),
            binding(StorefrontProvider::Steam, Some("456"), &["cached.exe"], &[]),
        ];
        cached.steam_id = Some("456".into());
        cached.exe_name = "cached.exe".into();
        let mut additional = cached.clone();
        additional.name = "Online suggestion".into();
        let mut legacy_only = additional.clone();
        legacy_only.name = "Online legacy suggestion".into();
        legacy_only.storefronts.clear();
        let catalog = merge_catalog(vec![embedded.clone()], vec![cached, additional, legacy_only]);
        let existing = catalog.iter().find(|entry| entry.name == embedded.name).unwrap();
        assert_eq!(existing.storefront_binding(StorefrontProvider::Xbox, None), embedded.storefront_binding(StorefrontProvider::Xbox, None));
        assert_eq!(existing.storefront_binding(StorefrontProvider::Steam, None), embedded.storefront_binding(StorefrontProvider::Steam, None));
        assert_eq!(existing.alternate_exes, ["global.exe"]);
        for suggestion in catalog.iter().filter(|entry| entry.name.starts_with("Online")) {
            assert_eq!(suggestion.exe_name, "cached.exe");
            assert_eq!(suggestion.steam_id.as_deref(), Some("456"));
            assert!(suggestion.storefronts.is_empty());
            assert_eq!(suggestion.storefront_binding(StorefrontProvider::Steam, None), Ok(None));
            assert_eq!(suggestion.storefront_binding(StorefrontProvider::Xbox, None), Ok(None));
            assert!(serde_json::to_value(suggestion).unwrap().get("storefront_authority").is_none());
            assert!(serde_json::to_value(suggestion).unwrap().get("executable_authority").is_none());
        }
    }

    #[test]
    fn online_sync_and_memory_cache_boundary_restore_only_embedded_authority() {
        let embedded = embedded_catalog();
        let mut xbox = embedded.iter().find(|entry| entry.name == "Age of Empires III: Definitive Edition").unwrap().clone();
        let mut legacy = embedded.iter().find(|entry| entry.steam_id.as_deref() == Some("1466860")).unwrap().clone();
        for entry in [&mut xbox, &mut legacy] {
            entry.exe_name = "untrusted.exe".into();
            entry.steam_id = Some("untrusted-id".into());
            entry.notes = Some("Online display note".into());
            entry.storefronts = vec![binding(StorefrontProvider::Steam, Some("untrusted-id"), &["untrusted.exe"], &[])];
        }
        let suggestion = legacy_entry();
        let entries = vec![xbox, legacy, suggestion];
        let synced = synced_catalog(true, entries.iter().cloned().map(|entry| (clean_key(&entry.name), entry)).collect()).unwrap();
        let cache_ready = with_embedded_authority(entries);
        for catalog in [synced, cache_ready] {
            let xbox = catalog.iter().find(|entry| entry.name == "Age of Empires III: Definitive Edition").unwrap();
            assert_eq!(xbox.storefront_binding(StorefrontProvider::Steam, None), Ok(None));
            assert_eq!(xbox.storefront_binding(StorefrontProvider::Xbox, None).unwrap().unwrap()
                .game_executables, ["aoe3de.exe"]);
            let legacy = catalog.iter().find(|entry| entry.name == "Age of Empires IV").unwrap();
            assert_eq!(legacy.storefront_binding(StorefrontProvider::Steam, None).unwrap().unwrap(),
                binding(StorefrontProvider::Steam, Some("1466860"), &["ageofempiresiv.exe"], &[]));
            assert_eq!(legacy.exe_name, "ageofempiresiv.exe");
            assert_eq!(legacy.notes.as_deref(), Some("Online display note"));
            let suggestion = catalog.iter().find(|entry| entry.name == "Example game").unwrap();
            assert_eq!(suggestion.storefront_binding(StorefrontProvider::Steam, None), Ok(None));
            let reloaded: Vec<CatalogEntry> = serde_json::from_str(&serde_json::to_string(&catalog).unwrap()).unwrap();
            let merged = merge_catalog(embedded_catalog(), reloaded);
            assert_eq!(merged, catalog);
            assert_eq!(merged.iter().find(|entry| entry.name == "Example game").unwrap()
                .storefront_binding(StorefrontProvider::Steam, None), Ok(None));
        }
    }

    fn correction_cache_declared_suggestions(
        provider: crate::automatic_authority::Provider,
        alternate: bool,
    ) {
        use crate::automatic_authority::{self, Authority, InstallEvidence};
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let nominated = if alternate { "cached-alternate.exe" } else { "cached-primary.exe" };
        fs::write(root.path().join(nominated), b"fixture, never executed").unwrap();
        let mut suggestion = legacy_entry();
        suggestion.name = "Cache-only title with an existing Steam ID".into();
        suggestion.exe_name = "cached-primary.exe".into();
        suggestion.alternate_exes = vec!["cached-alternate.exe".into()];
        let serialized = serde_json::to_string(&vec![suggestion.clone()]).unwrap();
        let reloaded = serde_json::from_str(&serialized).unwrap();
        let boundaries = [
            ("disk reload", merge_catalog(vec![legacy_entry()], reloaded)),
            ("in-memory cache", with_embedded_authority(vec![suggestion.clone()])),
            ("online sync", synced_catalog(true, HashMap::from([
                (clean_key(&suggestion.name), suggestion),
            ])).unwrap()),
        ];
        let observed = InstallEvidence::observe(
            provider, None, root.path(), &[nominated.into()],
        ).unwrap();
        for (boundary, catalog) in boundaries {
            assert!(
                matches!(automatic_authority::resolve(&catalog, Some(&observed)), Authority::Unresolved),
                "{provider:?}: {boundary} promoted untrusted {nominated}"
            );
        }
    }

    #[test]
    fn correction_cache_epic_primary_is_not_automatic_authority() {
        correction_cache_declared_suggestions(crate::automatic_authority::Provider::Epic, false);
    }

    #[test]
    fn correction_cache_epic_alternate_is_not_automatic_authority() {
        correction_cache_declared_suggestions(crate::automatic_authority::Provider::Epic, true);
    }

    #[test]
    fn correction_cache_gog_primary_is_not_automatic_authority() {
        correction_cache_declared_suggestions(crate::automatic_authority::Provider::Gog, false);
    }

    #[test]
    fn correction_cache_gog_alternate_is_not_automatic_authority() {
        correction_cache_declared_suggestions(crate::automatic_authority::Provider::Gog, true);
    }

    #[test]
    fn correction_cache_windows_primary_is_not_automatic_authority() {
        correction_cache_declared_suggestions(crate::automatic_authority::Provider::Windows, false);
    }

    #[test]
    fn correction_cache_windows_alternate_is_not_automatic_authority() {
        correction_cache_declared_suggestions(crate::automatic_authority::Provider::Windows, true);
    }

    #[test]
    fn correction_cache_absent_source_snapshot_cannot_be_laundered_by_product_or_title() {
        use crate::automatic_authority::{self, Authority, InstallEvidence, Provider};
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        fs::write(root.path().join("legacy.exe"), b"fixture, never executed").unwrap();
        let catalog: Vec<CatalogEntry> = serde_json::from_value(serde_json::json!([{
            "name": "Example game", "exe_name": "legacy.exe", "steam_id": "123",
            "hdr_type": "native", "support_tier": "native",
            "storefronts": [{"provider": "xbox", "game_executables": ["legacy.exe"]}]
        }])).unwrap();
        for provider in [Provider::Steam, Provider::Xbox, Provider::Epic, Provider::Gog, Provider::Windows] {
            let observed = InstallEvidence::observe(
                provider, Some("123"), root.path(), &["legacy.exe".into()],
            ).unwrap();
            assert!(
                matches!(automatic_authority::resolve(&catalog, Some(&observed)), Authority::Unresolved),
                "{provider:?}: deserialization alone supplied source authority"
            );
        }
    }

    #[test]
    fn correction_cache_fetched_title_and_id_cannot_authorize_replacement_executable() {
        use crate::automatic_authority::{self, Authority, InstallEvidence, Provider};
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        fs::write(root.path().join("fetched.exe"), b"fixture, never executed").unwrap();
        let mut fetched = embedded_catalog().into_iter()
            .find(|entry| entry.name == "Age of Empires III: Definitive Edition").unwrap();
        fetched.exe_name = "fetched.exe".into();
        fetched.alternate_exes = vec!["fetched.exe".into()];
        let catalog = with_embedded_authority(vec![fetched]);
        for provider in [Provider::Epic, Provider::Gog, Provider::Windows] {
            let observed = InstallEvidence::observe(
                provider, None, root.path(), &["fetched.exe".into()],
            ).unwrap();
            assert!(
                matches!(automatic_authority::resolve(&catalog, Some(&observed)), Authority::Unresolved),
                "{provider:?}: matching fetched title acquired executable authority"
            );
        }
    }

    #[test]
    fn correction_cache_embedded_primary_and_alias_survive_mutable_display_replacement() {
        use crate::automatic_authority::{self, Authority, InstallEvidence, Provider};
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        for exe in ["legacy.exe", "global.exe"] {
            fs::write(root.path().join(exe), b"fixture, never executed").unwrap();
        }
        let source = merge_catalog(vec![legacy_entry()], Vec::new()).remove(0);
        let mut fetched = source.clone();
        fetched.exe_name = "fetched.exe".into();
        fetched.alternate_exes = vec!["fetched-alias.exe".into()];
        fetched.steam_id = Some("999".into());
        restrict_storefront_authority(&mut fetched, Some(&source));
        assert_eq!(fetched.authoritative_primary(), Some("Legacy.EXE"));
        assert_eq!(fetched.authoritative_steam_id(), Some("123"));
        for catalog in [vec![source], vec![fetched]] {
            for provider in [Provider::Epic, Provider::Gog, Provider::Windows] {
                for exe in ["legacy.exe", "global.exe"] {
                    let observed = InstallEvidence::observe(
                        provider, None, root.path(), &[exe.into()],
                    ).unwrap();
                    let Authority::Resolved(resolved) = automatic_authority::resolve(&catalog, Some(&observed)) else {
                        panic!("{provider:?}: lost embedded executable nomination {exe}");
                    };
                    assert_eq!(resolved.as_app(true).exe_name, exe);
                }
            }
        }
    }

    #[test]
    fn correction_cache_fetched_canonical_cannot_associate_saved_row_with_verified_xbox_file() {
        use crate::automatic_authority::{self, Authority, InstallEvidence, Provider};
        use crate::config::AppConfig;
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        fs::write(root.path().join("aoe3de.exe"), b"fixture, never executed").unwrap();
        let mut fetched = embedded_catalog().into_iter()
            .find(|entry| entry.name == "Age of Empires III: Definitive Edition").unwrap();
        let canonical = fetched.exe_name.clone();
        fetched.exe_name = "fetched-canonical.exe".into();
        fetched.alternate_exes = vec!["fetched-alias.exe".into()];
        fetched.steam_id = Some("999".into());
        let catalog = with_embedded_authority(vec![fetched]);
        let observed = InstallEvidence::observe(
            Provider::Xbox, None, root.path(), &["aoe3de.exe".into()],
        ).unwrap();
        let Authority::Resolved(resolved) = automatic_authority::resolve(&catalog, Some(&observed)) else {
            panic!("The embedded Xbox binding must still resolve its real local executable");
        };
        let mut existing = resolved.as_app(false);
        existing.exe_name = "fetched-canonical.exe".into();
        existing.name = "My custom title".into();
        existing.hdr_type = HdrType::Custom;
        existing.path = None;
        existing.launcher = None;
        let mut config = AppConfig::default();
        config.apps = vec![existing.clone()];
        assert!(!crate::library::enrich_verified_aliases(&mut config, std::slice::from_ref(&resolved)));
        assert!(!crate::library::enrich_verified_metadata(&mut config, std::slice::from_ref(&resolved)));
        assert_eq!(config.apps, [existing.clone()]);
        let local = root.path().join("settings");
        let manager = crate::config::ConfigManager::load(local.clone(), root.path().join("legacy.json")).unwrap();
        let first = manager.snapshot().unwrap();
        let ready = manager.initialize(&first.context_token).unwrap();
        let before = manager.mutate(&ready.context_token, None, true, |settings| {
            settings.auto_detect_new_games = true;
            settings.apps = vec![existing.clone()];
            Ok(())
        }).unwrap();
        let artifacts = || -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
            fs::read_dir(&local).unwrap().map(|entry| entry.unwrap().path())
                .filter(|path| path.file_name().unwrap().to_string_lossy().starts_with("config-v2."))
                .map(|path| {
                    let bytes = fs::read(&path).unwrap();
                    (path, bytes)
                }).collect()
        };
        let before_bytes = artifacts();
        let result = manager.mutate_if_changed(
            &before.context_token, Some(&before.library_generation), true, |settings| {
                assert!(!crate::library::enrich_verified_aliases(settings, std::slice::from_ref(&resolved)));
                assert!(!crate::library::enrich_verified_metadata(settings, std::slice::from_ref(&resolved)));
                Ok(())
            },
        ).unwrap();
        assert!(result.is_none());
        assert_eq!(manager.snapshot().unwrap(), before);
        assert_eq!(artifacts(), before_bytes);
        existing.exe_name = canonical;
        config.apps = vec![existing.clone()];
        assert!(crate::library::enrich_verified_aliases(&mut config, std::slice::from_ref(&resolved)));
        existing.alternate_exes = vec!["aoe3de.exe".into()];
        assert_eq!(config.apps, [existing]);
    }

    #[test]
    fn catalog_copies_match_and_aoe3_xbox_authority_leaves_steam_unresolved() {
        let source = include_str!("../../database/hdr_games.json");
        assert_eq!(serde_json::from_str::<serde_json::Value>(source).unwrap(),
            serde_json::from_str::<serde_json::Value>(EMBEDDED_CATALOG_JSON).unwrap());
        let catalog = merge_catalog(embedded_catalog(), Vec::new());
        let aoe3 = catalog.iter().find(|entry| entry.name == "Age of Empires III: Definitive Edition").unwrap();
        assert_eq!(aoe3.exe_name, "ageofempiresiiidefinitiveedition.exe");
        assert!(aoe3.steam_id.is_none());
        assert!(aoe3.alternate_exes.is_empty());
        assert_eq!(aoe3.storefront_binding(StorefrontProvider::Steam, None), Ok(None));
        assert_eq!(aoe3.storefront_binding(StorefrontProvider::Xbox, None).unwrap().unwrap(),
            binding(StorefrontProvider::Xbox, None, &["aoe3de.exe"], &["gamelaunchhelper.exe"]));
        assert!(find_catalog_suggestion(&catalog, "aoe3de.exe").is_none());
        assert!(find_catalog_suggestion(&catalog, "gamelaunchhelper.exe").is_none());
        let aoe4 = catalog.iter().find(|entry| entry.steam_id.as_deref() == Some("1466860")).unwrap();
        assert!(aoe4.storefronts.is_empty());
        assert_eq!(aoe4.storefront_binding(StorefrontProvider::Steam, None).unwrap().unwrap()
            .game_executables, ["ageofempiresiv.exe"]);
    }

    #[test]
    fn empty_error_and_changed_format_pages_do_not_count_as_ingestion() {
        for input in [
            "", "   ", "<html>Service unavailable</html>",
            r#"{"error":{"code":"missingtitle"}}"#,
            "{{AutoHDR|New table format}}", "| no recognized game rows",
            "| [[File:HDR.png|Picture]]", "| [[Category:HDR|HDR games]]",
            "| [[|Missing page]]", "| [[Game|]]", "| [[...]]",
        ] {
            let mut catalog = HashMap::new();
            assert_eq!(parse_pcgw_autohdr_wikitext(input, &mut catalog), 0, "{input}");
            assert!(synced_catalog(false, catalog).is_err());
        }
    }

    #[test]
    fn valid_rows_count_even_when_the_catalog_already_contains_them() {
        let text = "{| class=\"wikitable\"\n| [[Game one]] || Yes\n|-\n| [[Game two|Game 2]] || Yes\n|}";
        let mut catalog = HashMap::new();
        assert_eq!(parse_pcgw_autohdr_wikitext(text, &mut catalog), 2);
        catalog.get_mut("gameone").unwrap().steam_id = Some("123".into());
        let recognized = parse_pcgw_autohdr_wikitext(text, &mut catalog);
        assert_eq!(recognized, 2);
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog["gameone"].steam_id.as_deref(), Some("123"));
        assert!(synced_catalog(recognized > 0, catalog).is_ok());
    }

    #[test]
    fn an_existing_catalog_does_not_turn_a_failed_fetch_into_success() {
        let mut catalog = HashMap::new();
        parse_pcgw_autohdr_wikitext("| [[Existing game]]", &mut catalog);
        let recognized = parse_pcgw_autohdr_wikitext("A nonempty upstream error", &mut catalog);
        assert!(synced_catalog(recognized > 0, catalog).unwrap_err().contains("No online catalog source succeeded"));
    }

    #[test]
    fn fallback_requires_valid_entries_and_native_rows_require_known_support() {
        let mut rows = Vec::new();
        parse_pcgw_table_html(
            "<td class=\"field_Name\"><a>Game</a></td><td class=\"field_Supported\">new unknown value</td>",
            &mut rows,
        );
        assert!(rows.is_empty());
        parse_pcgw_table_html(
            "<td class=\"field_Name\"><a>Game</a></td><td class=\"field_Supported\">true</td>",
            &mut rows,
        );
        assert_eq!(rows, vec![("Game".into(), "true".into())]);
        let mut map = HashMap::new();
        parse_pcgw_autohdr_wikitext("| [[Fallback game]]", &mut map);
        let mut entry = map.remove("fallbackgame").unwrap();
        assert!(valid_catalog_entry(&entry));
        entry.exe_name = ".exe".into();
        assert!(!valid_catalog_entry(&entry));
        entry.exe_name = r"C:\game.exe".into();
        assert!(!valid_catalog_entry(&entry));
    }

    const CANONICAL_PRODUCTS: [(&str, &str, &str, &str, &str); 5] = [
        ("Baldur's Gate 3", "Baldur's Gate 3 (DX11)", "1086940", "bg3.exe", "bg3_dx11.exe"),
        ("Dead Space (2023)", "Dead Space Remake", "1693980", "deadspace.exe", "deadspace.exe"),
        ("Resident Evil 2 (2019)", "Resident Evil 2 Remake", "883710", "re2.exe", "re2.exe"),
        ("Resident Evil 3 (2020)", "Resident Evil 3 Remake", "952060", "re3.exe", "re3.exe"),
        ("Resident Evil 4 (2023)", "Resident Evil 4 Remake", "2050650", "re4.exe", "re4.exe"),
    ];

    #[test]
    fn full_catalog_identities_and_provider_bindings_are_valid_and_unique() {
        use std::collections::{BTreeMap, BTreeSet};
        let catalog = merge_catalog(embedded_catalog(), Vec::new());
        let mut identities = BTreeSet::new();
        let mut products = BTreeSet::new();
        let mut executables: BTreeMap<String, Vec<String>> = BTreeMap::new();
        assert!(catalog.len() > 1000);
        for entry in &catalog {
            assert!(valid_catalog_entry(entry), "{}", entry.name);
            for name in std::iter::once(&entry.name).chain(&entry.name_aliases) {
                assert!(!clean_key(name).is_empty());
                assert!(identities.insert(clean_key(name)), "Duplicate identity: {name}");
            }
            let nominations: Vec<_> = std::iter::once(entry.exe_name.clone())
                .chain(entry.alternate_exes.clone()).collect();
            for exe in normalized_executables(&nominations).unwrap() {
                executables.entry(exe).or_default().push(entry.name.clone());
            }
            for binding in entry.authoritative_bindings() {
                let binding = binding.normalized().unwrap();
                if let Some(product) = binding.product_id.as_deref() {
                    if binding.provider == StorefrontProvider::Steam {
                        assert!(product.parse::<u32>().is_ok_and(|id| id > 0));
                    }
                    assert!(products.insert(format!("{:?}:{product}", binding.provider)),
                        "Duplicate product: {} {binding:?}", entry.name);
                    let (found, normalized) = find_storefront_binding(
                        &catalog, binding.provider, Some(product),
                    ).unwrap().unwrap();
                    assert_eq!(found.name, entry.name);
                    assert_eq!(normalized, binding);
                }
            }
        }
        let overlaps: BTreeMap<_, _> = executables.into_iter()
            .filter(|(_, entries)| entries.len() > 1).collect();
        let expected: BTreeMap<_, _> = [
            ("cod.exe", ["Call of Duty: Modern Warfare II (2022)", "Call of Duty: Warzone"]),
            ("hitman.exe", ["Hitman", "Hitman World of Assassination"]),
            ("masseffect1.exe", ["Mass Effect 1 (LE)", "Mass Effect Legendary Edition"]),
            ("masseffect2.exe", ["Mass Effect 2 (LE)", "Mass Effect Legendary Edition"]),
            ("masseffect3.exe", ["Mass Effect 3 (LE)", "Mass Effect Legendary Edition"]),
            ("tll.exe", ["Uncharted: Legacy of Thieves Collection", "Uncharted: The Lost Legacy"]),
        ].into_iter().map(|(exe, names)| (
            exe.to_string(), names.into_iter().map(str::to_string).collect::<Vec<_>>(),
        )).collect();
        assert_eq!(overlaps, expected, "New overlaps require an explicit identity audit");
    }

    #[test]
    fn canonical_products_resolve_with_real_provider_evidence_before_and_after_reload() {
        use crate::automatic_authority::{self, Authority, InstallEvidence, Provider};
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let catalog = merge_catalog(embedded_catalog(), Vec::new());
        let mut aliases = Vec::new();
        for (name, alias, id, primary, declared) in CANONICAL_PRODUCTS {
            let entry = catalog.iter().find(|entry| entry.name == name).unwrap();
            assert_eq!(entry.name_aliases, [alias]);
            assert_eq!(entry.steam_id.as_deref(), Some(id));
            assert_eq!(entry.exe_name, primary);
            assert!(!catalog.iter().any(|entry| entry.name == alias));
            assert_eq!(catalog_key(alias), clean_key(name));
            for exe in [primary, declared] {
                fs::write(root.path().join(exe), b"fixture, never executed").unwrap();
            }
            let mut old_row = entry.clone();
            old_row.name = alias.into();
            old_row.notes = Some("Updated display metadata".into());
            old_row.exe_name = "untrusted.exe".into();
            old_row.alternate_exes = vec!["untrusted-alias.exe".into()];
            old_row.steam_id = Some("999".into());
            aliases.push(old_row);
        }
        let synced = with_embedded_authority(aliases);
        let decoded = serde_json::from_slice(&serde_json::to_vec(&synced).unwrap()).unwrap();
        let reloaded = merge_catalog(embedded_catalog(), decoded);
        assert_eq!(synced, reloaded);
        for mut catalog in [catalog, synced, reloaded] {
            for _ in 0..2 {
                for (name, _, id, primary, declared) in CANONICAL_PRODUCTS {
                    for provider in [Provider::Steam, Provider::Epic, Provider::Gog, Provider::Windows] {
                        let exe = if provider == Provider::Steam { primary } else { declared };
                        let observed = InstallEvidence::observe(
                            provider, (provider == Provider::Steam).then_some(id),
                            root.path(), &[exe.into()],
                        ).unwrap();
                        let Authority::Resolved(resolved) = automatic_authority::resolve(&catalog, Some(&observed)) else {
                            panic!("{provider:?}: canonical product {name} did not resolve");
                        };
                        assert_eq!(resolved.catalog.name, name);
                        assert_eq!(resolved.as_app(true).exe_name, exe);
                    }
                    let observed = InstallEvidence::observe(
                        Provider::Xbox, Some(id), root.path(), &[declared.into()],
                    ).unwrap();
                    assert!(matches!(automatic_authority::resolve(&catalog, Some(&observed)), Authority::Unresolved));
                }
                catalog.reverse();
            }
        }
    }

    #[test]
    fn shared_basenames_stay_ambiguous_without_distinct_provider_products() {
        use crate::automatic_authority::{self, Authority, InstallEvidence, Provider};
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut catalog = merge_catalog(embedded_catalog(), Vec::new());
        for exe in ["cod.exe", "hitman.exe", "masseffect1.exe", "masseffect2.exe", "masseffect3.exe", "tll.exe"] {
            fs::write(root.path().join(exe), b"fixture, never executed").unwrap();
            for _ in 0..2 {
                assert!(find_catalog_suggestion(&catalog, exe).is_none(), "{exe}");
                for provider in [Provider::Epic, Provider::Gog, Provider::Windows] {
                    let observed = InstallEvidence::observe(provider, None, root.path(), &[exe.into()]).unwrap();
                    assert!(matches!(automatic_authority::resolve(&catalog, Some(&observed)), Authority::Ambiguous),
                        "{provider:?} must not pick the first {exe}");
                }
                catalog.reverse();
            }
        }
        for (id, name) in [
            ("1938090", "Call of Duty: Modern Warfare II (2022)"),
            ("1962663", "Call of Duty: Warzone"),
        ] {
            let observed = InstallEvidence::observe(
                Provider::Steam, Some(id), root.path(), &["cod.exe".into()],
            ).unwrap();
            let Authority::Resolved(resolved) = automatic_authority::resolve(&catalog, Some(&observed)) else {
                panic!("Distinct Steam product {id} must remain usable");
            };
            assert_eq!(resolved.catalog.name, name);
        }
    }

    #[test]
    fn explicit_name_overlays_are_order_independent_and_never_reintroduce_duplicate_authority() {
        let source = embedded_catalog().into_iter()
            .find(|entry| entry.name == "Dead Space (2023)").unwrap();
        let mut canonical = source.clone();
        canonical.notes = Some("Canonical update".into());
        let mut alias = source.clone();
        alias.name = "Dead Space Remake".into();
        alias.notes = Some("Alias update".into());
        for updates in [vec![canonical.clone(), alias.clone()], vec![alias.clone(), canonical.clone()]] {
            let merged = merge_catalog(vec![source.clone()], updates);
            assert_eq!(merged.len(), 1);
            assert_eq!(merged[0].notes, canonical.notes);
            assert_eq!(merged[0].exe_name, source.exe_name);
        }
        let alias_only = merge_catalog(vec![source.clone()], vec![alias.clone()]);
        assert_eq!(alias_only.len(), 1);
        assert_eq!(alias_only[0].name, source.name);
        assert_eq!(alias_only[0].notes, alias.notes);
        let mut conflict = alias.clone();
        conflict.notes = Some("Conflicting alias update".into());
        for updates in [vec![alias.clone(), conflict.clone()], vec![conflict, alias]] {
            assert_eq!(merge_catalog(vec![source.clone()], updates)[0].notes, source.notes);
        }
        let mut map = HashMap::from([(clean_key(&source.name), source.clone())]);
        assert_eq!(parse_pcgw_autohdr_wikitext("| [[Dead Space Remake]]", &mut map), 1);
        assert_eq!(map.len(), 1);
        assert_eq!(map[&clean_key(&source.name)].name, source.name);
    }

    fn display_update(note: &str) -> CatalogEntry {
        let mut entry = embedded_catalog().into_iter()
            .find(|entry| entry.name == "Dead Space (2023)").unwrap();
        entry.support_tier = "limited".into();
        entry.notes = Some(note.into());
        entry
    }

    #[test]
    fn cache_publication_and_restart_have_identical_display_and_immutable_authority() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let path = root.path().join("cache.json");
        let cache = CatalogCache::new();
        let source = merge_catalog(embedded_catalog(), Vec::new());
        let mut update = display_update("Persist this display note");
        update.exe_name = "untrusted.exe".into();
        update.alternate_exes = vec!["untrusted-alias.exe".into()];
        update.name_aliases = vec!["Untrusted name alias".into()];
        update.steam_id = Some("999".into());
        update.hdr_type = HdrType::Custom;
        update.storefronts = vec![binding(StorefrontProvider::Xbox, None, &["untrusted.exe"], &[])];
        let mut suggestion = legacy_entry();
        suggestion.name_aliases = vec!["Another untrusted alias".into()];
        let synced = synced_catalog(true, [update, suggestion].into_iter()
            .map(|entry| (clean_key(&entry.name), entry)).collect()).unwrap();
        let published = cache.publish(&path, &synced, None).unwrap();
        let reloaded = CatalogCache::new().snapshot(&path).unwrap().0;
        assert_eq!(published, synced);
        assert_eq!(cache.snapshot(&path).unwrap().0, reloaded);
        assert_eq!(published, reloaded);
        let existing = published.iter().find(|entry| entry.name == "Dead Space (2023)").unwrap();
        let mut expected = source.iter().find(|entry| entry.name == existing.name).unwrap().clone();
        expected.notes = Some("Persist this display note".into());
        expected.support_tier = "limited".into();
        assert_eq!(existing, &expected);
        let suggestion = published.iter().find(|entry| entry.name == "Example game").unwrap();
        assert!(suggestion.name_aliases.is_empty());
        assert!(suggestion.storefronts.is_empty());
        assert!(suggestion.executable_authority.is_none());
        assert_eq!(suggestion.storefront_binding(StorefrontProvider::Steam, None), Ok(None));
        let mut cleared = expected;
        cleared.notes = None;
        cache.publish(&path, &[cleared], None).unwrap();
        assert!(CatalogCache::new().snapshot(&path).unwrap().0.iter()
            .find(|entry| entry.name == "Dead Space (2023)").unwrap().notes.is_none());
    }

    #[test]
    fn interrupted_cache_publication_preserves_disk_memory_and_revision() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let path = root.path().join("cache.json");
        let cache = CatalogCache::new();
        cache.publish(&path, &[display_update("Previous complete snapshot")], None).unwrap();
        let before = cache.snapshot(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let result = cache.publish_with(&path, &[display_update("Interrupted update")], None, |path, bytes| {
            write_cache_atomically_with(path, bytes, |stage| {
                assert_eq!(fs::read(stage).unwrap(), bytes);
                assert!(serde_json::from_slice::<Vec<CatalogEntry>>(bytes).is_ok());
                Err("Injected interruption after flush, before replace".into())
            })
        });
        assert!(result.unwrap_err().contains("Injected interruption"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(cache.snapshot(&path).unwrap(), before);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1, "Owned stage must be cleaned");

        let blocked = root.path().join("blocked.json");
        fs::create_dir(&blocked).unwrap();
        assert!(cache.publish(&blocked, &[display_update("Replacement fails")], None).is_err());
        assert_eq!(cache.snapshot(&path).unwrap(), before);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);

        // A crash can leave an incomplete sibling stage. It is never a candidate for reload.
        let orphan = root.path().join(".catalog-cache-interrupted.stage");
        fs::write(&orphan, br#"[{"name":"unfinished"#).unwrap();
        assert_eq!(CatalogCache::new().snapshot(&path).unwrap().0, before.0);
        let missing = root.path().join("never-published.json");
        assert_eq!(CatalogCache::new().snapshot(&missing).unwrap().0,
            merge_catalog(embedded_catalog(), Vec::new()));
        fs::write(&path, br#"[{"name":"truncated old non-atomic cache"#).unwrap();
        assert_eq!(CatalogCache::new().snapshot(&path).unwrap().0,
            merge_catalog(embedded_catalog(), Vec::new()));
    }

    #[test]
    fn failed_first_cache_publication_leaves_no_published_file_or_memory_snapshot() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let path = root.path().join("cache.json");
        let cache = CatalogCache::new();
        assert!(cache.publish_with(&path, &[display_update("Never published")], None, |path, bytes| {
            write_cache_atomically_with(path, bytes, |_| Err("Interrupted initial save".into()))
        }).is_err());
        assert!(!path.exists());
        assert!(cache.state.read().unwrap().entries.is_none());
        assert_eq!(cache.state.read().unwrap().revision, 0);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn concurrent_cache_writers_and_readers_observe_only_complete_publications() {
        use std::sync::{Arc, Barrier};
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let path = root.path().join("cache.json");
        let cache = CatalogCache::new();
        cache.publish(&path, &[display_update("Initial snapshot")], None).unwrap();
        let start = Arc::new(Barrier::new(9));
        std::thread::scope(|scope| {
            for index in 0..8 {
                let start = start.clone();
                let path = &path;
                let cache = &cache;
                scope.spawn(move || {
                    start.wait();
                    cache.publish(path, &[display_update(&format!("Writer {index}"))], None).unwrap();
                });
            }
            start.wait();
            for _ in 0..24 {
                let decoded: Vec<CatalogEntry> = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                let complete = merge_catalog(embedded_catalog(), decoded.clone());
                assert_eq!(serde_json::to_value(decoded).unwrap(), serde_json::to_value(complete).unwrap());
            }
        });
        let (memory, revision) = cache.snapshot(&path).unwrap();
        assert_eq!(revision, 9);
        assert_eq!(memory, CatalogCache::new().snapshot(&path).unwrap().0);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn startup_loading_and_publication_share_the_same_serialization_boundary() {
        use std::sync::mpsc;
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let path = root.path().join("cache.json");
        write_cache_atomically(&path, &serde_json::to_vec(&vec![display_update("Disk at startup")]).unwrap()).unwrap();
        let cache = CatalogCache::new();
        let (loading, loaded) = mpsc::channel();
        let (release, proceed) = mpsc::channel();
        let (publishing, started) = mpsc::channel();
        std::thread::scope(|scope| {
            let cache = &cache;
            let path = &path;
            let reader = scope.spawn(move || cache.snapshot_with(path, |path| {
                let entries = read_cached_catalog(path);
                loading.send(()).unwrap();
                proceed.recv().unwrap();
                entries
            }).unwrap());
            loaded.recv().unwrap();
            let writer = scope.spawn(move || {
                publishing.send(()).unwrap();
                cache.publish(path, &[display_update("Newest publication")], None).unwrap()
            });
            started.recv().unwrap();
            release.send(()).unwrap();
            assert_eq!(reader.join().unwrap().1, 0);
            let published = writer.join().unwrap();
            assert_eq!(cache.snapshot(path).unwrap().0, published);
            assert_eq!(CatalogCache::new().snapshot(path).unwrap().0, published);
        });
    }

    #[test]
    fn overlapping_syncs_are_rejected_and_a_stale_fetch_cannot_overwrite_a_newer_publication() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let path = root.path().join("cache.json");
        let cache = CatalogCache::new();
        let sync = cache.begin_sync().unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| assert!(cache.begin_sync().is_err())).join().unwrap();
        });
        let (stale, revision) = cache.snapshot(&path).unwrap();
        let latest = cache.publish(&path, &[display_update("Newer publication")], None).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(cache.publish(&path, &stale, Some(revision)).unwrap_err().contains("changed during synchronization"));
        assert_eq!(cache.snapshot(&path).unwrap().0, latest);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        drop(sync);
        let _next = cache.begin_sync().unwrap();
        let revision = cache.snapshot(&path).unwrap().1;
        cache.publish(&path, &[display_update("Next synchronization")], Some(revision)).unwrap();
        assert_eq!(cache.snapshot(&path).unwrap().0, CatalogCache::new().snapshot(&path).unwrap().0);
    }

    #[test]
    fn default_test_catalog_access_never_uses_appdata_or_the_network() {
        use std::future::Future;
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};
        struct Noop;
        impl Wake for Noop {
            fn wake(self: Arc<Self>) {}
        }
        assert_eq!(get_full_catalog(), merge_catalog(embedded_catalog(), Vec::new()));
        assert!(get_cache_path().unwrap_err().contains("explicitly injected path"));
        assert!(save_to_cache(&[]).unwrap_err().contains("explicitly injected path"));
        let waker = Waker::from(Arc::new(Noop));
        let mut context = Context::from_waker(&waker);
        let mut fetch = Box::pin(fetch_online_database());
        assert!(matches!(
            fetch.as_mut().poll(&mut context),
            Poll::Ready(Err(error)) if error.contains("explicitly injected path")
        ));
    }
}
