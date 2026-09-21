//! Runtime authorization is exact and independent of catalog suggestions or row order.
use crate::config::{AppConfig, HdrApp};

pub fn permanently_excluded(exe: &str) -> bool {
    matches!(
        exe.trim().to_ascii_lowercase().as_str(),
        "gamelaunchhelper.exe"
            | "bssndrpt.exe"
            | "bssndrpt64.exe"
            | "bugsplat.exe"
            | "bugsplathd64.exe"
    )
}

pub fn is_quarantined(app: &HdrApp) -> bool {
    repair_reason(app).is_some()
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RepairReason {
    Helper,
    PrimaryPathMismatch,
}

pub fn primary_path_consistent(app: &HdrApp) -> bool {
    match app.path.as_deref() {
        None | Some("") => true,
        Some(path) => normalize_windows_path(path).is_some_and(|path| {
            path.rsplit('\\').next().is_some_and(|basename| basename.eq_ignore_ascii_case(&app.exe_name))
        }),
    }
}

fn repair_reason(app: &HdrApp) -> Option<RepairReason> {
    if permanently_excluded(&app.exe_name) {
        Some(RepairReason::Helper)
    } else if !primary_path_consistent(app) {
        Some(RepairReason::PrimaryPathMismatch)
    } else {
        None
    }
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct QuarantinedApp {
    pub row_index: usize,
    pub name: String,
    pub exe_name: String,
    pub path: Option<String>,
    pub reason: RepairReason,
}

pub fn quarantined_apps(config: &AppConfig) -> Vec<QuarantinedApp> {
    config
        .apps
        .iter()
        .enumerate()
        .filter_map(|(row_index, app)| repair_reason(app).map(|reason| QuarantinedApp {
            row_index,
            name: app.name.clone(),
            exe_name: app.exe_name.clone(),
            path: app.path.clone(),
            reason,
        }))
        .collect()
}

/// Lexical Win32 image-path comparison, including canonical verbatim paths from scanners.
/// No filesystem lookup, short-name guessing, or symlink resolution grants identity here.
pub fn normalize_windows_path(path: &str) -> Option<String> {
    let path = path.replace('/', "\\").to_lowercase();
    let path = if let Some(unc) = path.strip_prefix(r"\\?\unc\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    };
    let (prefix, rest, minimum) = if path.as_bytes().get(1) == Some(&b':')
        && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && path.as_bytes().get(2) == Some(&b'\\')
    {
        (path[..3].to_owned(), &path[3..], 0)
    } else if let Some(unc) = path.strip_prefix(r"\\") {
        (r"\\".to_owned(), unc, 2)
    } else {
        return None;
    };
    let mut parts = Vec::new();
    for part in rest.split('\\') {
        match part {
            "" | "." => {}
            ".." if parts.len() > minimum => {
                parts.pop();
            }
            ".." => return None,
            value
                if value
                    .chars()
                    .any(|c| c.is_control() || "<>:\"|?*".contains(c)) =>
            {
                return None;
            }
            value => parts.push(value),
        }
    }
    if parts.len() <= minimum {
        return None;
    }
    Some(format!("{prefix}{}", parts.join("\\")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution<'a> {
    Matched(&'a HdrApp),
    Disabled,
    Excluded,
    Quarantined,
    Ambiguous,
    NoMatch,
}

impl<'a> Resolution<'a> {
    pub fn matched(self) -> Option<&'a HdrApp> {
        match self {
            Self::Matched(app) => Some(app),
            _ => None,
        }
    }
}

fn has_path(app: &HdrApp) -> bool {
    app.path.as_deref().is_some_and(|path| !path.is_empty())
}

fn unscoped_claim(app: &HdrApp, exe: &str) -> bool {
    (app.exe_name.eq_ignore_ascii_case(exe) && !has_path(app))
        || app.alternate_exes.iter().any(|alias| {
            !alias.eq_ignore_ascii_case(&app.exe_name) && alias.eq_ignore_ascii_case(exe)
        })
}

/// Distinct path-bound primaries coexist; aliases and pathless primaries claim a basename.
pub fn claims_overlap(left: &HdrApp, right: &HdrApp) -> bool {
    std::iter::once(&left.exe_name).chain(&left.alternate_exes).any(|exe| {
        !permanently_excluded(exe)
            && std::iter::once(&right.exe_name).chain(&right.alternate_exes).any(|other| {
                if !exe.eq_ignore_ascii_case(other) {
                    return false;
                }
                if exe.eq_ignore_ascii_case(&left.exe_name) && other.eq_ignore_ascii_case(&right.exe_name) {
                    return !matches!(
                        (left.path.as_deref().and_then(normalize_windows_path),
                         right.path.as_deref().and_then(normalize_windows_path)),
                        (Some(left), Some(right)) if left != right
                    );
                }
                true
            })
    })
}

/// Disabled and quarantined claims veto unscoped enrollment/matching, not unrelated installations.
pub fn resolve<'a>(config: &'a AppConfig, path: Option<&str>, exe: &str) -> Resolution<'a> {
    if permanently_excluded(exe)
        || config
            .blacklist
            .iter()
            .any(|blocked| blocked.eq_ignore_ascii_case(exe))
    {
        return Resolution::Excluded;
    }
    let path = path.and_then(normalize_windows_path);
    let mut exact = config.apps.iter().filter(|app| {
        app.exe_name.eq_ignore_ascii_case(exe)
            && path.as_ref().is_some_and(|path| {
                app.path
                    .as_deref()
                    .and_then(normalize_windows_path)
                    .as_ref()
                    == Some(path)
            })
    });
    if let Some(owner) = exact.next() {
        return if exact.next().is_some() {
            Resolution::Ambiguous
        } else if is_quarantined(owner) {
            Resolution::Quarantined
        } else if !owner.enabled {
            Resolution::Disabled
        } else {
            Resolution::Matched(owner)
        };
    }

    let owners: Vec<_> = config
        .apps
        .iter()
        .filter(|app| unscoped_claim(app, exe))
        .collect();
    if owners.iter().any(|app| is_quarantined(app)) {
        Resolution::Quarantined
    } else if owners.iter().any(|app| !app.enabled) {
        Resolution::Disabled
    } else {
        match owners.as_slice() {
            [] => Resolution::NoMatch,
            [app] => Resolution::Matched(app),
            _ => Resolution::Ambiguous,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HdrType;

    fn app(exe: &str, path: Option<&str>, enabled: bool) -> HdrApp {
        HdrApp {
            name: exe.into(),
            exe_name: exe.into(),
            enabled,
            hdr_type: HdrType::Native,
            path: path.map(str::to_owned),
            alternate_exes: vec![],
            steam_id: None,
            launcher: None,
        }
    }

    #[test]
    fn permanent_names_are_anchored_not_generic_discovery_filters() {
        for exe in [
            "GameLaunchHelper.exe",
            "BsSndRpt.exe",
            "BsSndRpt64.exe",
            "BugSplat.exe",
            "BugSplatHD64.exe",
        ] {
            assert!(permanently_excluded(exe), "{exe}");
        }
        for exe in [
            "server.exe",
            "editor.exe",
            "game-server.exe",
            "bugsplat-game.exe",
            "mybssndrpt.exe",
            "crashbandicootnsanetrilogy.exe",
            "crash.exe",
        ] {
            assert!(!permanently_excluded(exe), "{exe}");
        }
    }

    #[test]
    fn windows_paths_compare_case_separators_verbatim_and_dot_components() {
        for path in [
            r"C:\Games\game.exe",
            "c:/GAMES/./bin/../game.exe",
            r"\\?\C:\Games\game.exe",
        ] {
            assert_eq!(
                normalize_windows_path(path).as_deref(),
                Some(r"c:\games\game.exe")
            );
        }
        assert_eq!(
            normalize_windows_path(r"\\?\UNC\Server\Share\Game.exe"),
            normalize_windows_path(r"\\server\share\game.exe")
        );
        for invalid in [
            r"game.exe",
            r"C:game.exe",
            r"C:\..\game.exe",
            r"\\server\share\..\game.exe",
        ] {
            assert!(normalize_windows_path(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn exact_path_beats_basename_and_disabled_exact_owners_veto() {
        let scoped = app("game.exe", Some(r"C:\Games\game.exe"), true);
        let mut alias = app("another.exe", None, true);
        alias.alternate_exes = vec!["game.exe".into()];
        let mut config = AppConfig::default();
        for rows in [
            vec![scoped.clone(), alias.clone()],
            vec![alias.clone(), scoped.clone()],
        ] {
            config.apps = rows;
            assert_eq!(
                resolve(&config, Some(r"\\?\c:\games\GAME.exe"), "game.exe"),
                Resolution::Matched(&scoped)
            );
        }
        config.apps[1].enabled = false;
        assert_eq!(
            resolve(&config, Some(r"C:\Games\game.exe"), "game.exe"),
            Resolution::Disabled
        );
        assert_eq!(
            resolve(&config, Some(r"D:\Other\game.exe"), "game.exe"),
            Resolution::Matched(&alias)
        );
    }

    #[test]
    fn mismatching_or_unavailable_paths_never_degrade_to_primary_basename() {
        let mut config = AppConfig::default();
        let mut scoped = app("game.exe", Some(r"C:\Games\game.exe"), true);
        scoped.alternate_exes = vec!["GAME.EXE".into(), "renderer.exe".into()];
        config.apps = vec![scoped];
        for path in [None, Some(r"D:\Other\game.exe"), Some("invalid")] {
            assert_eq!(resolve(&config, path, "game.exe"), Resolution::NoMatch);
        }
        assert!(matches!(
            resolve(&config, None, "renderer.exe"),
            Resolution::Matched(_)
        ));
        config.apps[0].enabled = false;
        assert_eq!(
            resolve(&config, Some(r"D:\Other\game.exe"), "game.exe"),
            Resolution::NoMatch
        );
        assert_eq!(resolve(&config, None, "renderer.exe"), Resolution::Disabled);
    }

    #[test]
    fn duplicate_owners_are_ambiguous_but_duplicate_claims_in_one_row_collapse() {
        let mut config = AppConfig::default();
        let mut game = app("game.exe", None, true);
        game.alternate_exes = vec![
            "GAME.EXE".into(),
            "renderer.exe".into(),
            "RENDERER.EXE".into(),
        ];
        config.apps = vec![game.clone()];
        assert_eq!(
            resolve(&config, None, "game.exe"),
            Resolution::Matched(&game)
        );
        assert_eq!(
            resolve(&config, None, "renderer.exe"),
            Resolution::Matched(&game)
        );
        config.apps.push(game);
        assert_eq!(resolve(&config, None, "game.exe"), Resolution::Ambiguous);
        assert_eq!(
            resolve(&config, None, "renderer.exe"),
            Resolution::Ambiguous
        );
        for row in &mut config.apps {
            row.path = Some(r"C:\game.exe".into());
        }
        config.apps[0].enabled = false;
        assert_eq!(
            resolve(&config, Some(r"C:\game.exe"), "game.exe"),
            Resolution::Ambiguous
        );
    }

    #[test]
    fn polluted_primary_quarantines_every_historical_alias_without_mutation() {
        let mut config = AppConfig::default();
        let mut polluted = app("BsSndRpt64.exe", Some(r"C:\AOE4\BsSndRpt64.exe"), false);
        polluted.alternate_exes = vec!["historical.exe".into()];
        polluted.steam_id = Some("1466860".into());
        config.apps = vec![polluted];
        let before = config.clone();
        assert_eq!(
            resolve(&config, None, "BsSndRpt64.exe"),
            Resolution::Excluded
        );
        assert_eq!(
            resolve(&config, Some(r"C:\AOE4\historical.exe"), "historical.exe"),
            Resolution::Quarantined
        );
        assert_eq!(quarantined_apps(&config).len(), 1);
        assert_eq!(config, before);
        config.apps[0].enabled = true;
        assert_eq!(
            resolve(&config, None, "historical.exe"),
            Resolution::Quarantined
        );
    }

    #[test]
    fn helper_aliases_and_fuzzy_names_never_authorize_runtime() {
        let mut config = AppConfig::default();
        let mut game = app("game.exe", None, true);
        game.name = "Game DX12".into();
        game.alternate_exes = vec!["GameLaunchHelper.exe".into(), "BsSndRpt64.exe".into()];
        config.apps = vec![game];
        for helper in ["GameLaunchHelper.exe", "BsSndRpt64.exe"] {
            assert_eq!(resolve(&config, None, helper), Resolution::Excluded);
        }
        for fuzzy in ["game_dx12.exe", "game-win64-shipping.exe", "GameDX12.exe"] {
            assert_eq!(resolve(&config, None, fuzzy), Resolution::NoMatch);
        }
    }

    #[test]
    fn audit_legacy_primary_alias_path_mismatches_are_derived_quarantine_without_rewrite() {
        for path in [r"C:\Game\renderer.exe", r"C:\Game\BsSndRpt.exe", "relative.exe"] {
            for enabled in [false, true] {
                let mut row = app("game.exe", Some(path), enabled);
                row.alternate_exes = vec!["renderer.exe".into(), "BsSndRpt.exe".into()];
                let mut config = AppConfig::default();
                config.apps = vec![row.clone(), row];
                let before = config.clone();
                assert_eq!(resolve(&config, Some(path), "renderer.exe"), Resolution::Quarantined);
                assert!(resolve(&config, Some(r"C:\Game\game.exe"), "game.exe").matched().is_none());
                let repairs = quarantined_apps(&config);
                assert_eq!(repairs.len(), 2);
                assert_eq!(repairs[0].reason, RepairReason::PrimaryPathMismatch);
                assert_eq!(repairs[0].row_index, 0);
                assert_eq!(repairs[1].row_index, 1);
                assert_eq!(repairs[0].path.as_deref(), Some(path));
                assert_eq!(config, before);
            }
        }
    }
}
