//! The v2 controller lock cannot exclude v1. Only registered installations are
//! supported predecessors; an executable basename is never ownership evidence.
//!
//! See windows\installer-contract.json for the inspected v1.0.5 bundle identities.
//! No function in this module sends WM_CLOSE or terminates another process.

use std::sync::{Mutex, MutexGuard};
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

const PRODUCT_NAME: &str = "HDR Auto-Switch";
const PUBLISHER: &str = "soptik";
const MAIN_BINARY: &str = "tauri-app.exe";
const OWNED_STARTUP_NAME: &str = "com.soptik.hdr-autoswitch";
const LEGACY_STARTUP_NAME: &str = "tauri-app";
const MSI_UPGRADE_CODE: &str = "{30B00949-76AA-5C71-B9AA-0599499AC6DE}";
const MSI_PATH_COMPONENT: &str = "{D180B15C-C144-5CFE-B17A-24E2E760016B}";
const QUIT_INSTRUCTION: &str =
    "Open HDR Auto-Switch's notification-area (tray) menu and choose Quit/Exit, \
     then retry. Closing its window only hides it. HDR control remains paused \
     until the installed predecessor's exit can be verified.";

static STARTUP_TRANSACTION: Mutex<()> = Mutex::new(());

/// Read-only, same-interactive-user check. An error must close every HDR setter
/// authorization, including manual operations and ownership cleanup. Rechecking,
/// not dismissing the error, is the only way to clear that controller issue.
pub fn check_predecessor() -> Result<(), String> {
    platform::check_predecessor()
}

/// Changes only verified, current-user startup registrations, never app config.
/// A failed multi-value change may have partially applied: callers must surface
/// the error and reconcile through `autostart_enabled`, not report a saved setting.
pub fn configure_autostart(enable: bool) -> Result<(), String> {
    apply_autostart_change(enable).map(drop)
}

/// A single-use, exact-byte undo receipt. Keep it until the canonical commit
/// completes while also holding the caller's config/history action lock.
/// Dropping it after success commits the startup change; rollback is explicit
/// so its failure can be reported. Do not hold it across an async await.
#[must_use = "Keep the startup receipt until config commits; call rollback on failure"]
pub struct AutostartChange {
    edits: Vec<StartupEdit>,
    _transaction: MutexGuard<'static, ()>,
}

impl AutostartChange {
    /// Revalidates installation ownership and refuses to replace concurrently
    /// changed Run values. Windows Startup Apps overrides remain untouched.
    pub fn rollback(self) -> Result<(), String> {
        platform::rollback_autostart_change(&self.edits)
    }
}

/// Applies and verifies owned startup edits, returning an exact undo receipt.
/// A rejected request performs no writes; a partially applied failure attempts
/// checked restoration internally and reports any restoration failure.
pub fn apply_autostart_change(enable: bool) -> Result<AutostartChange, String> {
    let transaction = STARTUP_TRANSACTION
        .lock()
        .map_err(|_| "Startup registration transaction is unavailable".to_owned())?;
    let edits = platform::apply_autostart_change(enable)?;
    Ok(AutostartChange {
        edits,
        _transaction: transaction,
    })
}

/// Applies owned startup changes, calls the supplied canonical commit, and on
/// failure restores only the exact Run values changed by this transaction.
///
/// The caller still owns config/history serialization and the commit itself.
/// Do not approximate rollback with `configure_autostart(previous_bool)`: that
/// loses disabled registrations, duplicate ownership evidence and exact bytes.
/// This helper holds the startup mutex; the closure must not reenter this module.
pub fn configure_autostart_with_commit<T>(
    enable: bool,
    commit: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _guard = STARTUP_TRANSACTION
        .lock()
        .map_err(|_| "Startup registration transaction is unavailable".to_owned())?;
    platform::configure_autostart_with_commit(enable, commit)
}

/// Reports effective, verified-owned startup registration. A conflicting stable
/// name, inaccessible metadata, or unknown Windows approval state is an error.
/// This status boolean is not an undo receipt; use `AutostartChange` for rollback.
pub fn autostart_enabled() -> Result<bool, String> {
    let _guard = STARTUP_TRANSACTION
        .lock()
        .map_err(|_| "Startup registration transaction is unavailable".to_owned())?;
    platform::autostart_enabled()
}

/// Call at the very beginning of `run`, BEFORE creating a Tauri builder, plugins,
/// config stores, controller locks, windows, hooks, or actors. `Some(code)` means
/// the caller must exit with that code and must not initialize the application.
///
/// Official installers use the executable only in these helper modes, including
/// retained NSIS registry retirement/recovery after payload deletion.
/// Full-UI installs offer Retry/Cancel; unattended installs fail instead of killing
/// processes. A preflight is not perpetual exclusion of later legacy launches.
pub fn installer_command() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let command = args.first()?.to_str()?;
    if !matches!(command, "--hdr-installer-preflight" | "--hdr-installer-uninstall"
        | "--hdr-installer-retirement-prepare" | "--hdr-installer-retirement-commit"
        | "--hdr-installer-retirement-restore") {
        return None;
    }
    let result = if args.len() != 4 {
        Err("Invalid HDR Auto-Switch installer preflight arguments".to_owned())
    } else {
        let format = args[1].to_str().and_then(InstallerKind::parse);
        let full_ui = args[3].to_str() == Some("5");
        match format {
            Some(InstallerKind::Nsis) if command.starts_with("--hdr-installer-retirement-") =>
                platform::nsis_retirement(command, std::path::Path::new(&args[2])),
            Some(_) if command.starts_with("--hdr-installer-retirement-") =>
                Err("Registry retirement is only supported for the owned NSIS installation".into()),
            Some(format) => platform::installer_handoff(
                format,
                std::path::Path::new(&args[2]),
                command == "--hdr-installer-uninstall",
                full_ui,
            ),
            None => Err("Unrecognized HDR Auto-Switch installer format".to_owned()),
        }
    };
    Some(match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("HDR Auto-Switch installer blocked: {error}");
            1
        }
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstallerKind {
    Nsis,
    Msi,
}

impl InstallerKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "nsis" => Some(Self::Nsis),
            "msi" => Some(Self::Msi),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct RegistryValue {
    kind: u32,
    bytes: Vec<u8>,
}

