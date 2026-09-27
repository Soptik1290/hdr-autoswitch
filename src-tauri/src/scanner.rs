use crate::config::{HdrApp, HdrType};
use crate::database;
use crate::automatic_authority::{self, Authority, InstallEvidence, LaunchField, Provider, ResolvedGame};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickedGameInfo {
    pub name: String,
    pub exe_name: String,
    pub path: String,
    pub hdr_type: HdrType,
    pub is_hdr_supported: bool,
    pub notes: Option<String>,
    pub launcher: Option<String>,
}

pub struct ScanResult {
    pub detected: Vec<HdrApp>,
    pub verified: Vec<ResolvedGame>,
}

#[derive(Default)]
struct ScanCollection {
    detected: HashMap<String, HdrApp>,
    verified: Vec<ResolvedGame>,
}

pub fn scan_installed_games(auto_detect: bool) -> Vec<HdrApp> {
    scan_installed_games_with_authority(auto_detect).detected
}

pub fn scan_installed_games_with_authority(auto_detect: bool) -> ScanResult {
    let catalog = database::get_full_catalog();
    let mut detected_map = ScanCollection::default();

    // 1. Scan Steam via appmanifest_*.acf (across all drives and libraryfolders)
    scan_steam_manifests(&catalog, &mut detected_map);

    // 2. Scan Epic Games Launcher via Manifests/*.item
    scan_epic_manifests(&catalog, &mut detected_map);

    // 3. Scan GOG Galaxy games registry
    scan_gog_registry(&catalog, &mut detected_map);

    // 4. Scan Windows Registry (EA App, Ubisoft Connect, standalone installers)
    scan_windows_registry(&catalog, &mut detected_map);

    // 5. Scan Xbox Games folders (C:\XboxGames, D:\XboxGames, etc.)
    scan_xbox_games(&catalog, &mut detected_map);

    // 6. Scan common media players
    scan_media_players(&catalog, &mut detected_map);

    let mut result: Vec<HdrApp> = detected_map.detected.into_values().collect();
    for game in &mut result {
        game.enabled &= auto_detect;
    }

    // Sort: HDR-enabled games first (alphabetically), then SDR games (alphabetically)
    result.sort_by(|a, b| {
        match (a.enabled, b.enabled) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase())
                .then(a.exe_name.cmp(&b.exe_name)).then(a.path.cmp(&b.path)),
        }
    });

    ScanResult { detected: result, verified: detected_map.verified }
}

// --------------------------------------------------------------------------------------
// Native File Dialog & Path Inspection
// --------------------------------------------------------------------------------------

pub fn pick_game_exe_dialog(czech: bool) -> Result<Option<PickedGameInfo>, String> {
    let file = rfd::FileDialog::new()
        .add_filter(if czech { "Spustitelný soubor (*.exe)" } else { "Executable (*.exe)" }, &["exe"])
        .set_title(if czech { "Vybrat herní soubor (.exe)" } else { "Select Game Executable (.exe)" })
        .pick_file();

    match file {
        Some(path_buf) => {
            let path_str = path_buf.to_string_lossy().to_string();
            inspect_exe_path(&path_str).map(Some)
        }
        None => Ok(None),
    }
}

pub fn inspect_exe_path(path: &str) -> Result<PickedGameInfo, String> {
    let p = PathBuf::from(path);
    if !p.exists() || !p.is_file() {
        return Err("Selected file does not exist.".to_string());
    }

    let exe_name = match p.file_name() {
        Some(n) => n.to_string_lossy().to_string(),
        None => return Err("Invalid file name.".to_string()),
    };

    if !exe_name.to_lowercase().ends_with(".exe") {
        return Err("Selected file is not an .exe.".to_string());
    }
    if crate::runtime_policy::permanently_excluded(&exe_name) {
        return Err(format!("Selected executable '{exe_name}' is a helper, not a game runtime. Select the game's executable instead."));
    }

    let catalog = database::get_full_catalog();

    // Determine smart game folder name by traversing up past "Win64", "Binaries", "x64", etc.
    let folder_name = p
        .parent()
        .and_then(|mut curr| {
            loop {
                let name = curr.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let lower = name.to_lowercase();
                if lower == "win64" || lower == "binaries" || lower == "bin" || lower == "x64" || lower == "shipping" {
                    if let Some(parent) = curr.parent() {
                        curr = parent;
                        continue;
                    }
                }
                break Some(name);
            }
        })
        .unwrap_or_default();

    // Check catalog match by exe name, alternate exes, or folder title
    let matched_cat = catalog.iter().find(|c| {
        c.exe_name.eq_ignore_ascii_case(&exe_name)
            || c.alternate_exes.iter().any(|a| a.eq_ignore_ascii_case(&exe_name))
            || (!folder_name.is_empty() && is_title_match(&c.name, &folder_name))
    });


    let (display_name, hdr_type, is_hdr, notes) = if let Some(cat) = matched_cat {
        (cat.name.clone(), cat.hdr_type.clone(), true, cat.notes.clone())
    } else {
        let clean_name = if !folder_name.is_empty() && !folder_name.eq_ignore_ascii_case("common") {
            clean_title_from_folder(&folder_name)
        } else {
            exe_name.trim_end_matches(".exe").trim_end_matches(".EXE").to_string()
        };
        (clean_name, HdrType::Native, false, None)
    };

    let path_lower = path.to_lowercase();
    let launcher = if path_lower.contains("steamapps") {
        Some("Steam".to_string())
    } else if path_lower.contains("epic games") || path_lower.contains(".egstore") {
        Some("Epic Games".to_string())
    } else if path_lower.contains("gog galaxy") || path_lower.contains("gog games") {
        Some("GOG".to_string())
    } else if path_lower.contains("xboxgames") {
        Some("Xbox".to_string())
    } else {
        None
    };

    Ok(PickedGameInfo {
        name: display_name,
        exe_name: exe_name.to_lowercase(),
        path: path.to_string(),
        hdr_type,
        is_hdr_supported: is_hdr,
        notes,
        launcher,
    })
}

