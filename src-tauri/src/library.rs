use crate::config::{AppConfig, HdrApp};
use crate::automatic_authority::{Provider, ResolvedGame};
use crate::runtime_policy::{claims_overlap, is_quarantined, normalize_windows_path, permanently_excluded, primary_path_consistent};

/// Snapshot-local identity, always accompanied by a library-generation fence at IPC.
/// The index distinguishes even identical legacy rows without a persisted migration.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct AppRowIdentity {
    pub index: usize,
    pub exe_name: String,
    pub path: Option<String>,
}

impl AppRowIdentity {
    #[cfg(test)]
    pub fn at(config: &AppConfig, index: usize) -> Result<Self, String> {
        let app = config.apps.get(index).ok_or("This game is no longer in the library. Refresh before editing it.")?;
        Ok(Self { index, exe_name: app.exe_name.clone(), path: app.path.clone() })
    }
}

fn row_index(config: &AppConfig, row: &AppRowIdentity) -> Result<usize, String> {
    let app = config.apps.get(row.index).ok_or("This game is no longer in the library. Refresh before editing it.")?;
    if app.exe_name != row.exe_name || app.path != row.path {
        return Err("This library row changed. Refresh before editing it.".into());
    }
    Ok(row.index)
}

pub fn remove_app(config: &mut AppConfig, row: &AppRowIdentity) -> Result<(), String> {
    let index = row_index(config, row)?;
    config.apps.remove(index);
    Ok(())
}

pub fn toggle_app(config: &mut AppConfig, row: &AppRowIdentity, enabled: bool) -> Result<(), String> {
    let index = row_index(config, row)?;
    if enabled && is_quarantined(&config.apps[index]) {
        return Err("Select the actual game executable before enabling this row.".into());
    }
    config.apps[index].enabled = enabled;
    Ok(())
}

fn same_path(left: &str, right: &str) -> bool {
    matches!((normalize_windows_path(left), normalize_windows_path(right)), (Some(left), Some(right)) if left == right)
}

/// An ID can veto duplicate creation, never establish an association for enrichment.
/// A saved path scopes only its primary; historical aliases have no saved path.
pub fn automatic_enrollment_veto(existing: &[HdrApp], candidate: &HdrApp) -> bool {
    existing.iter().any(|app| {
        if matches!((&app.steam_id, &candidate.steam_id), (Some(left), Some(right)) if !left.trim().is_empty() && left == right) {
            return true;
        }
        claims_overlap(app, candidate)
    })
}

fn same_game(existing: &HdrApp, item: &HdrApp) -> bool {
    if existing.exe_name.eq_ignore_ascii_case(&item.exe_name) {
        return true;
    }

    if matches!((&existing.steam_id, &item.steam_id), (Some(left), Some(right)) if left != right) {
        return false;
    }
    if existing.name.eq_ignore_ascii_case(&item.name) {
        return true;
    }
    std::iter::once(&existing.exe_name)
        .chain(&existing.alternate_exes)
        .any(|exe| {
            std::iter::once(&item.exe_name)
                .chain(&item.alternate_exes)
                .any(|other| exe.eq_ignore_ascii_case(other))
        })
}

fn merge_index(config: &AppConfig, item: &HdrApp) -> Result<Option<usize>, String> {
    let candidates: Vec<_> = config.apps.iter().enumerate()
        .filter(|(_, app)| same_game(app, item)).map(|(index, _)| index).collect();
    let exact: Vec<_> = candidates.iter().copied().filter(|&index| {
        let app = &config.apps[index];
        app.exe_name.eq_ignore_ascii_case(&item.exe_name)
            && matches!((&app.path, &item.path), (Some(left), Some(right)) if same_path(left, right))
    }).collect();
    let selected = if exact.is_empty() { &candidates } else { &exact };
    match selected.as_slice() {
        [] => Ok(None),
        [index] => Ok(Some(*index)),
        _ => Err("Multiple library rows match this game. Remove duplicate rows or select the intended row's executable in My Games.".into()),
    }
}

fn reject_other_claims(config: &AppConfig, except: Option<usize>, candidate: &HdrApp) -> Result<(), String> {
    if let Some((_, owner)) = config.apps.iter().enumerate()
        .find(|(index, app)| Some(*index) != except && claims_overlap(app, candidate))
    {
        return Err(format!(
            "Executable ownership conflicts with '{}' ({}). Remove or repair that library row first; no rows were merged.",
            owner.name, owner.exe_name,
        ));
    }
    Ok(())
}

fn merge_aliases(existing: &mut HdrApp, item: &HdrApp) {
    for exe in std::iter::once(&item.exe_name).chain(&item.alternate_exes) {
        if !existing.exe_name.eq_ignore_ascii_case(exe)
            && !existing
                .alternate_exes
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(exe))
        {
            existing.alternate_exes.push(exe.to_lowercase());
        }
    }
}

fn apply_explicit_executables(existing: &mut HdrApp, item: &HdrApp) {
    existing.alternate_exes.retain(|exe| !permanently_excluded(exe));
    if is_quarantined(existing) {
        existing.exe_name = item.exe_name.trim().to_lowercase();
        existing.path.clone_from(&item.path);
        existing.alternate_exes.clear();
    }
    merge_aliases(existing, item);
    if existing.exe_name.eq_ignore_ascii_case(&item.exe_name) && item.path.is_some() {
        existing.path.clone_from(&item.path);
    }
}