type RegistryValues = BTreeMap<String, RegistryValue>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NsisRegistrationSnapshot {
    view: u32,
    uninstall: RegistryValues,
    product_path: RegistryValue,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NsisRetirementReceipt {
    schema: u32,
    target: String,
    registrations: Vec<NsisRegistrationSnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum RetirementKey {
    Uninstall,
    Product,
}

trait RetirementRegistry {
    fn read(&mut self, key: RetirementKey, view: u32) -> Result<Option<RegistryValues>, String>;
    fn delete_uninstall(&mut self, view: u32) -> Result<(), String>;
    fn delete_product_path(&mut self, view: u32) -> Result<(), String>;
    fn write(&mut self, key: RetirementKey, view: u32, name: &str, value: &RegistryValue)
        -> Result<(), String>;
}

fn product_path(values: &Option<RegistryValues>) -> Option<&RegistryValue> {
    values.as_ref().and_then(|values| values.get(""))
}

fn retire_nsis_registration(
    registry: &mut impl RetirementRegistry,
    receipt: &NsisRetirementReceipt,
) -> Result<(), String> {
    // Precheck every view before any edit. The receipt was captured while both
    // cleanup executables still existed; their later absence is not authority.
    for entry in &receipt.registrations {
        if registry.read(RetirementKey::Uninstall, entry.view)?.as_ref() != Some(&entry.uninstall)
            || product_path(&registry.read(RetirementKey::Product, entry.view)?)
                != Some(&entry.product_path)
        {
            return Err("Installation metadata changed after recovery preparation; retirement refused".into());
        }
    }
    for entry in &receipt.registrations {
        if product_path(&registry.read(RetirementKey::Product, entry.view)?)
            .is_some_and(|value| value != &entry.product_path)
        {
            return Err("Product path changed before retirement".into());
        }
        registry.delete_product_path(entry.view)?;
        if product_path(&registry.read(RetirementKey::Product, entry.view)?).is_some() {
            return Err("Product-path retirement failed readback".into());
        }
        // HKCU registry views may alias. After the all-view precheck, a prior
        // deletion can legitimately make this view absent; read errors cannot.
        if let Some(actual) = registry.read(RetirementKey::Uninstall, entry.view)? {
            if actual != entry.uninstall {
                return Err("Uninstall registration changed before retirement".into());
            }
            registry.delete_uninstall(entry.view)?;
        }
        if registry.read(RetirementKey::Uninstall, entry.view)?.is_some() {
            return Err("Uninstall registration retirement failed readback".into());
        }
    }
    for entry in &receipt.registrations {
        if registry.read(RetirementKey::Uninstall, entry.view)?.is_some()
            || product_path(&registry.read(RetirementKey::Product, entry.view)?).is_some()
        {
            return Err("Installation metadata retirement was not confirmed".into());
        }
    }
    Ok(())
}

fn restore_nsis_registration(
    registry: &mut impl RetirementRegistry,
    receipt: &NsisRetirementReceipt,
) -> Result<(), String> {
    let mut errors = Vec::new();
    for entry in &receipt.registrations {
        let restore = (|| {
            if product_path(&registry.read(RetirementKey::Product, entry.view)?)
                .is_some_and(|value| value != &entry.product_path)
            {
                return Err("Product path has a concurrent replacement; recovery refused".into());
            }
            for (name, expected) in &entry.uninstall {
                let actual = registry.read(RetirementKey::Uninstall, entry.view)?.unwrap_or_default();
                // Allow our own incomplete restoration to be retried, but never
                // overwrite even one conflicting or newly added registry value.
                if actual.iter().any(|(name, value)| entry.uninstall.get(name) != Some(value)) {
                    return Err("Uninstall metadata has a concurrent replacement; recovery refused".into());
                }
                if !actual.contains_key(name) {
                    registry.write(RetirementKey::Uninstall, entry.view, name, expected)?;
                }
            }
            if registry.read(RetirementKey::Uninstall, entry.view)?.as_ref() != Some(&entry.uninstall) {
                return Err("Uninstall metadata restoration failed readback".into());
            }
            let actual = registry.read(RetirementKey::Product, entry.view)?;
            match product_path(&actual) {
                Some(value) if value != &entry.product_path =>
                    return Err("Product path has a concurrent replacement; recovery refused".into()),
                Some(_) => {}
                None => registry.write(RetirementKey::Product, entry.view, "", &entry.product_path)?,
            }
            if product_path(&registry.read(RetirementKey::Product, entry.view)?) != Some(&entry.product_path) {
                return Err("Product-path restoration failed readback".into());
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = restore {
            errors.push(error);
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

impl RegistryValue {
    fn string(value: &str) -> Self {
        Self {
            kind: 1, // REG_SZ
            bytes: value
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
        }
    }

    fn as_string(&self) -> Result<String, String> {
        if self.kind != 1 || self.bytes.len() < 2 || self.bytes.len() % 2 != 0 {
            return Err("Expected a terminated REG_SZ startup/installation value".to_owned());
        }
        let mut wide: Vec<_> = self
            .bytes
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect();
        if wide.pop() != Some(0) || wide.contains(&0) {
            return Err("Registry string contains an embedded/missing terminator".to_owned());
        }
        String::from_utf16(&wide).map_err(|_| "Registry string is not valid UTF-16".to_owned())
    }
}

/// auto-launch 0.5.0 actually wrote an UNQUOTED full path plus ` --minimized`.
/// Interpret only that exact serialization, or the corrected quoted equivalent;
/// ownership still requires resolving the entire path to installed metadata.
fn startup_executable(command: &str) -> Option<&str> {
    let path = command.strip_suffix(" --minimized")?;
    let path = if path.starts_with('"') {
        path.strip_prefix('"')?.strip_suffix('"')?
    } else {
        path
    };
    if path.is_empty() || path.contains(['"', '\0', '\r', '\n']) {
        return None;
    }
    let bytes = path.as_bytes();
    let absolute =
        bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    (absolute || path.starts_with(r"\\")).then_some(path)
}

fn startup_command(path: &str) -> Result<String, String> {
    let command = format!("\"{path}\" --minimized");
    if startup_executable(&command) != Some(path) {
        return Err("Cannot safely quote the installed startup executable path".to_owned());
    }
    Ok(command)
}

fn startup_approved(value: Option<&RegistryValue>) -> Result<bool, String> {
    match value {
        None => Ok(true),
        Some(value) if value.kind == 3 && value.bytes.len() == 12 => {
            let state = u32::from_le_bytes(value.bytes[..4].try_into().unwrap());
            match state {
                2 => Ok(true),
                3 => Ok(false),
                _ => Err("Unknown Windows Startup Apps approval state; inspect Startup Apps in Windows Settings".to_owned()),
            }
        }
        Some(_) => Err("Unrecognized Windows Startup Apps approval data".to_owned()),
    }
}

#[derive(Clone, Debug)]
struct StartupRegistration {
    value: RegistryValue,
    owned: bool,
    approved: bool,
}

#[derive(Clone, Debug, Default)]
struct StartupSnapshot {
    stable: Option<StartupRegistration>,
    legacy: Option<StartupRegistration>,
    stable_name_approved: bool,
}

impl StartupSnapshot {
    fn enabled(&self) -> Result<bool, String> {
        self.validate()?;
        Ok(self
            .stable
            .iter()
            .chain(self.legacy.iter())
            .any(|entry| entry.owned && entry.approved))
    }

    fn validate(&self) -> Result<(), String> {
        if self.stable.as_ref().is_some_and(|entry| !entry.owned) {
            return Err(format!(
                "HKCU Run entry '{OWNED_STARTUP_NAME}' is not proven to belong to an installed \
                 HDR Auto-Switch executable. It was left unchanged."
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StartupEdit {
    name: &'static str,
    expected: Option<RegistryValue>,
    desired: Option<RegistryValue>,
}

fn plan_startup(
    snapshot: &StartupSnapshot,
    enable: bool,
    target: &str,
) -> Result<Vec<StartupEdit>, String> {
    snapshot.validate()?;
    if enable && !snapshot.stable_name_approved {
        return Err(format!(
            "Windows Startup Apps has disabled '{OWNED_STARTUP_NAME}'. Enable it in Windows \
             Settings before enabling startup here; the Windows override was not changed."
        ));
    }
    if enable
        && snapshot.stable.is_none()
        && snapshot
            .legacy
            .as_ref()
            .is_some_and(|entry| entry.owned && !entry.approved)
    {
        return Err(
            "Windows Startup Apps has disabled the verified legacy HDR startup entry. \
             Enable that entry in Windows Settings before migrating to the new startup name; \
             the Windows override was not bypassed."
                .to_owned(),
        );
    }
    let desired = enable
        .then(|| startup_command(target).map(|command| RegistryValue::string(&command)))
        .transpose()?;
    let stable = snapshot.stable.as_ref().map(|entry| entry.value.clone());
    let mut edits = Vec::new();
    if stable != desired {
        edits.push(StartupEdit {
            name: OWNED_STARTUP_NAME,
            expected: stable,
            desired,
        });
    }
    if let Some(legacy) = snapshot.legacy.as_ref().filter(|entry| entry.owned) {
        edits.push(StartupEdit {
            name: LEGACY_STARTUP_NAME,
            expected: Some(legacy.value.clone()),
            desired: None,
        });
    }
    Ok(edits)
}

fn plan_installer_startup(
    snapshot: &StartupSnapshot,
    uninstall: bool,
    target: &str,
) -> Result<Vec<StartupEdit>, String> {
    snapshot.validate()?;
    if uninstall || snapshot.enabled()? {
        return plan_startup(snapshot, !uninstall, target);
    }
    // Renaming a Windows-disabled Run entry would bypass StartupApproved.
    // Keep one disabled, owned entry at the verified in-place destination.
    // A disabled legacy name migrates only after the user enables it in Windows.
    let desired = RegistryValue::string(&startup_command(target)?);
    let mut edits = Vec::new();
    let keep = snapshot
        .stable
        .as_ref()
        .map(|entry| (OWNED_STARTUP_NAME, entry))
        .or_else(|| {
            snapshot
                .legacy
                .as_ref()
                .filter(|entry| entry.owned)
                .map(|entry| (LEGACY_STARTUP_NAME, entry))
        });
    if let Some((name, entry)) = keep {
        if entry.value != desired {
            edits.push(StartupEdit {
                name,
                expected: Some(entry.value.clone()),
                desired: Some(desired),
            });
        }
        if name == OWNED_STARTUP_NAME {
            if let Some(legacy) = snapshot.legacy.as_ref().filter(|entry| entry.owned) {
                edits.push(StartupEdit {
                    name: LEGACY_STARTUP_NAME,
                    expected: Some(legacy.value.clone()),
                    desired: None,
                });
            }
        }
    }
    Ok(edits)
}

trait StartupWriter {
    fn read(&mut self, name: &str) -> Result<Option<RegistryValue>, String>;
    fn write(&mut self, name: &str, value: Option<&RegistryValue>) -> Result<(), String>;
}

fn apply_startup(writer: &mut impl StartupWriter, edits: &[StartupEdit]) -> Result<(), String> {
    apply_startup_recording(writer, edits, &mut Vec::new())
}

fn apply_startup_recording(
    writer: &mut impl StartupWriter,
    edits: &[StartupEdit],
    applied: &mut Vec<StartupEdit>,
) -> Result<(), String> {
    // Detect changes to either entry before beginning a multi-value operation.
    for edit in edits {
        if writer.read(edit.name)? != edit.expected {
            return Err(format!(
                "Startup entry '{}' changed concurrently; no planned writes were started",
                edit.name
            ));
        }
    }
    for (completed, edit) in edits.iter().enumerate() {
        let change = (|| {
            if writer.read(edit.name)? != edit.expected {
                return Err("the entry changed concurrently".to_owned());
            }
            match writer.write(edit.name, edit.desired.as_ref()) {
                Ok(()) => applied.push(edit.clone()),
                Err(error) => {
                    // A reported failure may have applied its value. Record it
                    // only when readback establishes the requested post-state;
                    // unchanged or unclassifiable writes are not undo authority.
                    match writer.read(edit.name) {
                        Ok(actual) if actual == edit.desired => applied.push(edit.clone()),
                        Ok(actual) if actual == edit.expected => {}
                        Ok(_) => {
                            return Err(format!("{error}; a concurrent replacement was observed"))
                        }
                        Err(read_error) => {
                            return Err(format!(
                                "{error}; the failed write's state is unresolved: {read_error}"
                            ));
                        }
                    }
                    return Err(error);
                }
            }
            if writer.read(edit.name)? != edit.desired {
                return Err("the written state could not be verified".to_owned());
            }
            Ok(())
        })();
        if let Err(error) = change {
            return Err(format!(
                "Startup change for '{}' was not confirmed ({error}); {completed} prior \
                 change(s) were confirmed and this operation may have partially applied. \
                 Recheck the actual startup state before retrying.",
                edit.name
            ));
        }
    }
    Ok(())
}

fn rollback_startup(
    writer: &mut impl StartupWriter,
    edits: &[StartupEdit],
    validate_ownership: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let mut inverse = Vec::new();
    for edit in edits.iter().rev() {
        let actual = writer.read(edit.name)?;
        if actual == edit.expected {
            continue;
        }
        if actual != edit.desired {
            return Err(format!(
                "Startup entry '{}' changed outside this transaction; rollback will not \
                 overwrite or remove that replacement",
                edit.name
            ));
        }
        inverse.push(StartupEdit {
            name: edit.name,
            expected: edit.desired.clone(),
            desired: edit.expected.clone(),
        });
    }
    if inverse.is_empty() {
        return Ok(());
    }
    validate_ownership()?;
    apply_startup(writer, &inverse)
}

fn commit_startup_change<T>(
    writer: &mut impl StartupWriter,
    edits: &[StartupEdit],
    validate_applied: impl FnOnce() -> Result<(), String>,
    validate_rollback: impl FnOnce() -> Result<(), String>,
    commit: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let mut applied_edits = Vec::new();
    let applied = apply_startup_recording(writer, edits, &mut applied_edits)
        .and_then(|()| validate_applied());
    let result = match applied {
        Ok(()) => commit(),
        Err(error) => Err(format!(
            "Startup registration was not confirmed; settings were not saved: {error}"
        )),
    };
    match result {
        Ok(value) => Ok(value),
        Err(error) => match rollback_startup(writer, &applied_edits, validate_rollback) {
            Ok(()) => Err(format!(
                "{error}. Recorded Run edits were restored to their exact prior state (or were \
                 unchanged). Any failed write whose state could not be classified remains \
                 unresolved. Windows Startup Apps overrides were untouched."
            )),
            Err(rollback) => Err(format!(
                "{error}. Exact startup rollback was not confirmed: {rollback}. Recheck the \
                 actual registrations; no unrelated replacement was authorized for deletion."
            )),
        },
    }
}

#[derive(Clone, Debug)]
struct ProcessEvidence {
    pid: u32,
    same_user: bool,
    registered_image: bool,
    exited: bool,
}

fn require_predecessor_exit(
    current_pid: u32,
    observations: impl IntoIterator<Item = Result<ProcessEvidence, String>>,
    replacing_shared_files: bool,
) -> Result<(), String> {
    for observation in observations {
        let process = observation.map_err(|error| {
            format!("Installed predecessor status is unresolved: {error}. {QUIT_INSTRUCTION}")
        })?;
        if process.pid == current_pid || !process.registered_image || process.exited {
            continue;
        }
        if !process.same_user && replacing_shared_files {
            return Err(
                "The registered installation is in use by another Windows user/session. \
                        Shared installation files cannot be replaced safely. This installer \
                        will not request termination of another user's process."
                    .to_owned(),
            );
        }
        if process.same_user {
            return Err(format!(
                "An installed HDR Auto-Switch controller (PID {}) is still running for this \
                 interactive user. {QUIT_INSTRUCTION}",
                process.pid
            ));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
mod platform {
    use super::{InstallerKind, StartupEdit};
    use std::path::Path;

    pub(super) fn check_predecessor() -> Result<(), String> {
        Err("Installed HDR predecessor verification is available only on Windows".to_owned())
    }

    pub(super) fn apply_autostart_change(_: bool) -> Result<Vec<StartupEdit>, String> {
        Err("Owned startup registration is available only on Windows".to_owned())
    }

    pub(super) fn rollback_autostart_change(_: &[StartupEdit]) -> Result<(), String> {
        Err("Owned startup registration is available only on Windows".to_owned())
    }

    pub(super) fn configure_autostart_with_commit<T>(
        _: bool,
        _: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        Err("Owned startup registration is available only on Windows".to_owned())
    }

    pub(super) fn autostart_enabled() -> Result<bool, String> {
        Err("Owned startup registration is available only on Windows".to_owned())
    }

    pub(super) fn installer_handoff(
        _: InstallerKind,
        _: &Path,
        _: bool,
        _: bool,
    ) -> Result<(), String> {
        Err("The HDR installer preflight requires Windows".to_owned())
    }

    pub(super) fn nsis_retirement(_: &str, _: &Path) -> Result<(), String> {
        Err("NSIS registration retirement requires Windows".into())
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::mem::{size_of, size_of_val};
    use std::io::Write;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{
        CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER,
        ERROR_MORE_DATA, ERROR_NO_MORE_FILES, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, HANDLE,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
    use windows::Win32::Security::{
        CopySid, EqualSid, GetLengthSid, GetTokenInformation, TokenUser, PSID, TOKEN_QUERY,
        TOKEN_USER,
    };
    use windows::Win32::System::ApplicationInstallationAndServicing::{
        MsiEnumRelatedProductsW, MsiGetComponentPathW, MsiGetProductInfoW, INSTALLSTATE_LOCAL,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteKeyExW, RegDeleteValueW, RegEnumKeyExW,
        RegEnumValueW, RegOpenKeyExW, RegQueryValueExW,
        RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_SET_VALUE, KEY_WOW64_32KEY,
        KEY_WOW64_64KEY, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_VALUE_TYPE,
    };
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
        QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetShellWindow, GetWindowThreadProcessId, MessageBoxW, IDRETRY, MB_ICONWARNING,
        MB_RETRYCANCEL, MB_SETFOREGROUND,
    };

    const NSIS_UNINSTALL: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\HDR Auto-Switch";
    const PRODUCT_KEY: &str = r"Software\soptik\HDR Auto-Switch";
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APPROVAL_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    struct RegistryKey(HKEY);

    impl Drop for RegistryKey {
        fn drop(&mut self) {
            let _ = unsafe { RegCloseKey(self.0) };
        }
    }

    fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
        value.as_ref().encode_wide().chain(Some(0)).collect()
    }

    fn ordinal_eq(left: impl AsRef<OsStr>, right: impl AsRef<OsStr>) -> bool {
        let left: Vec<_> = left.as_ref().encode_wide().collect();
        let right: Vec<_> = right.as_ref().encode_wide().collect();
        unsafe { CompareStringOrdinal(&left, &right, true) == CSTR_EQUAL }
    }

    fn canonical(path: &Path) -> Result<PathBuf, String> {
        if !path.is_absolute() {
            return Err(format!(
                "Installation path is not absolute: {}",
                path.display()
            ));
        }
        std::fs::canonicalize(path).map_err(|error| {
            format!(
                "Cannot resolve registered installation path '{}': {error}",
                path.display()
            )
        })
    }

    fn unquote_path(value: &str) -> Result<PathBuf, String> {
        let path = if let Some(quoted) = value.strip_prefix('"') {
            quoted
                .strip_suffix('"')
                .ok_or("Unbalanced quotes in installation metadata")?
        } else {
            value
        };
        if path.contains(['"', '\0', '\r', '\n']) {
            return Err("Invalid installation path metadata".to_owned());
        }
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("Installation metadata does not contain an absolute path".to_owned());
        }
        Ok(path)
    }

    impl RegistryKey {
        fn open(path: &str, view: REG_SAM_FLAGS) -> Result<Option<Self>, String> {
            Self::open_access(path, KEY_QUERY_VALUE | view)
        }

        fn open_access(path: &str, access: REG_SAM_FLAGS) -> Result<Option<Self>, String> {
            let path_w = wide(path);
            let mut handle = HKEY::default();
            let status = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    PCWSTR(path_w.as_ptr()),
                    Some(0),
                    access,
                    &mut handle,
                )
            };
            if status == ERROR_FILE_NOT_FOUND {
                Ok(None)
            } else if status == ERROR_SUCCESS {
                Ok(Some(Self(handle)))
            } else {
                Err(format!(
                    "Cannot read HKCU\\{path}: Windows error {}",
                    status.0
                ))
            }
        }

        fn read(&self, name: &str) -> Result<Option<RegistryValue>, String> {
            let name_w = wide(name);
            // The value may change between size and data queries. Retry a bounded
            // number of times, but never turn an unreadable value into "absent".
            for _ in 0..3 {
                let mut kind = REG_VALUE_TYPE::default();
                let mut length = 0u32;
                let status = unsafe {
                    RegQueryValueExW(
                        self.0,
                        PCWSTR(name_w.as_ptr()),
                        None,
                        Some(&mut kind),
                        None,
                        Some(&mut length),
                    )
                };
                if status == ERROR_FILE_NOT_FOUND {
                    return Ok(None);
                }
                if status != ERROR_SUCCESS || length > 131_072 {
                    return Err(format!(
                        "Cannot size registry value '{name}': Windows error {}, size {length}",
                        status.0
                    ));
                }
                let mut bytes = vec![0; length as usize];
                let status = unsafe {
                    RegQueryValueExW(
                        self.0,
                        PCWSTR(name_w.as_ptr()),
                        None,
                        Some(&mut kind),
                        Some(bytes.as_mut_ptr()),
                        Some(&mut length),
                    )
                };
                if status == ERROR_MORE_DATA || status == ERROR_FILE_NOT_FOUND {
                    continue;
                }
                if status != ERROR_SUCCESS {
                    return Err(format!(
                        "Cannot read registry value '{name}': Windows error {}",
                        status.0
                    ));
                }
                bytes.truncate(length as usize);
                return Ok(Some(RegistryValue {
                    kind: kind.0,
                    bytes,
                }));
            }
            Err(format!(
                "Registry value '{name}' kept changing while being read"
            ))
        }

        fn required_string(&self, name: &str) -> Result<String, String> {
            self.read(name)?
                .ok_or_else(|| format!("Installed product metadata is missing '{name}'"))?
                .as_string()
        }

        fn values(&self, reject_subkeys: bool) -> Result<RegistryValues, String> {
            if reject_subkeys {
                let mut name = [0u16; 256];
                let mut size = name.len() as u32;
                let status = unsafe {
                    RegEnumKeyExW(self.0, 0, Some(PWSTR(name.as_mut_ptr())), &mut size,
                        None, None, None, None)
                };
                if status != ERROR_NO_MORE_ITEMS {
                    return Err(format!("Uninstall key has subkeys or cannot be inspected ({}); no recursive deletion is permitted", status.0));
                }
            }
            let mut values = RegistryValues::new();
            for index in 0..1024 {
                let mut name = vec![0u16; 16_384];
                let mut size = name.len() as u32;
                let status = unsafe {
                    RegEnumValueW(self.0, index, Some(PWSTR(name.as_mut_ptr())), &mut size,
                        None, None, None, None)
                };
                if status == ERROR_NO_MORE_ITEMS {
                    return Ok(values);
                }
                if status != ERROR_SUCCESS {
                    return Err(format!("Cannot enumerate installation metadata: Windows error {}", status.0));
                }
                let name = String::from_utf16(&name[..size as usize])
                    .map_err(|_| "Installation value name is not valid Unicode")?;
                let value = self.read(&name)?.ok_or("Installation metadata changed while reading")?;
                if values.insert(name, value).is_some() {
                    return Err("Installation metadata changed while enumerating".into());
                }
            }
            Err("Installation metadata exceeds the bounded recovery snapshot".into())
        }
    }

    impl RetirementKey {
        fn path(self) -> &'static str {
            match self {
                Self::Uninstall => NSIS_UNINSTALL,
                Self::Product => PRODUCT_KEY,
            }
        }
    }

    struct NsisRetirementRegistry;

    impl RetirementRegistry for NsisRetirementRegistry {
        fn read(&mut self, key: RetirementKey, view: u32) -> Result<Option<RegistryValues>, String> {
            let access = KEY_QUERY_VALUE | REG_SAM_FLAGS(view)
                | if key == RetirementKey::Uninstall { KEY_ENUMERATE_SUB_KEYS } else { REG_SAM_FLAGS(0) };
            RegistryKey::open_access(key.path(), access)?
                .map(|handle| handle.values(key == RetirementKey::Uninstall)).transpose()
        }

        fn delete_uninstall(&mut self, view: u32) -> Result<(), String> {
            let path = wide(NSIS_UNINSTALL);
            let status = unsafe { RegDeleteKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), view, Some(0)) };
            if status == ERROR_SUCCESS { Ok(()) }
            else { Err(format!("Cannot retire uninstall registration: Windows error {}", status.0)) }
        }

        fn delete_product_path(&mut self, view: u32) -> Result<(), String> {
            let Some(key) = RegistryKey::open_access(PRODUCT_KEY, KEY_SET_VALUE | REG_SAM_FLAGS(view))?
            else { return Ok(()); };
            let name = wide("");
            let status = unsafe { RegDeleteValueW(key.0, PCWSTR(name.as_ptr())) };
            // Product keys can be shared across registry views. The caller also
            // requires explicit absence readback; denied queries never pass.
            if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND { Ok(()) }
            else { Err(format!("Cannot retire product path: Windows error {}", status.0)) }
        }

        fn write(&mut self, key: RetirementKey, view: u32, name: &str, value: &RegistryValue)
            -> Result<(), String> {
            let path = wide(key.path());
            let name = wide(name);
            let mut handle = HKEY::default();
            let status = unsafe {
                RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), Some(0), None,
                    REG_OPTION_NON_VOLATILE, KEY_QUERY_VALUE | KEY_SET_VALUE | REG_SAM_FLAGS(view),
                    None, &mut handle, None)
            };
            if status != ERROR_SUCCESS {
                return Err(format!("Cannot open metadata for recovery: Windows error {}", status.0));
            }
            let key = RegistryKey(handle);
            let status = unsafe {
                RegSetValueExW(key.0, PCWSTR(name.as_ptr()), Some(0), REG_VALUE_TYPE(value.kind),
                    Some(&value.bytes))
            };
            if status == ERROR_SUCCESS { Ok(()) }
            else { Err(format!("Cannot restore metadata: Windows error {}", status.0)) }
        }
    }

    fn retirement_receipt_path() -> Result<PathBuf, String> {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        Ok(executable.parent().ok_or("Recovery executable has no parent directory")?
            .join("hdr-uninstall-registration.json"))
    }

    fn write_recovery_artifact(path: &Path, bytes: &[u8]) -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true)
            .open(path).map_err(|error| format!("Cannot create recovery artifact: {error}"))?;
        file.write_all(bytes).and_then(|()| file.sync_all())
            .map_err(|error| format!("Cannot persist recovery artifact: {error}"))
    }

    fn cleanup_path_without_file(path: &Path) -> Result<PathBuf, String> {
        if !path.is_absolute() {
            return Err("Recovery target must be absolute".into());
        }
        let parent = path.parent().ok_or("Recovery target has no installation directory")?;
        let name = path.file_name().ok_or("Recovery target has no filename")?;
        Ok(canonical(parent)?.join(name))
    }

    fn validate_retirement_receipt(receipt: &NsisRetirementReceipt, target: &Path) -> Result<(), String> {
        let target = cleanup_path_without_file(target)?;
        if receipt.schema != 1 || !ordinal_eq(&target, &receipt.target)
            || !target.file_name().is_some_and(|name| ordinal_eq(name, MAIN_BINARY))
            || receipt.registrations.is_empty() || receipt.registrations.len() > 2
        {
            return Err("Invalid installation recovery receipt or target".into());
        }
        let mut views = std::collections::BTreeSet::new();
        for entry in &receipt.registrations {
            if ![KEY_WOW64_32KEY.0, KEY_WOW64_64KEY.0].contains(&entry.view) || !views.insert(entry.view) {
                return Err("Invalid or duplicate recovery registry view".into());
            }
            let string = |name: &str| entry.uninstall.get(name)
                .ok_or_else(|| format!("Recovery metadata is missing '{name}'"))?.as_string();
            if string("DisplayName")? != PRODUCT_NAME || string("Publisher")? != PUBLISHER
                || string("MainBinaryName")? != MAIN_BINARY
            {
                return Err("Recovery metadata does not identify this product".into());
            }
            let location = canonical(&unquote_path(&string("InstallLocation")?)?)?;
            if !ordinal_eq(location.join(MAIN_BINARY), &target)
                || !ordinal_eq(canonical(&unquote_path(&entry.product_path.as_string()?)?)?, &location)
                || !ordinal_eq(cleanup_path_without_file(&unquote_path(&string("DisplayIcon")?)?)?, &target)
                || !ordinal_eq(cleanup_path_without_file(&unquote_path(&string("UninstallString")?)?)?,
                    location.join("uninstall.exe"))
            {
                return Err("Recovery metadata paths disagree with the original installation".into());
            }
        }
        Ok(())
    }

    fn verify_recovery_pair(target: &Path, recovery: &Path) -> Result<(), String> {
        let installation = target.parent().ok_or("Missing installation directory")?;
        for name in [MAIN_BINARY, "uninstall.exe"] {
            let original = std::fs::read(installation.join(name))
                .map_err(|error| format!("Cannot verify restored {name}: {error}"))?;
            let backup = std::fs::read(recovery.join(name))
                .map_err(|error| format!("Cannot read recovery {name}: {error}"))?;
            if original.is_empty() || original != backup {
                return Err(format!("The existing {name} differs from the recovery copy; it will not be registered or overwritten"));
            }
        }
        Ok(())
    }

    pub(super) fn nsis_retirement(command: &str, target: &Path) -> Result<(), String> {
        let _user = interactive_user()?;
        let receipt_path = retirement_receipt_path()?;
        let recovery = receipt_path.parent().ok_or("Missing recovery directory")?;
        let executable = canonical(&std::env::current_exe().map_err(|error| error.to_string())?)?;
        if ordinal_eq(&executable, cleanup_path_without_file(target)?) {
            return Err("Registry retirement must run from the retained recovery helper".into());
        }
        let mut registry = NsisRetirementRegistry;
        if command == "--hdr-installer-retirement-prepare" {
            handoff_once(InstallerKind::Nsis, target, true)?;
            verify_recovery_pair(target, recovery)?;
            let mut receipt = NsisRetirementReceipt {
                schema: 1,
                target: cleanup_path_without_file(target)?.to_str()
                    .ok_or("Recovery target is not valid Unicode")?.into(),
                registrations: Vec::new(),
            };
            for view in [KEY_WOW64_64KEY.0, KEY_WOW64_32KEY.0] {
                if let Some(uninstall) = registry.read(RetirementKey::Uninstall, view)? {
                    let product = registry.read(RetirementKey::Product, view)?;
                    receipt.registrations.push(NsisRegistrationSnapshot {
                        view, uninstall,
                        product_path: product_path(&product).ok_or("Missing owned product path")?.clone(),
                    });
                }
            }
            validate_retirement_receipt(&receipt, target)?;
            let bytes = serde_json::to_vec(&receipt).map_err(|error| error.to_string())?;
            if bytes.len() > 2_097_152 {
                return Err("Recovery receipt exceeds its size limit".into());
            }
            write_recovery_artifact(&receipt_path, &bytes)?;
            let instructions = format!(
                "HDR Auto-Switch incomplete uninstall recovery\r\n\
                 Do NOT launch the application: bundled resources may already be gone and startup may be disabled.\r\n\
                 Keep this entire recovery directory, including both executables and hdr-uninstall-registration.json.\r\n\
                 After closing locks, copy tauri-app.exe and uninstall.exe from this directory to {} ONLY where the original is missing. Never overwrite a conflicting file; do not use partial .restore files.\r\n\
                 Resolve any registry access/conflict errors, then run the following recovery-only command from Command Prompt (cmd.exe) as the original signed-in user. It verifies complete cleanup executable bytes and restores only missing original registry values, without initializing the app, changing startup or terminating any process:\r\n\
                 \"{}\" --hdr-installer-retirement-restore nsis \"{}\" 2\r\n\
                 Only after that command succeeds, retry the restored uninstall.exe normally (without _?=). Keep these recovery files until uninstall succeeds.\r\n",
                target.parent().unwrap().display(), executable.display(), target.display(),
            );
            write_recovery_artifact(&recovery.join("RECOVERY.txt"), instructions.as_bytes())?;
            return Ok(());
        }
        let metadata = std::fs::metadata(&receipt_path).map_err(|error| error.to_string())?;
        if metadata.len() > 2_097_152 {
            return Err("Recovery receipt exceeds its size limit".into());
        }
        let bytes = std::fs::read(&receipt_path).map_err(|error| error.to_string())?;
        let receipt: NsisRetirementReceipt = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Cannot read recovery receipt: {error}"))?;
        validate_retirement_receipt(&receipt, target)?;
        match command {
            "--hdr-installer-retirement-commit" => {
                for name in [MAIN_BINARY, "uninstall.exe"] {
                    if target.parent().unwrap().join(name).try_exists().map_err(|error| error.to_string())? {
                        return Err("Cleanup executables are still present; registry retirement refused".into());
                    }
                }
                retire_nsis_registration(&mut registry, &receipt)
            }
            "--hdr-installer-retirement-restore" => {
                verify_recovery_pair(target, recovery)?;
                restore_nsis_registration(&mut registry, &receipt)
            }
            _ => Err("Unknown NSIS retirement command".into()),
        }
    }

    #[derive(Clone)]
    struct Installation {
        format: InstallerKind,
        image: PathBuf,
        launch_path: PathBuf,
    }

    fn nsis_installations() -> Result<Vec<Installation>, String> {
        let mut installations = Vec::new();
        // v1.0.5 shipped x64/currentUser. Read the explicit view, including the
        // other view to detect a conflicting registration rather than guess.
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let Some(key) = RegistryKey::open(NSIS_UNINSTALL, view)? else {
                continue;
            };
            if key.required_string("DisplayName")? != PRODUCT_NAME
                || key.required_string("Publisher")? != PUBLISHER
                || key.required_string("MainBinaryName")? != MAIN_BINARY
            {
                return Err("The NSIS product key has conflicting ownership metadata".to_owned());
            }
            let location = unquote_path(&key.required_string("InstallLocation")?)?;
            let image = canonical(&location.join(MAIN_BINARY))?;
            if !image.is_file() {
                return Err("Registered NSIS executable is not a regular file".to_owned());
            }
            let icon = canonical(&unquote_path(&key.required_string("DisplayIcon")?)?)?;
            let uninstall = canonical(&unquote_path(&key.required_string("UninstallString")?)?)?;
            if !ordinal_eq(&image, &icon)
                || !ordinal_eq(&uninstall, &canonical(&location.join("uninstall.exe"))?)
            {
                return Err(
                    "NSIS executable/uninstaller paths disagree with InstallLocation".to_owned(),
                );
            }
            let product = RegistryKey::open(PRODUCT_KEY, view)?
                .ok_or("NSIS installation is missing its product-path registration")?;
            let registered_dir = canonical(&unquote_path(&product.required_string("")?)?)?;
            if !ordinal_eq(&registered_dir, &canonical(&location)?) {
                return Err("NSIS product and uninstall registrations disagree".to_owned());
            }
            if !installations
                .iter()
                .any(|known: &Installation| ordinal_eq(&known.image, &image))
            {
                installations.push(Installation {
                    format: InstallerKind::Nsis,
                    image,
                    launch_path: location.join(MAIN_BINARY),
                });
            }
        }
        Ok(installations)
    }

    fn msi_property(product: &[u16], property: &str) -> Result<String, String> {
        let property_w = wide(property);
        let mut buffer = vec![0u16; 32_768];
        let mut size = buffer.len() as u32;
        let status = unsafe {
            MsiGetProductInfoW(
                PCWSTR(product.as_ptr()),
                PCWSTR(property_w.as_ptr()),
                Some(PWSTR(buffer.as_mut_ptr())),
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS.0 || size as usize >= buffer.len() {
            return Err(format!(
                "Cannot verify MSI property '{property}': Windows error {status}"
            ));
        }
        String::from_utf16(&buffer[..size as usize])
            .map_err(|_| "MSI product metadata is not valid UTF-16".to_owned())
    }

    fn msi_installations() -> Result<Vec<Installation>, String> {
        let upgrade = wide(MSI_UPGRADE_CODE);
        let component = wide(MSI_PATH_COMPONENT);
        let mut installations = Vec::new();
        for index in 0..128 {
            let mut product = [0u16; 39];
            let status = unsafe {
                MsiEnumRelatedProductsW(
                    PCWSTR(upgrade.as_ptr()),
                    Some(0),
                    index,
                    PWSTR(product.as_mut_ptr()),
                )
            };
            if status == ERROR_NO_MORE_ITEMS.0 {
                return Ok(installations);
            }
            if status != ERROR_SUCCESS.0 {
                return Err(format!(
                    "Cannot enumerate the shipped MSI upgrade identity: Windows error {status}"
                ));
            }
            if msi_property(&product, "ProductName")? != PRODUCT_NAME
                || msi_property(&product, "Publisher")? != PUBLISHER
            {
                return Err("A related MSI product has conflicting ownership metadata".to_owned());
            }
            let mut path = vec![0u16; 32_768];
            let mut size = path.len() as u32;
            let state = unsafe {
                MsiGetComponentPathW(
                    PCWSTR(product.as_ptr()),
                    PCWSTR(component.as_ptr()),
                    Some(PWSTR(path.as_mut_ptr())),
                    Some(&mut size),
                )
            };
            if state != INSTALLSTATE_LOCAL || size as usize >= path.len() {
                return Err(format!(
                    "The registered MSI executable component cannot be resolved (state {})",
                    state.0
                ));
            }
            let launch_path = PathBuf::from(OsString::from_wide(&path[..size as usize]));
            let image = canonical(&launch_path)?;
            if !image.is_file()
                || !image
                    .file_name()
                    .is_some_and(|name| ordinal_eq(name, MAIN_BINARY))
            {
                return Err(
                    "The MSI executable component is not the shipped main executable".to_owned(),
                );
            }
            installations.push(Installation {
                format: InstallerKind::Msi,
                image,
                launch_path,
            });
        }
        Err(
            "Too many products registered under the HDR Auto-Switch MSI upgrade identity"
                .to_owned(),
        )
    }

    fn installations() -> Result<Vec<Installation>, String> {
        let mut result = nsis_installations()?;
        result.extend(msi_installations()?);
        Ok(result)
    }

    // Word-aligned buffers are required for TOKEN_USER and SID pointers.
    struct UserSid(Vec<usize>);

    impl UserSid {
        fn for_process(process: HANDLE) -> Result<Self, String> {
            let mut token = HANDLE::default();
            unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }
                .map_err(|error| format!("Cannot inspect process owner: {error}"))?;
            let token = OwnedHandle(token);
            let mut length = 0u32;
            let probe = unsafe { GetTokenInformation(token.0, TokenUser, None, 0, &mut length) };
            if probe.is_ok()
                || length == 0
                || length > 65_536
                || probe.as_ref().err().is_some_and(|error| {
                    error.code() != windows::core::HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0)
                })
            {
                return Err("Cannot determine the process-owner token size".to_owned());
            }
            let mut data = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
            unsafe {
                GetTokenInformation(
                    token.0,
                    TokenUser,
                    Some(data.as_mut_ptr().cast()),
                    length,
                    &mut length,
                )
            }
            .map_err(|error| format!("Cannot read process owner: {error}"))?;
            let user = unsafe { &*data.as_ptr().cast::<TOKEN_USER>() };
            let length = unsafe { GetLengthSid(user.User.Sid) } as usize;
            if length == 0 || length > 1024 {
                return Err("Invalid process-owner SID length".to_owned());
            }
            let mut sid = vec![0usize; length.div_ceil(size_of::<usize>())];
            unsafe { CopySid(length as u32, PSID(sid.as_mut_ptr().cast()), user.User.Sid) }
                .map_err(|error| format!("Cannot copy process owner: {error}"))?;
            Ok(Self(sid))
        }

        fn equals(&self, other: &Self) -> bool {
            unsafe {
                EqualSid(
                    PSID(self.0.as_ptr() as *mut _),
                    PSID(other.0.as_ptr() as *mut _),
                )
                .is_ok()
            }
        }
    }

    struct InteractiveUser {
        sid: UserSid,
        session: u32,
        pid: u32,
    }

    fn session_id(pid: u32) -> Result<u32, String> {
        let mut session = 0;
        unsafe { ProcessIdToSessionId(pid, &mut session) }
            .map_err(|error| format!("Cannot inspect process session {pid}: {error}"))?;
        Ok(session)
    }

    fn interactive_user() -> Result<InteractiveUser, String> {
        let pid = unsafe { GetCurrentProcessId() };
        let session = session_id(pid)?;
        if session == 0 {
            return Err("HDR upgrade/startup verification requires the logged-in interactive user, not a service/SYSTEM session".to_owned());
        }
        let sid = UserSid::for_process(unsafe { GetCurrentProcess() })?;
        let shell = unsafe { GetShellWindow() };
        if shell.is_invalid() {
            return Err(
                "The interactive shell user could not be verified; sign in normally and retry"
                    .to_owned(),
            );
        }
        let mut shell_pid = 0;
        unsafe { GetWindowThreadProcessId(shell, Some(&mut shell_pid)) };
        let shell_process = OwnedHandle(
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, shell_pid) }
                .map_err(|error| format!("Cannot verify the interactive shell owner: {error}"))?,
        );
        if !sid.equals(&UserSid::for_process(shell_process.0)?) || session_id(shell_pid)? != session
        {
            return Err("The installer/app is running as a different account from the interactive shell. Run it as the signed-in user; other users' processes and startup entries will not be changed.".to_owned());
        }
        Ok(InteractiveUser { sid, session, pid })
    }

    fn exited(process: HANDLE) -> Result<bool, String> {
        let status = unsafe { WaitForSingleObject(process, 0) };
        if status == WAIT_OBJECT_0 {
            Ok(true)
        } else if status == WAIT_TIMEOUT {
            Ok(false)
        } else {
            Err(format!(
                "Cannot confirm process lifetime (wait status {})",
                status.0
            ))
        }
    }

    fn inspect_process(
        pid: u32,
        user: &InteractiveUser,
        known: &[Installation],
        replacing_shared_files: bool,
    ) -> Result<ProcessEvidence, String> {
        let mut evidence = ProcessEvidence {
            pid,
            same_user: false,
            registered_image: false,
            exited: false,
        };
        if pid == user.pid {
            return Ok(evidence);
        }
        let same_session = match session_id(pid) {
            Ok(session) if session != user.session && !replacing_shared_files => {
                return Ok(evidence)
            }
            Ok(session) => session == user.session,
            Err(error) => return Err(error),
        };
        let process = match unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
        } {
            Ok(process) => OwnedHandle(process),
            Err(error)
                if error.code()
                    == windows::core::HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) =>
            {
                evidence.exited = true;
                return Ok(evidence);
            }
            Err(error) => return Err(format!("Cannot inspect candidate process {pid}: {error}")),
        };
        if exited(process.0)? {
            evidence.exited = true;
            return Ok(evidence);
        }
        evidence.same_user = same_session && user.sid.equals(&UserSid::for_process(process.0)?);
        if !evidence.same_user && !replacing_shared_files {
            return Ok(evidence);
        }
        let mut path = vec![0u16; 32_768];
        let mut size = path.len() as u32;
        let query = unsafe {
            QueryFullProcessImageNameW(
                process.0,
                PROCESS_NAME_FORMAT(0),
                PWSTR(path.as_mut_ptr()),
                &mut size,
            )
        };
        if exited(process.0)? {
            evidence.exited = true;
            return Ok(evidence);
        }
        query
            .map_err(|error| format!("Cannot inspect candidate process {pid}'s image: {error}"))?;
        let path = canonical(&PathBuf::from(OsString::from_wide(&path[..size as usize])))?;
        evidence.registered_image = known
            .iter()
            .any(|install| ordinal_eq(&install.image, &path));
        evidence.exited = exited(process.0)?;
        Ok(evidence)
    }

    fn check_known_predecessors(
        user: &InteractiveUser,
        known: &[Installation],
        replacing_shared_files: bool,
    ) -> Result<(), String> {
        if known.is_empty() {
            return Ok(());
        }
        let snapshot = OwnedHandle(
            unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.map_err(|error| {
                format!("Cannot enumerate installed predecessor candidates: {error}")
            })?,
        );
        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = size_of_val(&entry) as u32;
        let mut next = unsafe { Process32FirstW(snapshot.0, &mut entry) };
        let mut observations = Vec::new();
        loop {
            if let Err(error) = next {
                if error.code() == windows::core::HRESULT::from_win32(ERROR_NO_MORE_FILES.0) {
                    break;
                }
                return Err(format!("Predecessor enumeration is unresolved: {error}"));
            }
            let length = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = OsString::from_wide(&entry.szExeFile[..length]);
            if known.iter().any(|install| {
                install
                    .image
                    .file_name()
                    .is_some_and(|registered| ordinal_eq(registered, &name))
            }) {
                // Basename filters work only; the image, token SID, session and
                // handle lifetime below supply the actual conflict evidence.
                observations.push(inspect_process(
                    entry.th32ProcessID,
                    user,
                    known,
                    replacing_shared_files,
                ));
            }
            next = unsafe { Process32NextW(snapshot.0, &mut entry) };
        }
        require_predecessor_exit(user.pid, observations, replacing_shared_files)
    }

    pub(super) fn check_predecessor() -> Result<(), String> {
        let user = interactive_user()?;
        check_known_predecessors(&user, &installations()?, false)
    }

    fn read_value(path: &str, name: &str) -> Result<Option<RegistryValue>, String> {
        RegistryKey::open(path, REG_SAM_FLAGS(0))?
            .map(|key| key.read(name))
            .transpose()
            .map(Option::flatten)
    }

    fn registration_owned(value: &RegistryValue, known: &[Installation]) -> bool {
        let Ok(command) = value.as_string() else {
            return false;
        };
        let Some(path) = startup_executable(&command) else {
            return false;
        };
        let Ok(image) = canonical(Path::new(path)) else {
            return false;
        };
        known
            .iter()
            .any(|install| ordinal_eq(&install.image, &image))
    }

    fn startup_snapshot(known: &[Installation]) -> Result<StartupSnapshot, String> {
        let mut snapshot = StartupSnapshot {
            stable_name_approved: true,
            ..StartupSnapshot::default()
        };
        for (name, stable) in [(OWNED_STARTUP_NAME, true), (LEGACY_STARTUP_NAME, false)] {
            let value = read_value(RUN_KEY, name)?;
            let owned = value
                .as_ref()
                .is_some_and(|value| registration_owned(value, known));
            // Do not interpret or modify another application's legacy approval.
            let approved = if stable || owned {
                startup_approved(read_value(APPROVAL_KEY, name)?.as_ref())?
            } else {
                false
            };
            if stable {
                snapshot.stable_name_approved = approved;
            }
            let entry = value.map(|value| StartupRegistration {
                value,
                owned,
                approved,
            });
            if stable {
                snapshot.stable = entry;
            } else {
                snapshot.legacy = entry;
            }
        }
        snapshot.validate()?;
        Ok(snapshot)
    }

    struct RunWriter;

    impl StartupWriter for RunWriter {
        fn read(&mut self, name: &str) -> Result<Option<RegistryValue>, String> {
            read_value(RUN_KEY, name)
        }

        fn write(&mut self, name: &str, value: Option<&RegistryValue>) -> Result<(), String> {
            let path_w = wide(RUN_KEY);
            let name_w = wide(name);
            let mut handle = HKEY::default();
            let status = unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    PCWSTR(path_w.as_ptr()),
                    Some(0),
                    None,
                    REG_OPTION_NON_VOLATILE,
                    KEY_QUERY_VALUE | KEY_SET_VALUE,
                    None,
                    &mut handle,
                    None,
                )
            };
            if status != ERROR_SUCCESS {
                return Err(format!(
                    "Cannot open owned startup registration for writing: Windows error {}",
                    status.0
                ));
            }
            let key = RegistryKey(handle);
            let status = match value {
                Some(value) => unsafe {
                    RegSetValueExW(
                        key.0,
                        PCWSTR(name_w.as_ptr()),
                        Some(0),
                        REG_VALUE_TYPE(value.kind),
                        Some(&value.bytes),
                    )
                },
                None => unsafe { RegDeleteValueW(key.0, PCWSTR(name_w.as_ptr())) },
            };
            if status == ERROR_SUCCESS || (value.is_none() && status == ERROR_FILE_NOT_FOUND) {
                Ok(())
            } else {
                Err(format!(
                    "Cannot change owned startup entry '{name}': Windows error {}",
                    status.0
                ))
            }
        }
    }

    pub(super) fn autostart_enabled() -> Result<bool, String> {
        interactive_user()?;
        startup_snapshot(&installations()?)?.enabled()
    }

    fn prepare_autostart(enable: bool) -> Result<(Vec<Installation>, Vec<StartupEdit>), String> {
        let user = interactive_user()?;
        let known = installations()?;
        check_known_predecessors(&user, &known, false)?;
        let current = canonical(&std::env::current_exe().map_err(|error| error.to_string())?)?;
        let installation = known
            .iter()
            .find(|install| ordinal_eq(&install.image, &current));
        if enable && installation.is_none() {
            return Err("Startup can only be enabled for the verified installed HDR Auto-Switch executable, not a development/portable copy".to_owned());
        }
        let target = installation
            .map(|install| install.launch_path.as_path())
            .unwrap_or(&current)
            .to_str()
            .ok_or("The installed startup executable path is not valid Unicode")?;
        let snapshot = startup_snapshot(&known)?;
        let edits = plan_startup(&snapshot, enable, target)?;
        Ok((known, edits))
    }

    fn verify_autostart(known: &[Installation], enable: bool) -> Result<(), String> {
        let actual = startup_snapshot(known)?;
        if actual.enabled()? != enable || actual.legacy.as_ref().is_some_and(|entry| entry.owned) {
            return Err(
                "Effective owned startup state changed or could not be confirmed; no canonical \
                 settings commit is authorized"
                    .to_owned(),
            );
        }
        Ok(())
    }

    fn verify_rollback_ownership(edits: &[StartupEdit]) -> Result<(), String> {
        let user = interactive_user()?;
        let known = installations()?;
        check_known_predecessors(&user, &known, false)?;
        for edit in edits {
            for value in edit.expected.iter().chain(edit.desired.iter()) {
                if !registration_owned(value, &known) {
                    return Err(format!(
                        "The installation that owned startup entry '{}' can no longer be \
                         verified; rollback will not restore or remove it",
                        edit.name
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn configure_autostart_with_commit<T>(
        enable: bool,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        // Preparation performs all policy/ownership checks before any registry
        // edit. A rejected request does not need, and must not attempt, rollback.
        let (known, edits) = prepare_autostart(enable)?;
        commit_startup_change(
            &mut RunWriter,
            &edits,
            || verify_autostart(&known, enable),
            || verify_rollback_ownership(&edits),
            commit,
        )
    }

    pub(super) fn apply_autostart_change(enable: bool) -> Result<Vec<StartupEdit>, String> {
        let (known, edits) = prepare_autostart(enable)?;
        commit_startup_change(
            &mut RunWriter,
            &edits,
            || verify_autostart(&known, enable),
            || verify_rollback_ownership(&edits),
            || Ok(edits.clone()),
        )
    }

    pub(super) fn rollback_autostart_change(edits: &[StartupEdit]) -> Result<(), String> {
        rollback_startup(&mut RunWriter, edits, || verify_rollback_ownership(edits))
    }

    fn handoff_once(format: InstallerKind, target: &Path, uninstall: bool) -> Result<(), String> {
        let _guard = STARTUP_TRANSACTION
            .lock()
            .map_err(|_| "Startup registration transaction is unavailable".to_owned())?;
        let user = interactive_user()?;
        let known = installations()?;
        if known.iter().any(|install| install.format != format) {
            return Err("Another HDR Auto-Switch installer format is registered. Cross-format replacement is blocked: use that installation's matching MSI/NSIS updater. This installer will not launch an old uninstaller.".to_owned());
        }
        // An MSI upgrade invokes the old MSI's removal sequence. Never let that
        // sequence encounter another user's still-running shared installation.
        check_known_predecessors(&user, &known, true)?;
        if !target.is_absolute()
            || !target
                .file_name()
                .is_some_and(|name| ordinal_eq(name, MAIN_BINARY))
        {
            return Err(
                "Installer destination is not the supported main executable path".to_owned(),
            );
        }
        if !known.is_empty() {
            let destination = canonical(target)?;
            if known
                .iter()
                .any(|install| !ordinal_eq(&install.image, &destination))
            {
                return Err("The registered installation and installer destination differ. In-place upgrades must keep the verified existing installation directory; no stale startup path was rewritten.".to_owned());
            }
        } else {
            if uninstall {
                return Err(
                    "Cannot uninstall without verified installed-product ownership metadata"
                        .to_owned(),
                );
            }
            if target
                .try_exists()
                .map_err(|error| format!("Cannot inspect installer destination: {error}"))?
            {
                return Err("The installer destination already contains an executable without verified HDR Auto-Switch installation metadata. It will not be overwritten.".to_owned());
            }
        }
        let snapshot = startup_snapshot(&known)?;
        // Preserve a Windows-disabled startup preference. StartupApproved is
        // deliberately never rewritten: its undocumented binary format is not
        // an authority to override a user's Windows Settings choice.
        let enabled = !uninstall && snapshot.enabled()?;
        let target = target
            .to_str()
            .ok_or("Installer destination is not valid Unicode")?;
        let edits = plan_installer_startup(&snapshot, uninstall, target)?;
        apply_startup(&mut RunWriter, &edits)?;
        let after = startup_snapshot(&known)?;
        if after.enabled()? != enabled
            || after.legacy.as_ref().is_some_and(|entry| {
                entry.owned && (uninstall || entry.approved || after.stable.is_some())
            })
        {
            return Err(
                "Startup handoff could not be verified; installation is blocked".to_owned(),
            );
        }
        // The handles in the first scan were short-lived. Reconfirm directly
        // before returning to the installer's replacement/removal sequence.
        check_known_predecessors(&user, &known, true)
    }

    pub(super) fn installer_handoff(
        format: InstallerKind,
        target: &Path,
        uninstall: bool,
        full_ui: bool,
    ) -> Result<(), String> {
        loop {
            match handoff_once(format, target, uninstall) {
                Ok(()) => return Ok(()),
                Err(error) if !full_ui => return Err(error),
                Err(error) => {
                    let message = wide(format!(
                        "{error}\n\n{QUIT_INSTRUCTION}\n\nRetry checks the actual installed \
                         process and registration again. Cancel stops this installer. \
                         No process will be terminated automatically."
                    ));
                    let title = wide("HDR Auto-Switch — safe upgrade");
                    let answer = unsafe {
                        MessageBoxW(
                            None,
                            PCWSTR(message.as_ptr()),
                            PCWSTR(title.as_ptr()),
                            MB_RETRYCANCEL | MB_ICONWARNING | MB_SETFOREGROUND,
                        )
                    };
                    if answer != IDRETRY {
                        return Err(error);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const IMAGE: &str = r"C:\Users\tester\AppData\Local\HDR Auto-Switch\tauri-app.exe";

    #[derive(Default)]
    struct RetirementMemoryRegistry {
        keys: BTreeMap<(RetirementKey, u32), RegistryValues>,
        delete_failure: Option<RetirementKey>,
        readback_failure: Option<RetirementKey>,
        retain_on_delete: Option<RetirementKey>,
        last_deleted: Option<RetirementKey>,
        restore_failure: bool,
        alias_views: bool,
        writes: usize,
    }

    impl RetirementMemoryRegistry {
        fn address(&self, key: RetirementKey, view: u32) -> (RetirementKey, u32) {
            (key, if self.alias_views { 256 } else { view })
        }

        fn delete(&mut self, key: RetirementKey, view: u32) -> Result<(), String> {
            if self.delete_failure == Some(key) {
                self.delete_failure = None;
                return Err("injected deletion access denial".into());
            }
            self.writes += 1;
            self.last_deleted = Some(key);
            let address = self.address(key, view);
            if self.retain_on_delete != Some(key) {
                if key == RetirementKey::Product {
                    if let Some(values) = self.keys.get_mut(&address) {
                        values.remove("");
                    }
                } else {
                    self.keys.remove(&address);
                }
            }
            Ok(())
        }
    }

    impl RetirementRegistry for RetirementMemoryRegistry {
        fn read(&mut self, key: RetirementKey, view: u32) -> Result<Option<RegistryValues>, String> {
            if self.last_deleted == Some(key) && self.readback_failure == Some(key) {
                self.readback_failure = None;
                return Err("injected readback access denial".into());
            }
            Ok(self.keys.get(&self.address(key, view)).cloned())
        }

        fn delete_uninstall(&mut self, view: u32) -> Result<(), String> {
            self.delete(RetirementKey::Uninstall, view)
        }

        fn delete_product_path(&mut self, view: u32) -> Result<(), String> {
            self.delete(RetirementKey::Product, view)
        }

        fn write(&mut self, key: RetirementKey, view: u32, name: &str, value: &RegistryValue)
            -> Result<(), String> {
            if self.restore_failure {
                return Err("injected restoration access denial".into());
            }
            self.writes += 1;
            let address = self.address(key, view);
            self.keys.entry(address).or_default().insert(name.into(), value.clone());
            Ok(())
        }
    }

    fn retirement_fixture() -> (RetirementMemoryRegistry, NsisRetirementReceipt) {
        let uninstall = BTreeMap::from([
            ("DisplayName".into(), RegistryValue::string(PRODUCT_NAME)),
            ("UninstallString".into(), RegistryValue::string(r#""C:\HDR\uninstall.exe""#)),
            ("ExtraBinaryMetadata".into(), RegistryValue { kind: 3, bytes: vec![0, 255, 2, 0] }),
        ]);
        let product_path = RegistryValue::string(r"C:\HDR");
        let product = BTreeMap::from([
            ("".into(), product_path.clone()),
            ("UnrelatedValue".into(), RegistryValue::string("leave me alone")),
        ]);
        let registry = RetirementMemoryRegistry {
            keys: BTreeMap::from([
                ((RetirementKey::Uninstall, 256), uninstall.clone()),
                ((RetirementKey::Product, 256), product),
            ]),
            ..Default::default()
        };
        let receipt = NsisRetirementReceipt {
            schema: 1, target: r"C:\HDR\tauri-app.exe".into(),
            registrations: vec![NsisRegistrationSnapshot { view: 256, uninstall, product_path }],
        };
        (registry, receipt)
    }

    #[test]
    fn nsis_retirement_checks_deletion_and_readback_before_success() {
        for key in [RetirementKey::Product, RetirementKey::Uninstall] {
            for failure in ["delete", "readback", "still-present"] {
                let (mut registry, receipt) = retirement_fixture();
                let before = registry.keys.clone();
                match failure {
                    "delete" => registry.delete_failure = Some(key),
                    "readback" => registry.readback_failure = Some(key),
                    _ => registry.retain_on_delete = Some(key),
                }
                assert!(retire_nsis_registration(&mut registry, &receipt).is_err(), "{key:?}: {failure}");
                restore_nsis_registration(&mut registry, &receipt).unwrap();
                assert_eq!(registry.keys, before, "{key:?}: {failure}");
            }
        }
    }

    #[test]
    fn nsis_retirement_preserves_unrelated_product_metadata() {
        let (mut registry, receipt) = retirement_fixture();
        retire_nsis_registration(&mut registry, &receipt).unwrap();
        assert!(!registry.keys.contains_key(&(RetirementKey::Uninstall, 256)));
        assert_eq!(registry.keys[&(RetirementKey::Product, 256)],
            BTreeMap::from([("UnrelatedValue".into(), RegistryValue::string("leave me alone"))]));
    }

    #[test]
    fn nsis_retirement_precheck_refuses_concurrent_metadata_without_writes() {
        for key in [RetirementKey::Uninstall, RetirementKey::Product] {
            let (mut registry, receipt) = retirement_fixture();
            let name = if key == RetirementKey::Product { "" } else { "DisplayName" };
            registry.keys.get_mut(&(key, 256)).unwrap()
                .insert(name.into(), RegistryValue::string("another owner"));
            let before = registry.keys.clone();
            assert!(retire_nsis_registration(&mut registry, &receipt).is_err());
            assert_eq!(registry.keys, before);
            assert_eq!(registry.writes, 0);
        }
    }

    #[test]
    fn nsis_retirement_rollback_restores_exact_bytes_after_recovery_cleanup_failure() {
        let (mut registry, receipt) = retirement_fixture();
        let before = registry.keys.clone();
        retire_nsis_registration(&mut registry, &receipt).unwrap();
        restore_nsis_registration(&mut registry, &receipt).unwrap();
        assert_eq!(registry.keys, before);
    }

    #[test]
    fn nsis_retirement_rollback_refuses_unknown_replacements() {
        let (mut registry, receipt) = retirement_fixture();
        retire_nsis_registration(&mut registry, &receipt).unwrap();
        registry.keys.insert((RetirementKey::Uninstall, 256),
            BTreeMap::from([("NewOwner".into(), RegistryValue::string("unrelated"))]));
        let before = registry.keys.clone();
        assert!(restore_nsis_registration(&mut registry, &receipt).is_err());
        assert_eq!(registry.keys, before);
    }

    #[test]
    fn nsis_retirement_rollback_failure_is_retryable_with_the_same_receipt() {
        let (mut registry, receipt) = retirement_fixture();
        let before = registry.keys.clone();
        retire_nsis_registration(&mut registry, &receipt).unwrap();
        registry.restore_failure = true;
        assert!(restore_nsis_registration(&mut registry, &receipt).is_err());
        registry.restore_failure = false;
        restore_nsis_registration(&mut registry, &receipt).unwrap();
        assert_eq!(registry.keys, before);
    }

    #[test]
    fn nsis_retirement_partial_metadata_restoration_is_retryable_without_overwriting() {
        let (mut registry, receipt) = retirement_fixture();
        let before = registry.keys.clone();
        retire_nsis_registration(&mut registry, &receipt).unwrap();
        registry.keys.insert((RetirementKey::Uninstall, 256), BTreeMap::from([
            ("DisplayName".into(), RegistryValue::string(PRODUCT_NAME)),
        ]));
        restore_nsis_registration(&mut registry, &receipt).unwrap();
        assert_eq!(registry.keys, before);
    }

    #[test]
    fn nsis_retirement_and_rollback_support_aliased_registry_views() {
        let (mut registry, mut receipt) = retirement_fixture();
        registry.alias_views = true;
        let mut second_view = receipt.registrations[0].clone();
        second_view.view = 512;
        receipt.registrations.push(second_view);
        let before = registry.keys.clone();

        retire_nsis_registration(&mut registry, &receipt).unwrap();
        for view in [256, 512] {
            assert!(registry.read(RetirementKey::Uninstall, view).unwrap().is_none());
            assert!(product_path(&registry.read(RetirementKey::Product, view).unwrap()).is_none());
        }
        restore_nsis_registration(&mut registry, &receipt).unwrap();
        assert_eq!(registry.keys, before);
    }

    #[test]
    fn nsis_retirement_checks_and_restores_independent_registry_views() {
        let (mut registry, mut receipt) = retirement_fixture();
        let mut second_view = receipt.registrations[0].clone();
        second_view.view = 512;
        receipt.registrations.push(second_view);
        for key in [RetirementKey::Uninstall, RetirementKey::Product] {
            registry.keys.insert((key, 512), registry.keys[&(key, 256)].clone());
        }
        let before = registry.keys.clone();
        registry.keys.get_mut(&(RetirementKey::Uninstall, 512)).unwrap()
            .insert("DisplayName".into(), RegistryValue::string("different owner"));
        assert!(retire_nsis_registration(&mut registry, &receipt).is_err());
        assert_eq!(registry.writes, 0, "all independent views must pass before any mutation");

        registry.keys = before.clone();
        retire_nsis_registration(&mut registry, &receipt).unwrap();
        for view in [256, 512] {
            assert!(registry.read(RetirementKey::Uninstall, view).unwrap().is_none());
            assert!(product_path(&registry.read(RetirementKey::Product, view).unwrap()).is_none());
        }
        restore_nsis_registration(&mut registry, &receipt).unwrap();
        assert_eq!(registry.keys, before);
    }

    fn registration(command: &str, owned: bool, approved: bool) -> StartupRegistration {
        StartupRegistration {
            value: RegistryValue::string(command),
            owned,
            approved,
        }
    }

    fn legacy_snapshot(approved: bool) -> StartupSnapshot {
        StartupSnapshot {
            stable: None,
            legacy: Some(registration(
                &format!("{IMAGE} --minimized"),
                true,
                approved,
            )),
            stable_name_approved: true,
        }
    }

    #[derive(Default)]
    struct MemoryRegistry {
        values: BTreeMap<String, RegistryValue>,
        writes: Vec<String>,
        fail_at: Option<usize>,
        fail_once_at: Option<usize>,
        fail_after_write_at: Option<usize>,
        corrupt_readback: bool,
    }

    impl StartupWriter for MemoryRegistry {
        fn read(&mut self, name: &str) -> Result<Option<RegistryValue>, String> {
            Ok(self.values.get(name).cloned())
        }

        fn write(&mut self, name: &str, value: Option<&RegistryValue>) -> Result<(), String> {
            if self.fail_at == Some(self.writes.len()) {
                return Err("injected access denied".to_owned());
            }
            if self.fail_once_at == Some(self.writes.len()) {
                self.fail_once_at = None;
                return Err("injected one-shot access denied".to_owned());
            }
            self.writes.push(name.to_owned());
            match value {
                Some(value) => {
                    self.values.insert(
                        name.to_owned(),
                        if self.corrupt_readback {
                            RegistryValue::string("concurrent unrelated replacement")
                        } else {
                            value.clone()
                        },
                    );
                }
                None => {
                    self.values.remove(name);
                }
            }
            if self.fail_after_write_at == Some(self.writes.len() - 1) {
                return Err("injected failure after the write applied".to_owned());
            }
            Ok(())
        }
    }

    fn registry_for(snapshot: &StartupSnapshot) -> MemoryRegistry {
        let mut registry = MemoryRegistry::default();
        for (name, value) in [
            (OWNED_STARTUP_NAME, snapshot.stable.as_ref()),
            (LEGACY_STARTUP_NAME, snapshot.legacy.as_ref()),
        ] {
            if let Some(value) = value {
                registry.values.insert(name.to_owned(), value.value.clone());
            }
        }
        registry
    }

    #[test]
    fn recognizes_the_actual_unquoted_v1_serialization_and_quoted_v2() {
        assert_eq!(
            startup_executable(&format!("{IMAGE} --minimized")),
            Some(IMAGE)
        );
        assert_eq!(
            startup_executable(&startup_command(IMAGE).unwrap()),
            Some(IMAGE)
        );
    }

    #[test]
    fn rejects_shell_commands_extra_arguments_relative_paths_and_broken_quotes() {
        for command in [
            "tauri-app.exe --minimized",
            "cmd.exe /c tauri-app.exe --minimized",
            "\"C:\\app\\tauri-app.exe\" --minimized --another-arg",
            "\"C:\\app\\tauri-app.exe --minimized",
            "\"C:\\app\\tauri-app.exe\" & other.exe --minimized",
            "C:\\app\\tauri-app.exe\0 --minimized",
        ] {
            assert_eq!(startup_executable(command), None, "{command}");
        }
    }

    #[test]
    fn registry_strings_require_exact_type_encoding_and_termination() {
        assert_eq!(RegistryValue::string(IMAGE).as_string().unwrap(), IMAGE);
        for value in [
            RegistryValue {
                kind: 2,
                bytes: vec![0, 0],
            },
            RegistryValue {
                kind: 1,
                bytes: vec![1],
            },
            RegistryValue {
                kind: 1,
                bytes: vec![0, 0, 0, 0],
            },
            RegistryValue {
                kind: 1,
                bytes: vec![0, 216, 0, 0],
            },
        ] {
            assert!(value.as_string().is_err());
        }
    }

    #[test]
    fn unrelated_generic_tauri_startup_is_never_changed() {
        let snapshot = StartupSnapshot {
            stable: None,
            legacy: Some(registration(
                r"C:\OtherApp\tauri-app.exe --minimized",
                false,
                true,
            )),
            stable_name_approved: true,
        };
        let mut registry = registry_for(&snapshot);
        let unrelated = registry.values.get(LEGACY_STARTUP_NAME).cloned();
        apply_startup(
            &mut registry,
            &plan_startup(&snapshot, true, IMAGE).unwrap(),
        )
        .unwrap();
        assert_eq!(registry.values.get(LEGACY_STARTUP_NAME).cloned(), unrelated);
        assert_eq!(registry.writes, [OWNED_STARTUP_NAME]);
        assert!(plan_startup(&snapshot, false, IMAGE).unwrap().is_empty());
    }

    #[test]
    fn conflicting_specific_name_blocks_before_any_mutation() {
        let mut snapshot = legacy_snapshot(true);
        snapshot.stable = Some(registration(
            r"C:\Unrelated\utility.exe --minimized",
            false,
            true,
        ));
        assert!(plan_startup(&snapshot, true, IMAGE).is_err());
        assert!(plan_startup(&snapshot, false, IMAGE).is_err());
        assert!(snapshot.enabled().is_err());
    }

    #[test]
    fn owned_legacy_is_retired_only_after_verified_specific_registration() {
        let snapshot = legacy_snapshot(true);
        let mut registry = registry_for(&snapshot);
        apply_startup(
            &mut registry,
            &plan_startup(&snapshot, true, IMAGE).unwrap(),
        )
        .unwrap();
        assert_eq!(registry.writes, [OWNED_STARTUP_NAME, LEGACY_STARTUP_NAME]);
        assert!(!registry.values.contains_key(LEGACY_STARTUP_NAME));
        assert_eq!(
            registry.values.get(OWNED_STARTUP_NAME),
            Some(&RegistryValue::string(&startup_command(IMAGE).unwrap()))
        );
    }

    #[test]
    fn disable_is_idempotent_and_never_creates_a_registration() {
        let snapshot = StartupSnapshot {
            stable_name_approved: true,
            ..Default::default()
        };
        assert!(plan_startup(&snapshot, false, IMAGE).unwrap().is_empty());
        let enabled = StartupSnapshot {
            stable: Some(registration(&startup_command(IMAGE).unwrap(), true, true)),
            ..snapshot
        };
        assert!(plan_startup(&enabled, true, IMAGE).unwrap().is_empty());
    }

    #[test]
    fn disabled_windows_startup_is_not_silently_reenabled_during_handoff() {
        let snapshot = legacy_snapshot(false);
        assert!(!snapshot.enabled().unwrap());
        let edits = plan_startup(&snapshot, snapshot.enabled().unwrap(), IMAGE).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].name, LEGACY_STARTUP_NAME);
        assert_eq!(edits[0].desired, None);
        assert!(plan_startup(&snapshot, true, IMAGE).is_err());
        let mut snapshot = snapshot;
        snapshot.stable_name_approved = false;
        assert!(plan_startup(&snapshot, true, IMAGE).is_err());
    }

    #[test]
    fn installer_preserves_a_disabled_legacy_entry_at_the_replaced_path() {
        let snapshot = legacy_snapshot(false);
        let edits = plan_installer_startup(&snapshot, false, IMAGE).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].name, LEGACY_STARTUP_NAME);
        assert_eq!(
            edits[0].desired,
            Some(RegistryValue::string(&startup_command(IMAGE).unwrap()))
        );
        let mut registry = registry_for(&snapshot);
        apply_startup(&mut registry, &edits).unwrap();
        assert!(!registry.values.contains_key(OWNED_STARTUP_NAME));
        assert!(registry.values.contains_key(LEGACY_STARTUP_NAME));
    }

    #[test]
    fn installer_preserves_disabled_specific_entry_and_retires_only_owned_duplicate() {
        let snapshot = StartupSnapshot {
            stable: Some(registration(&startup_command(IMAGE).unwrap(), true, false)),
            stable_name_approved: false,
            ..legacy_snapshot(false)
        };
        let edits = plan_installer_startup(&snapshot, false, IMAGE).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].name, LEGACY_STARTUP_NAME);
        assert_eq!(edits[0].desired, None);
        assert_eq!(
            plan_installer_startup(&snapshot, true, IMAGE)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn approval_absence_enabled_disabled_and_unknown_are_distinct() {
        assert!(startup_approved(None).unwrap());
        for (state, result) in [(2, Some(true)), (3, Some(false)), (8, None)] {
            let mut bytes = vec![0; 12];
            bytes[0] = state;
            assert_eq!(
                startup_approved(Some(&RegistryValue { kind: 3, bytes })).ok(),
                result
            );
        }
        assert!(startup_approved(Some(&RegistryValue::string("enabled"))).is_err());
    }

    #[test]
    fn concurrent_change_is_not_overwritten_or_deleted() {
        let snapshot = legacy_snapshot(true);
        let edits = plan_startup(&snapshot, true, IMAGE).unwrap();
        let mut registry = registry_for(&snapshot);
        registry.values.insert(
            LEGACY_STARTUP_NAME.to_owned(),
            RegistryValue::string(r"C:\AnotherApp\tauri-app.exe --minimized"),
        );
        assert!(apply_startup(&mut registry, &edits).is_err());
        assert!(registry.writes.is_empty());
    }

    #[test]
    fn failed_new_registration_does_not_remove_legacy() {
        let snapshot = legacy_snapshot(true);
        let mut registry = registry_for(&snapshot);
        registry.fail_at = Some(0);
        assert!(apply_startup(
            &mut registry,
            &plan_startup(&snapshot, true, IMAGE).unwrap()
        )
        .is_err());
        assert!(registry.values.contains_key(LEGACY_STARTUP_NAME));
        assert!(!registry.values.contains_key(OWNED_STARTUP_NAME));
    }

    #[test]
    fn partial_failure_is_explicit_and_keeps_the_confirmed_new_registration() {
        let snapshot = legacy_snapshot(true);
        let mut registry = registry_for(&snapshot);
        registry.fail_at = Some(1);
        let error = apply_startup(
            &mut registry,
            &plan_startup(&snapshot, true, IMAGE).unwrap(),
        )
        .unwrap_err();
        assert!(error.contains("1 prior"));
        assert!(error.contains("partially applied"));
        assert!(registry.values.contains_key(LEGACY_STARTUP_NAME));
        assert!(registry.values.contains_key(OWNED_STARTUP_NAME));
    }

    #[test]
    fn uncertain_write_does_not_retire_legacy_or_report_success() {
        let snapshot = legacy_snapshot(true);
        let mut registry = registry_for(&snapshot);
        registry.corrupt_readback = true;
        assert!(apply_startup(
            &mut registry,
            &plan_startup(&snapshot, true, IMAGE).unwrap()
        )
        .is_err());
        assert!(registry.values.contains_key(LEGACY_STARTUP_NAME));
        assert_eq!(registry.writes, [OWNED_STARTUP_NAME]);
    }

    #[test]
    fn canonical_commit_runs_only_after_verified_startup_and_preserves_its_result() {
        let snapshot = legacy_snapshot(true);
        let mut registry = registry_for(&snapshot);
        let edits = plan_startup(&snapshot, true, IMAGE).unwrap();
        let verified = std::cell::Cell::new(false);
        let result = commit_startup_change(
            &mut registry,
            &edits,
            || {
                verified.set(true);
                Ok(())
            },
            || panic!("successful commit must not attempt rollback"),
            || {
                assert!(verified.get());
                Ok(42)
            },
        );
        assert_eq!(result.unwrap(), 42);
        assert!(registry.values.contains_key(OWNED_STARTUP_NAME));
        assert!(!registry.values.contains_key(LEGACY_STARTUP_NAME));
    }

    #[test]
    fn canonical_commit_failure_restores_exact_unquoted_legacy_bytes() {
        let snapshot = legacy_snapshot(true);
        let mut registry = registry_for(&snapshot);
        let before = registry.values.clone();
        let error = commit_startup_change(
            &mut registry,
            &plan_startup(&snapshot, true, IMAGE).unwrap(),
            || Ok(()),
            || Ok(()),
            || Err::<(), _>("injected config write failure".to_owned()),
        )
        .unwrap_err();
        assert!(error.contains("injected config write failure"));
        assert!(error.contains("exact prior state"));
        assert_eq!(registry.values, before);
        assert_eq!(
            registry.writes,
            [
                OWNED_STARTUP_NAME,
                LEGACY_STARTUP_NAME,
                LEGACY_STARTUP_NAME,
                OWNED_STARTUP_NAME
            ]
        );
    }

    #[test]
    fn failed_disable_restores_disabled_registration_instead_of_only_boolean_state() {
        let snapshot = legacy_snapshot(false);
        let mut registry = registry_for(&snapshot);
        let before = registry.values.clone();
        assert!(commit_startup_change(
            &mut registry,
            &plan_startup(&snapshot, false, IMAGE).unwrap(),
            || Ok(()),
            || Ok(()),
            || Err::<(), _>("disk full".to_owned()),
        )
        .is_err());
        assert_eq!(registry.values, before);
        assert!(registry.values.contains_key(LEGACY_STARTUP_NAME));
        assert!(!snapshot.enabled().unwrap());
    }

    #[test]
    fn denied_enabling_disabled_registration_creates_no_transaction_or_rollback() {
        let snapshot = legacy_snapshot(false);
        let registry = registry_for(&snapshot);
        let before = registry.values.clone();
        assert!(plan_startup(&snapshot, true, IMAGE).is_err());
        assert_eq!(registry.values, before);
        assert!(registry.writes.is_empty());
    }

    #[test]
    fn partial_startup_failure_undoes_only_applied_values_without_calling_commit() {
        let snapshot = legacy_snapshot(true);
        let mut registry = registry_for(&snapshot);
        let before = registry.values.clone();
        registry.fail_once_at = Some(1);
        let error = commit_startup_change::<()>(
            &mut registry,
            &plan_startup(&snapshot, true, IMAGE).unwrap(),
            || panic!("partially applied startup must not be validated as complete"),
            || Ok(()),
            || panic!("partially applied startup must not commit configuration"),
        )
        .unwrap_err();
        assert!(error.contains("settings were not saved"));
        assert_eq!(registry.values, before);
        assert_eq!(registry.writes, [OWNED_STARTUP_NAME, OWNED_STARTUP_NAME]);
    }

    #[test]
    fn error_after_a_write_applied_is_reconciled_and_undone_without_committing() {
        let snapshot = legacy_snapshot(true);
        let mut registry = registry_for(&snapshot);
        let before = registry.values.clone();
        registry.fail_after_write_at = Some(0);
        let error = commit_startup_change::<()>(
            &mut registry,
            &plan_startup(&snapshot, true, IMAGE).unwrap(),
            || panic!("an error result must not be accepted as complete"),
            || Ok(()),
            || panic!("a reported startup failure must not commit"),
        )
        .unwrap_err();
        assert!(error.contains("failure after the write applied"));
        assert_eq!(registry.values, before);
        assert_eq!(registry.writes, [OWNED_STARTUP_NAME, OWNED_STARTUP_NAME]);
    }

    #[test]
    fn precheck_conflict_matching_desired_value_is_not_mistaken_for_our_write() {
        let snapshot = legacy_snapshot(true);
        let mut registry = registry_for(&snapshot);
        let edits = plan_startup(&snapshot, true, IMAGE).unwrap();
        registry.values.insert(
            OWNED_STARTUP_NAME.to_owned(),
            edits[0].desired.clone().unwrap(),
        );
        let before = registry.values.clone();
        let result: Result<(), _> = commit_startup_change(
            &mut registry,
            &edits,
            || panic!("a failed precheck must not validate applied writes"),
            || panic!("no attempted writes require ownership rollback"),
            || panic!("a failed precheck must not commit"),
        );
        assert!(result.is_err());
        assert_eq!(registry.values, before);
        assert!(registry.writes.is_empty());
    }

    #[test]
    fn effective_state_validation_failure_blocks_commit_and_restores_exact_values() {
        let snapshot = legacy_snapshot(true);
        let mut registry = registry_for(&snapshot);
        let before = registry.values.clone();
        let result: Result<(), _> = commit_startup_change(
            &mut registry,
            &plan_startup(&snapshot, true, IMAGE).unwrap(),
            || Err("Windows changed its startup approval".to_owned()),
            || Ok(()),
            || panic!("unverified effective startup must not commit"),
        );
        assert!(result
            .unwrap_err()
            .contains("Windows changed its startup approval"));
        assert_eq!(registry.values, before);
    }

    #[test]
    fn rollback_refuses_an_unknown_concurrent_registry_replacement() {
        let snapshot = legacy_snapshot(true);
        let mut registry = registry_for(&snapshot);
        registry.corrupt_readback = true;
        let result: Result<(), _> = commit_startup_change(
            &mut registry,
            &plan_startup(&snapshot, true, IMAGE).unwrap(),
            || panic!("unconfirmed write must not be accepted"),
            || panic!("unrelated replacement must not authorize restoration"),
            || panic!("unconfirmed write must not commit"),
        );
        assert!(result.unwrap_err().contains("outside this transaction"));
        assert_eq!(registry.writes, [OWNED_STARTUP_NAME]);
        assert!(registry.values.contains_key(LEGACY_STARTUP_NAME));
    }

    #[test]
    fn rollback_revalidates_installed_product_ownership_before_restoring_values() {
        let snapshot = legacy_snapshot(true);
        let mut registry = registry_for(&snapshot);
        let result = commit_startup_change(
            &mut registry,
            &plan_startup(&snapshot, true, IMAGE).unwrap(),
            || Ok(()),
            || Err("installed product was removed concurrently".to_owned()),
            || Err::<(), _>("commit failed".to_owned()),
        );
        let error = result.unwrap_err();
        assert!(error.contains("rollback was not confirmed"));
        assert!(error.contains("installed product was removed"));
        assert_eq!(registry.writes, [OWNED_STARTUP_NAME, LEGACY_STARTUP_NAME]);
    }

    #[test]
    fn rollback_io_failure_is_reported_in_addition_to_original_commit_failure() {
        let snapshot = legacy_snapshot(true);
        let mut registry = registry_for(&snapshot);
        registry.fail_at = Some(2);
        let result = commit_startup_change(
            &mut registry,
            &plan_startup(&snapshot, true, IMAGE).unwrap(),
            || Ok(()),
            || Ok(()),
            || Err::<(), _>("canonical commit failed".to_owned()),
        );
        let error = result.unwrap_err();
        assert!(error.contains("canonical commit failed"));
        assert!(error.contains("rollback was not confirmed"));
        assert!(error.contains("access denied"));
    }

    #[test]
    fn only_a_running_registered_same_user_image_blocks() {
        let evidence = ProcessEvidence {
            pid: 10,
            same_user: true,
            registered_image: true,
            exited: false,
        };
        assert!(require_predecessor_exit(20, [Ok(evidence.clone())], false).is_err());
        assert!(require_predecessor_exit(10, [Ok(evidence.clone())], false).is_ok());
        for other in [
            ProcessEvidence {
                same_user: false,
                ..evidence.clone()
            },
            ProcessEvidence {
                registered_image: false,
                ..evidence.clone()
            },
            ProcessEvidence {
                exited: true,
                ..evidence
            },
        ] {
            assert!(require_predecessor_exit(20, [Ok(other)], false).is_ok());
        }
    }

    #[test]
    fn unresolved_predecessor_status_pauses_and_retry_requires_new_evidence() {
        assert!(require_predecessor_exit(20, [Err("access denied".to_owned())], false).is_err());
        let still_running = ProcessEvidence {
            pid: 10,
            same_user: true,
            registered_image: true,
            exited: false,
        };
        assert!(require_predecessor_exit(20, [Ok(still_running.clone())], false).is_err());
        assert!(require_predecessor_exit(20, [Ok(still_running.clone())], false).is_err());
        assert!(require_predecessor_exit(
            20,
            [Ok(ProcessEvidence {
                exited: true,
                ..still_running
            })],
            false
        )
        .is_ok());
    }

    #[test]
    fn installer_refuses_shared_files_in_use_without_targeting_another_user() {
        let other_user = ProcessEvidence {
            pid: 10,
            same_user: false,
            registered_image: true,
            exited: false,
        };
        assert!(require_predecessor_exit(20, [Ok(other_user.clone())], false).is_ok());
        let error = require_predecessor_exit(20, [Ok(other_user)], true).unwrap_err();
        assert!(error.contains("will not request termination"));
    }
}
