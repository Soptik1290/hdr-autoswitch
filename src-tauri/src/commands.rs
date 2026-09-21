use crate::config::{ConfigMode, ConfigSnapshot, HdrApp, SettingsPatch, TargetMonitor};
use crate::database::CatalogEntry;
use crate::display::MonitorInfo;
use crate::monitor_hook::{HdrStatePayload, ManualRequestIdentity, ManualSetResult};
use crate::process::RunningProcessInfo;
use crate::{database, display, emit_config, legacy_upgrade, library, scanner, AppState};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

#[derive(Serialize)]
pub struct MonitorView {
    #[serde(flatten)]
    monitor: MonitorInfo,
    is_selected: bool,
}

#[derive(Serialize)]
pub struct MonitorInventoryView {
    inventory_revision: String,
    monitors: Vec<MonitorView>,
}

fn monitor_view(monitor: MonitorInfo, snapshot: &ConfigSnapshot) -> MonitorView {
    let is_selected = snapshot.mode == ConfigMode::Ready
        && display::monitor_is_selected(&monitor, &snapshot.settings.target_monitor);
    MonitorView { monitor, is_selected }
}

#[derive(Serialize)]
pub struct ScanResult {
    context_token: String,
    library_generation: String,
    games: Vec<scanner::ScanGame>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiLanguage {
    Cs,
    En,
}

fn publish(
    app: &AppHandle,
    state: &AppState,
    result: Result<ConfigSnapshot, String>,
) -> Result<ConfigSnapshot, String> {
    let snapshot = match &result {
        Ok(snapshot) => Some(snapshot.clone()),
        Err(error) => {
            eprintln!("Configuration command failed: {error}");
            match state.config_mgr.snapshot() {
                Ok(snapshot) => Some(snapshot),
                Err(error) => {
                    eprintln!("Cannot publish configuration failure state: {error}");
                    None
                }
            }
        }
    };
    if let Some(snapshot) = snapshot {
        emit_config(app, &snapshot);
    }
    state.monitor_service.config_committed();
    result
}

fn require_origin(state: &AppState, expected: &str) -> Result<ConfigSnapshot, String> {
    state.ensure_admission()?;
    let snapshot = state.config_mgr.snapshot()?;
    if snapshot.context_token != expected {
        return Err("Settings history changed. Start this action again.".into());
    }
    Ok(snapshot)
}

fn require_context(state: &AppState, expected: &str) -> Result<ConfigSnapshot, String> {
    let snapshot = require_origin(state, expected)?;
    if snapshot.mode != ConfigMode::Ready {
        return Err("Settings are read-only. Resolve configuration recovery first.".into());
    }
    Ok(snapshot)
}

fn check_predecessor(app: &AppHandle, state: &AppState) -> Result<(), String> {
    if state.safe_test_mode {
        return Ok(());
    }
    if let Err(error) = legacy_upgrade::check_predecessor() {
        let snapshot = state.config_mgr.set_controller_issue(Some(error.clone()))?;
        emit_config(app, &snapshot);
        state.monitor_service.config_committed();
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub async fn get_monitors(state: State<'_, AppState>) -> Result<MonitorInventoryView, String> {
    let inventory = state.monitor_service.refresh()?.resolve().await?;
    let snapshot = state.config_mgr.snapshot()?;
    Ok(MonitorInventoryView {
        inventory_revision: inventory.inventory_revision,
        monitors: inventory.monitors
        .into_iter()
        .map(|monitor| monitor_view(monitor, &snapshot))
        .collect(),
    })
}

#[tauri::command]
pub async fn set_hdr(
    state: State<'_, AppState>,
    scope: TargetMonitor,
    enable: bool,
    request: ManualRequestIdentity,
) -> Result<ManualSetResult, String> {
    state.ensure_admission()?;
    if state.safe_test_mode {
        return Err(crate::SAFE_TEST_ISSUE.into());
    }
    if !request.client_id.starts_with("gui:") {
        return Err("Window manual requests require a GUI correlation identity.".into());
    }
    state.monitor_service.manual_set(scope, enable, request)?.resolve().await
}

#[tauri::command]
pub async fn get_current_status(state: State<'_, AppState>) -> Result<HdrStatePayload, String> {
    state.monitor_service.refresh()?.resolve().await?;
    state.monitor_service.status()?.resolve().await
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Result<ConfigSnapshot, String> {
    state.config_mgr.snapshot()
}

#[tauri::command]
pub fn patch_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_context: String,
    patch: SettingsPatch,
) -> Result<ConfigSnapshot, String> {
    let _action = state
        .config_actions
        .lock()
        .map_err(|_| "Settings action lock is poisoned")?;
    require_context(&state, &expected_context)?;
    if state.safe_test_mode && patch.autostart.is_some() {
        return Err(crate::SAFE_TEST_ISSUE.into());
    }
    let result = match patch.autostart {
        Some(enabled) => {
            check_predecessor(&app, &state)?;
            legacy_upgrade::configure_autostart_with_commit(enabled, || {
                state.config_mgr.patch(&expected_context, patch)
            })
        }
        None => state.config_mgr.patch(&expected_context, patch),
    };
    publish(&app, &state, result)
}

fn change_history(
    app: &AppHandle,
    state: &AppState,
    expected_context: &str,
    change: impl FnOnce() -> Result<ConfigSnapshot, String>,
) -> Result<ConfigSnapshot, String> {
    let _action = state
        .config_actions
        .lock()
        .map_err(|_| "Settings action lock is poisoned")?;
    require_origin(state, expected_context)?;
    check_predecessor(app, state)?;
    let result = change();
    let reconciliation = crate::reconcile_controller(&state.config_mgr, state.safe_test_mode);
    let result = match result {
        Ok(_) => reconciliation,
        Err(error) => {
            if let Err(reconcile_error) = reconciliation {
                eprintln!(
                    "Controller reconciliation after failed history change: {reconcile_error}"
                );
            }
            Err(error)
        }
    };
    publish(app, state, result)
}

#[tauri::command]
pub fn initialize_config(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_context: String,
) -> Result<ConfigSnapshot, String> {
    change_history(&app, &state, &expected_context, || {
        state.config_mgr.initialize(&expected_context)
    })
}

#[tauri::command]
pub fn import_legacy_config(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_context: String,
) -> Result<ConfigSnapshot, String> {
    change_history(&app, &state, &expected_context, || {
        state.config_mgr.import_legacy(&expected_context)
    })
}

#[tauri::command]
pub fn restore_config(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_context: String,
    candidate_id: String,
) -> Result<ConfigSnapshot, String> {
    change_history(&app, &state, &expected_context, || {
        state.config_mgr.restore(&expected_context, &candidate_id)
    })
}

#[tauri::command]
pub fn reset_config(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_context: String,
) -> Result<ConfigSnapshot, String> {
    change_history(&app, &state, &expected_context, || {
        state.config_mgr.reset(&expected_context)
    })
}

#[tauri::command]
pub fn recheck_controller(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_context: String,
) -> Result<ConfigSnapshot, String> {
    let _action = state
        .config_actions
        .lock()
        .map_err(|_| "Settings action lock is poisoned")?;
    require_context(&state, &expected_context)?;
    publish(
        &app,
        &state,
        crate::reconcile_controller(&state.config_mgr, state.safe_test_mode),
    )
}

#[tauri::command]
pub fn get_catalog() -> Vec<CatalogEntry> {
    database::get_full_catalog()
}

#[tauri::command]
pub async fn sync_database(state: State<'_, AppState>) -> Result<usize, String> {
    state.ensure_admission()?;
    if state.safe_test_mode {
        return Err("Safe test mode blocks catalog network synchronization and cache writes.".into());
    }
    Ok(database::fetch_online_database().await?.len())
}

#[tauri::command]
pub async fn scan_installed_games(
    state: State<'_, AppState>,
    expected_context: String,
    expected_library_generation: String,
) -> Result<ScanResult, String> {
    let origin = require_context(&state, &expected_context)?;
    if origin.library_generation != expected_library_generation {
        return Err("The library changed before scanning. Start the scan again.".into());
    }
    let auto_detect = origin.settings.auto_detect_new_games;
    let games = tauri::async_runtime::spawn_blocking(move || scanner::scan_installed_games(auto_detect))
        .await
        .map_err(|error| format!("Game scan failed: {error}"))?;
    Ok(ScanResult {
        context_token: origin.context_token,
        library_generation: origin.library_generation,
        games,
    })
}

#[tauri::command]
pub fn import_detected_games(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_context: String,
    expected_library_generation: String,
    detected: Vec<HdrApp>,
) -> Result<ConfigSnapshot, String> {
    state.ensure_admission()?;
    let result = state.config_mgr.mutate(
        &expected_context,
        Some(&expected_library_generation),
        true,
        |settings| library::import_games(settings, detected),
    );
    publish(&app, &state, result)
}

#[tauri::command]
pub fn add_custom_app(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    expected_context: String,
    app: HdrApp,
) -> Result<ConfigSnapshot, String> {
    state.ensure_admission()?;
    let result = state
        .config_mgr
        .mutate(&expected_context, None, true, |settings| {
            library::add_app(settings, app)
        });
    publish(&app_handle, &state, result)
}

#[tauri::command]
pub fn repair_app_executable(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_context: String,
    expected_library_generation: String,
    row: library::AppRowIdentity,
    path: String,
) -> Result<ConfigSnapshot, String> {
    require_context(&state, &expected_context)?;
    let selected = scanner::inspect_exe_path(&path)?;
    let result = state.config_mgr.mutate(
        &expected_context, Some(&expected_library_generation), true,
        |settings| library::repair_executable(settings, &row, &selected.exe_name, &selected.path),
    );
    publish(&app, &state, result)
}

#[tauri::command]
pub fn remove_app(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_context: String,
    expected_library_generation: String,
    row: library::AppRowIdentity,
) -> Result<ConfigSnapshot, String> {
    state.ensure_admission()?;
    let result = state
        .config_mgr
        .mutate(&expected_context, Some(&expected_library_generation), true,
            |settings| library::remove_app(settings, &row));
    publish(&app, &state, result)
}

#[tauri::command]
pub fn toggle_app(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_context: String,
    expected_library_generation: String,
    row: library::AppRowIdentity,
    enabled: bool,
) -> Result<ConfigSnapshot, String> {
    state.ensure_admission()?;
    let result = state
        .config_mgr
        .mutate(&expected_context, Some(&expected_library_generation), true,
            |settings| library::toggle_app(settings, &row, enabled));
    publish(&app, &state, result)
}

#[derive(Serialize)]
pub struct RunningProcessView {
    #[serde(flatten)]
    process: RunningProcessInfo,
    tracked_primary: Option<String>,
}

fn running_process_view(process: RunningProcessInfo, snapshot: &ConfigSnapshot) -> RunningProcessView {
    let tracked_primary = snapshot.settings.resolve_app(Some(&process.path), &process.exe_name)
        .matched().map(|app| app.exe_name.clone());
    RunningProcessView { process, tracked_primary }
}

#[tauri::command]
pub fn get_running_processes(state: State<'_, AppState>) -> Result<Vec<RunningProcessView>, String> {
    let snapshot = state.config_mgr.snapshot()?;
    Ok(crate::process::get_running_processes().into_iter()
        .map(|process| running_process_view(process, &snapshot)).collect())
}

#[tauri::command]
pub fn pick_game_exe(language: UiLanguage) -> Result<Option<scanner::PickedGameInfo>, String> {
    scanner::pick_game_exe_dialog(matches!(language, UiLanguage::Cs))
}

#[tauri::command]
pub fn set_ui_language(app: AppHandle, language: UiLanguage) -> Result<(), String> {
    crate::tray::set_language(&app, matches!(language, UiLanguage::Cs))
}

#[tauri::command]
pub fn inspect_exe_path(path: String) -> Result<scanner::PickedGameInfo, String> {
    scanner::inspect_exe_path(&path)
}

#[tauri::command]
pub fn verify_game_paths(paths: Vec<String>) -> std::collections::HashMap<String, bool> {
    paths
        .into_iter()
        .map(|path| {
            let exists = std::path::Path::new(&path).exists();
            (path, exists)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigManager;

    #[test]
    fn running_process_badges_use_the_runtime_path_resolver_not_ui_fuzzy_lookup() {
        let root = tempfile::tempdir().unwrap();
        let manager = ConfigManager::load(root.path().join("local"), root.path().join("absent")).unwrap();
        let mut snapshot = manager.snapshot().unwrap();
        let mut row = crate::config::HdrApp {
            name: "Game".into(), exe_name: "game.exe".into(), enabled: true,
            hdr_type: crate::config::HdrType::Native, path: Some(r"C:\Game\game.exe".into()),
            alternate_exes: vec!["historical.exe".into()], steam_id: None, launcher: None,
        };
        let process = |exe: &str, path: &str| RunningProcessInfo {
            pid: 1, name: "Game".into(), exe_name: exe.into(), title: "Game".into(), path: path.into(),
        };
        snapshot.settings.apps = vec![row.clone()];
        assert_eq!(running_process_view(process("game.exe", r"C:\Game\game.exe"), &snapshot).tracked_primary.as_deref(), Some("game.exe"));
        assert_eq!(running_process_view(process("game.exe", r"D:\Game\game.exe"), &snapshot).tracked_primary, None);
        assert_eq!(running_process_view(process("game_dx12.exe", r"C:\Game\game_dx12.exe"), &snapshot).tracked_primary, None);
        row.exe_name = "BsSndRpt64.exe".into();
        snapshot.settings.apps = vec![row];
        assert_eq!(running_process_view(process("historical.exe", r"C:\Game\historical.exe"), &snapshot).tracked_primary, None);
    }

    #[test]
    fn monitor_ipc_does_not_label_untrusted_default_all_as_a_saved_target() {
        let root = tempfile::tempdir().unwrap();
        let manager = ConfigManager::load(
            root.path().join("local"), root.path().join("legacy.json"),
        ).unwrap();
        let first = manager.snapshot().unwrap();
        let monitor = crate::display::tests::monitor("chosen", 1, false);
        for mode in [
            ConfigMode::FirstRun, ConfigMode::ImportAvailable, ConfigMode::RecoveryRequired,
            ConfigMode::UnsupportedSchema, ConfigMode::Unavailable,
        ] {
            let mut snapshot = first.clone();
            snapshot.mode = mode;
            assert!(!monitor_view(monitor.clone(), &snapshot).is_selected);
        }
        let saved = manager.initialize(&first.context_token).unwrap();
        assert!(monitor_view(monitor, &saved).is_selected);
    }
}