pub fn import_games(config: &mut AppConfig, detected: Vec<HdrApp>) -> Result<(), String> {
    let mut primaries = std::collections::HashMap::<String, (&HdrApp, Option<&str>)>::new();
    for item in &detected {
        validate_app(item)?;
        let primary = item.exe_name.to_lowercase();
        if let Some((previous, known_id)) = primaries.get_mut(&primary) {
            let same_provider = match (&previous.launcher, &item.launcher) {
                (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
                (None, None) => true,
                _ => false,
            };
            let same_installation = match (&previous.path, &item.path) {
                (Some(left), Some(right)) => same_path(left, right),
                (None, None) => true,
                _ => false,
            };
            let conflicting_id = matches!((*known_id, item.steam_id.as_deref()),
                (Some(left), Some(right)) if left != right);
            if !same_provider || !same_installation || conflicting_id {
                return Err(format!(
                    "Multiple detected installations use '{}'. Select only one installation for this executable before importing.",
                    item.exe_name,
                ));
            }
            if known_id.is_none() {
                *known_id = item.steam_id.as_deref();
            }
        } else {
            primaries.insert(primary, (item, item.steam_id.as_deref()));
        }
    }
    let mut updated = config.clone();
    for mut item in detected {
        item.enabled = true;
        if let Some(index) = merge_index(&updated, &item)? {
            let mut existing = updated.apps[index].clone();
            apply_explicit_executables(&mut existing, &item);
            if item.launcher.is_some() {
                existing.launcher = item.launcher;
            }
            if item.steam_id.is_some() {
                existing.steam_id = item.steam_id;
            }
            existing.enabled = true;
            validate_app(&existing)?;
            reject_other_claims(&updated, Some(index), &existing)?;
            updated.apps[index] = existing;
        } else {
            reject_other_claims(&updated, None, &item)?;
            updated.apps.push(item);
        }
    }
    *config = updated;
    Ok(())
}

#[cfg(test)]
// Retained only for legacy saved-row/recovery contract fixtures, never automatic discovery.
pub fn enrich_existing(config: &mut AppConfig, detected: &[HdrApp]) -> bool {
    let mut changed = false;
    for item in detected {
        if let Some(existing) = config.apps.iter_mut().find(|app| same_game(app, item)) {
            let before = existing.clone();
            merge_aliases(existing, item);
            if existing.steam_id.is_none() {
                existing.steam_id.clone_from(&item.steam_id);
            }
            if existing.launcher.is_none() {
                existing.launcher.clone_from(&item.launcher);
            }
            changed |= *existing != before;
        }
    }
    changed
}

fn verified_association(existing: &HdrApp, resolved: &ResolvedGame) -> bool {
    let Some(primary) = resolved.executables.first() else { return false };
    let Some(canonical) = resolved.catalog.authoritative_primary() else { return false };
    if is_quarantined(existing)
        || resolved.executables.iter().any(|file| permanently_excluded(&file.basename))
        || !(existing.exe_name.eq_ignore_ascii_case(&primary.basename)
            || existing.exe_name.eq_ignore_ascii_case(canonical))
    {
        return false;
    }
    let known_id = if resolved.provider == Provider::Steam {
        resolved.product_id.as_deref()
    } else {
        resolved.catalog.authoritative_steam_id()
    };
    if matches!((existing.steam_id.as_deref(), known_id), (Some(left), Some(right)) if left != right)
        || (existing.steam_id.is_some() && resolved.provider != Provider::Steam)
        || existing.launcher.as_ref().is_some_and(|launcher| {
            !launcher.eq_ignore_ascii_case(resolved.provider.launcher())
        })
    {
        return false;
    }
    if let Some(path) = &existing.path {
        // A canonical name alone cannot associate an independently path-bound installation.
        return resolved.executables.iter().any(|file| {
            existing.exe_name.eq_ignore_ascii_case(&file.basename)
                && same_path(path, &file.path.to_string_lossy())
        });
    }
    true
}

fn unique_verified_associations(config: &AppConfig, detected: &[ResolvedGame]) -> Vec<Option<usize>> {
    let associations: Vec<Vec<usize>> = detected.iter().map(|resolved| {
        config.apps.iter().enumerate()
            .filter_map(|(index, existing)| verified_association(existing, resolved).then_some(index))
            .collect()
    }).collect();
    let mut candidate_counts = vec![0usize; config.apps.len()];
    for candidates in &associations {
        for &index in candidates {
            candidate_counts[index] += 1;
        }
    }
    associations.iter().map(|candidates| match candidates.as_slice() {
        [index] if candidate_counts[*index] == 1 => Some(*index),
        _ => None,
    }).collect()
}

/// Selected local basenames are the only additions; every saved metadata field stays unchanged.
pub fn enrich_verified_aliases(config: &mut AppConfig, detected: &[ResolvedGame]) -> bool {
    if !config.auto_detect_new_games {
        return false;
    }
    let associations = unique_verified_associations(config, detected);
    let mut changed = false;
    for (resolved, index) in detected.iter().zip(associations) {
        let Some(index) = index else { continue };
        let existing = &mut config.apps[index];
        for file in &resolved.executables {
            if !existing.exe_name.eq_ignore_ascii_case(&file.basename)
                && !existing.alternate_exes.iter().any(|exe| exe.eq_ignore_ascii_case(&file.basename))
            {
                existing.alternate_exes.push(file.basename.clone());
                changed = true;
            }
        }
    }
    changed
}

/// Layer3's separate metadata-only policy: fill missing values for an exact selected primary.
pub fn enrich_verified_metadata(config: &mut AppConfig, detected: &[ResolvedGame]) -> bool {
    if !config.auto_detect_new_games {
        return false;
    }
    let associations = unique_verified_associations(config, detected);
    let mut changed = false;
    for (resolved, index) in detected.iter().zip(associations) {
        let Some(index) = index else { continue };
        let existing = &mut config.apps[index];
        if existing.exe_name.eq_ignore_ascii_case(&resolved.executables[0].basename) {
            if existing.steam_id.is_none() && resolved.provider == Provider::Steam {
                if let Some(id) = &resolved.product_id {
                    existing.steam_id = Some(id.clone());
                    changed = true;
                }
            }
            if existing.launcher.is_none() {
                existing.launcher = Some(resolved.provider.launcher().into());
                changed = true;
            }
        }
    }
    changed
}

fn validate_executable(exe: &str) -> Result<(), String> {
    if permanently_excluded(exe) {
        return Err(format!("Executable '{exe}' is a helper, not a game runtime. Select the game's executable instead."));
    }
    if exe.trim().is_empty()
        || exe.contains(['\\', '/'])
        || !exe.to_ascii_lowercase().ends_with(".exe")
    {
        return Err("A game needs executable filenames ending in .exe, without a directory path.".into());
    }
    Ok(())
}

pub fn validate_app(app: &HdrApp) -> Result<(), String> {
    if app.name.trim().is_empty()
        || app.exe_name.trim().is_empty()
        || app.exe_name.contains(['\\', '/'])
        || !app.exe_name.to_ascii_lowercase().ends_with(".exe")
    {
        return Err("A game needs a name and an executable filename ending in .exe.".into());
    }
    for exe in std::iter::once(&app.exe_name).chain(&app.alternate_exes) {
        validate_executable(exe)?;
    }
    if !primary_path_consistent(app) {
        return Err("Saved path must identify the primary executable. Select the game's executable again.".into());
    }
    Ok(())
}

pub fn add_app(config: &mut AppConfig, mut app: HdrApp) -> Result<(), String> {
    validate_app(&app)?;
    app.exe_name = app.exe_name.trim().to_lowercase();
    if let Some(index) = merge_index(config, &app)? {
        let mut existing = config.apps[index].clone();
        apply_explicit_executables(&mut existing, &app);
        existing.name = app.name;
        existing.enabled = app.enabled;
        existing.hdr_type = app.hdr_type;
        if app.steam_id.is_some() {
            existing.steam_id = app.steam_id;
        }
        if app.launcher.is_some() {
            existing.launcher = app.launcher;
        }
        validate_app(&existing)?;
        reject_other_claims(config, Some(index), &existing)?;
        config.apps[index] = existing;
    } else {
        reject_other_claims(config, None, &app)?;
        config.apps.push(app);
    }
    Ok(())
}

/// Explicitly repair one quarantined row, without title matching or changing user policy.
pub fn repair_executable(
    config: &mut AppConfig,
    row: &AppRowIdentity,
    selected_exe: &str,
    selected_path: &str,
) -> Result<(), String> {
    let index = row_index(config, row)?;
    if !is_quarantined(&config.apps[index]) {
        return Err("This app is no longer quarantined. Refresh the library and try again.".into());
    }
    let selected_exe = selected_exe.trim().to_lowercase();
    validate_executable(&selected_exe)?;
    if !normalize_windows_path(selected_path).is_some_and(|path| {
        path.rsplit('\\').next() == Some(selected_exe.as_str())
    }) {
        return Err("Selected path must identify the selected game executable.".into());
    }
    let mut existing = config.apps[index].clone();
    existing.exe_name = selected_exe;
    existing.path = Some(selected_path.to_owned());
    existing.alternate_exes.clear();
    reject_other_claims(config, Some(index), &existing)?;
    config.apps[index] = existing;
    Ok(())
}

#[cfg(test)]
mod audit_identity_tests {
    use super::*;
    use crate::config::{ConfigManager, HdrType};

    fn app(exe: &str, path: Option<&str>, enabled: bool) -> HdrApp {
        HdrApp {
            name: format!("Custom {exe}"), exe_name: exe.into(), path: path.map(str::to_owned),
            enabled, hdr_type: HdrType::Custom, alternate_exes: vec![],
            steam_id: Some("user-choice".into()), launcher: Some("Custom launcher".into()),
        }
    }

    #[test]
    fn audit_primary_path_validation_rejects_add_import_and_update_before_mutation() {
        for path in [r"C:\Game\other.exe", r"C:\Game\BsSndRpt.exe", "relative.exe", r"C:\..\game.exe"] {
            let incoming = app("game.exe", Some(path), true);
            for rows in [vec![], vec![app("game.exe", Some(r"C:\Old\game.exe"), false)]] {
                for import in [false, true] {
                    let mut config = AppConfig::default();
                    config.apps = rows.clone();
                    let before = config.clone();
                    let result = if import { import_games(&mut config, vec![incoming.clone()]) }
                        else { add_app(&mut config, incoming.clone()) };
                    assert!(result.unwrap_err().contains("primary executable"));
                    assert_eq!(config, before);
                }
            }
        }
        for path in [None, Some(""), Some(r"\\?\C:\Game\.\GAME.EXE"), Some(r"\\server\share\Game.exe")] {
            validate_app(&app("game.exe", path, true)).unwrap();
        }
    }

    #[test]
    fn audit_repair_collisions_enabled_disabled_scoped_and_alias_owners_are_noops() {
        for enabled in [false, true] {
            for owner_kind in 0..3 {
                let original = app("BsSndRpt.exe", Some(r"C:\Old\BsSndRpt.exe"), false);
                let mut owner = app("game.exe", None, enabled);
                if owner_kind == 1 { owner.path = Some(r"\\?\d:\GAME\game.exe".into()); }
                if owner_kind == 2 {
                    owner.exe_name = "other.exe".into();
                    owner.alternate_exes = vec!["GAME.EXE".into()];
                }
                let mut config = AppConfig::default();
                config.apps = vec![original, owner];
                let before = config.clone();
                let row = AppRowIdentity::at(&config, 0).unwrap();
                assert!(repair_executable(&mut config, &row, "game.exe", r"D:\Game\game.exe")
                    .unwrap_err().contains("ownership conflicts"));
                assert_eq!(config, before);
            }
        }
    }

    #[test]
    fn audit_repair_allows_distinct_scoped_installations_and_preserves_preferences() {
        let mut original = app("legacy.exe", Some(r"C:\Wrong\other.exe"), false);
        original.alternate_exes = vec!["historical.exe".into()];
        let owner = app("game.exe", Some(r"C:\Game\game.exe"), true);
        let mut config = AppConfig::default();
        config.apps = vec![original.clone(), owner.clone()];
        let row = AppRowIdentity::at(&config, 0).unwrap();
        repair_executable(&mut config, &row, "game.exe", r"D:\Game\game.exe").unwrap();
        original.exe_name = "game.exe".into();
        original.path = Some(r"D:\Game\game.exe".into());
        original.alternate_exes.clear();
        assert_eq!(config.apps, [original, owner]);
        assert_eq!(config.resolve_app(Some(r"D:\Game\game.exe"), "game.exe"),
            crate::runtime_policy::Resolution::Disabled);
        assert!(config.resolve_app(Some(r"C:\Game\game.exe"), "game.exe").matched().is_some());
    }

    #[test]
    fn audit_duplicate_rows_edit_and_delete_exactly_one_generation_fenced_index() {
        for same_path in [false, true] {
            let first = app("game.exe", Some(r"C:\Game\game.exe"), true);
            let second = app("game.exe", Some(if same_path { r"C:\Game\game.exe" } else { r"D:\Game\game.exe" }), true);
            let mut config = AppConfig::default();
            config.apps = vec![first.clone(), second.clone()];
            let row = AppRowIdentity::at(&config, 1).unwrap();
            toggle_app(&mut config, &row, false).unwrap();
            assert_eq!(config.apps[0], first);
            assert!(!config.apps[1].enabled);
            remove_app(&mut config, &row).unwrap();
            assert_eq!(config.apps, [first]);
            assert!(remove_app(&mut config, &row).is_err());
        }
    }

    #[test]
    fn audit_rejected_actions_preserve_exact_bytes_revision_generation_and_unrelated_rows() {
        let root = tempfile::tempdir().unwrap();
        let manager = ConfigManager::load(root.path().join("local"), root.path().join("legacy.json")).unwrap();
        let initial = manager.snapshot().unwrap();
        let initialized = manager.initialize(&initial.context_token).unwrap();
        let before = manager.mutate(&initialized.context_token, None, true, |settings| {
            settings.apps = vec![
                app("BsSndRpt.exe", Some(r"C:\Old\BsSndRpt.exe"), false),
                app("game.exe", Some(r"D:\Game\game.exe"), true),
                app("unrelated.exe", None, false),
            ];
            Ok(())
        }).unwrap();
        let bytes = std::fs::read(&before.config_path).unwrap();
        for operation in 0..5 {
            let result = manager.mutate(&before.context_token, Some(&before.library_generation), true, |settings| {
                let row = AppRowIdentity::at(settings, 0)?;
                let invalid = app("game.exe", Some(r"C:\Game\other.exe"), true);
                match operation {
                    0 => add_app(settings, invalid),
                    1 => import_games(settings, vec![app("new.exe", None, true), invalid]),
                    2 => repair_executable(settings, &row, "game.exe", r"D:\Game\game.exe"),
                    3 => repair_executable(settings, &row, "game.exe", r"D:\Game\other.exe"),
                    _ => toggle_app(settings, &row, true),
                }
            });
            assert!(result.is_err());
            assert_eq!(manager.snapshot().unwrap(), before);
            assert_eq!(std::fs::read(&before.config_path).unwrap(), bytes);
        }
        let row = AppRowIdentity::at(&before.settings, 1).unwrap();
        let after = manager.mutate(&before.context_token, Some(&before.library_generation), true,
            |settings| {
                let row = AppRowIdentity::at(settings, 0)?;
                remove_app(settings, &row)
            }).unwrap();
        let after_bytes = std::fs::read(&after.config_path).unwrap();
        assert!(manager.mutate(&before.context_token, Some(&before.library_generation), true,
            |settings| toggle_app(settings, &row, false)).is_err());
        assert_eq!(manager.snapshot().unwrap(), after);
        assert_eq!(std::fs::read(&after.config_path).unwrap(), after_bytes);
    }

    #[test]
    fn audit_add_import_ambiguous_owner_rejections_are_atomic() {
        let mut config = AppConfig::default();
        config.apps = vec![app("game.exe", Some(r"C:\Game\game.exe"), true),
            app("game.exe", Some(r"D:\Game\game.exe"), false)];
        let before = config.clone();
        assert!(add_app(&mut config, app("game.exe", None, true)).is_err());
        assert_eq!(config, before);
        assert!(import_games(&mut config, vec![app("new.exe", None, true), app("game.exe", None, true)]).is_err());
        assert_eq!(config, before);
        let mut chosen = config.apps[1].clone();
        chosen.enabled = true;
        add_app(&mut config, chosen.clone()).unwrap();
        assert_eq!(config.apps, [before.apps[0].clone(), chosen]);
    }
}

#[cfg(test)]
mod import_selection_tests {
    use super::*;
    use crate::config::{ConfigManager, HdrType};

    fn detection(launcher: &str, path: &str) -> HdrApp {
        HdrApp {
            name: "Shared game".into(), exe_name: "game.exe".into(), enabled: false,
            hdr_type: HdrType::Native, path: Some(path.into()),
            alternate_exes: vec![], steam_id: None, launcher: Some(launcher.into()),
        }
    }

    #[test]
    fn correction_import_conflicting_same_primary_batch_is_rejected_before_mutating() {
        let steam = detection("Steam", r"C:\Steam\game.exe");
        let gog = detection("GOG", r"D:\GOG\game.exe");
        for rows in [vec![steam.clone(), gog.clone()], vec![gog.clone(), steam.clone()]] {
            for existing in [vec![], vec![steam.clone()]] {
                let mut config = AppConfig::default();
                config.apps = existing;
                let before = config.clone();
                let error = import_games(&mut config, rows.clone()).expect_err("must not choose the last path");
                assert!(error.contains("game.exe"));
                assert_eq!(config, before);
            }
        }
    }

    #[test]
    fn correction_import_conflict_preserves_config_bytes_revision_and_generation() {
        let root = tempfile::tempdir().unwrap();
        let manager = ConfigManager::load(root.path().join("local"), root.path().join("legacy.json")).unwrap();
        let first = manager.snapshot().unwrap();
        let before = manager.initialize(&first.context_token).unwrap();
        let bytes = std::fs::read(&before.config_path).unwrap();
        let result = manager.mutate(&before.context_token, Some(&before.library_generation), true, |settings| {
            import_games(settings, vec![
                detection("Steam", r"C:\Steam\game.exe"), detection("GOG", r"D:\GOG\game.exe"),
            ])
        });
        assert!(result.is_err());
        assert_eq!(manager.snapshot().unwrap(), before);
        assert_eq!(std::fs::read(&before.config_path).unwrap(), bytes);
    }

    #[test]
    fn correction_import_one_selected_installation_keeps_explicit_existing_row_semantics() {
        let original = detection("Steam", r"C:\Steam\game.exe");
        let chosen = detection("GOG", r"D:\GOG\game.exe");
        let mut config = AppConfig::default();
        config.apps = vec![original];
        import_games(&mut config, vec![chosen.clone()]).unwrap();
        assert_eq!(config.apps.len(), 1);
        assert_eq!(config.apps[0].path, chosen.path);
        assert_eq!(config.apps[0].launcher, chosen.launcher);
        assert!(config.apps[0].enabled);
    }

    #[test]
    fn correction_import_provider_path_or_known_id_conflicts_fail_closed() {
        let first = detection("Steam", r"C:\Game\game.exe");
        let mut conflicting_id = first.clone();
        conflicting_id.steam_id = Some("200".into());
        let mut with_id = first.clone();
        with_id.steam_id = Some("100".into());
        for (left, right) in [
            (first.clone(), detection("GOG", r"C:\Game\game.exe")),
            (first.clone(), detection("Steam", r"D:\Game\game.exe")),
            (with_id, conflicting_id),
        ] {
            let mut config = AppConfig::default();
            assert!(import_games(&mut config, vec![left, right]).is_err());
            assert!(config.apps.is_empty());
        }
        let mut equivalent = first.clone();
        equivalent.path = Some(r"\\?\c:\GAME\game.exe".into());
        equivalent.exe_name = "GAME.EXE".into();
        equivalent.launcher = Some("STEAM".into());
        let mut config = AppConfig::default();
        import_games(&mut config, vec![first, equivalent]).unwrap();
        assert_eq!(config.apps.len(), 1);
    }

    #[test]
    fn correction_import_missing_id_cannot_bridge_two_conflicting_ids() {
        let unknown = detection("Steam", r"C:\Game\game.exe");
        let mut first = unknown.clone();
        first.steam_id = Some("100".into());
        let mut second = unknown.clone();
        second.steam_id = Some("200".into());
        for rows in [
            vec![unknown.clone(), first.clone(), second.clone()],
            vec![first, unknown, second],
        ] {
            let mut config = AppConfig::default();
            assert!(import_games(&mut config, rows).is_err());
            assert!(config.apps.is_empty());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HdrType;

    fn game(name: &str, exe: &str) -> HdrApp {
        HdrApp {
            name: name.into(),
            exe_name: exe.into(),
            enabled: true,
            hdr_type: HdrType::Native,
            path: None,
            alternate_exes: Vec::new(),
            steam_id: None,
            launcher: None,
        }
    }

    #[test]
    fn background_enrichment_cannot_add_enable_or_overwrite_user_choices() {
        let mut existing = game("My game", "game.exe");
        existing.enabled = false;
        existing.path = Some("D:\\MyChoice\\game.exe".into());
        existing.steam_id = Some("user-choice".into());
        existing.launcher = Some("Custom".into());
        let mut detected = game("My game", "game.exe");
        detected.path = Some("C:\\Other\\game.exe".into());
        detected.steam_id = Some("scanner-choice".into());
        detected.launcher = Some("Steam".into());
        detected.alternate_exes = vec!["game-dx12.exe".into()];
        let mut config = AppConfig::default();
        config.apps = vec![existing.clone()];
        enrich_existing(&mut config, &[detected, game("Removed", "removed.exe")]);
        assert_eq!(config.apps.len(), 1);
        let actual = &config.apps[0];
        assert!(!actual.enabled);
        assert_eq!(actual.path, existing.path);
        assert_eq!(actual.steam_id, existing.steam_id);
        assert_eq!(actual.launcher, existing.launcher);
        assert_eq!(actual.alternate_exes, vec!["game-dx12.exe"]);
    }

    #[test]
    fn explicit_import_preserves_primary_path_when_adding_an_alias() {
        let mut existing = game("My game", "game.exe");
        existing.enabled = false;
        existing.path = Some("D:\\Old\\game.exe".into());
        let mut detected = game("My game", "game-dx12.exe");
        detected.path = Some("E:\\Moved\\game-dx12.exe".into());
        let mut config = AppConfig::default();
        config.apps = vec![existing.clone()];
        import_games(&mut config, vec![detected.clone()]).unwrap();
        assert_eq!(config.apps.len(), 1);
        assert!(config.apps[0].enabled);
        assert_eq!(config.apps[0].path, existing.path);
        assert_eq!(config.apps[0].alternate_exes, vec!["game-dx12.exe"]);
    }

    #[test]
    fn add_does_not_erase_previously_enriched_metadata() {
        let mut existing = game("My game", "game.exe");
        existing.path = Some("D:\\Games\\game.exe".into());
        existing.steam_id = Some("123".into());
        let mut config = AppConfig::default();
        config.apps = vec![existing.clone()];
        add_app(&mut config, game("My game", "GAME.EXE")).unwrap();
        assert_eq!(config.apps.len(), 1);
        assert_eq!(config.apps[0].path, existing.path);
        assert_eq!(config.apps[0].steam_id, existing.steam_id);
    }

    #[test]
    fn explicit_add_and_import_update_path_only_for_the_same_healthy_primary() {
        for import in [false, true] {
            for primary in ["GAME.EXE", "alternate.exe"] {
                let mut existing = game("Game", "game.exe");
                existing.enabled = false;
                existing.path = Some(r"D:\Old\game.exe".into());
                let mut incoming = game("Game", primary);
                incoming.path = Some(format!(r"E:\New\{primary}"));
                let mut config = AppConfig::default();
                config.apps = vec![existing.clone()];
                if import {
                    import_games(&mut config, vec![incoming.clone()]).unwrap();
                } else {
                    add_app(&mut config, incoming.clone()).unwrap();
                }
                assert_eq!(config.apps[0].exe_name, "game.exe");
                assert_eq!(config.apps[0].path, if primary == "GAME.EXE" { incoming.path } else { existing.path });
                assert!(config.apps[0].enabled);
            }
        }
    }

    #[test]
    fn primary_and_alias_overlap_is_symmetric_for_every_library_entry_point() {
        let mut existing = game("User title", "main.exe");
        existing.enabled = false;
        existing.path = Some(r"D:\Chosen\main.exe".into());
        existing.alternate_exes = vec!["renderer.exe".into()];
        for (primary, aliases) in [
            ("MAIN.EXE", vec![]),
            ("RENDERER.EXE", vec![]),
            ("new.exe", vec!["MAIN.EXE"]),
            ("new.exe", vec!["RENDERER.EXE"]),
        ] {
            let mut incoming = game("Catalog title", primary);
            incoming.alternate_exes = aliases.into_iter().map(str::to_owned).collect();
            incoming.launcher = Some("Steam".into());
            assert!(same_game(&existing, &incoming));
            assert!(same_game(&incoming, &existing));
            for operation in 0..3 {
                let mut config = AppConfig::default();
                config.apps = vec![existing.clone()];
                match operation {
                    0 => add_app(&mut config, incoming.clone()).unwrap(),
                    1 => import_games(&mut config, vec![incoming.clone()]).unwrap(),
                    _ => { assert!(enrich_existing(&mut config, &[incoming.clone()])); }
                }
                assert_eq!(config.apps.len(), 1);
                assert_eq!(config.apps[0].path, existing.path);
                assert_eq!(config.apps[0].exe_name, existing.exe_name);
                assert_eq!(config.apps[0].enabled, operation != 2);
                assert_eq!(config.apps[0].launcher.as_deref(), Some("Steam"));
            }
        }
    }

    #[test]
    fn unrelated_titles_and_conflicting_game_ids_are_not_merged() {
        let left = game("Game", "game.exe");
        let right = game("Game Deluxe", "game-deluxe.exe");
        assert!(!same_game(&left, &right));
        assert!(!same_game(&right, &left));
        let mut left = left;
        let mut right = right;
        left.steam_id = Some("100".into());
        right.steam_id = Some("200".into());
        left.alternate_exes.push("launcher.exe".into());
        right.alternate_exes.push("launcher.exe".into());
        assert!(!same_game(&left, &right));
        assert!(!same_game(&right, &left));
    }

    #[test]
    fn same_title_with_conflicting_steam_ids_preserves_independent_rows() {
        let mut existing = game("Shared title", "a.exe");
        existing.steam_id = Some("100".into());
        existing.path = Some(r"D:\Original\a.exe".into());
        existing.enabled = false;
        let mut incoming = game("Shared title", "b.exe");
        incoming.steam_id = Some("200".into());
        incoming.path = Some(r"E:\Different\b.exe".into());
        for import in [false, true] {
            let mut config = AppConfig::default();
            config.apps = vec![existing.clone()];
            if import {
                import_games(&mut config, vec![incoming.clone()]).unwrap();
            } else {
                add_app(&mut config, incoming.clone()).unwrap();
            }
            assert_eq!(config.apps, vec![existing.clone(), incoming.clone()]);
            assert!(!enrich_existing(&mut config, &[incoming.clone()]));
            assert_eq!(config.apps[0], existing);
        }
    }

    #[test]
    fn adding_a_catalog_alias_enables_but_preserves_the_canonical_primary_and_path() {
        let mut existing = game("Fixture game", "renderer.exe");
        existing.steam_id = Some("100".into());
        existing.path = Some(r"D:\Original\renderer.exe".into());
        existing.enabled = false;
        let mut incoming = game("Fixture game", "game.exe");
        incoming.steam_id = Some("100".into());
        let mut config = AppConfig::default();
        config.apps = vec![existing.clone()];
        add_app(&mut config, incoming).unwrap();
        assert_eq!(config.apps.len(), 1);
        assert_eq!(config.apps[0].exe_name, existing.exe_name);
        assert_eq!(config.apps[0].path, existing.path);
        assert_eq!(config.apps[0].steam_id, existing.steam_id);
        assert_eq!(config.apps[0].alternate_exes, vec!["game.exe"]);
        assert!(config.apps[0].enabled);
    }

    #[test]
    fn enrichment_reports_no_change_for_unknown_games_and_existing_metadata() {
        let existing = game("Known game", "known.exe");
        let mut config = AppConfig::default();
        config.apps = vec![existing.clone()];
        assert!(!enrich_existing(&mut config, &[game("Unknown", "unknown.exe")]));
        assert!(!enrich_existing(&mut config, &[existing]));
        let mut detected = game("Known game", "alternate.exe");
        detected.launcher = Some("Steam".into());
        assert!(enrich_existing(&mut config, &[detected.clone()]));
        assert!(!enrich_existing(&mut config, &[detected]));
    }

    fn verified_game(canonical: &str, selected: &[&str], provider: Provider) -> ResolvedGame {
        use crate::automatic_authority::{self, Authority, InstallEvidence};
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        for exe in selected {
            std::fs::write(root.path().join(exe), b"fixture, never executed").unwrap();
        }
        let storefronts = match provider {
            Provider::Steam => serde_json::json!([{
                "provider": "steam", "product_id": "123", "game_executables": selected
            }]),
            Provider::Xbox => serde_json::json!([{
                "provider": "xbox", "game_executables": selected
            }]),
            _ => serde_json::json!([]),
        };
        let catalog = serde_json::from_value(serde_json::json!({
            "name": "Catalog title", "exe_name": canonical, "steam_id": "123",
            "hdr_type": "native", "support_tier": "native",
            "alternate_exes": ["historical.exe"], "storefronts": storefronts
        })).unwrap();
        let declarations = selected.iter().map(|exe| exe.to_string()).collect::<Vec<_>>();
        let observed = InstallEvidence::observe(
            provider, (provider == Provider::Steam).then_some("123"), root.path(), &declarations,
        ).unwrap();
        match automatic_authority::resolve(&crate::database::authored_test_catalog(vec![catalog]), Some(&observed)) {
            Authority::Resolved(game) => game,
            other => panic!("Expected resolved fixture, got {other:?}"),
        }
    }

    #[test]
    fn verified_enrichment_preserves_disabled_custom_metadata_and_is_idempotent() {
        let incoming = verified_game("game.exe", &["game.exe", "new.exe"], Provider::Steam);
        let mut existing = game("User title", "game.exe");
        existing.enabled = false;
        existing.hdr_type = HdrType::Custom;
        existing.path = incoming.as_app(false).path;
        existing.alternate_exes = vec!["user.exe".into()];
        let mut config = AppConfig::default();
        config.apps = vec![existing.clone()];
        assert!(enrich_verified_aliases(&mut config, &[incoming.clone()]));
        existing.alternate_exes.push("new.exe".into());
        assert_eq!(config.apps, [existing.clone()]);
        assert!(!enrich_verified_aliases(&mut config, &[incoming.clone()]));
        assert!(enrich_verified_metadata(&mut config, &[incoming.clone()]));
        existing.steam_id = Some("123".into());
        existing.launcher = Some("Steam".into());
        assert_eq!(config.apps, [existing.clone()]);
        assert!(!enrich_verified_metadata(&mut config, &[incoming]));
        assert_eq!(config.apps, [existing]);
    }

    #[test]
    fn verified_association_requires_selected_primary_or_canonical_not_alias_title_or_id() {
        let incoming = verified_game("canonical.exe", &["selected.exe", "zalias.exe"], Provider::Steam);
        for exe in ["canonical.exe", "SELECTED.EXE"] {
            let mut config = AppConfig::default();
            config.apps = vec![game("Independent user title", exe)];
            assert!(enrich_verified_aliases(&mut config, &[incoming.clone()]), "{exe}");
            assert_eq!(config.apps[0].exe_name, exe);
            assert!(config.apps[0].alternate_exes.iter().any(|exe| exe == "zalias.exe"));
            assert!(!config.apps[0].alternate_exes.iter().any(|exe| exe == "historical.exe"));
        }
        for exe in ["other.exe", "historical.exe", "zalias.exe"] {
            let mut existing = game("Catalog title", exe);
            existing.steam_id = Some("123".into());
            existing.alternate_exes = vec!["canonical.exe".into(), "selected.exe".into()];
            let mut config = AppConfig::default();
            config.apps = vec![existing.clone()];
            assert!(!enrich_verified_aliases(&mut config, &[incoming.clone()]), "{exe}");
            assert!(!enrich_verified_metadata(&mut config, &[incoming.clone()]), "{exe}");
            assert_eq!(config.apps, [existing]);
        }
    }

    #[test]
    fn verified_association_is_unique_in_both_directions_independent_of_order() {
        let steam = verified_game("canonical.exe", &["steam.exe"], Provider::Steam);
        let xbox = verified_game("canonical.exe", &["xbox.exe"], Provider::Xbox);
        for candidates in [vec![steam.clone(), xbox.clone()], vec![xbox, steam.clone()]] {
            let existing = game("User title", "canonical.exe");
            let mut config = AppConfig::default();
            config.apps = vec![existing.clone()];
            assert!(!enrich_verified_aliases(&mut config, &candidates));
            assert!(!enrich_verified_metadata(&mut config, &candidates));
            assert_eq!(config.apps, [existing]);
        }
        let canonical = game("Canonical user title", "canonical.exe");
        let selected = game("Selected user title", "steam.exe");
        for existing in [vec![canonical.clone(), selected.clone()], vec![selected, canonical]] {
            let mut config = AppConfig::default();
            config.apps = existing.clone();
            assert!(!enrich_verified_aliases(&mut config, &[steam.clone()]));
            assert!(!enrich_verified_metadata(&mut config, &[steam.clone()]));
            assert_eq!(config.apps, existing);
        }
    }

    #[test]
    fn provider_conflicts_known_ids_and_different_installations_do_not_enrich() {
        let steam = verified_game("canonical.exe", &["steam.exe"], Provider::Steam);
        let xbox = verified_game("canonical.exe", &["xbox.exe"], Provider::Xbox);
        for (candidate, exe, id, launcher, path) in [
            (&steam, "canonical.exe", Some("999"), None, None),
            (&steam, "steam.exe", Some("999"), None, None),
            (&steam, "canonical.exe", None, Some("Xbox"), None),
            (&steam, "steam.exe", None, Some("Custom"), None),
            (&steam, "steam.exe", None, None, Some(r"D:\Other\steam.exe")),
            (&steam, "canonical.exe", None, None, Some(r"D:\Other\canonical.exe")),
            (&xbox, "canonical.exe", Some("123"), None, None),
            (&xbox, "canonical.exe", None, Some("GOG"), None),
        ] {
            let mut existing = game("Catalog title", exe);
            existing.steam_id = id.map(str::to_owned);
            existing.launcher = launcher.map(str::to_owned);
            existing.path = path.map(str::to_owned);
            let mut config = AppConfig::default();
            config.apps = vec![existing.clone()];
            assert!(!enrich_verified_aliases(&mut config, std::slice::from_ref(candidate)));
            assert!(!enrich_verified_metadata(&mut config, std::slice::from_ref(candidate)));
            assert_eq!(config.apps, [existing]);
        }
    }

    #[test]
    fn exact_primary_path_disambiguates_separate_installations_without_order_dependence() {
        let first = verified_game("game.exe", &["game.exe", "renderer.exe"], Provider::Steam);
        let second = verified_game("game.exe", &["game.exe", "renderer.exe"], Provider::Steam);
        let mut first_row = game("First installation", "game.exe");
        first_row.path = first.as_app(false).path;
        let mut second_row = game("Second installation", "game.exe");
        second_row.path = second.as_app(false).path;
        for mut rows in [
            vec![first_row.clone(), second_row.clone()],
            vec![second_row, first_row],
        ] {
            let mut config = AppConfig::default();
            config.apps = rows.clone();
            assert!(enrich_verified_aliases(&mut config, &[second.clone(), first.clone()]));
            for row in &mut rows {
                row.alternate_exes.push("renderer.exe".into());
            }
            assert_eq!(config.apps, rows);
            assert!(!enrich_verified_aliases(&mut config, &[first.clone(), second.clone()]));
        }
    }

    #[test]
    fn canonical_unscoped_row_gains_only_selected_provider_aliases_without_metadata_changes() {
        let xbox = verified_game("canonical.exe", &["xbox.exe"], Provider::Xbox);
        let mut existing = game("My chosen title", "canonical.exe");
        existing.hdr_type = HdrType::Custom;
        existing.enabled = false;
        existing.alternate_exes = vec!["user.exe".into()];
        let mut config = AppConfig::default();
        config.apps = vec![existing.clone()];
        assert!(enrich_verified_aliases(&mut config, &[xbox.clone()]));
        existing.alternate_exes.push("xbox.exe".into());
        assert_eq!(config.apps, [existing.clone()]);
        assert!(!enrich_verified_aliases(&mut config, &[xbox.clone()]));
        assert!(!enrich_verified_metadata(&mut config, &[xbox]));
        assert_eq!(config.apps, [existing]);
    }

    #[test]
    fn review_legacy_helper_aliases_are_removed_only_by_confirmed_safe_updates() {
        let mut original = game("Customized title", "game.exe");
        original.enabled = false;
        original.hdr_type = HdrType::Custom;
        original.path = Some(r"C:\Old\game.exe".into());
        original.launcher = Some("Steam".into());
        original.steam_id = Some("123".into());
        original.alternate_exes = vec!["BsSndRpt64.exe".into(), "user.exe".into()];
        let mut config = AppConfig::default();
        config.apps = vec![original.clone()];
        assert!(!is_quarantined(&original));
        assert_eq!(config.resolve_app(None, "BsSndRpt64.exe"), crate::runtime_policy::Resolution::Excluded);
        let verified = verified_game("game.exe", &["game.exe", "renderer.exe"], Provider::Steam);
        // The selected installation differs: even discovery may not repair or rewrite it.
        assert!(!enrich_verified_aliases(&mut config, &[verified.clone()]));
        assert!(!enrich_verified_metadata(&mut config, &[verified]));
        assert_eq!(config.apps, [original.clone()]);
        for import in [false, true] {
            config.apps = vec![original.clone()];
            let mut update = original.clone();
            update.alternate_exes = vec!["new-safe.exe".into()];
            update.path = Some(r"D:\Moved\game.exe".into());
            if import { import_games(&mut config, vec![update.clone()]).unwrap(); }
            else { add_app(&mut config, update.clone()).unwrap(); }
            let mut expected = original.clone();
            expected.path = update.path;
            expected.enabled = import;
            expected.alternate_exes = vec!["user.exe".into(), "new-safe.exe".into()];
            assert_eq!(config.apps, [expected]);
            assert_eq!(config.resolve_app(None, "BsSndRpt64.exe"), crate::runtime_policy::Resolution::Excluded);
        }
    }

    #[test]
    fn review_excluded_alias_cleanup_never_accepts_incoming_helpers_or_partially_commits() {
        use crate::config::ConfigManager;
        let root = tempfile::tempdir().unwrap();
        let manager = ConfigManager::load(root.path().join("local"), root.path().join("legacy.json")).unwrap();
        let first = manager.snapshot().unwrap();
        let initialized = manager.initialize(&first.context_token).unwrap();
        let original = manager.mutate(&initialized.context_token, None, true, |settings| {
            let mut legacy = game("Customized", "game.exe");
            legacy.enabled = false;
            legacy.hdr_type = HdrType::Custom;
            legacy.alternate_exes = vec!["BsSndRpt64.exe".into(), "user.exe".into()];
            settings.apps = vec![legacy, game("Other", "other.exe")];
            Ok(())
        }).unwrap();
        let bytes = std::fs::read(&original.config_path).unwrap();
        let safe = game("Customized", "game.exe");
        for import in [false, true] {
            let mut unsafe_item = safe.clone();
            unsafe_item.alternate_exes = vec!["BUGSPLAT.EXE".into()];
            let result = manager.mutate(&original.context_token, Some(&original.library_generation), true, |settings| {
                if import { import_games(settings, vec![safe.clone(), unsafe_item]) }
                else { add_app(settings, unsafe_item) }
            });
            assert!(result.unwrap_err().contains("helper"));
            assert_eq!(manager.snapshot().unwrap(), original);
            assert_eq!(std::fs::read(&original.config_path).unwrap(), bytes);
        }
        let mut ambiguous = game("Other", "different.exe");
        ambiguous.alternate_exes = vec!["user.exe".into()];
        assert!(manager.mutate(&original.context_token, Some(&original.library_generation), true,
            |settings| import_games(settings, vec![safe.clone(), ambiguous])).is_err());
        assert_eq!(manager.snapshot().unwrap(), original, "a later row conflict cannot commit earlier helper cleanup");
        assert_eq!(std::fs::read(&original.config_path).unwrap(), bytes);
        let committed = manager.mutate(&original.context_token, Some(&original.library_generation), true,
            |settings| import_games(settings, vec![safe])).unwrap();
        assert_eq!(committed.settings.apps[0].alternate_exes, ["user.exe"]);
        assert_eq!(committed.settings.apps[0].hdr_type, HdrType::Custom);
        assert_eq!(committed.settings.apps[1], original.settings.apps[1]);
        assert_eq!(committed.revision.parse::<u64>().unwrap(), original.revision.parse::<u64>().unwrap() + 1);
    }

    #[test]
    fn review_background_alias_enrichment_keeps_legacy_helpers_inert_and_disabled() {
        let resolved = verified_game("game.exe", &["game.exe", "renderer.exe"], Provider::Steam);
        let mut original = game("Customized", "game.exe");
        original.enabled = false;
        original.hdr_type = HdrType::Custom;
        original.alternate_exes = vec!["BsSndRpt64.exe".into(), "user.exe".into()];
        let mut config = AppConfig::default();
        config.apps = vec![original.clone()];
        assert!(enrich_verified_aliases(&mut config, &[resolved]));
        original.alternate_exes.push("renderer.exe".into());
        assert_eq!(config.apps, [original]);
        assert_eq!(config.resolve_app(None, "BsSndRpt64.exe"), crate::runtime_policy::Resolution::Excluded);
    }

    #[test]
    fn alias_enrichment_preserves_exact_customized_storefront_metadata() {
        for (provider, id, launcher) in [
            (Provider::Steam, Some("123"), "sTeAm"),
            (Provider::Xbox, None, "xBoX"),
        ] {
            let incoming = verified_game("game.exe", &["game.exe", "renderer.exe"], provider);
            let mut existing = game("Custom title", "game.exe");
            existing.enabled = false;
            existing.hdr_type = HdrType::Custom;
            existing.steam_id = id.map(str::to_owned);
            existing.launcher = Some(launcher.into());
            let mut config = AppConfig::default();
            config.apps = vec![existing.clone()];
            assert!(enrich_verified_aliases(&mut config, &[incoming.clone()]));
            existing.alternate_exes.push("renderer.exe".into());
            assert_eq!(config.apps, [existing.clone()]);
            assert!(!enrich_verified_metadata(&mut config, &[incoming.clone()]));
            assert!(!enrich_verified_aliases(&mut config, &[incoming]));
            assert_eq!(config.apps, [existing]);
        }
    }

    #[test]
    fn exact_primary_missing_metadata_policy_is_separate_from_alias_enrichment() {
        let incoming = verified_game("canonical.exe", &["game.exe", "renderer.exe"], Provider::Steam);
        let mut existing = game("User title", "game.exe");
        existing.enabled = false;
        existing.alternate_exes = vec!["user.exe".into()];
        let mut config = AppConfig::default();
        config.apps = vec![existing.clone()];
        assert!(enrich_verified_metadata(&mut config, &[incoming.clone()]));
        existing.steam_id = Some("123".into());
        existing.launcher = Some("Steam".into());
        assert_eq!(config.apps, [existing.clone()]);
        assert!(!enrich_verified_metadata(&mut config, &[incoming.clone()]));
        assert!(enrich_verified_aliases(&mut config, &[incoming.clone()]));
        existing.alternate_exes.push("renderer.exe".into());
        assert_eq!(config.apps, [existing.clone()]);
        assert!(!enrich_verified_aliases(&mut config, &[incoming]));
        assert_eq!(config.apps, [existing]);
    }

    #[test]
    fn automatic_metadata_is_gated_and_duplicate_saved_primaries_remain_untouched() {
        let existing = game("Game", "game.exe");
        let incoming = verified_game("game.exe", &["game.exe", "renderer.exe"], Provider::Steam);
        let mut config = AppConfig::default();
        config.apps = vec![existing.clone(), existing.clone()];
        assert!(!enrich_verified_aliases(&mut config, &[incoming.clone()]));
        assert!(!enrich_verified_metadata(&mut config, &[incoming.clone()]));
        assert_eq!(config.apps, [existing.clone(), existing.clone()]);
        config.apps.pop();
        config.auto_detect_new_games = false;
        assert!(!enrich_verified_aliases(&mut config, &[incoming.clone()]));
        assert!(!enrich_verified_metadata(&mut config, &[incoming]));
        assert_eq!(config.apps, [existing]);
    }

    #[test]
    fn excluded_primary_or_supplied_alias_cannot_be_added_or_imported() {
        for exe in ["GameLaunchHelper.exe", "BsSndRpt.exe", "BsSndRpt64.exe", "BugSplat.exe"] {
            for alias in [false, true] {
                let mut incoming = game("Game", if alias { "game.exe" } else { exe });
                if alias {
                    incoming.alternate_exes.push(exe.into());
                }
                for import in [false, true] {
                    let mut config = AppConfig::default();
                    config.apps.clear();
                    let result = if import {
                        import_games(&mut config, vec![incoming.clone()])
                    } else {
                        add_app(&mut config, incoming.clone())
                    };
                    assert!(result.unwrap_err().contains("helper"), "{exe}");
                    assert!(config.apps.is_empty());
                }
            }
        }
    }

    #[test]
    fn explicit_actions_repair_quarantined_primary_and_discard_historical_aliases() {
        for import in [false, true] {
            for new_path in [None, Some(r"E:\XboxGames\Game\Content\game.exe".to_string())] {
                let mut existing = game("Game", "BsSndRpt.exe");
                existing.enabled = false;
                existing.hdr_type = HdrType::Custom;
                existing.path = Some(r"D:\Old\BsSndRpt.exe".into());
                existing.alternate_exes = vec!["gamelaunchhelper.exe".into(), "suspect.exe".into()];
                let mut incoming = game("Game", "game.exe");
                incoming.path = new_path.clone();
                incoming.alternate_exes = vec!["safe.exe".into()];
                incoming.launcher = Some("Xbox".into());
                let mut config = AppConfig::default();
                config.apps = vec![existing.clone()];
                if import {
                    import_games(&mut config, vec![incoming]).unwrap();
                } else {
                    add_app(&mut config, incoming).unwrap();
                }
                assert_eq!(config.apps.len(), 1);
                let repaired = &config.apps[0];
                assert_eq!(repaired.exe_name, "game.exe");
                assert_eq!(repaired.path, new_path);
                assert_eq!(repaired.alternate_exes, ["safe.exe"]);
                assert_eq!(repaired.launcher.as_deref(), Some("Xbox"));
                assert!(repaired.enabled);
                assert_eq!(repaired.hdr_type, if import { HdrType::Custom } else { HdrType::Native });
                assert!(!is_quarantined(repaired));
            }
        }
    }

    #[test]
    fn targeted_repair_preserves_custom_title_and_all_nonexecutable_metadata() {
        for enabled in [false, true] {
            let mut existing = game("My custom-renamed favorite", "BsSndRpt.exe");
            existing.enabled = enabled;
            existing.hdr_type = HdrType::Custom;
            existing.path = Some(r"D:\Old\BsSndRpt.exe".into());
            existing.steam_id = Some("1466860".into());
            existing.launcher = Some("User chosen launcher".into());
            existing.alternate_exes = vec!["gamelaunchhelper.exe".into(), "suspect.exe".into()];
            let other = game("Unrelated app", "other.exe");
            let mut config = AppConfig::default();
            config.apps = vec![other.clone(), existing.clone()];
            let row = AppRowIdentity::at(&config, 1).unwrap();
            repair_executable(&mut config, &row, "Game.EXE", r"E:\Chosen\Game.EXE").unwrap();
            existing.exe_name = "game.exe".into();
            existing.path = Some(r"E:\Chosen\Game.EXE".into());
            existing.alternate_exes.clear();
            assert_eq!(config.apps, [other, existing]);
            let repaired = config.apps.clone();
            assert!(repair_executable(&mut config, &row, "new.exe", r"E:\Chosen\new.exe").is_err());
            let row = AppRowIdentity::at(&config, 1).unwrap();
            assert!(repair_executable(&mut config, &row, "new.exe", r"E:\Chosen\new.exe").unwrap_err().contains("no longer quarantined"));
            assert_eq!(config.apps, repaired);
        }
    }

    #[test]
    fn targeted_repair_requires_current_index_primary_and_path_not_title_or_alias() {
        let helper = game("Game", "BsSndRpt.exe");
        let mut alias_only = game("BsSndRpt.exe", "healthy.exe");
        alias_only.alternate_exes.push("bssndrpt.exe".into());
        let mut duplicate = helper.clone();
        duplicate.exe_name = "BSSNDRPT.EXE".into();
        for rows in [vec![], vec![alias_only]] {
            let mut config = AppConfig::default();
            config.apps = rows.clone();
            let row = AppRowIdentity { index: 0, exe_name: "bssndrpt.exe".into(), path: None };
            assert!(repair_executable(&mut config, &row, "game.exe", r"D:\Game\game.exe").is_err());
            assert_eq!(config.apps, rows);
        }
        let mut config = AppConfig::default();
        config.apps = vec![helper.clone(), duplicate];
        let row = AppRowIdentity::at(&config, 1).unwrap();
        repair_executable(&mut config, &row, "game.exe", r"D:\Game\game.exe").unwrap();
        assert_eq!(config.apps[0], helper);
        assert_eq!(config.apps[1].exe_name, "game.exe");
    }

    #[test]
    fn targeted_repair_rejects_helpers_and_invalid_selected_bindings_without_mutation() {
        for selected in [
            "GameLaunchHelper.exe", "BsSndRpt.exe", "BsSndRpt64.exe", "BugSplat.exe",
            "BugSplatHD64.exe", "game.txt", r"bin\game.exe", "",
        ] {
            let original = game("Preserved title", "bssndrpt.exe");
            let mut config = AppConfig::default();
            config.apps = vec![original.clone()];
            let row = AppRowIdentity::at(&config, 0).unwrap();
            assert!(repair_executable(&mut config, &row, selected, &format!(r"D:\Game\{selected}")).is_err());
            assert_eq!(config.apps, [original]);
        }
        for path in ["", "game.exe", r"D:\Game\other.exe", r"D:\Game\BsSndRpt.exe"] {
            let original = game("Preserved title", "bssndrpt.exe");
            let mut config = AppConfig::default();
            config.apps = vec![original.clone()];
            let row = AppRowIdentity::at(&config, 0).unwrap();
            assert!(repair_executable(&mut config, &row, "game.exe", path).is_err());
            assert_eq!(config.apps, [original]);
        }
    }

    #[test]
    fn quarantine_is_never_implicitly_repaired_by_verified_enrichment() {
        let incoming = verified_game("BsSndRpt.exe", &["game.exe"], Provider::Xbox);
        let mut existing = game("Catalog title", "BsSndRpt.exe");
        existing.alternate_exes.push("game.exe".into());
        let mut config = AppConfig::default();
        config.apps = vec![existing.clone()];
        assert!(!enrich_verified_aliases(&mut config, &[incoming.clone()]));
        assert!(!enrich_verified_metadata(&mut config, &[incoming]));
        assert_eq!(config.apps, [existing]);
    }

    #[test]
    fn automatic_enrollment_id_is_only_a_veto_and_primary_paths_scope_exact_vetoes() {
        let mut existing = game("User title", "unrelated.exe");
        existing.steam_id = Some("123".into());
        let incoming = verified_game("game.exe", &["game.exe"], Provider::Steam);
        assert!(automatic_enrollment_veto(&[existing.clone()], &incoming.as_app(true)));
        let mut config = AppConfig::default();
        config.apps = vec![existing.clone()];
        assert!(!enrich_verified_aliases(&mut config, &[incoming.clone()]));
        assert!(!enrich_verified_metadata(&mut config, &[incoming]));
        assert_eq!(config.apps, [existing]);

        let mut existing = game("Title", "game.exe");
        existing.path = Some(r"D:\InstallA\game.exe".into());
        let mut candidate = existing.clone();
        candidate.path = Some(r"E:\InstallB\game.exe".into());
        assert!(!automatic_enrollment_veto(&[existing.clone()], &candidate));
        candidate.path = Some(r"\\?\D:\INSTALLA\GAME.EXE".into());
        assert!(automatic_enrollment_veto(&[existing.clone()], &candidate));
        candidate.path = Some("D:/InstallA/sub/../game.exe".into());
        assert!(automatic_enrollment_veto(&[existing.clone()], &candidate));
        candidate.path = None;
        assert!(automatic_enrollment_veto(&[existing.clone()], &candidate));
        candidate.exe_name = "other.exe".into();
        candidate.name = existing.name.clone();
        assert!(!automatic_enrollment_veto(&[existing.clone()], &candidate));
        existing.alternate_exes.push("other.exe".into());
        assert!(automatic_enrollment_veto(&[existing.clone()], &candidate));
        existing.exe_name = "BsSndRpt.exe".into();
        assert!(automatic_enrollment_veto(&[existing], &candidate));
    }
}