// --------------------------------------------------------------------------------------
// 1. Steam scanner using appmanifest_*.acf
// --------------------------------------------------------------------------------------
fn scan_steam_manifests(catalog: &[database::CatalogEntry], map: &mut ScanCollection) {
    let mut steam_roots = Vec::new();

    // Check registry keys for Steam
    let reg_checks = [
        (HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
        (HKEY_CURRENT_USER, r"Software\Valve\Steam", "InstallPath"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
    ];

    for (root, subkey, val_name) in reg_checks {
        if let Ok(reg_val) = read_registry_string(root, subkey, val_name) {
            let normalized = reg_val.replace('/', "\\").trim_end_matches('\\').to_string();
            let p = PathBuf::from(normalized);
            if p.exists() && !steam_roots.contains(&p) {
                steam_roots.push(p);
            }
        }
    }

    // Check common drive roots for Steam
    for drive in ['C', 'D', 'E', 'F', 'G', 'H'] {
        for sub in [
            r"Program Files (x86)\Steam",
            r"Program Files\Steam",
            r"Steam",
            r"SteamLibrary",
            r"SteamHry",
            r"Games\Steam",
            r"Games\SteamLibrary",
        ] {
            let p = PathBuf::from(format!(r"{}:\{}", drive, sub));
            if p.exists() && !steam_roots.contains(&p) {
                steam_roots.push(p);
            }
        }
    }

    let mut library_paths = Vec::new();

    for steam_root in steam_roots {
        let direct_steamapps = steam_root.join("steamapps");
        if direct_steamapps.exists() && !library_paths.contains(&direct_steamapps) {
            library_paths.push(direct_steamapps);
        }

        let vdf = steam_root.join(r"steamapps\libraryfolders.vdf");
        if let Ok(content) = fs::read_to_string(&vdf) {
            for line in content.lines() {
                let trimmed = line.trim();
                // Find lines with "path" followed by library path
                if trimmed.contains("\"path\"") {
                    let parts: Vec<&str> = trimmed.split('"').collect();
                    for (idx, &part) in parts.iter().enumerate() {
                        if part.eq_ignore_ascii_case("path") && idx + 2 < parts.len() {
                            let path_candidate = parts[idx + 2].replace(r"\\", r"\").replace('/', r"\");
                            let p = PathBuf::from(path_candidate).join("steamapps");
                            if p.exists() && !library_paths.contains(&p) {
                                library_paths.push(p);
                            }
                        }
                    }
                }
            }
        }
    }

    // Blacklisted Steam app IDs (Redistributables, runtimes, VR, dedicated servers)
    let ignored_app_ids = [
        "228980",  // Steamworks Common Redistributables
        "1070560", // Steam Linux Runtime
        "1391110", // Proton
        "1493710", // Proton Hotfix
        "221410",  // SteamVR
        "250820",  // SteamVR
    ];

    // Scan each library's appmanifest_*.acf
    for steamapps in library_paths {
        let entries = match fs::read_dir(&steamapps) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            if file_name.starts_with("appmanifest_") && file_name.ends_with(".acf") {
                let steam_appid = file_name
                    .trim_start_matches("appmanifest_")
                    .trim_end_matches(".acf")
                    .to_string();

                if ignored_app_ids.contains(&steam_appid.as_str()) {
                    continue;
                }

                if let Ok(content) = fs::read_to_string(&path) {
                    let mut game_name = String::new();
                    let mut install_dir_name = String::new();
                    let mut declared_appid = None;

                    for line in content.lines() {
                        let trimmed = line.trim();
                        if trimmed.starts_with("\"appid\"") {
                            let parts: Vec<&str> = trimmed.split('"').collect();
                            if parts.len() >= 4 {
                                if declared_appid.is_some() {
                                    declared_appid = Some(String::new());
                                    break;
                                }
                                declared_appid = Some(parts[3].to_string());
                            }
                        } else if trimmed.starts_with("\"name\"") {
                            let parts: Vec<&str> = trimmed.split('"').collect();
                            if parts.len() >= 4 {
                                game_name = parts[3].replace(r#"™"#, "").replace(r#"®"#, "").trim().to_string();
                            }
                        } else if trimmed.starts_with("\"installdir\"") {
                            let parts: Vec<&str> = trimmed.split('"').collect();
                            if parts.len() >= 4 {
                                install_dir_name = parts[3].trim().to_string();
                            }
                        }
                    }

                    // Skip non-game Steam tools
                    let lower_name = game_name.to_lowercase();
                    if lower_name.contains("steamworks")
                        || lower_name.contains("proton")
                        || lower_name.contains("steam linux runtime")
                        || lower_name.contains("soundtrack")
                        || lower_name.contains("dedicated server")
                        || lower_name.contains("benchmark tool")
                    {
                        continue;
                    }

                    if declared_appid.as_deref() != Some(&steam_appid)
                        || !valid_steam_install_dir(&install_dir_name)
                    {
                        eprintln!("Skipping Steam manifest with invalid installation identity: {}", path.display());
                        continue;
                    }
                    if !install_dir_name.is_empty() {
                        let full_game_dir = steamapps.join("common").join(&install_dir_name);
                        if full_game_dir.exists() {
                            let clean_game_name = if let Some(slash_idx) = game_name.find(" / ") {
                                game_name[..slash_idx].trim().to_string()
                            } else {
                                game_name.clone()
                            };

                            match_and_insert_game(
                                catalog,
                                &clean_game_name,
                                &full_game_dir,
                                &[],
                                map,
                                Provider::Steam,
                                Some(&steam_appid),
                            );
                        }
                    }

                }
            }
        }
    }
}

// --------------------------------------------------------------------------------------
// 2. Epic Games manifests scanner
// --------------------------------------------------------------------------------------
fn scan_epic_manifests(catalog: &[database::CatalogEntry], map: &mut ScanCollection) {
    let mut manifest_dirs = Vec::new();
    if let Ok(prog_data) = std::env::var("ProgramData") {
        manifest_dirs.push(PathBuf::from(prog_data).join(r"Epic\EpicGamesLauncher\Data\Manifests"));
    }
    manifest_dirs.push(PathBuf::from(r"C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests"));

    for manifest_dir in manifest_dirs {
        if !manifest_dir.exists() {
            continue;
        }

        let entries = match fs::read_dir(&manifest_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "item").unwrap_or(false) {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                        // Skip incomplete installations
                        if val["bIsIncompleteInstall"].as_bool().unwrap_or(false) {
                            continue;
                        }

                        let display_name = val["DisplayName"].as_str().unwrap_or_default();
                        let install_loc = val["InstallLocation"].as_str().unwrap_or_default();
                        let launch_exe = val["LaunchExecutable"].as_str().unwrap_or_default();

                        if display_name.is_empty() || install_loc.is_empty() {
                            continue;
                        }

                        let game_dir = PathBuf::from(install_loc);
                        if game_dir.exists() {
                            let declarations = parsed_launch(&game_dir, launch_exe, LaunchField::Path);
                            match_and_insert_game(
                                catalog,
                                display_name,
                                &game_dir,
                                &declarations,
                                map,
                                Provider::Epic,
                                None,
                            );
                        }
                    }
                }
            }
        }
    }
}

