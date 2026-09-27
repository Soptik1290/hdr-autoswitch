//! Automatic catalog association is independent of saved-library association.
//! Filesystem observation supplies evidence; the pure resolver never promotes suggestions.

use crate::config::HdrApp;
use crate::database::{self, CatalogEntry, StorefrontBinding, StorefrontProvider};
use crate::runtime_policy::permanently_excluded;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf, Prefix};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Steam,
    Xbox,
    Epic,
    Gog,
    Windows,
}

impl Provider {
    pub fn launcher(self) -> &'static str {
        match self {
            Self::Steam => "Steam",
            Self::Xbox => "Xbox",
            Self::Epic => "Epic Games",
            Self::Gog => "GOG",
            Self::Windows => "Windows",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceError {
    InvalidPath,
    NotLocal,
    ReparsePoint,
    Inaccessible,
    NotRegularFile,
    Incomplete,
}

impl std::fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

fn normalize_relative(value: &str) -> Result<String, EvidenceError> {
    if value.is_empty() || value.len() > 1024 || value.trim() != value {
        return Err(EvidenceError::InvalidPath);
    }
    let normalized = value.replace('/', "\\").to_lowercase();
    for part in normalized.split('\\') {
        let stem = part.split('.').next().unwrap_or_default();
        if part.is_empty()
            || part.trim() != part
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || "<>:\"|?*".contains(c))
            || matches!(stem, "con" | "prn" | "aux" | "nul" | "conin$" | "conout$")
            || (stem.chars().count() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && matches!(
                    stem.chars().last(),
                    Some('1'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}')
                ))
        {
            return Err(EvidenceError::InvalidPath);
        }
    }
    Ok(normalized)
}

pub fn normalize_relative_exe(value: &str) -> Result<String, EvidenceError> {
    let normalized = normalize_relative(value)?;
    let basename = normalized.rsplit('\\').next().unwrap_or_default();
    if basename.len() <= 4 || !basename.ends_with(".exe") {
        return Err(EvidenceError::InvalidPath);
    }
    Ok(normalized)
}

fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0 || metadata.file_type().is_symlink()
}

pub(crate) fn checked_root(root: &Path) -> Result<PathBuf, EvidenceError> {
    let text = root.to_str().ok_or(EvidenceError::InvalidPath)?;
    if text.len() < 4
        || text.as_bytes().get(1) != Some(&b':')
        || !text.as_bytes()[0].is_ascii_alphabetic()
        || !matches!(text.as_bytes().get(2), Some(b'\\' | b'/'))
    {
        return Err(EvidenceError::NotLocal);
    }
    normalize_relative(text[3..].trim_end_matches(['\\', '/']))?;
    let mut components = root.components();
    if !matches!(components.next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_)))
        || !matches!(components.next(), Some(Component::RootDir))
    {
        return Err(EvidenceError::NotLocal);
    }
    let drive: Vec<u16> = format!("{}:\\", &text[..1])
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let drive_type = unsafe {
        windows::Win32::Storage::FileSystem::GetDriveTypeW(windows::core::PCWSTR(drive.as_ptr()))
    };
    if !matches!(drive_type, 2 | 3 | 5 | 6) {
        return Err(EvidenceError::NotLocal);
    }
    let relative = components
        .map(|part| match part {
            Component::Normal(value) => value.to_str().ok_or(EvidenceError::InvalidPath),
            _ => Err(EvidenceError::InvalidPath),
        })
        .collect::<Result<Vec<_>, _>>()?
        .join("\\");
    normalize_relative(&relative)?;
    // Inspect all ancestors, not just the final directory (junctions may hide in parents).
    for ancestor in root.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| EvidenceError::Inaccessible)?;
        if is_reparse(&metadata) {
            return Err(EvidenceError::ReparsePoint);
        }
        if !metadata.is_dir() {
            return Err(EvidenceError::NotRegularFile);
        }
    }
    fs::canonicalize(root).map_err(|_| EvidenceError::Inaccessible)
}

/// Resolve only a validated relative local file, refusing every reparse traversal.
pub fn local_file(root: &Path, relative: &str) -> Result<PathBuf, EvidenceError> {
    let relative = normalize_relative(relative)?;
    let canonical_root = checked_root(root)?;
    let mut path = root.to_path_buf();
    let parts: Vec<_> = relative.split('\\').collect();
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        let metadata = fs::symlink_metadata(&path).map_err(|_| EvidenceError::Inaccessible)?;
        if is_reparse(&metadata) {
            return Err(EvidenceError::ReparsePoint);
        }
        if (index + 1 == parts.len() && !metadata.is_file())
            || (index + 1 < parts.len() && !metadata.is_dir())
        {
            return Err(EvidenceError::NotRegularFile);
        }
    }
    let canonical = fs::canonicalize(path).map_err(|_| EvidenceError::Inaccessible)?;
    if !canonical.starts_with(&canonical_root) {
        return Err(EvidenceError::NotLocal);
    }
    Ok(canonical)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedExecutable {
    pub basename: String,
    pub path: PathBuf,
    relative: String,
}

#[derive(Debug)]
pub struct InstallEvidence {
    pub provider: Provider,
    pub product_id: Option<String>,
    pub install_root: PathBuf,
    pub declarations: Vec<String>,
    // Private: only complete, validated observations can supply file authority.
    files: Vec<SelectedExecutable>,
    pub recursive_suggestions: Vec<String>,
}

impl InstallEvidence {
    pub fn manual_suggestion(&self) -> Option<SelectedExecutable> {
        match self.declarations.as_slice() {
            [declaration] => self.declared(declaration).ok(),
            _ => None,
        }
    }

