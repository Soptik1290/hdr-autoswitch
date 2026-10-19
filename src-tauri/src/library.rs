use crate::config::{AppConfig, HdrApp};
use crate::automatic_authority::{Provider, ResolvedGame};
use crate::runtime_policy::{is_quarantined, normalize_windows_path, permanently_excluded};

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
        std::iter::once(&app.exe_name).chain(&app.alternate_exes).any(|exe| {
            !permanently_excluded(exe)
                && std::iter::once(&candidate.exe_name).chain(&candidate.alternate_exes).any(|other| {
                    if !exe.eq_ignore_ascii_case(other) || permanently_excluded(other) {
                        return false;
                    }
                    if exe.eq_ignore_ascii_case(&app.exe_name)
                        && other.eq_ignore_ascii_case(&candidate.exe_name)
                    {
                        return !matches!(
                            (app.path.as_deref().and_then(normalize_windows_path),
                             candidate.path.as_deref().and_then(normalize_windows_path)),
                            (Some(left), Some(right)) if left != right
                        );
                    }
                    true
                })
        })
    })
}

fn same_game(existing: &HdrApp, item: &HdrApp) -> bool {
    let conflicting_ids = matches!(
        (&existing.steam_id, &item.steam_id),
        (Some(left), Some(right)) if left != right
    );
    let same_steam_id = matches!(
        (&existing.steam_id, &item.steam_id),
        (Some(left), Some(right)) if !left.trim().is_empty() && left == right
    );
    let different_paths = matches!(
        (&existing.path, &item.path),
        (Some(left), Some(right)) if !same_path(left, right)
    );
    let same_name = existing.name.trim().eq_ignore_ascii_case(item.name.trim());
    if !same_name && (conflicting_ids || (different_paths && !same_steam_id)) {
        return false;
    }
    if existing.exe_name.eq_ignore_ascii_case(&item.exe_name) {
        return true;
    }
    if conflicting_ids {
        return false;
    }
    if same_name {
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
    let mut primaries = Vec::<(&HdrApp, Option<&str>)>::new();
    for item in &detected {
        validate_app(item)?;
        if let Some((previous, known_id)) = primaries.iter_mut().find(|(previous, _)| {
            if !previous.exe_name.eq_ignore_ascii_case(&item.exe_name) {
                return false;
            }
            let same_installation = match (&previous.path, &item.path) {
                (Some(left), Some(right)) => same_path(left, right),
                (None, None) => true,
                _ => false,
            };
            same_installation
                || previous.path.is_none()
                || item.path.is_none()
                || same_game(previous, item)
        }) {
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
            primaries.push((item, item.steam_id.as_deref()));
        }
    }
    for mut item in detected {
        item.enabled = true;
        if let Some(existing) = config.apps.iter_mut().find(|app| same_game(app, &item)) {
            apply_explicit_executables(existing, &item);
            if item.launcher.is_some() {
                existing.launcher = item.launcher;
            }
            if item.steam_id.is_some() {
                existing.steam_id = item.steam_id;
            }
            existing.enabled = true;
        } else {
            config.apps.push(item);
        }
    }
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
    Ok(())
}

pub fn add_app(config: &mut AppConfig, mut app: HdrApp) -> Result<(), String> {
    validate_app(&app)?;
    app.exe_name = app.exe_name.trim().to_lowercase();
    if let Some(existing) = config
        .apps
        .iter_mut()
        .find(|entry| same_game(entry, &app))
    {
        apply_explicit_executables(existing, &app);
        existing.name = app.name;
        existing.enabled = app.enabled;
        existing.hdr_type = app.hdr_type;
        if app.steam_id.is_some() {
            existing.steam_id = app.steam_id;
        }
        if app.launcher.is_some() {
            existing.launcher = app.launcher;
        }
    } else {
        config.apps.push(app);
    }
    Ok(())
}

/// Explicitly repair one quarantined row, without title matching or changing user policy.
pub fn repair_executable(
    config: &mut AppConfig,
    exe_name: &str,
    selected_exe: &str,
    selected_path: &str,
) -> Result<(), String> {
    let mut matches = config.apps.iter().enumerate()
        .filter_map(|(index, app)| app.exe_name.eq_ignore_ascii_case(exe_name).then_some(index));
    let index = matches.next().ok_or("No saved app has that executable. Refresh the library and try again.")?;
    if matches.next().is_some() {
        return Err("More than one saved app has that executable. Resolve duplicate rows before repairing.".into());
    }
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
    let existing = &mut config.apps[index];
    existing.exe_name = selected_exe;
    existing.path = Some(selected_path.to_owned());
    existing.alternate_exes.clear();
    Ok(())
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
            repair_executable(&mut config, "BSSNDRPT.EXE", "Game.EXE", r"E:\Chosen\Game.EXE").unwrap();
            existing.exe_name = "game.exe".into();
            existing.path = Some(r"E:\Chosen\Game.EXE".into());
            existing.alternate_exes.clear();
            assert_eq!(config.apps, [other, existing]);
            let repaired = config.apps.clone();
            assert!(repair_executable(&mut config, "bssndrpt.exe", "new.exe", r"E:\Chosen\new.exe").is_err());
            assert!(repair_executable(&mut config, "game.exe", "new.exe", r"E:\Chosen\new.exe").unwrap_err().contains("no longer quarantined"));
            assert_eq!(config.apps, repaired);
        }
    }

    #[test]
    fn targeted_repair_requires_unique_current_primary_not_title_or_alias() {
        let helper = game("Game", "BsSndRpt.exe");
        let mut alias_only = game("BsSndRpt.exe", "healthy.exe");
        alias_only.alternate_exes.push("bssndrpt.exe".into());
        let mut duplicate = helper.clone();
        duplicate.exe_name = "BSSNDRPT.EXE".into();
        for rows in [vec![], vec![alias_only], vec![helper, duplicate]] {
            let mut config = AppConfig::default();
            config.apps = rows.clone();
            assert!(repair_executable(&mut config, "bssndrpt.exe", "game.exe", r"D:\Game\game.exe").is_err());
            assert_eq!(config.apps, rows);
        }
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
            assert!(repair_executable(&mut config, "bssndrpt.exe", selected, &format!(r"D:\Game\{selected}")).is_err());
            assert_eq!(config.apps, [original]);
        }
        for path in ["", "game.exe", r"D:\Game\other.exe", r"D:\Game\BsSndRpt.exe"] {
            let original = game("Preserved title", "bssndrpt.exe");
            let mut config = AppConfig::default();
            config.apps = vec![original.clone()];
            assert!(repair_executable(&mut config, "bssndrpt.exe", "game.exe", path).is_err());
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

    #[test]
    fn distinct_games_sharing_executable_name_can_be_imported_and_stored_separately() {
        let mut hl2 = game("Half-Life 2", "hl2.exe");
        hl2.path = Some(r"C:\Steam\steamapps\common\Half-Life 2\hl2.exe".into());
        hl2.steam_id = Some("220".into());
        hl2.launcher = Some("Steam".into());

        let mut portal = game("Portal", "hl2.exe");
        portal.path = Some(r"C:\Steam\steamapps\common\Portal\hl2.exe".into());
        portal.steam_id = Some("400".into());
        portal.launcher = Some("Steam".into());

        let mut soc = game("S.T.A.L.K.E.R.: Shadow of Chernobyl", "xrengine.exe");
        soc.path = Some(r"D:\Games\STALKER Shadow of Chernobyl\bin\xrEngine.exe".into());
        let mut cs = game("S.T.A.L.K.E.R.: Clear Sky", "xrengine.exe");
        cs.path = Some(r"D:\Games\STALKER Clear Sky\bin\xrEngine.exe".into());
        let mut cop = game("S.T.A.L.K.E.R.: Call of Pripyat", "xrengine.exe");
        cop.path = Some(r"D:\Games\STALKER Call of Pripyat\bin\xrEngine.exe".into());

        let mut config = AppConfig::default();
        import_games(
            &mut config,
            vec![hl2.clone(), portal.clone(), soc.clone(), cs.clone(), cop.clone()],
        )
        .unwrap();
        assert_eq!(config.apps.len(), 5);
        assert_eq!(config.apps[0].name, "Half-Life 2");
        assert_eq!(config.apps[0].path, hl2.path);
        assert_eq!(config.apps[1].name, "Portal");
        assert_eq!(config.apps[1].path, portal.path);

        // Moving Half-Life 2 updates only Half-Life 2, leaving Portal untouched
        let mut moved_hl2 = hl2.clone();
        moved_hl2.path = Some(r"E:\SteamLibrary\steamapps\common\Half-Life 2\hl2.exe".into());
        import_games(&mut config, vec![moved_hl2.clone()]).unwrap();
        assert_eq!(config.apps.len(), 5);
        assert_eq!(config.apps[0].path, moved_hl2.path);
        assert_eq!(config.apps[1].path, portal.path);
    }
}