// --------------------------------------------------------------------------------------
// 3. GOG Galaxy Registry Scanner
// --------------------------------------------------------------------------------------
fn scan_gog_registry(catalog: &[database::CatalogEntry], map: &mut ScanCollection) {
    let gog_keys = [
        r"SOFTWARE\GOG.com\Games",
        r"SOFTWARE\WOW6432Node\GOG.com\Games",
    ];

    for key_path in gog_keys {
        let subkey_wide = to_wide(key_path);
        let mut h_key = HKEY::default();

        unsafe {
            if RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(subkey_wide.as_ptr()), Some(0), KEY_READ, &mut h_key) != WIN32_ERROR(0) {
                continue;
            }

            let mut index = 0;
            let mut key_name_buf = [0u16; 256];

            loop {
                let mut key_name_len = key_name_buf.len() as u32;
                let status = RegEnumKeyExW(
                    h_key,
                    index,
                    Some(windows::core::PWSTR(key_name_buf.as_mut_ptr())),
                    &mut key_name_len,
                    None,
                    None,
                    None,
                    None,
                );

                if status != WIN32_ERROR(0) {
                    break;
                }

                index += 1;

                let child_id = String::from_utf16_lossy(&key_name_buf[..key_name_len as usize]);
                let child_path = format!(r"{}\{}", key_path, child_id);

                if let Ok(game_name) = read_registry_string(HKEY_LOCAL_MACHINE, &child_path, "gameName") {
                    if let Ok(path_str) = read_registry_string(HKEY_LOCAL_MACHINE, &child_path, "path") {
                        let game_dir = PathBuf::from(&path_str);
                        if game_dir.exists() && game_dir.is_dir() {
                            let launch_exe = read_registry_string(HKEY_LOCAL_MACHINE, &child_path, "exe").ok();
                            let declarations = launch_exe.map(|raw| {
                                // GOG's exe value is a path; quoted command forms also occur.
                                let field = if raw.trim().starts_with('"') { LaunchField::Command } else { LaunchField::Path };
                                parsed_launch(&game_dir, &raw, field)
                            }).unwrap_or_default();
                            match_and_insert_game(
                                catalog,
                                &game_name,
                                &game_dir,
                                &declarations,
                                map,
                                Provider::Gog,
                                None,
                            );
                        }
                    }
                }
            }

            let _ = RegCloseKey(h_key);
        }
    }
}

// --------------------------------------------------------------------------------------
// 4. Windows Registry Uninstall Keys (EA App, Ubisoft, Custom Installers)
// --------------------------------------------------------------------------------------
fn scan_windows_registry(catalog: &[database::CatalogEntry], map: &mut ScanCollection) {
    let reg_paths = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];

    for (root, subkey) in reg_paths {
        scan_registry_uninstall_hive(root, subkey, catalog, map);
    }
}

fn scan_registry_uninstall_hive(
    root: HKEY,
    subkey: &str,
    catalog: &[database::CatalogEntry],
    map: &mut ScanCollection,
) {
    let subkey_wide = to_wide(subkey);
    let mut h_key = HKEY::default();

    unsafe {
        if RegOpenKeyExW(root, PCWSTR(subkey_wide.as_ptr()), Some(0), KEY_READ, &mut h_key) != WIN32_ERROR(0) {
            return;
        }

        let mut index = 0;
        let mut key_name_buf = [0u16; 256];

        loop {
            let mut key_name_len = key_name_buf.len() as u32;
            let status = RegEnumKeyExW(
                h_key,
                index,
                Some(windows::core::PWSTR(key_name_buf.as_mut_ptr())),
                &mut key_name_len,
                None,
                None,
                None,
                None,
            );

            if status != WIN32_ERROR(0) {
                break;
            }

            index += 1;

            let child_name = String::from_utf16_lossy(&key_name_buf[..key_name_len as usize]);
            if child_name.to_ascii_lowercase().starts_with("steam app ") {
                continue;
            }
            let child_path = format!(r"{}\{}", subkey, child_name);

            if let Ok(display_name) = read_registry_string(root, &child_path, "DisplayName") {
                if is_known_non_game_software(&display_name) {
                    continue;
                }

                let icon = read_registry_string(root, &child_path, "DisplayIcon").ok();

                if let Ok(install_location) = read_registry_string(root, &child_path, "InstallLocation") {
                    let clean_dir = install_location.trim().trim_matches('"');
                    let p = PathBuf::from(clean_dir);
                    // NEVER scan entire Program Files or Drive root!
                    if p.exists() && p.is_dir() && p.parent().is_some()
                        && p.components().count() >= 3 && windows_fallback_root(&p)
                    {
                        let path_lower = clean_dir.to_lowercase();
                        if !path_lower.ends_with(r"program files")
                            && !path_lower.ends_with(r"program files (x86)")
                            && !path_lower.ends_with(r"windows")
                        {
                            if let Some(icon) = &icon {
                                let declarations = parsed_launch(&p, icon, LaunchField::DisplayIcon);
                                match_and_insert_game(catalog, &display_name, &p, &declarations, map, Provider::Windows, None);
                            }
                        }
                    }
                }

                // DisplayIcon alone does not establish an install root.
            }
        }

        let _ = RegCloseKey(h_key);
    }
}

// --------------------------------------------------------------------------------------
// 5. Xbox Games Folders
// --------------------------------------------------------------------------------------
fn scan_xbox_games(catalog: &[database::CatalogEntry], map: &mut ScanCollection) {
    for drive in ["C", "D", "E", "F", "G"] {
        let xbox_path = PathBuf::from(format!(r"{}:\XboxGames", drive));
        if !xbox_path.exists() {
            continue;
        }

        let entries = match fs::read_dir(&xbox_path) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                match crate::xbox_config::declarations(&p) {
                    Ok(declarations) => match_and_insert_game(catalog, &name, &p, &declarations, map, Provider::Xbox, None),
                    Err(error) => eprintln!("Skipping Xbox installation {}: {error}", p.display()),
                }
            }
        }
    }
}