    pub fn candidate_executable(&self, hint_name: &str) -> Option<SelectedExecutable> {
        let is_candidate = |f: &SelectedExecutable| -> bool {
            let lower = f.basename.to_ascii_lowercase();
            !permanently_excluded(&lower)
                && !lower.contains("crash")
                && !lower.contains("setup")
                && !lower.contains("installer")
                && !lower.contains("unins")
                && !lower.contains("unitycrashhandler")
                && !lower.contains("crashreportclient")
                && !lower.contains("protocol")
                && !lower.contains("helper")
                && !lower.contains("eula")
                && !lower.contains("reporter")
                && !lower.contains("prereq")
                && !lower.contains("redist")
                && !lower.contains("benchmark")
                && !lower.contains("dxsetup")
                && !lower.contains("vcredist")
                && !lower.contains("touchup")
                && !lower.contains("config")
                && !lower.contains("editor")
                && !lower.contains("tool")
        };

        let mut valid_files: Vec<_> = self.files.iter().filter(|f| is_candidate(f)).collect();
        if valid_files.is_empty() {
            return None;
        }

        let hint_norm = hint_name
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();

        if !hint_norm.is_empty() {
            if let Some(matched) = valid_files.iter().find(|f| {
                let stem = f
                    .basename
                    .trim_end_matches(".exe")
                    .trim_end_matches(".EXE")
                    .to_ascii_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>();
                !stem.is_empty() && (stem == hint_norm || hint_norm.contains(&stem) || stem.contains(&hint_norm))
            }) {
                return Some((*matched).clone());
            }
        }

        valid_files.sort_by_key(|f| {
            let depth = f.relative.matches('\\').count();
            let is_top = depth == 0;
            let is_bin = f.relative.contains(r"bin\") || f.relative.contains(r"binaries\") || f.relative.contains(r"win64\");
            if is_top { 0 } else if is_bin { 1 } else { 2 + depth }
        });

        valid_files.first().map(|f| (*f).clone())
    }

    pub fn observe(
        provider: Provider,
        product_id: Option<&str>,
        root: &Path,
        declarations: &[String],
    ) -> Result<Self, EvidenceError> {
        checked_root(root)?;
        let declarations = declarations
            .iter()
            .map(|value| normalize_relative_exe(value))
            .collect::<Result<Vec<_>, _>>()?;
        let mut files = Vec::new();
        let mut remaining = 100_000usize;
        let deadline = Instant::now() + Duration::from_secs(2);
        observe_directory(root, root, 0, &mut remaining, deadline, &mut files)?;
        files.sort_by(|a, b| a.relative.cmp(&b.relative).then(a.path.cmp(&b.path)));
        Ok(Self {
            provider,
            product_id: product_id.map(str::to_owned),
            install_root: root.to_path_buf(),
            declarations,
            files,
            recursive_suggestions: Vec::new(),
        })
    }

    fn nominated(&self, nomination: &str) -> Result<SelectedExecutable, Authority> {
        self.select(nomination, true)
    }

    fn declared(&self, declaration: &str) -> Result<SelectedExecutable, Authority> {
        self.select(declaration, false)
    }

    fn select(&self, nomination: &str, recursive_basename: bool) -> Result<SelectedExecutable, Authority> {
        let nomination = normalize_relative_exe(nomination).map_err(|_| Authority::Unresolved)?;
        if permanently_excluded(nomination.rsplit('\\').next().unwrap_or_default()) {
            return Err(Authority::Unresolved);
        }
        let mut matches = self.files.iter().filter(|file| {
            if recursive_basename && !nomination.contains('\\') {
                file.basename == nomination
            } else {
                file.relative == nomination
            }
        });
        let selected = matches.next().ok_or(Authority::Unresolved)?;
        if matches.next().is_some() {
            return Err(Authority::Ambiguous);
        }
        Ok(selected.clone())
    }
}

fn observe_directory(
    root: &Path,
    directory: &Path,
    depth: usize,
    remaining: &mut usize,
    deadline: Instant,
    files: &mut Vec<SelectedExecutable>,
) -> Result<(), EvidenceError> {
    if depth > 32 || Instant::now() >= deadline {
        return Err(EvidenceError::Incomplete);
    }
    let entries = fs::read_dir(directory).map_err(|_| EvidenceError::Inaccessible)?;
    for entry in entries {
        if *remaining == 0 || Instant::now() >= deadline {
            return Err(EvidenceError::Incomplete);
        }
        *remaining -= 1;
        let path = entry.map_err(|_| EvidenceError::Inaccessible)?.path();
        let metadata = fs::symlink_metadata(&path).map_err(|_| EvidenceError::Inaccessible)?;
        if is_reparse(&metadata) {
            return Err(EvidenceError::ReparsePoint);
        }
        if metadata.is_dir() {
            observe_directory(root, &path, depth + 1, remaining, deadline, files)?;
        } else if metadata.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| EvidenceError::NotLocal)?;
            let relative =
                normalize_relative_exe(relative.to_str().ok_or(EvidenceError::InvalidPath)?)?;
            files.push(SelectedExecutable {
                basename: relative.rsplit('\\').next().unwrap().to_owned(),
                path: local_file(root, &relative)?,
                relative,
            });
        }
    }
    if Instant::now() >= deadline {
        return Err(EvidenceError::Incomplete);
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct ResolvedGame {
    pub catalog: CatalogEntry,
    pub provider: Provider,
    pub product_id: Option<String>,
    pub executables: Vec<SelectedExecutable>,
}

impl ResolvedGame {
    pub fn as_app(&self, auto_detect: bool) -> HdrApp {
        HdrApp {
            name: self.catalog.name.clone(),
            exe_name: self.executables[0].basename.clone(),
            enabled: auto_detect,
            hdr_type: self.catalog.hdr_type.clone(),
            path: Some(crate::runtime_policy::clean_path_display(&self.executables[0].path)),
            alternate_exes: self.executables[1..]
                .iter()
                .map(|file| file.basename.clone())
                .collect(),
            steam_id: (self.provider == Provider::Steam)
                .then(|| self.product_id.clone())
                .flatten(),
            launcher: Some(self.provider.launcher().into()),
        }
    }
}

#[derive(Debug)]
pub enum Authority {
    Resolved(ResolvedGame),
    Unresolved,
    Conflict,
    Ambiguous,
}

fn binding_matches(binding: &StorefrontBinding, basename: &str) -> bool {
    !permanently_excluded(basename)
        && binding.game_executables.iter().any(|exe| exe == basename)
        && !binding
            .excluded_executables
            .iter()
            .any(|exe| exe == basename)
}

fn finish(entry: &CatalogEntry, evidence: &InstallEvidence, nominations: &[String]) -> Authority {
    let mut selected = Vec::new();
    for nomination in nominations {
        let selected_nomination = if evidence.provider == Provider::Steam {
            evidence.nominated(nomination)
        } else {
            evidence.declared(nomination)
        };
        match selected_nomination {
            Ok(file) => selected.push(file),
            Err(result) => return result,
        }
    }
    selected.sort_by(|a, b| {
        a.basename
            .cmp(&b.basename)
            .then(a.relative.cmp(&b.relative))
    });
    selected.dedup();
    if selected.is_empty() {
        return Authority::Unresolved;
    }
    if selected
        .windows(2)
        .any(|pair| pair[0].basename == pair[1].basename)
    {
        return Authority::Ambiguous;
    }
    Authority::Resolved(ResolvedGame {
        catalog: entry.clone(),
        provider: evidence.provider,
        product_id: evidence.product_id.clone(),
        executables: selected,
    })
}

/// Pure authority decision over a completed filesystem observation. No title/stem or saved-row
/// matching occurs here. A foreground basename alone supplies no install/provider evidence.
pub fn resolve(catalog: &[CatalogEntry], evidence: Option<&InstallEvidence>) -> Authority {
    let Some(evidence) = evidence else {
        return Authority::Unresolved;
    };
    // Suggestions and install-root names deliberately do not participate in association.
    let _ = (&evidence.recursive_suggestions, &evidence.install_root);
    match evidence.provider {
        Provider::Steam => {
            let Some(id) = evidence.product_id.as_deref() else {
                return Authority::Unresolved;
            };
            if id.parse::<u32>().ok().is_none_or(|id| id == 0) || id.trim() != id {
                return Authority::Conflict;
            }
            match database::find_storefront_binding(catalog, StorefrontProvider::Steam, Some(id)) {
                Err(_) => Authority::Ambiguous,
                Ok(None) => {
                    let conflict = catalog.iter().any(|entry| {
                        entry
                            .storefront_binding(StorefrontProvider::Steam, None)
                            .ok()
                            .flatten()
                            .is_some_and(|binding| {
                                binding
                                    .product_id
                                    .as_deref()
                                    .is_some_and(|known| known != id)
                                    && evidence
                                        .files
                                        .iter()
                                        .any(|file| binding_matches(&binding, &file.basename))
                            })
                    });
                    if conflict {
                        Authority::Conflict
                    } else {
                        Authority::Unresolved
                    }
                }
                Ok(Some((entry, binding))) => {
                    let nominations = binding.game_executables.iter()
                        .filter(|exe| binding_matches(&binding, exe))
                        .cloned()
                        .collect::<Vec<_>>();
                    finish(entry, evidence, &nominations)
                }
            }
        }
        Provider::Xbox => resolve_declared(catalog, evidence, true),
        Provider::Epic | Provider::Gog | Provider::Windows => {
            resolve_declared(catalog, evidence, false)
        }
    }
}

fn resolve_declared(catalog: &[CatalogEntry], evidence: &InstallEvidence, xbox: bool) -> Authority {
    if evidence.declarations.is_empty() {
        return Authority::Unresolved;
    }
    let mut declarations = BTreeSet::new();
    for declaration in &evidence.declarations {
        let Ok(declaration) = normalize_relative_exe(declaration) else {
            return Authority::Unresolved;
        };
        if !declarations.insert(declaration) {
            return Authority::Ambiguous;
        }
    }
    let mut matches = Vec::new();
    let mut conflict = false;
    for entry in catalog {
        let binding = if xbox {
            match entry.storefront_binding(StorefrontProvider::Xbox, None) {
                Ok(Some(binding)) => Some(binding),
                Ok(None) => continue,
                Err(_) => return Authority::Ambiguous,
            }
        } else {
            None
        };
        let nominations: BTreeSet<String> = declarations
            .iter()
            .filter(|declaration| {
                let basename = declaration.rsplit('\\').next().unwrap_or_default();
                if permanently_excluded(basename) {
                    return false;
                }
                binding.as_ref().map_or_else(
                    || entry.authorizes_declared_executable(basename),
                    |binding| binding_matches(binding, basename),
                )
            })
            .cloned()
            .collect();
        if nominations.is_empty() {
            continue;
        }
        if let Some(binding) = binding {
            if matches!((&evidence.product_id, &binding.product_id), (Some(actual), Some(expected)) if actual != expected)
            {
                conflict = true;
                continue;
            }
        }
        matches.push((entry, nominations.into_iter().collect::<Vec<_>>()));
    }
    if conflict {
        return Authority::Conflict;
    }
    match matches.as_slice() {
        [] => Authority::Unresolved,
        [(entry, nominations)] => finish(entry, evidence, nominations),
        _ => Authority::Ambiguous,
    }
}

/// Provider fields have distinct grammars. Commands are tokenized, never used as paths wholesale.
#[derive(Debug, Clone, Copy)]
pub enum LaunchField {
    Path,
    Command,
    DisplayIcon,
}

pub fn launch_declaration(
    root: &Path,
    raw: &str,
    field: LaunchField,
) -> Result<String, EvidenceError> {
    let raw = raw.trim();
    let target = match field {
        LaunchField::Path => raw,
        LaunchField::Command | LaunchField::DisplayIcon => {
            if let Some(quoted) = raw.strip_prefix('"') {
                let end = quoted.find('"').ok_or(EvidenceError::InvalidPath)?;
                let suffix = &quoted[end + 1..];
                match field {
                    LaunchField::Command
                        if !suffix.is_empty() && !suffix.starts_with(char::is_whitespace) =>
                    {
                        return Err(EvidenceError::InvalidPath)
                    }
                    LaunchField::DisplayIcon => validate_icon_suffix(suffix)?,
                    _ => {}
                }
                &quoted[..end]
            } else if matches!(field, LaunchField::DisplayIcon) {
                if let Some((path, index)) = raw.rsplit_once(',') {
                    validate_icon_suffix(&format!(",{index}"))?;
                    path.trim_end()
                } else {
                    raw
                }
            } else {
                raw.split_whitespace()
                    .next()
                    .ok_or(EvidenceError::InvalidPath)?
            }
        }
    };
    // Absolute provider paths are converted only after a lexical, drive-local root check.
    let root_text = root
        .to_str()
        .ok_or(EvidenceError::InvalidPath)?
        .replace('/', "\\")
        .to_lowercase();
    let target = target.replace('/', "\\");
    let relative = if target.as_bytes().get(1) == Some(&b':') {
        let prefix = format!("{}\\", root_text.trim_end_matches('\\'));
        let lowered = target.to_lowercase();
        lowered
            .strip_prefix(&prefix)
            .ok_or(EvidenceError::NotLocal)?
            .to_owned()
    } else {
        target
    };
    normalize_relative_exe(&relative)
}

fn validate_icon_suffix(suffix: &str) -> Result<(), EvidenceError> {
    let suffix = suffix.trim();
    if suffix.is_empty()
        || suffix
            .strip_prefix(',')
            .is_some_and(|index| index.trim().parse::<i32>().is_ok())
    {
        Ok(())
    } else {
        Err(EvidenceError::InvalidPath)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry() -> CatalogEntry {
        database::authored_test_catalog(vec![serde_json::from_value(json!({
            "name": "Shared title", "exe_name": "steam.exe", "steam_id": "123",
            "hdr_type": "Native", "support_tier": "native", "alternate_exes": ["global.exe"],
            "storefronts": [{
                "provider": "xbox", "product_id": "xbox-id",
                "game_executables": ["Xbox.EXE"], "excluded_executables": ["GameLaunchHelper.exe"]
            }]
        }))
        .unwrap()]).remove(0)
    }

    fn fixture(files: &[&str]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for name in files {
            let path = root.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"fixture, never executed").unwrap();
        }
        root
    }

    fn evidence(
        root: &Path,
        provider: Provider,
        id: Option<&str>,
        declarations: &[&str],
    ) -> InstallEvidence {
        InstallEvidence::observe(
            provider,
            id,
            root,
            &declarations
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    fn resolved(result: Authority) -> ResolvedGame {
        match result {
            Authority::Resolved(game) => game,
            other => panic!("Expected resolved, got {other:?}"),
        }
    }

    #[test]
    fn same_title_uses_only_selected_provider_binding() {
        let root = fixture(&[
            "Steam.EXE",
            "Xbox.exe",
            "global.exe",
            "GameLaunchHelper.exe",
        ]);
        let catalog = vec![entry()];
        let steam = resolved(resolve(
            &catalog,
            Some(&evidence(root.path(), Provider::Steam, Some("123"), &[])),
        ));
        assert_eq!(steam.as_app(true).exe_name, "steam.exe");
        assert!(steam.as_app(true).alternate_exes.is_empty());
        let xbox = resolved(resolve(
            &catalog,
            Some(&evidence(
                root.path(),
                Provider::Xbox,
                Some("xbox-id"),
                &[
                    "Xbox.exe",
                    "global.exe",
                    "Steam.exe",
                    "GameLaunchHelper.exe",
                ],
            )),
        ));
        let app = xbox.as_app(true);
        assert_eq!(app.exe_name, "xbox.exe");
        assert!(app.alternate_exes.is_empty());
        assert_eq!(app.steam_id, None);
        assert!(!xbox.as_app(false).enabled);
    }

    #[test]
    fn injected_catalog_cannot_authorize_permanent_helpers_for_any_provider() {
        let root = fixture(&["GameLaunchHelper.exe", "BsSndRpt.exe", "BsSndRpt64.exe", "BugSplat.exe"]);
        for helper in ["GameLaunchHelper.exe", "BsSndRpt.exe", "BsSndRpt64.exe", "BugSplat.exe"] {
            let catalog: CatalogEntry = serde_json::from_value(json!({
                "name": "Injected helper", "exe_name": helper, "steam_id": "123",
                "hdr_type": "native", "support_tier": "native", "alternate_exes": [helper],
                "storefronts": [{"provider": "xbox", "game_executables": [helper]}]
            })).unwrap();
            let catalog = database::authored_test_catalog(vec![catalog]).remove(0);
            for provider in [Provider::Steam, Provider::Xbox, Provider::Epic, Provider::Gog, Provider::Windows] {
                let observed = evidence(root.path(), provider, (provider == Provider::Steam).then_some("123"), &[helper]);
                assert!(matches!(resolve(&[catalog.clone()], Some(&observed)), Authority::Unresolved), "{provider:?}: {helper}");
                assert!(observed.manual_suggestion().is_none());
            }
        }
    }

    #[test]
    fn selected_runtimes_never_inherit_permanent_helpers_from_injected_positives() {
        let root = fixture(&["game.exe", "BsSndRpt.exe"]);
        let catalog: CatalogEntry = serde_json::from_value(json!({
            "name": "Game", "exe_name": "game.exe", "steam_id": "123",
            "hdr_type": "native", "support_tier": "native", "alternate_exes": ["BsSndRpt.exe"],
            "storefronts": [
                {"provider": "steam", "product_id": "123", "game_executables": ["game.exe", "BsSndRpt.exe"]},
                {"provider": "xbox", "game_executables": ["game.exe", "BsSndRpt.exe"]}
            ]
        })).unwrap();
        let catalog = database::authored_test_catalog(vec![catalog]).remove(0);
        for provider in [Provider::Steam, Provider::Xbox, Provider::Epic, Provider::Gog, Provider::Windows] {
            let observed = evidence(root.path(), provider, (provider == Provider::Steam).then_some("123"), &["game.exe", "BsSndRpt.exe"]);
            let app = resolved(resolve(&[catalog.clone()], Some(&observed))).as_app(true);
            assert_eq!(app.exe_name, "game.exe", "{provider:?}");
            assert!(app.alternate_exes.is_empty());
        }
    }

    #[test]
    fn provider_exclusions_override_positives_only_for_their_own_provider() {
        let root = fixture(&["steam.exe", "xbox.exe"]);
        let mut catalog = entry();
        catalog.storefronts[0].excluded_executables.push("steam.exe".into());
        catalog.storefronts.push(StorefrontBinding {
            provider: StorefrontProvider::Steam,
            product_id: Some("123".into()),
            game_executables: vec!["steam.exe".into()],
            excluded_executables: vec!["xbox.exe".into()],
        });
        catalog = database::authored_test_catalog(vec![catalog]).remove(0);
        for (provider, expected) in [
            (Provider::Steam, "steam.exe"), (Provider::Xbox, "xbox.exe"),
            (Provider::Epic, "steam.exe"), (Provider::Gog, "steam.exe"), (Provider::Windows, "steam.exe"),
        ] {
            let observed = evidence(root.path(), provider, (provider == Provider::Steam).then_some("123"), &["steam.exe", "xbox.exe"]);
            let app = resolved(resolve(&[catalog.clone()], Some(&observed))).as_app(true);
            assert_eq!(app.exe_name, expected, "{provider:?}");
            assert!(app.alternate_exes.is_empty());
        }
        catalog.storefronts[0].excluded_executables.push("xbox.exe".into());
        catalog = database::authored_test_catalog(vec![catalog]).remove(0);
        let xbox = evidence(root.path(), Provider::Xbox, None, &["xbox.exe"]);
        assert!(matches!(resolve(&[catalog.clone()], Some(&xbox)), Authority::Ambiguous));
        let steam = evidence(root.path(), Provider::Steam, Some("123"), &[]);
        assert_eq!(resolved(resolve(&[catalog], Some(&steam))).as_app(true).exe_name, "steam.exe");
    }

    #[test]
    fn steam_id_without_authorized_file_cannot_promote_candidates() {
        let catalog = database::get_full_catalog();
        let root = fixture(&[
            "BsSndRpt.exe",
            "BsSndRpt64.exe",
            "BugSplat.exe",
            "editor.exe",
            "tool.exe",
        ]);
        let mut observed = evidence(
            root.path(),
            Provider::Steam,
            Some("1466860"),
            &["BsSndRpt.exe"],
        );
        observed.recursive_suggestions = vec!["BsSndRpt.exe".into(), "RelicCardinal.exe".into()];
        assert!(matches!(
            resolve(&catalog, Some(&observed)),
            Authority::Unresolved
        ));
        let root = fixture(&["global.exe", "xbox.exe"]);
        assert!(matches!(
            resolve(
                &[entry()],
                Some(&evidence(root.path(), Provider::Steam, Some("123"), &[]))
            ),
            Authority::Unresolved
        ));
    }

    #[test]
    fn recursive_files_and_suggestions_never_nominate_for_any_provider() {
        let root = fixture(&["steam.exe", "xbox.exe", "global.exe", r"bin\tool.exe"]);
        for provider in [
            Provider::Steam,
            Provider::Xbox,
            Provider::Epic,
            Provider::Gog,
            Provider::Windows,
        ] {
            let mut observed = evidence(root.path(), provider, None, &[]);
            observed.recursive_suggestions =
                vec!["steam.exe".into(), "xbox.exe".into(), "global.exe".into()];
            assert!(
                matches!(resolve(&[entry()], Some(&observed)), Authority::Unresolved),
                "{provider:?}"
            );
        }
    }

    #[test]
    fn only_authorized_files_become_aliases_in_deterministic_order() {
        let root = fixture(&["steam.exe", "zgame.exe", "global.exe", r"bin\editor.exe"]);
        let mut cat = entry();
        cat.storefronts.push(StorefrontBinding {
            provider: StorefrontProvider::Steam,
            product_id: Some("123".into()),
            game_executables: vec!["ZGAME.EXE".into(), "Steam.EXE".into()],
            excluded_executables: vec!["editor.exe".into()],
        });
        cat = database::authored_test_catalog(vec![cat]).remove(0);
        let first = resolved(resolve(
            &[cat.clone()],
            Some(&evidence(root.path(), Provider::Steam, Some("123"), &[])),
        ))
        .as_app(true);
        assert_eq!(first.exe_name, "steam.exe");
        assert_eq!(first.alternate_exes, ["zgame.exe"]);
        cat.storefronts[1].game_executables.reverse();
        cat = database::authored_test_catalog(vec![cat]).remove(0);
        let second = resolved(resolve(
            &[cat],
            Some(&evidence(root.path(), Provider::Steam, Some("123"), &[])),
        ))
        .as_app(true);
        assert_eq!(first, second);
    }

    #[test]
    fn explicit_nonsteam_launch_requires_exact_unique_catalog_and_file() {
        let root = fixture(&[
            "steam.exe",
            "global.exe",
            "unlisted.exe",
            "Shared title.exe",
            "editor.exe",
        ]);
        for provider in [Provider::Epic, Provider::Gog, Provider::Windows] {
            let observed = evidence(root.path(), provider, None, &["global.exe"]);
            let app = resolved(resolve(&[entry()], Some(&observed))).as_app(true);
            assert_eq!(app.exe_name, "global.exe");
            assert!(app.alternate_exes.is_empty());
            assert!(app.steam_id.is_none());
            for declaration in ["missing.exe", "Shared title.exe", "unlisted.exe"] {
                let observed = evidence(root.path(), provider, None, &[declaration]);
                assert!(matches!(
                    resolve(&[entry()], Some(&observed)),
                    Authority::Unresolved
                ));
            }
            assert!(matches!(
                resolve(&[entry(), entry()], Some(&observed)),
                Authority::Ambiguous
            ));
        }
    }

    #[test]
    fn conflicting_known_ids_and_duplicate_catalog_rows_fail_closed() {
        let root = fixture(&["steam.exe", "xbox.exe", "global.exe"]);
        let steam = evidence(root.path(), Provider::Steam, Some("999"), &["steam.exe"]);
        assert!(matches!(
            resolve(&[entry()], Some(&steam)),
            Authority::Conflict
        ));
        let xbox = evidence(root.path(), Provider::Xbox, Some("other-id"), &["xbox.exe"]);
        assert!(matches!(
            resolve(&[entry()], Some(&xbox)),
            Authority::Conflict
        ));
        for provider in [Provider::Steam, Provider::Xbox] {
            let observed = evidence(
                root.path(),
                provider,
                if provider == Provider::Steam {
                    Some("123")
                } else {
                    None
                },
                &["xbox.exe"],
            );
            assert!(matches!(
                resolve(&[entry(), entry()], Some(&observed)),
                Authority::Ambiguous
            ));
        }
        let mut cat = entry();
        cat.storefronts[0].game_executables.push("XBOX.EXE".into());
        cat = database::authored_test_catalog(vec![cat]).remove(0);
        assert!(matches!(
            resolve(
                &[cat],
                Some(&evidence(root.path(), Provider::Xbox, None, &["xbox.exe"]))
            ),
            Authority::Ambiguous
        ));
    }

    #[test]
    fn multiple_declarations_are_filtered_not_inherited_and_collisions_do_not_use_titles() {
        let root = fixture(&["xbox.exe", "other.exe", "GameLaunchHelper.exe"]);
        let observed = evidence(
            root.path(),
            Provider::Xbox,
            None,
            &["GameLaunchHelper.exe", "xbox.exe", "other.exe"],
        );
        let app = resolved(resolve(&[entry()], Some(&observed))).as_app(true);
        assert_eq!(app.exe_name, "xbox.exe");
        assert!(app.alternate_exes.is_empty());
        let mut other = entry();
        other.name = "Entirely different title".into();
        other.storefronts[0].game_executables = vec!["other.exe".into()];
        other = database::authored_test_catalog(vec![other]).remove(0);
        for catalog in [vec![entry(), other.clone()], vec![other, entry()]] {
            assert!(matches!(
                resolve(&catalog, Some(&observed)),
                Authority::Ambiguous
            ));
        }
    }

    #[test]
    fn duplicate_basenames_need_an_explicit_relative_path() {
        let root = fixture(&[r"one\steam.exe", r"two\STEAM.EXE"]);
        for provider in [
            Provider::Steam,
            Provider::Epic,
            Provider::Gog,
            Provider::Windows,
        ] {
            let observed = evidence(root.path(), provider, Some("123"), &["steam.exe"]);
            let result = resolve(&[entry()], Some(&observed));
            if provider == Provider::Steam {
                assert!(matches!(result, Authority::Ambiguous));
            } else {
                assert!(matches!(result, Authority::Unresolved));
            }
            if provider != Provider::Steam {
                let observed = evidence(root.path(), provider, None, &[r"two\steam.exe"]);
                assert_eq!(
                    resolved(resolve(&[entry()], Some(&observed))).executables[0].relative,
                    r"two\steam.exe"
                );
            }
        }
        let root = fixture(&[r"one\xbox.exe", r"two\xbox.exe"]);
        assert!(matches!(
            resolve(
                &[entry()],
                Some(&evidence(root.path(), Provider::Xbox, None, &["xbox.exe"]))
            ),
            Authority::Unresolved
        ));
        assert!(matches!(
            resolve(
                &[entry()],
                Some(&evidence(
                    root.path(),
                    Provider::Xbox,
                    None,
                    &[r"one\xbox.exe"]
                ))
            ),
            Authority::Resolved(_)
        ));
    }

    #[test]
    fn unsafe_paths_never_resolve_and_regular_executable_is_required() {
        let root = fixture(&["steam.exe", "not-executable.txt"]);
        for invalid in [
            "",
            ".exe",
            "..\\steam.exe",
            "bin\\..\\steam.exe",
            ".\\steam.exe",
            "\\steam.exe",
            "\\\\server\\share\\steam.exe",
            "\\\\?\\C:\\steam.exe",
            "\\\\.\\C:\\steam.exe",
            "C:\\steam.exe",
            "C:steam.exe",
            "steam.exe:stream",
            "steam.exe ",
            "steam.exe.",
            "steam?.exe",
            "nul.exe",
            "LPT1.exe",
            "a\\\\steam.exe",
            "bin\\ steam.exe",
            "steam.exe\0",
            "not-executable.txt",
        ] {
            assert!(normalize_relative_exe(invalid).is_err(), "{invalid:?}");
        }
        fs::create_dir(root.path().join("directory.exe")).unwrap();
        assert_eq!(
            local_file(root.path(), "directory.exe"),
            Err(EvidenceError::NotRegularFile)
        );
        assert!(local_file(root.path(), r"..\steam.exe").is_err());
        assert!(checked_root(Path::new(r"\\server\share")).is_err());
        assert!(checked_root(Path::new(r"\\?\C:\Games")).is_err());
        assert!(checked_root(Path::new("C:\u{e9}folder")).is_err());
        assert!(checked_root(&root.path().join(".")).is_err());
        assert_eq!(
            checked_root(Path::new(&format!("{}\\", root.path().display()))).unwrap(),
            checked_root(root.path()).unwrap()
        );
    }

    #[test]
    fn junction_escape_is_rejected_in_inventory_file_and_root_ancestors() {
        let root = fixture(&[]);
        let outside = fixture(&["steam.exe"]);
        let junction = root.path().join("linked");
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(outside.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            local_file(root.path(), r"linked\steam.exe"),
            Err(EvidenceError::ReparsePoint)
        );
        assert_eq!(
            InstallEvidence::observe(Provider::Steam, Some("123"), root.path(), &[]).unwrap_err(),
            EvidenceError::ReparsePoint
        );
        assert_eq!(
            local_file(&junction, "steam.exe"),
            Err(EvidenceError::ReparsePoint)
        );
        fs::remove_dir(junction).unwrap();
        assert!(outside.path().join("steam.exe").is_file());
    }

    #[test]
    fn config_never_probes_through_a_content_junction() {
        let root = fixture(&["MicrosoftGame.config"]);
        let outside = fixture(&["MicrosoftGame.config"]);
        let junction = root.path().join("Content");
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(outside.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            crate::xbox_config::declarations(root.path()),
            Err(crate::xbox_config::ConfigError::UnsafeConfigPath)
        );
        fs::remove_dir(junction).unwrap();
        assert!(outside.path().join("MicrosoftGame.config").is_file());
    }

    #[test]
    fn incomplete_inventory_never_supplies_partial_unique_evidence() {
        let root = fixture(&["steam.exe"]);
        let mut files = Vec::new();
        assert_eq!(
            observe_directory(
                root.path(),
                root.path(),
                0,
                &mut 0,
                Instant::now() + Duration::from_secs(1),
                &mut files
            ),
            Err(EvidenceError::Incomplete)
        );
        assert_eq!(
            observe_directory(
                root.path(),
                root.path(),
                0,
                &mut 10,
                Instant::now(),
                &mut files
            ),
            Err(EvidenceError::Incomplete)
        );
        assert_eq!(
            observe_directory(
                root.path(),
                root.path(),
                33,
                &mut 10,
                Instant::now() + Duration::from_secs(1),
                &mut files
            ),
            Err(EvidenceError::Incomplete)
        );
    }

    #[test]
    fn provider_path_command_and_icon_grammars_are_distinct() {
        let root = Path::new(r"C:\Games\A Game");
        for (raw, field, expected) in [
            (r"Bin/Game.EXE", LaunchField::Path, r"bin\game.exe"),
            (
                r#""C:\Games\A Game\Bin\Game.exe" -fullscreen"#,
                LaunchField::Command,
                r"bin\game.exe",
            ),
            (r"game.exe --launch", LaunchField::Command, "game.exe"),
            (
                r#""C:\Games\A Game\game.exe",0"#,
                LaunchField::DisplayIcon,
                "game.exe",
            ),
            (
                r"C:\Games\A Game\game.exe,-1",
                LaunchField::DisplayIcon,
                "game.exe",
            ),
            (
                r#""C:\Games\A Game\file,with,commas.exe",2"#,
                LaunchField::DisplayIcon,
                "file,with,commas.exe",
            ),
        ] {
            assert_eq!(launch_declaration(root, raw, field).unwrap(), expected);
        }
        for (raw, field) in [
            (r#""game.exe" --launch"#, LaunchField::Path),
            ("game.exe --launch", LaunchField::Path),
            (r"C:\Games\A Game\game.exe --launch", LaunchField::Command),
            (r#""game.exe"junk"#, LaunchField::Command),
            (r#""game.exe",-bad"#, LaunchField::DisplayIcon),
            (r#""game.exe" --launch"#, LaunchField::DisplayIcon),
            (r"C:\Games\A Game Other\game.exe", LaunchField::Path),
            (r"\\server\share\game.exe", LaunchField::Path),
            (r"\\?\C:\Games\A Game\game.exe", LaunchField::Path),
            (r"C:\Games\A Game\..\outside.exe", LaunchField::Path),
        ] {
            assert!(launch_declaration(root, raw, field).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn foreground_without_install_evidence_has_no_automatic_authority() {
        assert!(matches!(resolve(&[entry()], None), Authority::Unresolved));
    }

    fn correction_declared_catalog() -> Vec<CatalogEntry> {
        database::authored_test_catalog(vec![serde_json::from_value(json!({
            "name": "Declared fixture", "exe_name": "game.exe", "steam_id": "123",
            "hdr_type": "native", "support_tier": "native",
            "storefronts": [{"provider": "xbox", "game_executables": ["game.exe"]}]
        })).unwrap()])
    }

    fn correction_declared_fixture(files: &[&str]) -> tempfile::TempDir {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        for name in files {
            let path = root.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"fixture, never executed").unwrap();
        }
        root
    }

    fn correction_declared_root_scope(provider: Provider) {
        let catalog = correction_declared_catalog();
        let exact = correction_declared_fixture(&["game.exe"]);
        let observed = evidence(exact.path(), provider, None, &["game.exe"]);
        assert_eq!(resolved(resolve(&catalog, Some(&observed))).executables[0].relative, "game.exe");
        let explicit = correction_declared_fixture(&[r"Tools\game.exe", r"Other\game.exe"]);
        let observed = evidence(explicit.path(), provider, None, &[r"Tools\game.exe"]);
        assert_eq!(resolved(resolve(&catalog, Some(&observed))).executables[0].relative, r"tools\game.exe");
        let descendant = correction_declared_fixture(&[r"Tools\game.exe"]);
        let observed = evidence(descendant.path(), provider, None, &["game.exe"]);
        assert!(
            matches!(resolve(&catalog, Some(&observed)), Authority::Unresolved),
            "{provider:?}: a root declaration selected an undeclared descendant"
        );
    }

    #[test]
    fn correction_declared_xbox_root_filename_is_an_exact_relative_path() {
        correction_declared_root_scope(Provider::Xbox);
    }

    #[test]
    fn correction_declared_epic_root_filename_is_an_exact_relative_path() {
        correction_declared_root_scope(Provider::Epic);
    }

    #[test]
    fn correction_declared_gog_root_filename_is_an_exact_relative_path() {
        correction_declared_root_scope(Provider::Gog);
    }

    #[test]
    fn correction_declared_windows_root_filename_is_an_exact_relative_path() {
        correction_declared_root_scope(Provider::Windows);
    }

    #[test]
    fn correction_declared_manual_suggestion_never_recursively_promotes_a_basename() {
        let root = correction_declared_fixture(&[r"Tools\game.exe"]);
        for provider in [Provider::Steam, Provider::Xbox, Provider::Epic, Provider::Gog, Provider::Windows] {
            let observed = evidence(root.path(), provider, None, &[r"Tools\game.exe"]);
            assert_eq!(observed.manual_suggestion().unwrap().relative, r"tools\game.exe");
            let observed = evidence(root.path(), provider, None, &["game.exe"]);
            assert!(observed.manual_suggestion().is_none(), "{provider:?}: suggestion escaped declaration scope");
        }
    }

    #[test]
    fn correction_declared_steam_catalog_nomination_and_provider_declaration_are_distinct() {
        let root = correction_declared_fixture(&[r"Tools\game.exe"]);
        let observed = evidence(root.path(), Provider::Steam, Some("123"), &["game.exe"]);
        let game = resolved(resolve(&correction_declared_catalog(), Some(&observed)));
        assert_eq!(game.executables[0].relative, r"tools\game.exe");
        assert!(observed.manual_suggestion().is_none());
    }

    #[test]
    fn correction_declared_duplicate_descendants_do_not_turn_root_path_into_basename_search() {
        let root = correction_declared_fixture(&[r"One\game.exe", r"Two\game.exe"]);
        let catalog = correction_declared_catalog();
        for provider in [Provider::Xbox, Provider::Epic, Provider::Gog, Provider::Windows] {
            let observed = evidence(root.path(), provider, None, &["game.exe"]);
            assert!(matches!(resolve(&catalog, Some(&observed)), Authority::Unresolved), "{provider:?}");
            assert!(observed.manual_suggestion().is_none());
            let observed = evidence(root.path(), provider, None, &[r"Two\game.exe"]);
            assert_eq!(resolved(resolve(&catalog, Some(&observed))).executables[0].relative, r"two\game.exe");
        }
    }

    #[test]
    fn correction_declared_existing_root_is_not_ambiguous_with_same_named_descendants() {
        let root = correction_declared_fixture(&["game.exe", r"One\game.exe", r"Two\game.exe"]);
        let catalog = correction_declared_catalog();
        for provider in [Provider::Xbox, Provider::Epic, Provider::Gog, Provider::Windows] {
            let observed = evidence(root.path(), provider, None, &["game.exe"]);
            let selected = resolved(resolve(&catalog, Some(&observed)));
            assert_eq!(selected.executables[0].relative, "game.exe", "{provider:?}");
            assert_eq!(observed.manual_suggestion().unwrap().relative, "game.exe");
        }
    }

    #[test]
    fn correction_declared_duplicate_normalized_declarations_fail_closed() {
        let catalog = correction_declared_catalog();
        for provider in [Provider::Xbox, Provider::Epic, Provider::Gog, Provider::Windows] {
            for (file, declarations) in [
                ("game.exe", vec!["game.exe".into(), "GAME.EXE".into()]),
                (r"Tools\game.exe", vec!["Tools/game.exe".into(), r"tools\GAME.EXE".into()]),
            ] {
                let root = correction_declared_fixture(&[file]);
                if let Ok(observed) = InstallEvidence::observe(provider, None, root.path(), &declarations) {
                    assert!(
                        !matches!(resolve(&catalog, Some(&observed)), Authority::Resolved(_)),
                        "{provider:?}: duplicate declarations gained authority after deduplication"
                    );
                }
            }
        }
    }

    #[test]
    fn correction_declared_missing_root_and_subpath_cannot_collapse_into_one_selection() {
        let root = correction_declared_fixture(&[r"Tools\game.exe"]);
        let catalog = correction_declared_catalog();
        for provider in [Provider::Xbox, Provider::Epic, Provider::Gog, Provider::Windows] {
            let declarations = vec!["game.exe".into(), r"Tools\game.exe".into()];
            if let Ok(observed) = InstallEvidence::observe(provider, None, root.path(), &declarations) {
                assert!(
                    !matches!(resolve(&catalog, Some(&observed)), Authority::Resolved(_)),
                    "{provider:?}: conflicting relative paths collapsed onto a descendant"
                );
            }
        }
    }

    #[test]
    fn aoe3_config_selects_only_xbox_declaration_and_invalid_configs_cannot_activate() {
        let catalog = database::get_full_catalog();
        let root = fixture(&[r"Content\AoE3DE.exe", r"Content\GameLaunchHelper.exe"]);
        let config = root.path().join(r"Content\MicrosoftGame.config");
        assert!(crate::xbox_config::declarations(root.path()).is_err());
        for text in [
            "<Game><ExecutableList><Executable Name=\"AoE3DE.exe\"/></ExecutableList>",
            &" ".repeat(65_537),
        ] {
            fs::write(&config, text).unwrap();
            assert!(crate::xbox_config::declarations(root.path()).is_err());
        }
        fs::write(&config, r#"<Game><ExecutableList><Executable Name="GameLaunchHelper.exe"/><Executable Name="AoE3DE.exe"/></ExecutableList></Game>"#).unwrap();
        let declarations = crate::xbox_config::declarations(root.path()).unwrap();
        let observed =
            InstallEvidence::observe(Provider::Xbox, None, root.path(), &declarations).unwrap();
        let app = resolved(resolve(&catalog, Some(&observed))).as_app(true);
        assert_eq!(app.exe_name, "aoe3de.exe");
        assert!(app.alternate_exes.is_empty());
        assert!(app.steam_id.is_none());
        let observed = evidence(
            root.path(),
            Provider::Xbox,
            None,
            &[r"Content\GameLaunchHelper.exe"],
        );
        assert!(matches!(
            resolve(&catalog, Some(&observed)),
            Authority::Unresolved
        ));
    }
}