// --------------------------------------------------------------------------------------
// 6. Media Players
// --------------------------------------------------------------------------------------
fn scan_media_players(catalog: &[database::CatalogEntry], map: &mut ScanCollection) {
    for (prog_name, _exe_str, paths) in [
        ("VLC Media Player", "vlc.exe", vec![r"C:\Program Files\VideoLAN\VLC\vlc.exe", r"C:\Program Files (x86)\VideoLAN\VLC\vlc.exe"]),
        ("MPC-HC", "mpc-hc64.exe", vec![r"C:\Program Files\MPC-HC\mpc-hc64.exe", r"C:\Program Files (x86)\MPC-HC\mpc-hc.exe"]),
        ("MPC-BE", "mpc-be64.exe", vec![r"C:\Program Files\MPC-BE\mpc-be64.exe", r"C:\Program Files (x86)\MPC-BE\mpc-be.exe"]),
        ("mpv Media Player", "mpv.exe", vec![r"C:\Program Files\mpv\mpv.exe", r"C:\mpv\mpv.exe", r"D:\mpv\mpv.exe", r"E:\mpv\mpv.exe"]),
        ("PotPlayer", "potplayer64.exe", vec![r"C:\Program Files\DAUM\PotPlayer\PotPlayer64.exe", r"C:\Program Files (x86)\DAUM\PotPlayer\PotPlayer.exe"]),
        ("Kodi", "kodi.exe", vec![r"C:\Program Files\Kodi\kodi.exe"]),
    ] {
        for p in paths {
            let path = Path::new(p);
            if path.is_file() {
                if let Some(root) = path.parent() {
                    let declarations = parsed_launch(root, p, LaunchField::Path);
                    match_and_insert_game(catalog, prog_name, root, &declarations, map, Provider::Windows, None);
                }
            }
        }
    }
}

pub(crate) fn is_known_non_game_software(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("update")
        || lower.contains("redistributable")
        || lower.contains("driver")
        || lower.contains("visual c++")
        || lower.contains("directx")
        || lower.contains("sdk")
        || lower.contains("microsoft")
        || lower.contains("windows")
        || lower.contains("security")
        || lower.contains("antivirus")
        || lower.contains("vpn")
        || lower.contains("launcher")
        || lower.contains("game center")
        || lower.contains("anti-cheat")
        || lower.contains("service")
        || lower == "steam"
        || lower == "gog galaxy"
        || lower == "ubisoft connect"
        || lower == "epic games launcher"
        // Hardware & system monitoring utilities
        || lower.contains("aida64")
        || lower.contains("ryzen master")
        || lower.contains("amd software")
        || lower.contains("nvidia")
        || lower.contains("geforce")
        || lower.contains("intel")
        || lower.contains("corsair")
        || lower.contains("icue")
        || lower.contains("razer")
        || lower.contains("synapse")
        || lower.contains("logitech")
        || lower.contains("g hub")
        || lower.contains("afterburner")
        || lower.contains("rivatuner")
        || lower.contains("hwmonitor")
        || lower.contains("cpu-z")
        || lower.contains("gpu-z")
        || lower.contains("crystaldisk")
        || lower.contains("furmark")
        || lower.contains("cinebench")
        || lower.contains("armoury crate")
        || lower.contains("gigabyte")
        || lower.contains("asus")
        || lower.contains("msi center")
        || lower.contains("nzxt")
        || lower.contains("realtek")
        // Productivity, developer tools & utilities
        || lower.contains("visual studio")
        || lower.contains("vscode")
        || lower.contains("git")
        || lower.contains("python")
        || lower.contains("node.js")
        || lower.contains("docker")
        || lower.contains("libreoffice")
        || lower.contains("office")
        || lower.contains("excel")
        || lower.contains("word")
        || lower.contains("powerpoint")
        || lower.contains("adobe")
        || lower.contains("acrobat")
        || lower.contains("photoshop")
        || lower.contains("premiere")
        || lower.contains("7-zip")
        || lower.contains("winrar")
        || lower.contains("notepad++")
        || lower.contains("sublime")
        || lower.contains("vmware")
        || lower.contains("virtualbox")
        // Browsers & communication
        || lower.contains("chrome")
        || lower.contains("firefox")
        || lower.contains("edge")
        || lower.contains("brave")
        || lower.contains("opera")
        || lower.contains("discord")
        || lower.contains("slack")
        || lower.contains("teams")
        || lower.contains("zoom")
        || lower.contains("telegram")
        || lower.contains("whatsapp")
        || lower.contains("spotify")
        || lower.contains("obs studio")
        || lower.contains("streamlabs")
}

fn is_generic_software_binary(exe: &str) -> bool {
    let lower = exe.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "setup.exe"
            | "installer.exe"
            | "install.exe"
            | "uninstall.exe"
            | "unins000.exe"
            | "update.exe"
            | "patch.exe"
            | "helper.exe"
            | "launcher.exe"
            | "crashreporter.exe"
            | "unitycrashhandler64.exe"
            | "unitycrashhandler32.exe"
    )
}

fn is_valid_windows_game(hint_name: &str, game_dir: &Path) -> bool {
    let dir_str = game_dir.to_string_lossy().to_ascii_lowercase();
    let hint_str = hint_name.to_ascii_lowercase();

    if is_known_non_game_software(&hint_str) || is_known_non_game_software(&dir_str) {
        return false;
    }

    let positive_patterns = [
        r"\games\", r"\spiele\", r"\hry\", r"\jeux\", r"\igry\",
        r"\ea games\", r"\electronic arts\", r"\origin games\",
        r"\ubisoft\", r"\battle.net\", r"\blizzard\",
        r"\riot games\", r"\epic games\", r"\gog galaxy\", r"\gog games\",
        r"\rockstar games\", r"\bethesda\", r"\warframe\",
        r"\genshin", r"\hoyoverse", r"\daum\",
    ];
    if positive_patterns.iter().any(|needle| dir_str.contains(needle)) {
        return true;
    }

    if let Some(folder_name) = game_dir.file_name().and_then(|n| n.to_str()) {
        let f_lower = folder_name.to_ascii_lowercase();
        if f_lower.contains("game") || f_lower.contains("spiel") {
            return true;
        }
    }

    let parent = game_dir.parent().map(|p| p.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let is_generic_program_files = parent.ends_with(r"\program files") || parent.ends_with(r"\program files (x86)");
    !is_generic_program_files
}

// --------------------------------------------------------------------------------------
// Matching and Insertion Helpers
// --------------------------------------------------------------------------------------
fn match_and_insert_game(
    catalog: &[database::CatalogEntry],
    hint_name: &str,
    game_dir: &Path,
    declarations: &[String],
    map: &mut ScanCollection,
    provider: Provider,
    product_id: Option<&str>,
) {
    if declarations.is_empty() && provider != Provider::Steam {
        return;
    }
    let evidence = match InstallEvidence::observe(provider, product_id, game_dir, declarations) {
        Ok(evidence) => evidence,
        Err(error) => {
            eprintln!("Skipping {} installation {}: {error}", provider.launcher(), game_dir.display());
            return;
        }
    };
    let app = match automatic_authority::resolve(catalog, Some(&evidence)) {
        Authority::Resolved(resolved) => {
            let app = resolved.as_app(true);
            if !map.verified.iter().any(|known| {
                known.provider == resolved.provider
                    && known.product_id == resolved.product_id
                    && known.catalog.authoritative_primary() == resolved.catalog.authoritative_primary()
                    && known.executables == resolved.executables
            }) {
                map.verified.push(resolved);
            }
            app
        }
        Authority::Unresolved if matches!(provider, Provider::Steam | Provider::Xbox | Provider::Epic | Provider::Gog | Provider::Windows) => {
            if provider == Provider::Steam && product_id.is_none() {
                return;
            }
            if provider == Provider::Windows && !is_valid_windows_game(hint_name, game_dir) {
                return;
            }
            let selected = if provider == Provider::Steam {
                evidence.candidate_executable(hint_name)
            } else {
                evidence.manual_suggestion()
            };
            let Some(selected) = selected else {
                eprintln!("No unique declared executable for {}", game_dir.display());
                return;
            };
            if is_generic_software_binary(&selected.basename) {
                return;
            }
            HdrApp {
                name: hint_name.trim().to_owned(),
                exe_name: selected.basename,
                enabled: false,
                hdr_type: HdrType::Custom,
                path: Some(crate::runtime_policy::clean_path_display(&selected.path)),
                alternate_exes: Vec::new(),
                steam_id: (provider == Provider::Steam).then(|| product_id.map(str::to_owned)).flatten(),
                launcher: Some(provider.launcher().into()),
            }
        }
        result => {
            eprintln!("Unresolved automatic {} authority for {}: {result:?}", provider.launcher(), game_dir.display());
            return;
        }
    };
    // Detection is not saved-row association: same titles on different storefronts stay separate.
    let key = format!("{:?}|{}|{}|{}", provider, product_id.unwrap_or_default(), app.exe_name, app.path.as_deref().unwrap_or_default().to_lowercase());
    map.detected.entry(key).and_modify(|existing| {
        if app.name < existing.name {
            *existing = app.clone();
        }
    }).or_insert(app);
}

fn parsed_launch(root: &Path, raw: &str, field: LaunchField) -> Vec<String> {
    if raw.is_empty() {
        return Vec::new();
    }
    match automatic_authority::launch_declaration(root, raw, field) {
        Ok(declaration) => vec![declaration],
        Err(error) => {
            eprintln!("Ignoring invalid provider launch field for {}: {error}", root.display());
            Vec::new()
        }
    }
}

fn valid_steam_install_dir(value: &str) -> bool {
    !value.contains(['\\', '/'])
        && automatic_authority::normalize_relative_exe(&format!("{value}.exe")).is_ok()
        && !value.is_empty()
        && value != "."
        && value != ".."
        && !value.ends_with(['.', ' '])
}

fn windows_fallback_root(root: &Path) -> bool {
    // Registry fallback cannot reinterpret a known Steam/Xbox install as generic Windows
    // authority when that storefront's manifest, binding, or config did not resolve.
    !root.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name.eq_ignore_ascii_case("steamapps") || name.eq_ignore_ascii_case("xboxgames")
    })
}

// --------------------------------------------------------------------------------------
// String & Title Helpers
// --------------------------------------------------------------------------------------

fn strip_editions(s: &str) -> String {
    let mut lower = s.to_lowercase();
    let editions = [
        "director's cut",
        "directors cut",
        "game of the year edition",
        "goty edition",
        "goty",
        "enhanced edition",
        "definitive edition",
        "ultimate edition",
        "digital deluxe edition",
        "deluxe edition",
        "complete edition",
        "royal edition",
        "anniversary edition",
        "special edition",
        "remastered",
        "remaster",
        "standard edition",
        "gold edition",
        "legendary edition",
        "vr edition",
    ];
    for ed in editions {
        lower = lower.replace(ed, " ");
    }
    lower
}

fn extract_title_base(title: &str) -> &str {
    if let Some(pos) = title.find(':') {
        return &title[..pos];
    }
    if let Some(pos) = title.find(" - ") {
        return &title[..pos];
    }
    if let Some(pos) = title.find(" – ") {
        return &title[..pos];
    }
    if let Some(pos) = title.find(" — ") {
        return &title[..pos];
    }
    title
}

pub fn is_title_match(cat_name: &str, candidate_name: &str) -> bool {
    // If either name contains " / " (e.g. bilingual Steam Capcom titles), test both full and primary part
    if let Some(pos) = candidate_name.find(" / ") {
        if is_title_match(cat_name, candidate_name[..pos].trim()) {
            return true;
        }
    }
    if let Some(pos) = cat_name.find(" / ") {
        if is_title_match(cat_name[..pos].trim(), candidate_name) {
            return true;
        }
    }

    let norm_cat = normalize_game_title(cat_name);
    let norm_cand = normalize_game_title(candidate_name);

    if norm_cat.is_empty() || norm_cand.is_empty() {
        return false;
    }

    // 1. Direct equality on normalized titles
    if norm_cat == norm_cand {
        return true;
    }

    // 2. Direct equality after stripping edition suffixes (e.g. "Director's Cut", "Remastered", "GOTY")
    let stripped_cat = normalize_game_title(&strip_editions(cat_name));
    let stripped_cand = normalize_game_title(&strip_editions(candidate_name));

    if !stripped_cat.is_empty() && !stripped_cand.is_empty() && stripped_cat == stripped_cand {
        return true;
    }

    // 3. One title includes a subtitle (e.g. "The Witcher 3: Wild Hunt"), while the other is just the base ("The Witcher 3").
    // Never match if both have subtitles with different endings (e.g. "Star Wars: Squadrons" vs "Star Wars: Outlaws")!
    let cat_base = normalize_game_title(&strip_editions(extract_title_base(cat_name)));
    let cand_base = normalize_game_title(&strip_editions(extract_title_base(candidate_name)));

    let cat_has_sub = cat_base != stripped_cat;
    let cand_has_sub = cand_base != stripped_cand;

    if cat_has_sub != cand_has_sub {
        if cat_has_sub && cat_base.len() >= 4 && cat_base == stripped_cand {
            return true;
        }
        if cand_has_sub && cand_base.len() >= 4 && cand_base == stripped_cat {
            return true;
        }
    }

    false
}

fn strip_year_parens(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '(' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j + 4 <= chars.len() && chars[j..j + 4].iter().all(|c| c.is_ascii_digit()) {
                let mut k = j + 4;
                while k < chars.len() && chars[k].is_whitespace() {
                    k += 1;
                }
                if k < chars.len() && chars[k] == ')' {
                    i = k + 1;
                    continue;
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

fn normalize_game_title(s: &str) -> String {
    let stripped = strip_year_parens(s);
    let mut lower = stripped.to_lowercase();

    // Normalize Roman numerals commonly used in game titles
    lower = lower
        .replace(" viii", " 8")
        .replace(" vii", " 7")
        .replace(" vi", " 6")
        .replace(" iv", " 4")
        .replace(" v", " 5")
        .replace(" iii", " 3")
        .replace(" ii", " 2");

    clean_string(&lower)
}


fn clean_string(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn clean_title_from_folder(folder: &str) -> String {
    folder
        .replace('_', " ")
        .replace('-', " ")
        .replace("Win64", "")
        .replace("Shipping", "")
        .trim()
        .to_string()
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn read_registry_string(root: HKEY, subkey: &str, value_name: &str) -> Result<String, ()> {
    let subkey_wide = to_wide(subkey);
    let val_name_wide = to_wide(value_name);
    let mut h_key = HKEY::default();

    unsafe {
        if RegOpenKeyExW(root, PCWSTR(subkey_wide.as_ptr()), Some(0), KEY_READ, &mut h_key) != WIN32_ERROR(0) {
            return Err(());
        }

        let mut buf = [0u16; 1024];
        let mut buf_size = (buf.len() * 2) as u32;
        let mut val_type = REG_SZ;

        let status = RegQueryValueExW(
            h_key,
            PCWSTR(val_name_wide.as_ptr()),
            None,
            Some(&mut val_type),
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut buf_size),
        );

        let _ = RegCloseKey(h_key);

        if status == WIN32_ERROR(0) {
            let len = (buf_size / 2) as usize;
            let end = buf[..len].iter().position(|&c| c == 0).unwrap_or(len);
            Ok(String::from_utf16_lossy(&buf[..end]))
        } else {
            Err(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_files(root: &Path, files: &[&str]) {
        for name in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"fixture, never executed").unwrap();
        }
    }

    #[test]
    fn scanner_never_guesses_aoe4_helper_rows_or_aliases_and_repeats_without_churn() {
        let catalog = database::get_full_catalog();
        let root = tempfile::tempdir().unwrap();
        create_files(root.path(), &["BsSndRpt.exe", "BsSndRpt64.exe", "BugSplat.exe", "editor.exe", "tool.exe"]);
        let mut map = ScanCollection::default();
        match_and_insert_game(&catalog, "Age of Empires IV", root.path(), &[], &mut map, Provider::Steam, Some("1466860"));
        assert!(map.detected.is_empty());
        assert!(map.verified.is_empty());
        let binding = database::find_storefront_binding(&catalog, database::StorefrontProvider::Steam, Some("1466860")).unwrap().unwrap().1;
        create_files(root.path(), &[&binding.game_executables[0]]);
        match_and_insert_game(&catalog, "Age of Empires IV", root.path(), &[], &mut map, Provider::Steam, Some("1466860"));
        assert_eq!(map.detected.len(), 1);
        assert_eq!(map.verified.len(), 1);
        let app = map.detected.values().next().unwrap();
        assert_eq!(app.exe_name, binding.game_executables[0]);
        assert!(app.alternate_exes.is_empty());
        let before = map.detected.clone();
        match_and_insert_game(&catalog, "Changed display title", root.path(), &[], &mut map, Provider::Steam, Some("1466860"));
        assert_eq!(map.detected, before);
        assert_eq!(map.verified.len(), 1);
    }

    #[test]
    fn every_provider_ignores_recursive_candidates_and_nonhdr_launch_is_only_a_suggestion() {
        let root = tempfile::tempdir().unwrap();
        create_files(root.path(), &["unlisted.exe", "re7.exe", "tool.exe"]);
        let catalog = database::get_full_catalog();
        for provider in [Provider::Steam, Provider::Xbox, Provider::Epic, Provider::Gog, Provider::Windows] {
            let mut map = ScanCollection::default();
            match_and_insert_game(&catalog, "Resident Evil 7: Biohazard", root.path(), &[], &mut map, provider, None);
            assert!(map.detected.is_empty(), "{provider:?}");
            assert!(map.verified.is_empty());
            if matches!(provider, Provider::Epic | Provider::Gog | Provider::Windows) {
                match_and_insert_game(&catalog, "Resident Evil 7: Biohazard", root.path(), &["unlisted.exe".into()], &mut map, provider, None);
                let app = map.detected.values().next().unwrap();
                assert!(!app.enabled);
                assert_eq!(app.hdr_type, HdrType::Custom);
                assert_eq!(app.exe_name, "unlisted.exe");
                assert!(app.alternate_exes.is_empty());
                assert!(map.verified.is_empty());
            }
        }
    }

    #[test]
    fn same_title_storefront_detections_do_not_merge_or_inherit_executables() {
        let root = tempfile::tempdir().unwrap();
        create_files(root.path(), &["steam.exe", "xbox.exe", "global.exe"]);
        let catalog = database::authored_test_catalog(vec![serde_json::from_value(serde_json::json!({
            "name": "Shared title", "exe_name": "steam.exe", "steam_id": "123",
            "hdr_type": "native", "support_tier": "native", "alternate_exes": ["global.exe"],
            "storefronts": [{"provider": "xbox", "game_executables": ["xbox.exe"]}]
        })).unwrap()]);
        let mut map = ScanCollection::default();
        match_and_insert_game(&catalog, "Shared title", root.path(), &[], &mut map, Provider::Steam, Some("123"));
        match_and_insert_game(&catalog, "Shared title", root.path(), &["xbox.exe".into()], &mut map, Provider::Xbox, None);
        assert_eq!(map.detected.len(), 2);
        assert_eq!(map.verified.len(), 2);
        assert!(map.detected.values().all(|app| app.alternate_exes.is_empty()));
        assert!(map.detected.values().any(|app| app.exe_name == "xbox.exe" && app.steam_id.is_none()));
    }

    #[test]
    fn correction_cache_scanner_never_authorizes_deserialized_suggestion_rows() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        create_files(root.path(), &["fetched.exe", "fetched-alternate.exe"]);
        let catalog = vec![serde_json::from_value(serde_json::json!({
            "name": "Fetched title", "exe_name": "fetched.exe", "steam_id": "123",
            "hdr_type": "native", "support_tier": "native",
            "alternate_exes": ["fetched-alternate.exe"]
        })).unwrap()];
        for provider in [Provider::Epic, Provider::Gog, Provider::Windows] {
            for exe in ["fetched.exe", "fetched-alternate.exe"] {
                let mut scan = ScanCollection::default();
                match_and_insert_game(
                    &catalog, "Fetched title", root.path(), &[exe.into()], &mut scan, provider, None,
                );
                assert!(scan.verified.is_empty(), "{provider:?}: {exe} became verified without source authority");
                assert!(scan.detected.values().all(|app| !app.enabled && app.hdr_type == HdrType::Custom));
            }
        }
    }

    #[test]
    fn correction_declared_scanner_emits_neither_verified_rows_nor_recursive_manual_suggestions() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        create_files(root.path(), &[r"Tools\game.exe"]);
        let catalog = database::authored_test_catalog(vec![serde_json::from_value(serde_json::json!({
            "name": "Game", "exe_name": "game.exe", "hdr_type": "native", "support_tier": "native"
        })).unwrap()]);
        for provider in [Provider::Epic, Provider::Gog, Provider::Windows] {
            for entries in [&catalog[..], &[][..]] {
                let mut scan = ScanCollection::default();
                match_and_insert_game(
                    entries, "Game", root.path(), &["game.exe".into()], &mut scan, provider, None,
                );
                assert!(scan.detected.is_empty(), "{provider:?}: nonexistent root declaration emitted a row");
                assert!(scan.verified.is_empty());
            }
        }
    }

    #[test]
    fn correction_declared_xbox_config_root_filename_cannot_select_tools_aoe3() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        create_files(root.path(), &[r"Tools\AoE3DE.exe"]);
        fs::write(root.path().join("MicrosoftGame.config"),
            r#"<Game><ExecutableList><Executable Name="AoE3DE.exe"/></ExecutableList></Game>"#,
        ).unwrap();
        let catalog: Vec<database::CatalogEntry> = serde_json::from_str(include_str!("../catalog.json")).unwrap();
        let catalog = database::authored_test_catalog(catalog);
        let declarations = crate::xbox_config::declarations(root.path()).unwrap();
        let mut scan = ScanCollection::default();
        match_and_insert_game(
            &catalog, "Age of Empires III", root.path(), &declarations, &mut scan, Provider::Xbox, None,
        );
        assert!(scan.detected.is_empty(), "An Xbox root declaration selected Tools\\AoE3DE.exe");
        assert!(scan.verified.is_empty());
    }

    #[test]
    fn manual_picker_rejects_permanent_helpers_before_catalog_title_matching() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let game_dir = root.path().join("Age of Empires IV");
        create_files(&game_dir, &["BsSndRpt.exe", "BsSndRpt64.exe", "BugSplat.exe", "GameLaunchHelper.exe"]);
        for exe in ["BsSndRpt.exe", "BsSndRpt64.exe", "BugSplat.exe", "GameLaunchHelper.exe"] {
            let error = inspect_exe_path(game_dir.join(exe).to_str().unwrap()).unwrap_err();
            assert!(error.contains("helper"), "{error}");
            assert!(error.contains(exe), "{error}");
        }
    }

    #[test]
    fn xbox_aoe3_scan_retains_provenance_for_explicit_import_and_canonical_enrichment() {
        use crate::config::AppConfig;
        use crate::library;
        let catalog = database::get_full_catalog();
        let canonical = catalog.iter().find(|entry| entry.name == "Age of Empires III: Definitive Edition").unwrap();
        assert_eq!(canonical.exe_name, "ageofempiresiiidefinitiveedition.exe");
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let install = root.path().join(r"XboxGames\Age of Empires III");
        create_files(&install, &[r"Content\AoE3DE.exe", r"Content\GameLaunchHelper.exe", r"Content\BsSndRpt.exe"]);
        fs::write(install.join(r"Content\MicrosoftGame.config"), r#"<Game><ExecutableList><Executable Name="GameLaunchHelper.exe"/><Executable Name="AoE3DE.exe"/><Executable Name="BsSndRpt.exe"/></ExecutableList></Game>"#).unwrap();
        let declarations = crate::xbox_config::declarations(&install).unwrap();
        let mut scan = ScanCollection::default();
        match_and_insert_game(&catalog, "User-facing title", &install, &declarations, &mut scan, Provider::Xbox, None);
        assert_eq!(scan.detected.len(), 1);
        assert_eq!(scan.verified.len(), 1);
        let app = scan.detected.values().next().unwrap().clone();
        assert_eq!(app.exe_name, "aoe3de.exe");
        assert!(app.alternate_exes.is_empty());
        assert!(app.steam_id.is_none());
        assert_eq!(app.launcher.as_deref(), Some("Xbox"));
        let mut explicit = AppConfig::default();
        explicit.apps.clear();
        library::import_games(&mut explicit, vec![app.clone()]).unwrap();
        assert_eq!(explicit.apps, [app.clone()]);

        let mut saved = app;
        saved.exe_name = canonical.exe_name.clone();
        saved.name = "My AOE3".into();
        saved.hdr_type = HdrType::Custom;
        saved.enabled = false;
        saved.launcher = None;
        saved.path = None;
        let mut startup = AppConfig::default();
        startup.apps = vec![saved.clone()];
        assert!(library::enrich_verified_aliases(&mut startup, &scan.verified));
        saved.alternate_exes.push("aoe3de.exe".into());
        assert_eq!(startup.apps, [saved.clone()]);
        assert!(!library::enrich_verified_aliases(&mut startup, &scan.verified));
        assert_eq!(startup.apps, [saved]);

        let mut injected = canonical.clone();
        injected.exe_name = "age3y.exe".into();
        let mut alternate_catalog = ScanCollection::default();
        match_and_insert_game(&database::authored_test_catalog(vec![injected]), "Irrelevant title", &install, &declarations, &mut alternate_catalog, Provider::Xbox, None);
        startup.apps[0].exe_name = "age3y.exe".into();
        startup.apps[0].alternate_exes.clear();
        assert!(library::enrich_verified_aliases(&mut startup, &alternate_catalog.verified));
        assert_eq!(startup.apps[0].alternate_exes, ["aoe3de.exe"]);
    }

    #[test]
    fn steam_install_folder_is_a_single_safe_component() {
        for invalid in ["", ".", "..", r"..\outside", r"C:\outside", r"\\server\share", "game.", "game ", "NUL"] {
            assert!(!valid_steam_install_dir(invalid), "{invalid}");
        }
        assert!(valid_steam_install_dir("Age of Empires IV"));
    }

    #[test]
    fn windows_registry_fallback_cannot_bypass_steam_or_xbox_authority() {
        for path in [r"C:\Steam\steamapps\common\Game", r"D:\XboxGames\Game\Content"] {
            assert!(!windows_fallback_root(Path::new(path)));
        }
        assert!(windows_fallback_root(Path::new(r"D:\Games\Standalone")));
    }

    #[test]
    fn test_assetto_corsa_does_not_match_competizione() {
        assert!(!is_title_match("Assetto Corsa Competizione", "Assetto Corsa"));
        assert!(!is_title_match("Assetto Corsa", "Assetto Corsa Competizione"));
        assert!(is_title_match("Assetto Corsa", "Assetto Corsa"));
        assert!(is_title_match("Assetto Corsa Competizione", "Assetto Corsa Competizione"));
    }

    #[test]
    fn test_editions_and_subtitles() {
        assert!(is_title_match("Death Stranding Director's Cut", "Death Stranding"));
        assert!(is_title_match("The Witcher 3: Wild Hunt", "The Witcher 3"));
        assert!(is_title_match("Cyberpunk 2077: Phantom Liberty", "Cyberpunk 2077"));
        assert!(is_title_match("Ghost of Tsushima DIRECTOR'S CUT", "Ghost of Tsushima"));
    }

    #[test]
    fn test_sequels_do_not_match() {
        assert!(!is_title_match("Doom Eternal", "Doom"));
        assert!(!is_title_match("Marvel's Spider-Man Remastered", "Marvel's Spider-Man 2"));
        assert!(!is_title_match("Alan Wake 2", "Alan Wake"));
        assert!(!is_title_match("Star Wars: Squadrons", "Star Wars: Outlaws"));
    }

    #[test]
    fn test_bilingual_capcom_titles() {
        assert!(is_title_match(
            "Resident Evil 7: Biohazard",
            "RESIDENT EVIL 7 biohazard / BIOHAZARD 7 resident evil"
        ));
        assert!(is_title_match(
            "RESIDENT EVIL 7 biohazard / BIOHAZARD 7 resident evil",
            "Resident Evil 7: Biohazard"
        ));
    }

    #[test]
    fn test_year_parens_stripping() {
        assert!(is_title_match("Silent Hill 2 (2024)", "SILENT HILL 2"));
        assert!(is_title_match("SILENT HILL 2", "Silent Hill 2 (2024)"));
        assert!(is_title_match("Alone in the Dark (2024)", "Alone in the Dark"));
    }

    #[test]
    fn test_alternate_exes_and_catalog_lookup() {
        let cat = database::get_full_catalog();

        // Check Silent Hill 2
        let sh2 = cat.iter().find(|c| c.steam_id.as_deref() == Some("2124490"));
        assert!(sh2.is_some(), "Silent Hill 2 must exist with Steam AppID 2124490");
        let sh2_entry = sh2.unwrap();
        assert_eq!(sh2_entry.hdr_type, HdrType::Native);

        // Check lookup by alternate or main exes
        let by_shipping = database::find_in_catalog("shproto-win64-shipping.exe");
        assert!(by_shipping.is_some(), "Should find SH2 by shproto-win64-shipping.exe");

        let by_eos = database::find_in_catalog("game_f_x64_eos.exe");
        assert!(by_eos.is_some(), "Should find Alan Wake Remastered by game_f_x64_eos.exe");

        let by_goty = database::find_in_catalog("borderlandsgoty.exe");
        assert!(by_goty.is_some(), "Should find Borderlands GOTY by borderlandsgoty.exe");

        let by_re7 = database::find_in_catalog("re7.exe");
        assert!(by_re7.is_some(), "Should find Resident Evil 7 by re7.exe");
    }

    #[test]
    fn steam_sdr_game_is_suggested_with_auto_off_and_clean_path() {
        let root = tempfile::tempdir().unwrap();
        create_files(root.path(), &["sdrgame.exe", "unins000.exe", "UnityCrashHandler64.exe"]);
        let catalog = database::get_full_catalog();
        let mut map = ScanCollection::default();
        match_and_insert_game(&catalog, "SDR Game Title", root.path(), &[], &mut map, Provider::Steam, Some("999999"));
        assert_eq!(map.detected.len(), 1);
        assert!(map.verified.is_empty());
        let app = map.detected.values().next().unwrap();
        assert_eq!(app.name, "SDR Game Title");
        assert_eq!(app.exe_name, "sdrgame.exe");
        assert!(!app.enabled);
        assert_eq!(app.hdr_type, HdrType::Custom);
        assert_eq!(app.launcher.as_deref(), Some("Steam"));
        assert_eq!(app.steam_id.as_deref(), Some("999999"));
        assert!(!app.path.as_deref().unwrap().starts_with(r"\\?\"));
    }

    #[test]
    fn windows_registry_scanner_ignores_non_game_desktop_software() {
        assert!(is_known_non_game_software("AIDA64 Extreme v8.20"));
        assert!(is_known_non_game_software("AMD Ryzen Master"));
        assert!(is_known_non_game_software("7-Zip 23.01 (x64)"));
        assert!(is_known_non_game_software("Discord"));
        assert!(is_known_non_game_software("Google Chrome"));
        assert!(is_known_non_game_software("OBS Studio"));
        assert!(!is_known_non_game_software("Borderlands: Game of the Year Enhanced"));
        assert!(!is_known_non_game_software("Cyberpunk 2077"));
    }
}
