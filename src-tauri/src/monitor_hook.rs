use crate::config::{
    ConfigManager, ConfigMode, ConfigSnapshot, SwitchMethod, TargetMonitor,
};
#[cfg(test)]
use crate::config::{AppConfig, HdrApp};
pub use crate::display::ScopeHdrState;
use crate::display::{
    DisplayFailure, FailureKind, MonitorInfo, MonitorOutcome, NativeAttempt, NativePurpose,
    OutcomeKind, TargetStatus, WindowsDisplay,
};
use crate::hdr_controller::{same_target, HdrController, ProcessIdentity, WriteAuthority};
use crate::runtime_policy::{quarantined_apps, QuarantinedApp, Resolution};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use windows::core::PWSTR;
use windows::Win32::Foundation::{
    CloseHandle, FILETIME, HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentThreadId, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
    SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE, PROCESS_NAME_FORMAT,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, PeekMessageW,
    PostThreadMessageW, TranslateMessage, EVENT_SYSTEM_FOREGROUND, MSG, PM_NOREMOVE,
    WINEVENT_OUTOFCONTEXT, WM_QUIT,
};

const SHUTDOWN_ATTEMPT_BUDGET: usize = 32;
const FOREGROUND_WATCHDOG_INTERVAL: Duration = Duration::from_secs(1);
const INVENTORY_WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);
const FOREGROUND_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];
const FOREGROUND_RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ManualControl {
    Available,
    Blocked { reason: String },
}

impl ManualControl {
    fn require(self) -> Result<(), String> {
        match self {
            Self::Available => Ok(()),
            Self::Blocked { reason } => Err(reason),
        }
    }
}

fn manual_admission(snapshot: &ConfigSnapshot, admitted: bool) -> ManualControl {
    let reason = if !admitted {
        Some("The HDR controller is shutting down".into())
    } else if let Some(issue) = &snapshot.controller_issue {
        Some(issue.clone())
    } else if snapshot.mode == ConfigMode::Unavailable {
        Some("Configuration authority is unavailable; manual HDR is blocked".into())
    } else {
        None
    };
    match reason {
        Some(reason) => ManualControl::Blocked { reason },
        None => ManualControl::Available,
    }
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct ManualScopeResult {
    pub revision: String,
    pub scope: TargetMonitor,
    pub request: ManualRequestIdentity,
    pub verified: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManualRequestIdentity {
    pub client_id: String,
    pub sequence: String,
}

impl ManualRequestIdentity {
    fn validate(&self) -> Result<u64, String> {
        let sequence = self.sequence.parse::<u64>().map_err(|_| "Invalid manual request sequence")?;
        if sequence == 0 || sequence.to_string() != self.sequence || self.client_id.is_empty()
            || self.client_id.len() > 128
            || !self.client_id.bytes().all(|c| c.is_ascii_alphanumeric() || b":-".contains(&c))
        {
            return Err("Invalid manual request identity".into());
        }
        Ok(sequence)
    }
}

#[derive(Default)]
struct ManualObservations {
    revision: u64,
    scopes: Vec<ManualScopeResult>,
}

impl ManualObservations {
    fn begin(&mut self) -> Result<(), String> {
        self.revision = self.revision.checked_add(1).ok_or("Manual HDR sequence exhausted")?;
        Ok(())
    }

    fn admit(&self, scope: &TargetMonitor, request: &ManualRequestIdentity) -> Result<(), String> {
        let sequence = request.validate()?;
        if self.scopes.iter().any(|entry| same_target(&entry.scope, scope)
            && entry.request.client_id == request.client_id
            && entry.request.validate().is_ok_and(|completed| completed >= sequence))
        {
            return Err("This manual request was superseded or already completed. Retry the control.".into());
        }
        Ok(())
    }

    fn finish(&mut self, scope: TargetMonitor, request: ManualRequestIdentity, verified: bool, error: Option<String>) {
        // Keep correlation proofs across origins, but current admission errors belong
        // to the latest executed request for this scope, not each client's history.
        for entry in self.scopes.iter_mut().filter(|entry| same_target(&entry.scope, &scope)) {
            entry.error = None;
        }
        let observed = ManualScopeResult { revision: self.revision.to_string(), scope, request, verified, error };
        if let Some(previous) = self.scopes.iter_mut().find(|entry| same_target(&entry.scope, &observed.scope)
            && entry.request.client_id == observed.request.client_id)
        {
            *previous = observed;
        } else {
            self.scopes.push(observed);
        }
    }

    fn append_errors(&self, warnings: &mut Vec<String>) {
        for error in self.scopes.iter().filter_map(|entry| entry.error.as_ref()) {
            if !warnings.contains(error) {
                warnings.push(error.clone());
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct HdrStatePayload {
    /// Decimal strings preserve ordering beyond JavaScript's integer precision.
    pub status_revision: String,
    pub inventory_revision: String,
    pub manual_revision: String,
    pub manual_results: Vec<ManualScopeResult>,
    /// Observed frozen activation scope, or the saved scope when no activation exists.
    pub is_hdr_active: bool,
    pub scope_hdr_state: ScopeHdrState,
    pub manual_control: ManualControl,
    pub current_app_name: Option<String>,
    pub current_exe: Option<String>,
    pub switched_by_app: bool,
    pub steam_id: Option<String>,
    pub launcher: Option<String>,
    pub hdr_type: Option<String>,
    pub warning: Option<String>,
    pub quarantined_apps: Vec<QuarantinedApp>,
    /// Availability of the saved target for the next activation.
    pub target_status: TargetStatus,
    pub active_target: Option<TargetMonitor>,
    pub target_deferred: bool,
    pub any_hdr_active: bool,
    pub inventory_stale: bool,
    pub uncertain_targets: Vec<String>,
    pub operation_outcomes: Vec<MonitorOutcome>,
}

impl HdrStatePayload {
    fn unavailable(message: String) -> Self {
        Self {
            status_revision: "0".into(),
            inventory_revision: "0".into(),
            manual_revision: "0".into(),
            manual_results: Vec::new(),
            is_hdr_active: false,
            scope_hdr_state: ScopeHdrState::Unknown,
            manual_control: ManualControl::Blocked { reason: message.clone() },
            current_app_name: None,
            current_exe: None,
            switched_by_app: false,
            steam_id: None,
            launcher: None,
            hdr_type: None,
            warning: Some(message),
            quarantined_apps: Vec::new(),
            target_status: TargetStatus::AutomationPaused,
            active_target: None,
            target_deferred: false,
            any_hdr_active: false,
            inventory_stale: true,
            uncertain_targets: Vec::new(),
            operation_outcomes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ManualSetResult {
    pub scope: TargetMonitor,
    pub request: ManualRequestIdentity,
    pub outcomes: Vec<MonitorOutcome>,
    pub partial: bool,
    pub status: HdrStatePayload,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MonitorInventorySnapshot {
    pub inventory_revision: String,
    pub monitors: Vec<MonitorInfo>,
}

enum Command {
    ForegroundObserved,
    ConfigCommitted,
    Refresh(Sender<Result<MonitorInventorySnapshot, String>>),
    ManualSet(TargetMonitor, bool, ManualRequestIdentity, Sender<Result<ManualSetResult, String>>),
    Status(Sender<Result<HdrStatePayload, String>>),
    GameExited {
        generation: u64,
        process: ProcessIdentity,
    },
    WatcherFailed {
        generation: u64,
        process: ProcessIdentity,
        message: String,
    },
    HookUnavailable(String),
    Shutdown,
}

#[derive(Clone)]
struct EventSink {
    sender: Sender<Command>,
    admitted: Arc<AtomicBool>,
    foreground_pending: Arc<AtomicBool>,
    foreground_generation: Arc<AtomicU64>,
    config_pending: Arc<AtomicBool>,
    hook_available: Arc<AtomicBool>,
    shutdown_budget: Arc<AtomicUsize>,
}

impl EventSink {
    fn foreground(&self) {
        if !self.admitted.load(Ordering::Acquire) {
            return;
        }
        // Count even coalesced hints so an in-flight observation cannot cross a focus change.
        self.foreground_generation.fetch_add(1, Ordering::AcqRel);
        if !self.foreground_pending.swap(true, Ordering::AcqRel)
            && self.sender.send(Command::ForegroundObserved).is_err()
        {
            self.foreground_pending.store(false, Ordering::Release);
        }
    }

    fn foreground_key(&self) -> ForegroundKey {
        ForegroundKey {
            generation: self.foreground_generation.load(Ordering::Acquire),
            pid: foreground_pid(),
        }
    }
}

struct HookThread {
    id: u32,
    thread: JoinHandle<()>,
}

#[derive(Default)]
struct Threads {
    actor: Option<JoinHandle<()>>,
    hook: Option<HookThread>,
}

pub struct MonitorService {
    config: Arc<ConfigManager>,
    events: EventSink,
    threads: Mutex<Threads>,
}

static HOOK_EVENTS: OnceLock<Mutex<Option<EventSink>>> = OnceLock::new();

pub struct PendingRequest<T> {
    receiver: Receiver<Result<T, String>>,
}

impl<T: Send + 'static> PendingRequest<T> {
    pub async fn resolve(self) -> Result<T, String> {
        tauri::async_runtime::spawn_blocking(move || {
            self.receiver
                .recv()
                .map_err(|_| "The HDR controller stopped before responding".to_string())?
        })
        .await
        .map_err(|error| format!("Cannot receive HDR controller response: {error}"))?
    }
}

impl MonitorService {
    pub fn new(config_mgr: Arc<ConfigManager>, app_handle: AppHandle) -> Arc<Self> {
        let (sender, receiver) = channel();
        let events = EventSink {
            sender,
            admitted: Arc::new(AtomicBool::new(true)),
            foreground_pending: Arc::new(AtomicBool::new(false)),
            foreground_generation: Arc::new(AtomicU64::new(0)),
            config_pending: Arc::new(AtomicBool::new(false)),
            hook_available: Arc::new(AtomicBool::new(false)),
            shutdown_budget: Arc::new(AtomicUsize::new(SHUTDOWN_ATTEMPT_BUDGET)),
        };
        let actor_events = events.clone();
        let actor_config = config_mgr.clone();
        let actor = thread::Builder::new()
            .name("hdr-controller".into())
            .spawn(move || {
                Actor::new(actor_config, app_handle, actor_events).run(receiver);
            })
            .expect("Unable to start the HDR controller");
        Arc::new(Self {
            config: config_mgr,
            events,
            threads: Mutex::new(Threads {
                actor: Some(actor),
                hook: None,
            }),
        })
    }

    pub fn start_hook(self: &Arc<Self>) {
        let mut threads = self
            .threads
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.events.admitted.load(Ordering::Acquire) || threads.hook.is_some() {
            return;
        }
        let global = HOOK_EVENTS.get_or_init(|| Mutex::new(None));
        *global.lock().unwrap_or_else(|error| error.into_inner()) = Some(self.events.clone());
        let (ready, ready_receiver) = channel();
        let events = self.events.clone();
        let hook = thread::Builder::new()
            .name("hdr-foreground-hook".into())
            .spawn(move || unsafe {
                let id = GetCurrentThreadId();
                let mut message = MSG::default();
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                let hook = SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(win_event_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );
                if hook.is_invalid() {
                    let _ = ready.send(Err(
                        "Unable to install the foreground window hook".to_string()
                    ));
                    return;
                }
                events.hook_available.store(true, Ordering::Release);
                let _ = ready.send(Ok(id));
                events.foreground();
                loop {
                    let result = GetMessageW(&mut message, None, 0, 0);
                    if result.0 <= 0 {
                        break;
                    }
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                let _ = UnhookWinEvent(hook);
                events.hook_available.store(false, Ordering::Release);
                if events.admitted.load(Ordering::Acquire) {
                    let _ = events.sender.send(Command::HookUnavailable(
                        "The foreground hook stopped; automatic HDR is paused".into(),
                    ));
                }
            });
        match hook {
            Ok(thread) => match ready_receiver.recv() {
                Ok(Ok(id)) => threads.hook = Some(HookThread { id, thread }),
                result => {
                    let _ = thread.join();
                    let message = match result {
                        Ok(Err(error)) => error,
                        _ => "The foreground hook stopped during initialization".into(),
                    };
                    let _ = self.events.sender.send(Command::HookUnavailable(message));
                }
            },
            Err(error) => {
                let _ = self.events.sender.send(Command::HookUnavailable(format!(
                    "Unable to start the foreground hook: {error}"
                )));
            }
        }
        self.events.foreground();
    }

    fn enqueue<T: Send + 'static>(
        &self,
        command: impl FnOnce(Sender<Result<T, String>>) -> Command,
    ) -> Result<PendingRequest<T>, String> {
        if !self.events.admitted.load(Ordering::Acquire) {
            return Err("The HDR controller is shutting down".into());
        }
        let (reply, receive) = channel();
        self.events
            .sender
            .send(command(reply))
            .map_err(|_| "The HDR controller is unavailable".to_string())?;
        Ok(PendingRequest { receiver: receive })
    }

    pub fn refresh(&self) -> Result<PendingRequest<MonitorInventorySnapshot>, String> {
        self.enqueue(Command::Refresh)
    }

    pub fn manual_set(
        &self,
        scope: TargetMonitor,
        enable: bool,
        request: ManualRequestIdentity,
    ) -> Result<PendingRequest<ManualSetResult>, String> {
        request.validate()?;
        manual_admission(&self.config.snapshot()?, self.events.admitted.load(Ordering::Acquire))
            .require()?;
        self.enqueue(|reply| Command::ManualSet(scope, enable, request, reply))
    }

    pub fn status(&self) -> Result<PendingRequest<HdrStatePayload>, String> {
        self.enqueue(Command::Status)
    }

    pub fn config_committed(&self) {
        if self.events.admitted.load(Ordering::Acquire)
            && !self.events.config_pending.swap(true, Ordering::AcqRel)
            && self.events.sender.send(Command::ConfigCommitted).is_err()
        {
            self.events.config_pending.store(false, Ordering::Release);
        }
    }

    pub fn shutdown(&self) {
        let mut threads = self
            .threads
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.events.admitted.store(false, Ordering::Release);
        self.events.hook_available.store(false, Ordering::Release);
        if let Some(global) = HOOK_EVENTS.get() {
            *global.lock().unwrap_or_else(|error| error.into_inner()) = None;
        }
        if let Some(hook) = threads.hook.take() {
            let _ = unsafe { PostThreadMessageW(hook.id, WM_QUIT, WPARAM(0), LPARAM(0)) };
            let _ = hook.thread.join();
        }
        if let Some(actor) = threads.actor.take() {
            let _ = self.events.sender.send(Command::Shutdown);
            // Never detach a timed-out actor: a synchronous native call cannot be cancelled.
            // The caller retains controller exclusion until this join has completed.
            if actor.join().is_err() {
                eprintln!("HDR controller exited unexpectedly; hardware state may require manual verification");
            }
        }
    }
}

impl Drop for MonitorService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if let Some(events) = HOOK_EVENTS.get() {
        if let Ok(events) = events.lock() {
            if let Some(events) = events.as_ref() {
                events.foreground();
            }
        }
    }
}

struct OwnedHandle(HANDLE);

// Kernel process/event handles support cross-thread waits; one RAII owner closes each handle.
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[derive(Clone)]
struct TrackedProcess {
    identity: ProcessIdentity,
    exe: String,
    path: String,
    handle: Arc<OwnedHandle>,
}

impl TrackedProcess {
    fn is_alive(&self) -> bool {
        unsafe { WaitForSingleObject(self.handle.0, 0) == WAIT_TIMEOUT }
    }

    fn is_foreground(&self) -> bool {
        self.is_alive() && foreground_pid() == self.identity.pid
    }
}

fn foreground_pid() -> u32 {
    let window = unsafe { GetForegroundWindow() };
    if window.0.is_null() {
        return 0;
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut pid));
    }
    pid
}

fn observe_foreground() -> Result<Option<TrackedProcess>, String> {
    observe_foreground_pid(foreground_pid())
}

fn observe_foreground_pid(pid: u32) -> Result<Option<TrackedProcess>, String> {
    if pid == 0 {
        return Ok(None);
    }
    let handle = Arc::new(OwnedHandle(
        unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
        }
        .map_err(|error| format!("Unable to inspect foreground process {pid}: {error}"))?,
    ));
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            handle.0,
            PROCESS_NAME_FORMAT(0),
            PWSTR(path.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(|error| format!("Unable to read foreground executable for process {pid}: {error}"))?;
    let path = String::from_utf16_lossy(&path[..length as usize]);
    let exe = Path::new(&path)
        .file_name()
        .ok_or_else(|| format!("Foreground executable for process {pid} has no filename"))?
        .to_string_lossy()
        .to_lowercase();
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(handle.0, &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(|error| format!("Unable to identify foreground process {pid} lifetime: {error}"))?;
    let process = TrackedProcess {
        identity: ProcessIdentity {
            pid,
            created_at: (u64::from(created.dwHighDateTime) << 32)
                | u64::from(created.dwLowDateTime),
        },
        exe,
        path,
        handle,
    };
    if process.is_foreground() {
        Ok(Some(process))
    } else {
        Ok(None)
    }
}

struct ProcessWatcher {
    generation: u64,
    process: ProcessIdentity,
    cancelled: Arc<OwnedHandle>,
    thread: Option<JoinHandle<()>>,
}

impl ProcessWatcher {
    fn new(process: TrackedProcess, generation: u64, events: EventSink) -> Result<Self, String> {
        let cancelled = Arc::new(OwnedHandle(
            unsafe { CreateEventW(None, true, false, None) }.map_err(|error| {
                format!("Unable to create process-watcher cancellation event: {error}")
            })?,
        ));
        let cancellation = cancelled.clone();
        let identity = process.identity;
        let thread = thread::Builder::new()
            .name("hdr-game-exit".into())
            .spawn(move || {
                let result = unsafe {
                    WaitForMultipleObjects(&[process.handle.0, cancellation.0], false, INFINITE)
                };
                if result == WAIT_OBJECT_0
                    && events.admitted.load(Ordering::Acquire)
                    && unsafe { WaitForSingleObject(cancellation.0, 0) } == WAIT_TIMEOUT
                {
                    let _ = events.sender.send(Command::GameExited {
                        generation,
                        process: identity,
                    });
                } else if result.0 != WAIT_OBJECT_0.0 + 1
                    && result != WAIT_OBJECT_0
                    && events.admitted.load(Ordering::Acquire)
                {
                    let _ = events.sender.send(Command::WatcherFailed {
                        generation,
                        process: identity,
                        message: format!(
                            "The process-exit watcher failed (wait result {:#x})",
                            result.0
                        ),
                    });
                }
            })
            .map_err(|error| format!("Unable to start process-exit watcher: {error}"))?;
        Ok(Self {
            generation,
            process: identity,
            cancelled,
            thread: Some(thread),
        })
    }
}

impl Drop for ProcessWatcher {
    fn drop(&mut self) {
        let _ = unsafe { SetEvent(self.cancelled.0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone, Copy)]
struct Debounce {
    deadline: Instant,
    generation: u64,
    process: ProcessIdentity,
    seconds: u64,
}

fn actor_wait_timeout(
    debounce: Option<Debounce>,
    watchdog_deadline: Instant,
    now: Instant,
) -> Duration {
    let watchdog_wait = watchdog_deadline.saturating_duration_since(now);
    debounce
        .map(|timer| {
            timer
                .deadline
                .saturating_duration_since(now)
                .min(watchdog_wait)
        })
        .unwrap_or(watchdog_wait)
}

fn take_expired_debounce(debounce: &mut Option<Debounce>, now: Instant) -> Option<Debounce> {
    if debounce.is_some_and(|timer| timer.deadline <= now) {
        debounce.take()
    } else {
        None
    }
}

struct InventoryObservation {
    deadline: Instant,
}

impl InventoryObservation {
    fn new(now: Instant) -> Self {
        Self { deadline: now + INVENTORY_WATCHDOG_INTERVAL }
    }

    fn due(&self, now: Instant) -> bool {
        self.deadline <= now
    }

    fn refreshed(&mut self, finished: Instant) {
        self.deadline = finished + INVENTORY_WATCHDOG_INTERVAL;
    }
}

#[derive(Default)]
struct ActivationPreparation {
    warning: Option<String>,
}

impl ActivationPreparation {
    fn observed(&mut self, result: Result<(), String>) {
        self.warning = result.err();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ForegroundKey {
    pid: u32,
    generation: u64,
}

#[derive(Default)]
struct ForegroundObservation {
    key: Option<ForegroundKey>,
    // This cache is independent of the activation retained for exit policy and owned cleanup.
    process: Option<TrackedProcess>,
    completed: bool,
    failures: usize,
    retry_at: Option<Instant>,
    warning: Option<String>,
}

impl ForegroundObservation {
    fn synchronize(&mut self, key: ForegroundKey) {
        let changed_pid = self.key.map(|previous| previous.pid) != Some(key.pid);
        // The retained handle binds the creation identity; a reused PID cannot keep it alive.
        let exited = self.process.as_ref().is_some_and(|process| !process.is_alive());
        if changed_pid || exited {
            self.process = None;
            self.completed = false;
            self.failures = 0;
            self.retry_at = None;
            self.warning = None;
        }
        self.key = Some(key);
    }

    fn needs_observation(&mut self, key: ForegroundKey, now: Instant) -> bool {
        self.synchronize(key);
        !self.completed && self.retry_at.is_none_or(|deadline| deadline <= now)
    }

    fn retry(&mut self, now: Instant, message: String) {
        // Exhaustion rearms one slow probe at a time, not another burst of fast retries.
        let delay = FOREGROUND_RETRY_DELAYS
            .get(self.failures)
            .copied()
            .unwrap_or(FOREGROUND_RECOVERY_INTERVAL);
        self.failures = (self.failures + 1).min(FOREGROUND_RETRY_DELAYS.len());
        self.retry_at = Some(now + delay);
        if self.warning.as_ref() != Some(&message) {
            eprintln!("{message}");
            self.warning = Some(message);
        }
    }

    fn sample(
        &mut self,
        key: ForegroundKey,
        now: Instant,
        inspect: impl FnOnce(u32) -> Result<Option<TrackedProcess>, String>,
        current: impl FnOnce() -> (ForegroundKey, Instant),
    ) -> Option<TrackedProcess> {
        if !self.needs_observation(key, now) {
            return self.process.clone();
        }
        let result = inspect(key.pid);
        let (latest, finished) = current();
        self.synchronize(latest);
        if latest != key {
            if latest.pid == key.pid {
                self.retry(finished, format!(
                    "Foreground process {} changed during inspection; retrying", key.pid
                ));
            }
            return None;
        }
        let failure = match result {
            Ok(Some(process)) if process.identity.pid == key.pid && process.is_alive() => {
                self.process = Some(process);
                None
            }
            Ok(None) if key.pid == 0 => None,
            Ok(_) => Some(format!(
                "Foreground process {} was not fully observed; retrying", key.pid
            )),
            Err(error) => Some(error),
        };
        if let Some(message) = failure {
            self.process = None;
            self.retry(finished, message);
        } else {
            self.completed = true;
            self.failures = 0;
            self.retry_at = None;
            self.warning = None;
        }
        self.process.clone()
    }

    fn warnings(&self, persistent: Option<String>) -> Vec<String> {
        persistent.into_iter().chain(self.warning.clone()).collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OperationKind {
    AutomaticEnable,
    Manual,
    Cleanup,
}

#[cfg(test)]
fn eligible_app<'a>(config: &'a AppConfig, exe: &str) -> Option<&'a HdrApp> {
    config.resolve_app(None, exe).matched()
}

#[cfg(test)]
fn automatic_pause(snapshot: &ConfigSnapshot, exe: &str, origin_context: &str) -> Option<String> {
    automatic_pause_for_path(snapshot, None, exe, origin_context)
}

fn automatic_pause_for_path(
    snapshot: &ConfigSnapshot, path: Option<&str>, exe: &str, origin_context: &str,
) -> Option<String> {
    if let Some(issue) = &snapshot.controller_issue {
        return Some(issue.clone());
    }
    if snapshot.mode != ConfigMode::Ready {
        return Some(
            snapshot
                .issue
                .clone()
                .unwrap_or_else(|| "Automation is paused until configuration is ready".into()),
        );
    }
    if snapshot.context_token != origin_context {
        return Some("The originating configuration history has been retired".into());
    }
    if !matches!(snapshot.settings.switch_method, SwitchMethod::Native) {
        return Some(
            "Native HDR consent is required; keyboard-shortcut automation is disabled".into(),
        );
    }
    if snapshot.settings.resolve_app(path, exe).matched().is_none() {
        return Some("The tracked application is no longer eligible for automatic HDR".into());
    }
    None
}

#[derive(Debug, PartialEq, Eq)]
enum UnmatchedAction {
    Finish,
    KeepUntilExit,
    Debounce,
}

fn unmatched_action(eligible: bool, alive: bool, exit_only: bool, expired: bool) -> UnmatchedAction {
    if !eligible || !alive {
        UnmatchedAction::Finish
    } else if exit_only {
        UnmatchedAction::KeepUntilExit
    } else if expired {
        UnmatchedAction::Finish
    } else {
        UnmatchedAction::Debounce
    }
}

#[derive(Default)]
struct StatusPublication {
    last: Option<HdrStatePayload>,
    current: Option<HdrStatePayload>,
    revision: u64,
}

impl StatusPublication {
    fn observe(&mut self, mut payload: HdrStatePayload) -> HdrStatePayload {
        payload.status_revision = self.revision.to_string();
        if self.current.as_ref() != Some(&payload) {
            self.revision += 1;
            payload.status_revision = self.revision.to_string();
            self.current = Some(payload.clone());
        }
        payload
    }

    fn changed(&self, payload: &HdrStatePayload) -> bool {
        self.last.as_ref() != Some(payload)
    }

    fn published(&mut self, payload: HdrStatePayload) {
        self.last = Some(payload);
    }
}

fn publish_enrollment_result(
    result: Result<ConfigSnapshot, String>,
    latest_snapshot: impl FnOnce() -> Result<ConfigSnapshot, String>,
    emit: impl FnOnce(&ConfigSnapshot) -> Result<(), String>,
) -> Vec<String> {
    let mut diagnostics = Vec::new();
    let snapshot = match result {
        Ok(snapshot) => Ok(snapshot),
        Err(error) => {
            diagnostics.push(format!("Automatic game enrollment was skipped: {error}"));
            latest_snapshot()
        }
    };
    match snapshot {
        Ok(snapshot) => {
            if let Err(error) = emit(&snapshot) {
                diagnostics.push(format!(
                    "Unable to publish the canonical configuration after automatic enrollment: {error}"
                ));
            }
        }
        Err(error) => diagnostics.push(format!(
            "Unable to read the canonical configuration after automatic enrollment failed: {error}"
        )),
    }
    diagnostics
}

struct Authority {
    config: Arc<ConfigManager>,
    admitted: Arc<AtomicBool>,
    kind: OperationKind,
    process: Option<TrackedProcess>,
    context_token: Option<String>,
    hook_available: Arc<AtomicBool>,
    shutdown_budget: Arc<AtomicUsize>,
}

impl WriteAuthority for Authority {
    fn authorize(
        &mut self,
        attempt: &NativeAttempt,
        mark_issued: &mut dyn FnMut(),
    ) -> Result<(), DisplayFailure> {
        let valid_purpose = match self.kind {
            OperationKind::AutomaticEnable => {
                attempt.purpose == NativePurpose::AutomaticEnable && attempt.requested_hdr
            }
            OperationKind::Manual => attempt.purpose == NativePurpose::Manual,
            OperationKind::Cleanup => {
                attempt.purpose == NativePurpose::Cleanup && !attempt.requested_hdr
            }
        };
        if !valid_purpose {
            return Err(DisplayFailure::new(
                FailureKind::AuthorityDenied,
                "The HDR operation does not match its authorization purpose",
            ));
        }
        let live = self.process.as_ref().is_some_and(TrackedProcess::is_alive);
        let foreground = self
            .process
            .as_ref()
            .is_some_and(TrackedProcess::is_foreground);
        self.config
            .with_control_snapshot(|snapshot| {
                let denied =
                    |message| Err(DisplayFailure::new(FailureKind::AuthorityDenied, message));
                if let Some(issue) = &snapshot.controller_issue {
                    return denied(issue.clone());
                }
                if self.kind == OperationKind::Manual {
                    if let Err(reason) = manual_admission(
                        snapshot,
                        self.admitted.load(Ordering::Acquire),
                    ).require() {
                        return denied(reason);
                    }
                }
                if self.kind != OperationKind::Cleanup && !self.admitted.load(Ordering::Acquire) {
                    return denied("The HDR controller is shutting down".into());
                }
                if self.kind == OperationKind::AutomaticEnable {
                    if !self.hook_available.load(Ordering::Acquire) {
                        return denied(
                            "Automatic HDR is paused because the foreground hook is unavailable"
                                .into(),
                        );
                    }
                    let Some(process) = &self.process else {
                        return denied("There is no tracked application process".into());
                    };
                    if !live || (!snapshot.settings.exit_only_hdr && !foreground) {
                        return denied(
                            "The tracked game no longer satisfies the foreground/exit policy"
                                .into(),
                        );
                    }
                    if let Some(reason) = automatic_pause_for_path(
                        snapshot,
                        Some(&process.path),
                        &process.exe,
                        self.context_token.as_deref().unwrap_or(""),
                    ) {
                        return denied(reason);
                    }
                }
                if self.kind == OperationKind::Cleanup
                    && !self.admitted.load(Ordering::Acquire)
                    && self
                        .shutdown_budget
                        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                            remaining.checked_sub(1)
                        })
                        .is_err()
                {
                    return Err(DisplayFailure::new(
                        FailureKind::AttemptBudgetExhausted,
                        "The bounded shutdown cleanup attempt budget has been exhausted",
                    ));
                }
                // No native call, process wait, or disk I/O is made while the publication lock is held.
                mark_issued();
                Ok(())
            })
            .map_err(|error| DisplayFailure::new(FailureKind::AuthorityDenied, error))?
    }
}

struct Actor {
    config: Arc<ConfigManager>,
    app: AppHandle,
    events: EventSink,
    controller: HdrController<WindowsDisplay>,
    tracked: Option<TrackedProcess>,
    watcher: Option<ProcessWatcher>,
    debounce: Option<Debounce>,
    foreground_observation: ForegroundObservation,
    inventory_observation: InventoryObservation,
    activation_preparation: ActivationPreparation,
    manual_observations: ManualObservations,
    last_outcomes: Vec<MonitorOutcome>,
    publication: StatusPublication,
}

impl Actor {
    fn new(config: Arc<ConfigManager>, app: AppHandle, events: EventSink) -> Self {
        Self {
            config,
            app,
            events,
            controller: HdrController::new(WindowsDisplay),
            tracked: None,
            watcher: None,
            debounce: None,
            foreground_observation: ForegroundObservation::default(),
            inventory_observation: InventoryObservation::new(Instant::now()),
            activation_preparation: ActivationPreparation::default(),
            manual_observations: ManualObservations::default(),
            last_outcomes: Vec::new(),
            publication: StatusPublication::default(),
        }
    }

    fn authority(&self, kind: OperationKind) -> Authority {
        Authority {
            config: self.config.clone(),
            admitted: self.events.admitted.clone(),
            kind,
            process: self.tracked.clone(),
            context_token: self
                .controller
                .activation()
                .map(|active| active.context_token.clone()),
            hook_available: self.events.hook_available.clone(),
            shutdown_budget: self.events.shutdown_budget.clone(),
        }
    }

    fn refresh_inventory(&mut self) -> Result<Vec<MonitorInfo>, String> {
        let result = self.controller.refresh_inventory();
        self.inventory_observation.refreshed(Instant::now());
        result
    }

    fn run(mut self, receiver: Receiver<Command>) {
        let mut watchdog_deadline = Instant::now() + FOREGROUND_WATCHDOG_INTERVAL;
        loop {
            let now = Instant::now();
            let expired = take_expired_debounce(&mut self.debounce, now);
            let watchdog_due = watchdog_deadline <= now;
            let inventory_due = self.inventory_observation.due(now);
            if expired.is_some() || watchdog_due || inventory_due {
                if self.events.admitted.load(Ordering::Acquire) {
                    if expired.is_some()
                        || (watchdog_due && self.foreground_observation.needs_observation(
                            self.events.foreground_key(), now,
                        ))
                    {
                        let _ = self.observe(expired, true);
                        self.publish();
                    } else if inventory_due {
                        // Inventory can change without a foreground hint. This read-only probe
                        // never retries automatic writes or replaces the activation's members.
                        let _ = self.refresh_inventory();
                        self.publish();
                    }
                } else if inventory_due {
                    self.inventory_observation.refreshed(Instant::now());
                }
                if watchdog_due {
                    // Schedule from completion rather than hot-looping to catch up after slow work.
                    watchdog_deadline = Instant::now() + FOREGROUND_WATCHDOG_INTERVAL;
                }
                continue;
            }
            let received = receiver.recv_timeout(actor_wait_timeout(
                self.debounce, watchdog_deadline.min(self.inventory_observation.deadline),
                Instant::now(),
            ));
            let command = match received {
                Ok(command) => command,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            match command {
                Command::Shutdown => break,
                Command::ForegroundObserved => {
                    self.events
                        .foreground_pending
                        .store(false, Ordering::Release);
                    if self.events.admitted.load(Ordering::Acquire) {
                        let _ = self.observe(None, true);
                        self.publish();
                    }
                }
                Command::ConfigCommitted => {
                    self.events.config_pending.store(false, Ordering::Release);
                    if self.events.admitted.load(Ordering::Acquire) {
                        let _ = self.observe(None, true);
                        self.publish();
                    }
                }
                Command::Refresh(reply) => {
                    let result = if self.events.admitted.load(Ordering::Acquire) {
                        self.refresh_inventory().map(|monitors| MonitorInventorySnapshot {
                            inventory_revision: self.controller.inventory_revision(),
                            monitors,
                        })
                    } else {
                        Err("The HDR controller is shutting down".into())
                    };
                    self.publish();
                    let _ = reply.send(result);
                }
                Command::ManualSet(scope, enable, request, reply) => {
                    let result = self.manual_set(scope, enable, request);
                    self.publish();
                    let _ = reply.send(result);
                }
                Command::Status(reply) => {
                    if !self.events.admitted.load(Ordering::Acquire) {
                        let _ = reply.send(Err("The HDR controller is shutting down".into()));
                        continue;
                    }
                    self.reconcile_gate();
                    let result = self
                        .config
                        .snapshot()
                        .map(|snapshot| self.payload(&snapshot));
                    self.publish();
                    let _ = reply.send(result);
                }
                Command::GameExited {
                    generation,
                    process,
                } => {
                    if self.controller.matches_activation(generation, process)
                        && self.events.admitted.load(Ordering::Acquire)
                    {
                        let _ = self.observe(None, true);
                        self.publish();
                    }
                }
                Command::WatcherFailed {
                    generation,
                    process,
                    message,
                } => {
                    if self.events.admitted.load(Ordering::Acquire)
                        && self.controller.matches_activation(generation, process)
                    {
                        self.activation_preparation.observed(Err(message));
                        self.finish_activation(usize::MAX);
                        self.publish();
                    }
                }
                Command::HookUnavailable(message) => {
                    self.events.hook_available.store(false, Ordering::Release);
                    self.controller.warn(message);
                    if self.events.admitted.load(Ordering::Acquire) {
                        self.finish_activation(usize::MAX);
                        self.publish();
                    }
                }
            }
        }
        self.events.admitted.store(false, Ordering::Release);
        self.finish_activation(SHUTDOWN_ATTEMPT_BUDGET);
        self.publish();
    }

    fn enroll(&mut self, origin: &ConfigSnapshot, process: &TrackedProcess) {
        if !self.events.admitted.load(Ordering::Acquire)
            || !self.events.hook_available.load(Ordering::Acquire)
            || origin.mode != ConfigMode::Ready
            || origin.controller_issue.is_some()
            || !matches!(origin.settings.switch_method, SwitchMethod::Native)
            || !origin.settings.auto_detect_new_games
            || origin.settings.resolve_app(Some(&process.path), &process.exe) != Resolution::NoMatch
        {
            return;
        }
        // A foreground basename is not verified provider/install evidence.
        let crate::automatic_authority::Authority::Resolved(resolved) =
            crate::automatic_authority::resolve(&crate::database::get_full_catalog(), None)
        else {
            return;
        };
        if !process.is_foreground() {
            return;
        }
        let app = resolved.as_app(origin.settings.auto_detect_new_games);
        let admitted = self.events.admitted.clone();
        let hook_available = self.events.hook_available.clone();
        let result = self.config.mutate(
            &origin.context_token,
            Some(&origin.library_generation),
            true,
            |current| {
                if !admitted.load(Ordering::Acquire)
                    || !hook_available.load(Ordering::Acquire)
                    || !process.is_foreground()
                    || !current.auto_detect_new_games
                    || !matches!(current.switch_method, SwitchMethod::Native)
                    || current.resolve_app(Some(&process.path), &process.exe) != Resolution::NoMatch
                    || crate::library::automatic_enrollment_veto(&current.apps, &app)
                {
                    return Err("Automatic enrollment is no longer eligible".into());
                }
                current.apps.push(app.clone());
                Ok(())
            },
        );
        let diagnostics = publish_enrollment_result(
            result,
            || self.config.snapshot(),
            |snapshot| {
                self.app
                    .emit("config-changed", snapshot.clone())
                    .map_err(|error| error.to_string())
            },
        );
        for diagnostic in diagnostics {
            eprintln!("{diagnostic}");
            self.controller.warn(diagnostic);
        }
    }

    fn ensure_watcher(&mut self) -> Result<(), String> {
        let (Some(active), Some(process)) = (self.controller.activation(), self.tracked.clone())
        else {
            return Ok(());
        };
        if self.watcher.as_ref().is_some_and(|watcher| {
            watcher.generation == active.generation && watcher.process == active.process
        }) {
            self.activation_preparation.observed(Ok(()));
            return Ok(());
        }
        let generation = active.generation;
        self.watcher = None;
        self.watcher = Some(ProcessWatcher::new(
            process,
            generation,
            self.events.clone(),
        )?);
        self.activation_preparation.observed(Ok(()));
        Ok(())
    }

    fn observe(&mut self, expired: Option<Debounce>, enable_automatic: bool) -> Result<(), String> {
        let _ = self.refresh_inventory();
        let origin = match self.config.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.controller.warn(error.clone());
                self.finish_activation(usize::MAX);
                return Err(error);
            }
        };
        let foreground = self.foreground_observation.sample(
            self.events.foreground_key(),
            Instant::now(),
            observe_foreground_pid,
            || (self.events.foreground_key(), Instant::now()),
        );
        if let Some(process) = &foreground {
            self.enroll(&origin, process);
        }
        let snapshot = self.config.snapshot()?;
        let game = foreground.filter(|process| {
            automatic_pause_for_path(&snapshot, Some(&process.path), &process.exe, &snapshot.context_token).is_none()
                && process.is_foreground()
                && self.events.hook_available.load(Ordering::Acquire)
        });
        if let Some(active) = self.controller.activation().cloned() {
            let transferable =
                game.as_ref().is_some() && active.context_token == snapshot.context_token;
            if transferable {
                let process = game.as_ref().expect("transfer requires a game");
                self.controller
                    .transfer(process.identity, process.exe.clone());
                self.tracked = Some(process.clone());
                self.debounce = None;
            } else {
                let action = unmatched_action(
                    automatic_pause_for_path(
                        &snapshot, self.tracked.as_ref().map(|process| process.path.as_str()),
                        &active.exe, &active.context_token,
                    ).is_none(),
                    self.tracked.as_ref().is_some_and(TrackedProcess::is_alive),
                    snapshot.settings.exit_only_hdr,
                    expired.is_some_and(|timer| self.controller.matches_activation(timer.generation, timer.process)),
                );
                match action {
                    UnmatchedAction::Finish => self.finish_activation(usize::MAX),
                    UnmatchedAction::KeepUntilExit => self.debounce = None,
                    UnmatchedAction::Debounce => {
                        let seconds = snapshot.settings.alt_tab_delay_seconds;
                        if !self.debounce.is_some_and(|timer| {
                            timer.generation == active.generation && timer.seconds == seconds
                        }) {
                            let delay = Duration::from_secs(seconds);
                            let deadline = Instant::now()
                                .checked_add(delay)
                                .unwrap_or_else(|| Instant::now() + Duration::from_secs(86400));
                            self.debounce = Some(Debounce {
                                deadline,
                                generation: active.generation,
                                process: active.process,
                                seconds,
                            });
                        }
                    }
                }
            }
        }
        let mut outcomes = Vec::new();
        if self.controller.activation().is_none() {
            if let Some(process) = game {
                match self.controller.begin(
                    process.identity,
                    process.exe.clone(),
                    snapshot.context_token.clone(),
                    snapshot.settings.target_monitor.clone(),
                ) {
                    Ok(skipped) => {
                        outcomes = skipped;
                        self.tracked = Some(process);
                    }
                    Err(error) => self.activation_preparation.observed(Err(error.message)),
                }
            } else {
                self.activation_preparation.observed(Ok(()));
            }
        }
        if self.controller.activation().is_some() {
            if let Err(error) = self.ensure_watcher() {
                self.activation_preparation.observed(Err(error));
                self.finish_activation(usize::MAX);
            } else if enable_automatic {
                let mut authority = self.authority(OperationKind::AutomaticEnable);
                outcomes.extend(self.controller.enable_activation(&mut authority));
                if !outcomes.is_empty() {
                    self.last_outcomes = outcomes.clone();
                }
                let _ = self.refresh_inventory();
                self.reconcile_gate();
                if self.controller.activation().is_some()
                    && self
                        .controller
                        .scope_is_active(&snapshot.settings.target_monitor)
                {
                    self.notify_verified(&outcomes, true);
                }
            }
        }
        Ok(())
    }

    fn reconcile_gate(&mut self) {
        let Some(active) = self.controller.activation() else {
            return;
        };
        let reason = match self.config.snapshot() {
            Ok(snapshot) => automatic_pause_for_path(
                &snapshot, self.tracked.as_ref().map(|process| process.path.as_str()),
                &active.exe, &active.context_token,
            ),
            Err(error) => Some(error),
        };
        let reason = reason.or_else(|| {
            (!self.events.hook_available.load(Ordering::Acquire))
                .then(|| "The foreground hook is unavailable; automatic HDR is paused".into())
        });
        if let Some(reason) = reason {
            self.controller.warn(reason);
            self.finish_activation(usize::MAX);
        }
    }

    fn finish_activation(&mut self, budget: usize) {
        self.debounce = None;
        self.watcher = None;
        let mut authority = self.authority(OperationKind::Cleanup);
        let budget = if self.events.admitted.load(Ordering::Acquire) {
            budget
        } else {
            budget.min(SHUTDOWN_ATTEMPT_BUDGET)
        };
        let outcomes = self.controller.end(&mut authority, budget);
        if !outcomes.is_empty() {
            self.last_outcomes = outcomes.clone();
        }
        self.tracked = None;
        let _ = self.refresh_inventory();
        self.notify_verified(&outcomes, false);
    }

    fn manual_set(
        &mut self,
        scope: TargetMonitor,
        enable: bool,
        request: ManualRequestIdentity,
    ) -> Result<ManualSetResult, String> {
        self.manual_observations.admit(&scope, &request)?;
        self.manual_observations.begin()?;
        let result = self.execute_manual_set(&scope, enable);
        let verified = result.as_ref().is_ok_and(|(outcomes, _)| {
            !outcomes.is_empty() && outcomes.iter().all(MonitorOutcome::is_verified)
        });
        let error = match &result {
            Err(error) => Some(error.clone()),
            Ok((outcomes, _)) if outcomes.is_empty() => Some("No display result was returned.".into()),
            Ok(_) => None,
        };
        self.manual_observations.finish(scope.clone(), request.clone(), verified, error);
        result.map(|(outcomes, snapshot)| ManualSetResult {
            scope, request, outcomes, partial: !verified, status: self.payload(&snapshot),
        })
    }

    fn execute_manual_set(
        &mut self,
        scope: &TargetMonitor,
        enable: bool,
    ) -> Result<(Vec<MonitorOutcome>, ConfigSnapshot), String> {
        let snapshot = self.config.snapshot()?;
        manual_admission(&snapshot, self.events.admitted.load(Ordering::Acquire)).require()?;
        // Establish the logical interval without enabling HDR first. A manual Off while a game is
        // foreground must not produce an automatic On followed by a second hidden setter.
        let _ = self.refresh_inventory();
        if let Ok(process) = observe_foreground() {
            let preparation = self.prepare_manual_activation(&snapshot, process);
            self.activation_preparation.observed(preparation);
        }
        let mut authority = self.authority(OperationKind::Manual);
        let outcomes = self.controller.manual_set(scope, enable, &mut authority);
        self.last_outcomes = outcomes.clone();
        let _ = self.refresh_inventory();
        self.reconcile_gate();
        let snapshot = self.config.snapshot()?;
        let partial = outcomes.is_empty() || outcomes.iter().any(|outcome| !outcome.is_verified());
        if !partial {
            self.notify_verified(&outcomes, enable);
        }
        Ok((outcomes, snapshot))
    }

    fn prepare_manual_activation(
        &mut self,
        snapshot: &ConfigSnapshot,
        process: Option<TrackedProcess>,
    ) -> Result<(), String> {
        if let Some(process) = process.filter(|process| automatic_pause_for_path(
            snapshot, Some(&process.path), &process.exe, &snapshot.context_token,
        ).is_none()) {
            if self.controller.activation().is_none() {
                self.controller.begin(
                    process.identity,
                    process.exe.clone(),
                    snapshot.context_token.clone(),
                    snapshot.settings.target_monitor.clone(),
                ).map_err(|error| error.message)?;
                self.tracked = Some(process);
            } else if self.controller.activation()
                .is_some_and(|active| active.context_token == snapshot.context_token)
            {
                self.controller.transfer(process.identity, process.exe.clone());
                self.tracked = Some(process);
                self.debounce = None;
            }
        }
        // A retained exit-only activation still needs its watcher when focus leaves the game.
        self.ensure_watcher()
    }

    fn notify_verified(&self, outcomes: &[MonitorOutcome], enabled: bool) {
        if outcomes.is_empty()
            || outcomes.iter().any(|outcome| !outcome.is_verified())
            || !outcomes
                .iter()
                .any(|outcome| outcome.outcome == OutcomeKind::Changed)
        {
            return;
        }
        let Ok(snapshot) = self.config.snapshot() else {
            return;
        };
        if !snapshot.settings.notifications_enabled {
            return;
        }
        let body = if enabled {
            "The requested HDR state was verified"
        } else {
            "The requested SDR state was verified"
        };
        let _ = self
            .app
            .notification()
            .builder()
            .title("HDR Auto-Switch")
            .body(body)
            .show();
    }

    fn payload(&mut self, snapshot: &ConfigSnapshot) -> HdrStatePayload {
        let active = self.controller.activation();
        let app = active.and_then(|active| {
            snapshot.settings.resolve_app(
                self.tracked.as_ref().map(|process| process.path.as_str()), &active.exe,
            ).matched()
        });
        let mut warnings = self.foreground_observation.warnings(self.controller.warning());
        warnings.extend(self.activation_preparation.warning.clone());
        if let Some(issue) = &snapshot.issue {
            warnings.push(issue.clone());
        }
        let mut target_status = self
            .controller
            .target_status(&snapshot.settings.target_monitor);
        if snapshot.mode != ConfigMode::Ready {
            target_status = TargetStatus::AutomationPaused;
            warnings.push("Automatic HDR is paused until configuration is ready".into());
        }
        if !matches!(snapshot.settings.switch_method, SwitchMethod::Native) {
            target_status = TargetStatus::AutomationPaused;
            warnings.push(
                "Native HDR consent is required; keyboard-shortcut automation is disabled".into(),
            );
        }
        if let Some(issue) = &snapshot.controller_issue {
            target_status = TargetStatus::ControllerConflict;
            warnings.push(issue.clone());
        }
        if let Some(error) = self.controller.inventory_error() {
            warnings.push(format!("Display inventory is stale: {error}"));
        }
        if !self.events.hook_available.load(Ordering::Acquire)
            && snapshot.controller_issue.is_none()
        {
            target_status = TargetStatus::AutomationPaused;
            warnings.push("The foreground hook is unavailable; automatic HDR is paused".into());
        }
        let target_deferred = snapshot.mode == ConfigMode::Ready && active
            .is_some_and(|active| !same_target(&active.target, &snapshot.settings.target_monitor));
        if target_deferred {
            warnings.push("The saved monitor target will apply to the next activation".into());
        }
        let observed_scope = active.map(|active| &active.target).or_else(|| {
            (snapshot.mode == ConfigMode::Ready).then_some(&snapshot.settings.target_monitor)
        });
        let scope_hdr_state = observed_scope
            .map(|scope| self.controller.scope_hdr_state(scope))
            .unwrap_or(ScopeHdrState::Unknown);
        self.manual_observations.append_errors(&mut warnings);
        let payload = HdrStatePayload {
            status_revision: "0".into(),
            inventory_revision: self.controller.inventory_revision(),
            manual_revision: self.manual_observations.revision.to_string(),
            manual_results: self.manual_observations.scopes.clone(),
            is_hdr_active: scope_hdr_state == ScopeHdrState::Hdr,
            scope_hdr_state,
            manual_control: manual_admission(snapshot, self.events.admitted.load(Ordering::Acquire)),
            current_app_name: active.map(|active| {
                app.map(|app| app.name.clone())
                    .unwrap_or_else(|| active.exe.clone())
            }),
            current_exe: active.map(|active| active.exe.clone()),
            switched_by_app: self.controller.has_ownership(),
            steam_id: app.and_then(|app| app.steam_id.clone()),
            launcher: app.and_then(|app| app.launcher.clone()),
            hdr_type: app.map(|app| app.hdr_type.as_str().to_string()),
            warning: if warnings.is_empty() {
                None
            } else {
                Some(warnings.join("\n"))
            },
            quarantined_apps: quarantined_apps(&snapshot.settings),
            target_status,
            active_target: active.map(|active| active.target.clone()),
            target_deferred,
            any_hdr_active: self.controller.inventory_error().is_none()
                && self
                    .controller
                    .inventory()
                    .iter()
                    .any(|monitor| monitor.hdr_state_known && monitor.is_hdr_enabled),
            inventory_stale: self.controller.inventory_error().is_some(),
            uncertain_targets: self.controller.uncertain_targets(),
            operation_outcomes: self.last_outcomes.clone(),
        };
        self.publication.observe(payload)
    }

    fn publish(&mut self) {
        let mut payload = match self.config.snapshot() {
            Ok(snapshot) => self.payload(&snapshot),
            Err(error) => {
                let mut warnings = self.foreground_observation.warnings(self.controller.warning());
                warnings.extend(self.activation_preparation.warning.clone());
                warnings.push(error);
                self.manual_observations.append_errors(&mut warnings);
                let mut payload = HdrStatePayload::unavailable(warnings.join("\n"));
                payload.inventory_revision = self.controller.inventory_revision();
                payload.manual_revision = self.manual_observations.revision.to_string();
                payload.manual_results = self.manual_observations.scopes.clone();
                self.publication.observe(payload)
            }
        };
        if !self.publication.changed(&payload) {
            return;
        }
        if let Err(error) = self.app.emit("hdr-status-changed", payload.clone()) {
            let warning = format!("Unable to publish HDR status: {error}");
            eprintln!("{warning}");
            self.controller.warn(warning.clone());
            payload.warning = Some(match payload.warning.take() {
                Some(existing) => format!("{existing}\n{warning}"),
                None => warning,
            });
        } else {
            self.publication.published(payload.clone());
        }
        crate::tray::update_status(&self.app, &payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_request() -> ManualRequestIdentity {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        ManualRequestIdentity {
            client_id: "gui:test".into(),
            sequence: NEXT.fetch_add(1, Ordering::Relaxed).to_string(),
        }
    }
    use crate::config::HdrType;
    use crate::display::{NativeApi, RuntimeAddress};

    fn ready_snapshot() -> ConfigSnapshot {
        let mut settings = AppConfig::default();
        settings.apps = vec![HdrApp {
            name: "Game".into(),
            exe_name: "game.exe".into(),
            enabled: true,
            hdr_type: HdrType::Native,
            path: None,
            alternate_exes: Vec::new(),
            steam_id: None,
            launcher: None,
        }];
        ConfigSnapshot {
            settings,
            mode: ConfigMode::Ready,
            store_id: Some("store".into()),
            revision: "1".into(),
            context_token: "context".into(),
            library_generation: "0".into(),
            control_epoch: "0".into(),
            issue: None,
            controller_issue: None,
            candidates: Vec::new(),
            config_path: "isolated-test".into(),
        }
    }

    struct GateFixture {
        manager: Arc<ConfigManager>,
        _directory: tempfile::TempDir,
    }

    impl GateFixture {
        fn new() -> Self {
            let directory = tempfile::Builder::new()
                .prefix(".hdr-gate-test-")
                .tempdir_in(env!("CARGO_MANIFEST_DIR"))
                .unwrap();
            let manager = Arc::new(
                ConfigManager::load(
                    directory.path().join("local"),
                    directory.path().join("absent-legacy.json"),
                )
                .unwrap(),
            );
            Self {
                manager,
                _directory: directory,
            }
        }

        fn authority(&self, kind: OperationKind) -> Authority {
            Authority {
                config: self.manager.clone(),
                admitted: Arc::new(AtomicBool::new(true)),
                kind,
                process: None,
                context_token: None,
                hook_available: Arc::new(AtomicBool::new(true)),
                shutdown_budget: Arc::new(AtomicUsize::new(SHUTDOWN_ATTEMPT_BUDGET)),
            }
        }

        fn ready(&self) -> ConfigSnapshot {
            let first = self.manager.snapshot().unwrap();
            let ready = self.manager.initialize(&first.context_token).unwrap();
            self.manager
                .mutate(&ready.context_token, None, true, |settings| {
                    settings.apps = ready_snapshot().settings.apps;
                    Ok(())
                })
                .unwrap()
        }
    }

    fn idle_service(config: Arc<ConfigManager>) -> (MonitorService, Receiver<Command>) {
        let (sender, receiver) = channel();
        let service = MonitorService {
            config,
            events: EventSink {
                sender,
                admitted: Arc::new(AtomicBool::new(true)),
                foreground_pending: Arc::new(AtomicBool::new(false)),
                foreground_generation: Arc::new(AtomicU64::new(0)),
                config_pending: Arc::new(AtomicBool::new(false)),
                hook_available: Arc::new(AtomicBool::new(true)),
                shutdown_budget: Arc::new(AtomicUsize::new(SHUTDOWN_ATTEMPT_BUDGET)),
            },
            threads: Mutex::new(Threads::default()),
        };
        (service, receiver)
    }

    fn mock_attempt() -> NativeAttempt {
        NativeAttempt {
            device_path: "mock-monitor".into(),
            address: RuntimeAddress {
                adapter_id_low: 1,
                adapter_id_high: 0,
                target_id: 2,
            },
            api: NativeApi::Hdr,
            requested_hdr: true,
            previous_hdr: false,
            previous_hdr_user_enabled: false,
            purpose: NativePurpose::Manual,
        }
    }

    fn manual_fixture(mode: ConfigMode) -> GateFixture {
        let directory = tempfile::Builder::new()
            .prefix(".hdr-manual-test-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let local = directory.path().join("local");
        let legacy = directory.path().join("legacy.json");
        std::fs::create_dir(&local).unwrap();
        match mode {
            ConfigMode::RecoveryRequired => {
                std::fs::write(local.join("config-v2.json"), b"unreadable settings").unwrap();
            }
            ConfigMode::UnsupportedSchema => {
                std::fs::write(local.join("config-v2.json"), br#"{"schema_version":3}"#).unwrap();
            }
            ConfigMode::Unavailable => std::fs::create_dir(&legacy).unwrap(),
            ConfigMode::Ready | ConfigMode::ImportAvailable => {
                let mut settings = serde_json::to_value(AppConfig::default()).unwrap();
                settings["switch_method"] = "shortcut".into();
                settings["target_monitor"] = "all".into();
                std::fs::write(&legacy, serde_json::to_vec(&settings).unwrap()).unwrap();
            }
            ConfigMode::FirstRun => {}
        }
        let manager = Arc::new(ConfigManager::load(local, legacy).unwrap());
        if mode == ConfigMode::Ready {
            let snapshot = manager.snapshot().unwrap();
            manager.import_legacy(&snapshot.context_token).unwrap();
        }
        assert_eq!(manager.snapshot().unwrap().mode, mode);
        GateFixture { manager, _directory: directory }
    }

    #[test]
    fn scoped_manual_hdr_in_paused_modes_does_not_grant_automatic_consent() {
        use crate::display::tests::{monitor, MockDisplay};

        for mode in [
            ConfigMode::FirstRun, ConfigMode::ImportAvailable, ConfigMode::RecoveryRequired,
            ConfigMode::UnsupportedSchema, ConfigMode::Ready,
        ] {
            let fixture = manual_fixture(mode);
            let before = fixture.manager.snapshot().unwrap();
            assert_eq!(manual_admission(&before, true), ManualControl::Available);
            assert!(automatic_pause(&before, "game.exe", &before.context_token).is_some());
            let mut authority = fixture.authority(OperationKind::Manual);
            let mut controller = HdrController::new(MockDisplay::new(vec![
                monitor("chosen", 1, false), monitor("other", 2, true),
            ]));
            let scope = TargetMonitor::Monitor {
                device_path: "chosen".into(), display_name: "Chosen display".into(),
            };
            for enabled in [true, false] {
                let outcomes = controller.manual_set(&scope, enabled, &mut authority);
                assert_eq!(outcomes.len(), 1);
                assert!(outcomes[0].is_verified());
                let inventory = controller.refresh_inventory().unwrap();
                assert_eq!(inventory[0].is_hdr_enabled, enabled);
                assert!(inventory[1].is_hdr_enabled);
                assert_eq!(fixture.manager.snapshot().unwrap(), before);
            }
            assert!(controller.activation().is_none());
            assert!(!controller.has_ownership());
        }
    }

    #[test]
    fn manual_entry_and_issuance_share_conflict_shutdown_and_unavailable_gates() {
        for mode in [ConfigMode::FirstRun, ConfigMode::Unavailable] {
            let fixture = manual_fixture(mode);
            let (service, receiver) = idle_service(fixture.manager.clone());
            if mode == ConfigMode::Unavailable {
                assert!(service.manual_set(TargetMonitor::All, true, test_request()).is_err());
                let mut authority = fixture.authority(OperationKind::Manual);
                assert!(authority.authorize(&mock_attempt(), &mut || panic!("issued")).is_err());
            } else {
                fixture.manager.set_controller_issue(Some("controller conflict".into())).unwrap();
                assert!(service.manual_set(TargetMonitor::All, true, test_request()).is_err());
                fixture.manager.set_controller_issue(None).unwrap();
                service.events.admitted.store(false, Ordering::Release);
                assert!(service.manual_set(TargetMonitor::All, false, test_request()).is_err());
            }
            assert!(receiver.try_recv().is_err());
        }
        let unknown = HdrStatePayload::unavailable("Unreadable authority".into());
        assert!(matches!(unknown.manual_control, ManualControl::Blocked { .. }));
    }

    #[test]
    fn safe_test_mode_blocks_manual_control_without_granting_consent() {
        for mode in [
            ConfigMode::FirstRun,
            ConfigMode::ImportAvailable,
            ConfigMode::RecoveryRequired,
            ConfigMode::UnsupportedSchema,
            ConfigMode::Ready,
            ConfigMode::Unavailable,
        ] {
            let fixture = manual_fixture(mode);
            let before = fixture.manager.snapshot().unwrap();
            let blocked = crate::reconcile_controller(&fixture.manager, true).unwrap();
            assert_eq!(blocked.controller_issue.as_deref(), Some(crate::SAFE_TEST_ISSUE));
            assert_eq!(
                manual_admission(&blocked, true),
                ManualControl::Blocked { reason: crate::SAFE_TEST_ISSUE.into() }
            );
            assert_eq!(
                automatic_pause(&blocked, "game.exe", &blocked.context_token).as_deref(),
                Some(crate::SAFE_TEST_ISSUE)
            );
            let (service, receiver) = idle_service(fixture.manager.clone());
            for scope in [
                TargetMonitor::All,
                TargetMonitor::Monitor {
                    device_path: "chosen".into(),
                    display_name: "Chosen display".into(),
                },
            ] {
                for enabled in [true, false] {
                    assert_eq!(
                        service.manual_set(scope.clone(), enabled, test_request()).err().as_deref(),
                        Some(crate::SAFE_TEST_ISSUE)
                    );
                }
            }
            assert!(receiver.try_recv().is_err());
            let mut authority = fixture.authority(OperationKind::Manual);
            for enabled in [true, false] {
                let mut attempt = mock_attempt();
                attempt.requested_hdr = enabled;
                let error = authority
                    .authorize(&attempt, &mut || panic!("Safe test mode issued native HDR"))
                    .unwrap_err();
                assert_eq!(error.kind, FailureKind::AuthorityDenied);
                assert_eq!(error.message, crate::SAFE_TEST_ISSUE);
            }
            assert_eq!(fixture.manager.snapshot().unwrap(), blocked);
            assert_eq!(blocked.settings, before.settings);
            assert_eq!(blocked.mode, before.mode);
            assert_eq!(blocked.revision, before.revision);
            assert_eq!(blocked.context_token, before.context_token);
        }
    }

    #[test]
    fn manual_requests_enqueue_in_click_order_without_waiting_for_the_actor() {
        let fixture = GateFixture::new();
        let (service, receiver) = idle_service(fixture.manager.clone());
        let gui = test_request();
        let tray = ManualRequestIdentity { client_id: "tray".into(), sequence: "1".into() };
        let on = service.manual_set(TargetMonitor::All, true, gui.clone()).unwrap();
        let off = service.manual_set(TargetMonitor::All, false, tray.clone()).unwrap();
        for (expected, identity, result) in [(true, gui, "first result"), (false, tray, "second result")] {
            let Command::ManualSet(scope, enabled, request, reply) = receiver.try_recv().unwrap() else {
                panic!("Expected manual request");
            };
            assert_eq!(scope, TargetMonitor::All);
            assert_eq!(enabled, expected);
            assert_eq!(request, identity, "admission must preserve the caller's correlation identity");
            reply.send(Err(result.into())).unwrap();
        }
        // Awaiting in the opposite order cannot change the already-enqueued native order.
        assert_eq!(tauri::async_runtime::block_on(off.resolve()).unwrap_err(), "second result");
        assert_eq!(tauri::async_runtime::block_on(on.resolve()).unwrap_err(), "first result");
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn manual_waiter_reports_a_dropped_actor_response_without_hanging() {
        let fixture = GateFixture::new();
        let (service, receiver) = idle_service(fixture.manager.clone());
        let pending = service.manual_set(TargetMonitor::All, false, test_request()).unwrap();
        drop(receiver);
        assert!(tauri::async_runtime::block_on(pending.resolve())
            .unwrap_err().contains("stopped before responding"));
        assert!(service.manual_set(TargetMonitor::All, true, test_request()).is_err());
    }

    #[test]
    fn queued_manual_requests_cannot_bypass_a_later_conflict_or_shutdown() {
        use crate::display::tests::{monitor, MockDisplay};

        for conflict in [true, false] {
            let fixture = GateFixture::new();
            let (service, receiver) = idle_service(fixture.manager.clone());
            let pending = service.manual_set(TargetMonitor::All, true, test_request()).unwrap();
            if conflict {
                fixture.manager.set_controller_issue(Some("new conflict".into())).unwrap();
            } else {
                service.events.admitted.store(false, Ordering::Release);
            }
            let Command::ManualSet(scope, enabled, _, reply) = receiver.try_recv().unwrap() else {
                panic!("Expected queued manual request");
            };
            let mut authority = fixture.authority(OperationKind::Manual);
            authority.admitted = service.events.admitted.clone();
            let mut controller = HdrController::new(MockDisplay::new(vec![
                monitor("chosen", 1, false),
            ]));
            let outcomes = controller.manual_set(&scope, enabled, &mut authority);
            assert_eq!(outcomes[0].failure, Some(FailureKind::AuthorityDenied));
            assert!(!controller.refresh_inventory().unwrap()[0].is_hdr_enabled);
            reply.send(Err(outcomes[0].message.clone().unwrap())).unwrap();
            assert!(tauri::async_runtime::block_on(pending.resolve()).is_err());
        }
    }

    #[test]
    fn verified_metadata_noop_does_not_write_publish_or_wake() {
        let fixture = GateFixture::new();
        let before = fixture.ready();
        let bytes = std::fs::read(&before.config_path).unwrap();
        let (service, receiver) = idle_service(fixture.manager.clone());
        let unchanged = fixture.manager.mutate_if_changed(
            &before.context_token, Some(&before.library_generation), true, |settings| {
                assert!(!crate::library::enrich_verified_metadata(settings, &[]));
                Ok(())
            },
        );
        assert!(matches!(&unchanged, Ok(None)));
        crate::background::publish_enrichment_result(
            &fixture.manager, &service, unchanged, |_| panic!("no-op emitted"),
        );
        assert!(receiver.try_recv().is_err());
        assert_eq!(fixture.manager.snapshot().unwrap(), before);
        assert_eq!(std::fs::read(&before.config_path).unwrap(), bytes);
    }

    #[test]
    fn correction_cache_untrusted_catalog_cannot_write_publish_or_wake_enrichment() {
        use crate::automatic_authority::{self, Authority as GameAuthority, InstallEvidence, Provider};
        let root = tempfile::tempdir().unwrap();
        for exe in ["untrusted.exe", "extra.exe"] {
            std::fs::write(root.path().join(exe), b"fixture, never executed").unwrap();
        }
        let catalog: Vec<crate::database::CatalogEntry> = serde_json::from_value(serde_json::json!([{
            "name": "Downloaded title", "exe_name": "untrusted.exe", "steam_id": "123",
            "hdr_type": "native", "support_tier": "native", "alternate_exes": ["extra.exe"],
        }])).unwrap();
        for provider in [Provider::Epic, Provider::Gog, Provider::Windows] {
            let fixture = GateFixture::new();
            let ready = fixture.ready();
            let before = fixture.manager.mutate(&ready.context_token, None, true, |settings| {
                settings.apps[0].exe_name = "untrusted.exe".into();
                settings.apps[0].enabled = false;
                Ok(())
            }).unwrap();
            let bytes = std::fs::read(&before.config_path).unwrap();
            let evidence = InstallEvidence::observe(
                provider, None, root.path(), &["untrusted.exe".into(), "extra.exe".into()],
            ).unwrap();
            let verified = match automatic_authority::resolve(&catalog, Some(&evidence)) {
                GameAuthority::Resolved(resolved) => vec![resolved],
                _ => Vec::new(),
            };
            let (service, receiver) = idle_service(fixture.manager.clone());
            let unchanged = fixture.manager.mutate_if_changed(
                &before.context_token, Some(&before.library_generation), true, |settings| {
                    crate::library::enrich_verified_aliases(settings, &verified);
                    crate::library::enrich_verified_metadata(settings, &verified);
                    Ok(())
                },
            );
            assert!(matches!(&unchanged, Ok(None)), "{provider:?}: untrusted catalog changed settings");
            assert!(verified.is_empty(), "{provider:?}: an automatic row acquired authority");
            crate::background::publish_enrichment_result(
                &fixture.manager, &service, unchanged, |_| panic!("untrusted catalog emitted"),
            );
            assert!(receiver.try_recv().is_err());
            assert_eq!(fixture.manager.snapshot().unwrap(), before);
            assert_eq!(std::fs::read(&before.config_path).unwrap(), bytes);
        }
    }

    #[test]
    fn verified_aoe3_alias_enrichment_publishes_once_then_is_byte_and_actor_noop() {
        use crate::automatic_authority::{self, Authority as GameAuthority, InstallEvidence, Provider};
        let fixture = GateFixture::new();
        let ready = fixture.ready();
        let install = fixture._directory.path().join(r"XboxGames\AOE3");
        std::fs::create_dir_all(install.join("Content")).unwrap();
        std::fs::write(install.join(r"Content\AoE3DE.exe"), b"fixture, never executed").unwrap();
        std::fs::write(install.join(r"Content\GameLaunchHelper.exe"), b"fixture, never executed").unwrap();
        std::fs::write(install.join(r"Content\MicrosoftGame.config"), br#"<Game><ExecutableList>
            <Executable Name="AoE3DE.exe"/><Executable Name="GameLaunchHelper.exe"/>
            </ExecutableList></Game>"#).unwrap();
        let declarations = crate::xbox_config::declarations(&install).unwrap();
        let evidence = InstallEvidence::observe(Provider::Xbox, None, &install, &declarations).unwrap();
        let catalog: Vec<crate::database::CatalogEntry> =
            serde_json::from_str(include_str!("../catalog.json")).unwrap();
        let catalog = crate::database::authored_test_catalog(catalog);
        let GameAuthority::Resolved(resolved) = automatic_authority::resolve(&catalog, Some(&evidence))
        else { panic!("AOE3 Xbox fixture must resolve"); };
        assert_eq!(resolved.as_app(true).exe_name, "aoe3de.exe");
        assert!(resolved.as_app(true).alternate_exes.is_empty());
        let before = fixture.manager.mutate(&ready.context_token, None, true, |settings| {
            let row = &mut settings.apps[0];
            row.exe_name = resolved.catalog.exe_name.clone();
            row.name = "My custom AOE3 title".into();
            row.enabled = false;
            row.hdr_type = HdrType::Custom;
            row.launcher = Some("Xbox".into());
            Ok(())
        }).unwrap();
        let (service, receiver) = idle_service(fixture.manager.clone());
        let mut publications = 0;
        for attempt in 0..5 {
            let origin = fixture.manager.snapshot().unwrap();
            let bytes = std::fs::read(&origin.config_path).unwrap();
            let changed = fixture.manager.mutate_if_changed(
                &origin.context_token, Some(&origin.library_generation), true,
                |settings| {
                    assert_eq!(crate::library::enrich_verified_aliases(
                        settings, std::slice::from_ref(&resolved),
                    ), attempt == 0);
                    Ok(())
                },
            );
            assert_eq!(matches!(&changed, Ok(Some(_))), attempt == 0);
            crate::background::publish_enrichment_result(
                &fixture.manager, &service, changed, |_| publications += 1,
            );
            if attempt == 0 {
                assert!(matches!(receiver.try_recv(), Ok(Command::ConfigCommitted)));
                service.events.config_pending.store(false, Ordering::Release);
            } else {
                assert!(receiver.try_recv().is_err());
                assert_eq!(fixture.manager.snapshot().unwrap(), origin);
                assert_eq!(std::fs::read(&origin.config_path).unwrap(), bytes);
            }
        }
        assert_eq!(publications, 1);
        let after = fixture.manager.snapshot().unwrap();
        let mut expected = before.settings.apps[0].clone();
        expected.alternate_exes = vec!["aoe3de.exe".into()];
        assert_eq!(after.settings.apps, [expected]);
        assert_eq!(after.settings.resolve_app(None, "AoE3DE.exe"), Resolution::Disabled);
        assert_eq!(after.settings.resolve_app(None, "GameLaunchHelper.exe"), Resolution::Excluded);
        assert_eq!(after.revision.parse::<u64>().unwrap(), before.revision.parse::<u64>().unwrap() + 1);
    }

    #[test]
    fn known_id_creation_veto_is_not_merge_authority_and_is_persistently_quiet() {
        use crate::automatic_authority::{self, Authority as GameAuthority, InstallEvidence, Provider};
        let root = tempfile::tempdir().unwrap();
        for exe in ["game.exe", "renderer.exe"] {
            std::fs::write(root.path().join(exe), b"fixture, never executed").unwrap();
        }
        let catalog = vec![serde_json::from_value::<crate::database::CatalogEntry>(serde_json::json!({
            "name": "Catalog game", "exe_name": "game.exe", "steam_id": "123",
            "hdr_type": "native", "support_tier": "native", "alternate_exes": ["renderer.exe"],
        })).unwrap()];
        let catalog = crate::database::authored_test_catalog(catalog);
        let evidence = InstallEvidence::observe(Provider::Steam, Some("123"), root.path(), &[]).unwrap();
        let GameAuthority::Resolved(resolved) = automatic_authority::resolve(&catalog, Some(&evidence))
        else { panic!("fixture must resolve"); };
        for enabled in [false, true] {
            for equal_id in [false, true] {
                let fixture = GateFixture::new();
                let ready = fixture.ready();
                let before = fixture.manager.mutate(&ready.context_token, None, true, |settings| {
                    let row = &mut settings.apps[0];
                    row.name = "My independent binding".into();
                    row.exe_name = if equal_id { "my-choice.exe" } else { "game.exe" }.into();
                    row.steam_id = Some(if equal_id { "123" } else { "999" }.into());
                    row.launcher = Some("Steam".into());
                    row.enabled = enabled;
                    Ok(())
                }).unwrap();
                let bytes = std::fs::read(&before.config_path).unwrap();
                let (service, receiver) = idle_service(fixture.manager.clone());
                assert!(crate::library::automatic_enrollment_veto(&before.settings.apps, &resolved.as_app(true)));
                let unchanged = fixture.manager.mutate_if_changed(
                    &before.context_token, Some(&before.library_generation), true, |settings| {
                        assert!(!crate::library::enrich_verified_aliases(settings, std::slice::from_ref(&resolved)));
                        assert!(!crate::library::enrich_verified_metadata(settings, std::slice::from_ref(&resolved)));
                        Ok(())
                    },
                );
                assert!(matches!(&unchanged, Ok(None)));
                crate::background::publish_enrichment_result(
                    &fixture.manager, &service, unchanged, |_| panic!("ID-only/ID-conflict emitted"),
                );
                assert!(receiver.try_recv().is_err());
                assert_eq!(fixture.manager.snapshot().unwrap(), before);
                assert_eq!(std::fs::read(&before.config_path).unwrap(), bytes);
            }
        }
    }

    #[test]
    fn legacy_aoe4_quarantine_is_derived_quiet_and_repair_retires_status() {
        let fixture = GateFixture::new();
        let ready = fixture.ready();
        let before = fixture.manager.mutate(&ready.context_token, None, true, |settings| {
            let row = &mut settings.apps[0];
            row.name = "My Age of Empires IV".into();
            row.exe_name = "BsSndRpt64.exe".into();
            row.path = Some(r"D:\AOE4\BsSndRpt64.exe".into());
            row.alternate_exes = vec!["historical.exe".into(), "BugSplatHD64.exe".into()];
            row.enabled = false;
            row.hdr_type = HdrType::Custom;
            row.steam_id = Some("1466860".into());
            row.launcher = Some("My launcher".into());
            Ok(())
        }).unwrap();
        let bytes = std::fs::read(&before.config_path).unwrap();
        let (service, receiver) = idle_service(fixture.manager.clone());
        let mut publication = StatusPublication::default();
        let mut publications = 0;
        for _ in 0..100 {
            let latest = fixture.manager.snapshot().unwrap();
            assert_eq!(latest.settings.resolve_app(None, "historical.exe"), Resolution::Quarantined);
            assert_eq!(latest.settings.resolve_app(None, "BsSndRpt64.exe"), Resolution::Excluded);
            let mut payload = HdrStatePayload::unavailable("test display unavailable".into());
            payload.quarantined_apps = quarantined_apps(&latest.settings);
            assert_eq!(payload.quarantined_apps.len(), 1);
            assert_eq!(payload.quarantined_apps[0].name, "My Age of Empires IV");
            assert_eq!(payload.quarantined_apps[0].exe_name, "BsSndRpt64.exe");
            if publication.changed(&payload) {
                publications += 1;
                publication.published(payload);
            }
        }
        assert_eq!(publications, 1);
        assert_eq!(fixture.manager.snapshot().unwrap(), before);
        assert_eq!(std::fs::read(&before.config_path).unwrap(), bytes);
        assert!(receiver.try_recv().is_err(), "derivation must not wake the actor");

        let invalid = fixture.manager.mutate(
            &before.context_token, Some(&before.library_generation), true,
            |settings| {
                let row = crate::library::AppRowIdentity::at(settings, 0)?;
                crate::library::repair_executable(settings, &row, "GameLaunchHelper.exe", r"D:\AOE4\GameLaunchHelper.exe")
            },
        );
        assert!(invalid.is_err());
        assert_eq!(fixture.manager.snapshot().unwrap(), before);
        assert_eq!(std::fs::read(&before.config_path).unwrap(), bytes);

        // A fixture-selected executable, not a claim about AOE4's real binary.
        let repaired = fixture.manager.mutate(
            &before.context_token, Some(&before.library_generation), true,
            |settings| {
                let row = crate::library::AppRowIdentity::at(settings, 0)?;
                crate::library::repair_executable(settings, &row, "user-selected.exe", r"D:\AOE4\user-selected.exe")
            },
        ).unwrap();
        let mut expected = before.settings.apps[0].clone();
        expected.exe_name = "user-selected.exe".into();
        expected.path = Some(r"D:\AOE4\user-selected.exe".into());
        expected.alternate_exes.clear();
        assert_eq!(repaired.settings.apps, [expected]);
        assert!(quarantined_apps(&repaired.settings).is_empty());
        assert_eq!(repaired.settings.resolve_app(None, "historical.exe"), Resolution::NoMatch);
        assert_eq!(repaired.settings.resolve_app(
            Some(r"D:\AOE4\user-selected.exe"), "user-selected.exe",
        ), Resolution::Disabled);
        let mut payload = publication.last.clone().unwrap();
        payload.quarantined_apps = quarantined_apps(&repaired.settings);
        assert!(publication.changed(&payload));
        publication.published(payload.clone());
        assert!(!publication.changed(&payload));
        assert_eq!(repaired.revision.parse::<u64>().unwrap(), before.revision.parse::<u64>().unwrap() + 1);
        assert!(fixture.manager.mutate(
            &before.context_token, Some(&before.library_generation), true,
            |_| panic!("stale repair must not reach a mutation"),
        ).is_err());
        drop(service);
    }

    #[test]
    fn unchanged_enrichment_is_quiet_but_real_changes_and_conflicts_wake_the_actor() {
        let fixture = GateFixture::new();
        let before = fixture.ready();
        let (service, receiver) = idle_service(fixture.manager.clone());
        let unchanged = fixture.manager.mutate_if_changed(
            &before.context_token, Some(&before.library_generation), true, |settings| {
                assert!(!crate::library::enrich_existing(settings, &before.settings.apps));
                Ok(())
            },
        );
        assert!(matches!(&unchanged, Ok(None)));
        crate::background::publish_enrichment_result(
            &fixture.manager, &service, unchanged, |_| panic!("no-op emitted"),
        );
        assert!(receiver.try_recv().is_err());
        assert_eq!(fixture.manager.snapshot().unwrap(), before);
        let changed = fixture.manager.mutate_if_changed(
            &before.context_token, Some(&before.library_generation), true, |settings| {
                let mut detected = settings.apps[0].clone();
                detected.alternate_exes.push("new-alias.exe".into());
                assert!(crate::library::enrich_existing(settings, &[detected]));
                Ok(())
            },
        );
        assert!(matches!(&changed, Ok(Some(_))));
        let mut emitted = None;
        crate::background::publish_enrichment_result(
            &fixture.manager, &service, changed, |snapshot| emitted = Some(snapshot.clone()),
        );
        assert!(matches!(receiver.try_recv(), Ok(Command::ConfigCommitted)));
        assert_eq!(emitted, Some(fixture.manager.snapshot().unwrap()));
        service.events.config_pending.store(false, Ordering::Release);
        std::fs::write(&before.config_path, b"external conflict").unwrap();
        let failed = fixture.manager.mutate_if_changed(
            &before.context_token, None, true, |_| Ok(()),
        );
        assert!(failed.is_err());
        crate::background::publish_enrichment_result(
            &fixture.manager, &service, failed, |snapshot| {
                assert_eq!(snapshot.mode, ConfigMode::RecoveryRequired);
            },
        );
        assert!(matches!(receiver.try_recv(), Ok(Command::ConfigCommitted)));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn failed_enrollment_publishes_the_latest_canonical_recovery_snapshot() {
        let origin = ready_snapshot();
        let mut current = origin.clone();
        current.mode = ConfigMode::RecoveryRequired;
        current.context_token = "replacement-load-authority".into();
        current.control_epoch = "2".into();
        current.issue = Some("Replacement outcome is uncertain".into());
        let mut emitted = None;
        let diagnostics = publish_enrollment_result(
            Err("Commit outcome is uncertain".into()),
            || Ok(current.clone()),
            |snapshot| {
                emitted = Some(snapshot.clone());
                Ok(())
            },
        );
        assert_eq!(emitted, Some(current));
        assert_ne!(
            emitted.as_ref().unwrap().context_token,
            origin.context_token
        );
        assert!(diagnostics
            .iter()
            .any(|message| message.contains("Commit outcome is uncertain")));
    }

    #[test]
    fn failed_enrollment_surfaces_snapshot_errors_without_emitting_stale_configuration() {
        let mut emitted = false;
        let diagnostics = publish_enrollment_result(
            Err("Write failed".into()),
            || Err("Configuration gate is poisoned".into()),
            |_| {
                emitted = true;
                Ok(())
            },
        );
        assert!(!emitted);
        assert!(diagnostics
            .iter()
            .any(|message| message.contains("Write failed")));
        assert!(diagnostics
            .iter()
            .any(|message| message.contains("Configuration gate is poisoned")));
    }

    #[test]
    fn enrollment_surfaces_event_failures_on_both_commit_and_error_paths() {
        for failed_commit in [false, true] {
            let snapshot = ready_snapshot();
            let result = if failed_commit {
                Err("Write failed".into())
            } else {
                Ok(snapshot.clone())
            };
            let mut reads = 0;
            let diagnostics = publish_enrollment_result(
                result,
                || {
                    reads += 1;
                    Ok(snapshot.clone())
                },
                |_| Err("Event delivery failed".into()),
            );
            assert_eq!(reads, usize::from(failed_commit));
            assert!(diagnostics
                .iter()
                .any(|message| message.contains("Event delivery failed")));
        }
    }

    #[test]
    fn scope_hdr_state_is_mandatory_and_serializes_all_four_distinct_states() {
        for (state, expected) in [
            (ScopeHdrState::Hdr, "hdr"),
            (ScopeHdrState::Sdr, "sdr"),
            (ScopeHdrState::Mixed, "mixed"),
            (ScopeHdrState::Unknown, "unknown"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), expected);
        }
        let payload = HdrStatePayload::unavailable("Read failed".into());
        let serialized = serde_json::to_value(payload).unwrap();
        assert_eq!(serialized["scope_hdr_state"], "unknown");
        assert_eq!(serialized["is_hdr_active"], false);
    }

    #[test]
    fn correlation_proofs_survive_later_cross_origin_results_without_retaining_old_condition_errors() {
        let mut observations = ManualObservations::default();
        let gui = ManualRequestIdentity { client_id: "gui:window".into(), sequence: "2".into() };
        let tray = ManualRequestIdentity { client_id: "tray".into(), sequence: "1".into() };
        observations.admit(&TargetMonitor::All, &gui).unwrap();
        observations.begin().unwrap();
        observations.finish(TargetMonitor::All, gui.clone(), false, Some("Old GUI admission failure".into()));
        observations.admit(&TargetMonitor::All, &tray).unwrap();
        observations.begin().unwrap();
        observations.finish(TargetMonitor::All, tray.clone(), true, None);
        assert_eq!(observations.scopes.len(), 2);
        assert_eq!(observations.scopes[0].request, gui);
        assert_eq!(observations.scopes[1].request, tray);
        let mut warnings = vec!["Unresolved controller conflict".into()];
        observations.append_errors(&mut warnings);
        assert_eq!(warnings, ["Unresolved controller conflict"]);
        let serialized = serde_json::to_value(&observations.scopes).unwrap();
        assert_eq!(serialized[0]["request"]["client_id"], "gui:window");
        assert_eq!(serialized[0]["request"]["sequence"], "2");
    }

    #[test]
    fn correlation_rejects_duplicate_or_reordered_same_client_commands_before_another_operation() {
        let mut observations = ManualObservations::default();
        let request = |sequence: &str| ManualRequestIdentity { client_id: "gui:window".into(), sequence: sequence.into() };
        observations.admit(&TargetMonitor::All, &request("2")).unwrap();
        observations.begin().unwrap();
        observations.finish(TargetMonitor::All, request("2"), true, None);
        for sequence in ["1", "2"] {
            assert!(observations.admit(&TargetMonitor::All, &request(sequence)).is_err());
        }
        assert_eq!(observations.revision, 1);
        observations.admit(&TargetMonitor::All, &request("3")).unwrap();
        observations.admit(&TargetMonitor::Monitor {
            device_path: "another".into(), display_name: "Another".into(),
        }, &request("1")).unwrap();
    }

    #[test]
    fn correlation_rejects_malformed_identifiers_before_queue_admission() {
        let fixture = GateFixture::new();
        let (service, receiver) = idle_service(fixture.manager.clone());
        for (client_id, sequence) in [
            ("", "1"), ("gui:bad\nid", "1"), ("gui:test", "0"), ("gui:test", "01"),
            ("gui:test", "-1"), ("gui:test", "18446744073709551616"),
        ] {
            assert!(service.manual_set(TargetMonitor::All, true, ManualRequestIdentity {
                client_id: client_id.into(), sequence: sequence.into(),
            }).is_err());
        }
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn review_manual_observations_order_both_origins_and_keep_scope_recovery_proofs() {
        let mut observations = ManualObservations::default();
        let other = TargetMonitor::Monitor { device_path: "other".into(), display_name: "Other".into() };
        observations.begin().unwrap();
        observations.finish(TargetMonitor::All, test_request(), false, Some("All request refused".into()));
        let failed_all = observations.scopes.clone();
        observations.begin().unwrap();
        observations.finish(other.clone(), test_request(), true, None);
        let mut warnings = vec!["Unresolved controller conflict".into()];
        observations.append_errors(&mut warnings);
        assert_eq!(warnings, ["Unresolved controller conflict", "All request refused"]);
        assert_eq!(observations.scopes[0], failed_all[0]);
        observations.begin().unwrap();
        observations.finish(TargetMonitor::All, test_request(), true, None);
        assert_eq!(observations.scopes.len(), 2);
        assert_eq!(observations.scopes[0].revision, "3");
        assert!(observations.scopes[0].verified);
        assert_eq!(observations.scopes[1].revision, "2");
        let mut current = vec!["Unresolved controller conflict".into()];
        observations.append_errors(&mut current);
        assert_eq!(current, ["Unresolved controller conflict"]);
        observations.begin().unwrap();
        observations.finish(other, test_request(), false, Some("Newer Other failure".into()));
        assert_eq!(observations.scopes[0].revision, "3", "full snapshots retain missed recovery proofs");
        assert_eq!(observations.scopes[1].revision, "4");
        assert!(!observations.scopes[1].verified);
        assert_eq!(failed_all[0].error.as_deref(), Some("All request refused"), "old replies are immutable");
    }

    #[test]
    fn review_manual_scope_revisions_use_identity_and_do_not_duplicate_warning_messages() {
        let mut observations = ManualObservations::default();
        for (path, name) in [("Chosen", "Display"), ("CHOSEN", "Renamed")] {
            observations.begin().unwrap();
            observations.finish(TargetMonitor::Monitor {
                device_path: path.into(), display_name: name.into(),
            }, test_request(), false, Some("Request refused".into()));
        }
        assert_eq!(observations.scopes.len(), 1);
        assert_eq!(observations.scopes[0].revision, "2");
        let mut warnings = vec!["Request refused".into()];
        observations.append_errors(&mut warnings);
        assert_eq!(warnings, ["Request refused"]);
        let mut payload = HdrStatePayload::unavailable("Read failed".into());
        payload.manual_revision = observations.revision.to_string();
        payload.manual_results = observations.scopes.clone();
        let serialized = serde_json::to_value(payload).unwrap();
        assert_eq!(serialized["manual_revision"], "2");
        assert_eq!(serialized["manual_results"][0]["revision"], "2");
        assert_eq!(serialized["manual_results"][0]["scope"]["device_path"], "CHOSEN");
        observations.revision = u64::MAX;
        assert!(observations.begin().is_err(), "exhausted ordering must not admit another native operation");
    }

    #[test]
    fn status_exposes_inventory_and_status_versions_as_decimal_strings() {
        let serialized = serde_json::to_value(
            HdrStatePayload::unavailable("Read failed".into()),
        ).unwrap();
        assert_eq!(serialized["inventory_revision"], "0");
        assert_eq!(serialized["status_revision"], "0");
    }

    #[test]
    fn inventory_changes_publish_with_unchanged_aggregate_and_identical_polls_stay_quiet() {
        let mut publication = StatusPublication::default();
        let mut payload = HdrStatePayload::unavailable("unchanged automatic policy".into());
        payload.inventory_revision = "1".into();
        let first = publication.observe(payload.clone());
        assert_eq!(first.status_revision, "1");
        publication.published(first.clone());
        let repeated = publication.observe(payload.clone());
        assert!(!publication.changed(&repeated));

        payload.inventory_revision = "2".into();
        let connected = publication.observe(payload.clone());
        assert_eq!(connected.scope_hdr_state, first.scope_hdr_state);
        assert_eq!(connected.any_hdr_active, first.any_hdr_active);
        assert_eq!(connected.status_revision, "2");
        assert!(publication.changed(&connected));
        publication.published(connected);
        for _ in 0..100 {
            let repeated = publication.observe(payload.clone());
            assert_eq!(repeated.status_revision, "2");
            assert!(!publication.changed(&repeated));
        }
    }

    #[test]
    fn manual_warning_recovery_has_a_new_status_revision_without_inventory_changes() {
        let mut publication = StatusPublication::default();
        let mut payload = HdrStatePayload::unavailable("The last manual HDR request failed".into());
        payload.inventory_revision = "7".into();
        let failed = publication.observe(payload.clone());
        publication.published(failed.clone());
        payload.warning = None;
        let recovered = publication.observe(payload);
        assert_eq!(recovered.inventory_revision, failed.inventory_revision);
        assert_ne!(recovered.status_revision, failed.status_revision);
        assert!(publication.changed(&recovered));
        publication.published(recovered.clone());
        assert!(!publication.changed(&recovered));
    }

    #[test]
    fn manual_native_success_does_not_clear_unresolved_preparation_but_verified_retry_does() {
        use crate::display::tests::{monitor, MockDisplay};

        struct NoWrite;
        impl WriteAuthority for NoWrite {
            fn authorize(
                &mut self, _: &NativeAttempt, _: &mut dyn FnMut(),
            ) -> Result<(), DisplayFailure> {
                panic!("an already-satisfied mock observation must not issue a native write");
            }
        }
        let mut controller = HdrController::new(MockDisplay::new(vec![monitor("chosen", 1, false)]));
        controller.warn("Unresolved controller conflict");
        for failure in ["Activation target unavailable", "Process watcher unavailable"] {
            let mut preparation = ActivationPreparation::default();
            preparation.observed(Err(failure.into()));
            let mut publication = StatusPublication::default();
            let mut failed = HdrStatePayload::unavailable(failure.into());
            failed.inventory_revision = "1".into();
            let failed = publication.observe(failed);
            publication.published(failed.clone());

            // A native state verification alone does not prove automatic setup recovered.
            let manual = controller.manual_set(
                &TargetMonitor::All, false, &mut NoWrite,
            );
            assert!(manual[0].is_verified());
            assert_eq!(preparation.warning.as_deref(), Some(failure));
            // The actor records this only after successful begin/watch setup, or a verified
            // observation that automatic setup is not required for the foreground process.
            preparation.observed(Ok(()));
            assert!(preparation.warning.is_none());
            assert_eq!(controller.warning().as_deref(), Some("Unresolved controller conflict"));
            let mut recovered = failed.clone();
            recovered.warning = controller.warning();
            let recovered = publication.observe(recovered);
            assert!(publication.changed(&recovered));
            assert_ne!(recovered.status_revision, failed.status_revision);
        }
    }

    #[test]
    fn eligibility_requires_enabled_exact_or_alternate_executable_and_no_blacklist() {
        let mut config = AppConfig::default();
        config.apps = vec![HdrApp {
            name: "Game".into(),
            exe_name: "game.exe".into(),
            enabled: true,
            hdr_type: HdrType::Native,
            path: None,
            alternate_exes: vec!["game-dx12.exe".into()],
            steam_id: None,
            launcher: None,
        }];
        assert!(eligible_app(&config, "GAME.EXE").is_some());
        assert!(eligible_app(&config, "game-dx12.exe").is_some());
        assert!(eligible_app(&config, "game-unlisted.exe").is_none());
        config.apps[0].enabled = false;
        assert!(eligible_app(&config, "game-dx12.exe").is_none());
        config.apps[0].enabled = true;
        config.blacklist.push("GAME-DX12.EXE".into());
        assert!(eligible_app(&config, "game-dx12.exe").is_none());
    }

    #[test]
    fn producer_hints_are_coalesced_and_stop_after_admission_closes() {
        let fixture = GateFixture::new();
        let (service, receiver) = idle_service(fixture.manager.clone());
        let events = &service.events;
        events.foreground();
        events.foreground();
        assert_eq!(events.foreground_generation.load(Ordering::Acquire), 2);
        assert!(matches!(
            receiver.try_recv(),
            Ok(Command::ForegroundObserved)
        ));
        assert!(receiver.try_recv().is_err());
        events.foreground_pending.store(false, Ordering::Release);
        service.config_committed();
        service.config_committed();
        assert!(matches!(receiver.try_recv(), Ok(Command::ConfigCommitted)));
        assert!(receiver.try_recv().is_err());
        events.config_pending.store(false, Ordering::Release);
        events.admitted.store(false, Ordering::Release);
        events.foreground();
        service.config_committed();
        assert!(receiver.try_recv().is_err());
        assert_eq!(events.foreground_generation.load(Ordering::Acquire), 2);
    }

    #[test]
    fn background_enrichment_wakes_controller_without_a_foreground_event() {
        let fixture = GateFixture::new();
        let before = fixture.ready();
        let (service, receiver) = idle_service(fixture.manager.clone());
        let foreground_exe = "game-dx12.exe";
        assert!(automatic_pause(&before, foreground_exe, &before.context_token).is_some());
        let mut detected = before.settings.apps[0].clone();
        detected.exe_name = foreground_exe.into();
        let result = fixture.manager.mutate(
            &before.context_token,
            Some(&before.library_generation),
            true,
            |settings| {
                crate::library::enrich_existing(settings, &[detected]);
                Ok(())
            },
        );
        let committed = result.as_ref().unwrap().clone();
        let mut emitted = None;
        crate::background::publish_result(&fixture.manager, &service, result, |snapshot| {
            emitted = Some(snapshot.clone());
            assert!(receiver.try_recv().is_err());
        });
        assert_eq!(emitted, Some(committed.clone()));
        assert!(matches!(receiver.try_recv(), Ok(Command::ConfigCommitted)));
        assert!(receiver.try_recv().is_err());
        assert!(!service.events.foreground_pending.load(Ordering::Acquire));
        let current = fixture.manager.snapshot().unwrap();
        assert_eq!(current, committed);
        assert!(automatic_pause(&current, foreground_exe, &before.context_token).is_none());
    }

    #[test]
    fn background_recovery_wakes_controller_after_failed_commit() {
        let fixture = GateFixture::new();
        let before = fixture.ready();
        let (service, receiver) = idle_service(fixture.manager.clone());
        assert!(automatic_pause(&before, "game.exe", &before.context_token).is_none());
        let conflicting_bytes = b"unreadable settings";
        std::fs::write(&before.config_path, conflicting_bytes).unwrap();
        let result = fixture.manager.mutate(
            &before.context_token,
            None,
            false,
            |settings| {
                settings.last_sync_timestamp = Some(123);
                Ok(())
            },
        );
        assert!(result.is_err());
        let current = fixture.manager.snapshot().unwrap();
        assert_eq!(current.mode, ConfigMode::RecoveryRequired);
        assert_ne!(current.context_token, before.context_token);
        let mut emitted = None;
        crate::background::publish_result(&fixture.manager, &service, result, |snapshot| {
            emitted = Some(snapshot.clone());
            assert!(receiver.try_recv().is_err());
        });
        assert_eq!(emitted, Some(current.clone()));
        assert!(matches!(receiver.try_recv(), Ok(Command::ConfigCommitted)));
        assert!(receiver.try_recv().is_err());
        assert!(!service.events.foreground_pending.load(Ordering::Acquire));
        assert!(automatic_pause(&current, "game.exe", &before.context_token).is_some());
        assert_eq!(std::fs::read(&before.config_path).unwrap(), conflicting_bytes);
    }

    #[test]
    fn foreground_retries_unchanged_pid_after_error_or_incomplete_observation() {
        for incomplete in [false, true] {
            let now = Instant::now();
            let key = observation_key(42);
            let mut observation = ForegroundObservation::default();
            let result = if incomplete { Ok(None) } else { Err("Access denied".into()) };
            assert!(sample_at(&mut observation, key, now, result).is_none());
            assert!(observation.warning.is_some());
            let deadline = now + FOREGROUND_RETRY_DELAYS[0];
            assert_eq!(observation.retry_at, Some(deadline));
            assert!(!observation.needs_observation(key, deadline - Duration::from_millis(1)));
            assert!(observation.needs_observation(key, deadline));
            let process = test_process(42, 100, "game.exe");
            let recovered = sample_at(
                &mut observation, key, deadline, Ok(Some(process.clone())),
            ).unwrap();
            assert_eq!(recovered.identity, process.identity);
            assert!(observation.warning.is_none());
            assert!(observation.retry_at.is_none());
        }
    }

    fn observation_key(pid: u32) -> ForegroundKey {
        ForegroundKey { pid, generation: 1 }
    }

    fn test_process(pid: u32, created_at: u64, exe: &str) -> TrackedProcess {
        // An owned, unnamed event gives deterministic live/exited handle waits without a real app.
        let handle = unsafe { CreateEventW(None, true, false, None) }.unwrap();
        TrackedProcess {
            identity: ProcessIdentity { pid, created_at },
            exe: exe.into(),
            path: format!(r"C:\Fixture\{exe}"),
            handle: Arc::new(OwnedHandle(handle)),
        }
    }

    fn sample_at(
        observation: &mut ForegroundObservation,
        key: ForegroundKey,
        now: Instant,
        result: Result<Option<TrackedProcess>, String>,
    ) -> Option<TrackedProcess> {
        observation.sample(key, now, |_| result, || (key, now))
    }

    #[test]
    fn foreground_backoff_is_bounded_and_exhaustion_rearms_at_a_slow_cadence() {
        let mut now = Instant::now();
        let key = observation_key(42);
        let mut observation = ForegroundObservation::default();
        for seconds in [1, 2, 4, 30, 30, 30, 30] {
            let delay = Duration::from_secs(seconds);
            assert!(sample_at(&mut observation, key, now, Err("Access denied".into())).is_none());
            let deadline = now + delay;
            assert_eq!(observation.retry_at, Some(deadline));
            for offset in [Duration::ZERO, delay / 2, delay - Duration::from_millis(1)] {
                assert!(observation.sample(
                    key, now + offset,
                    |_| panic!("inspection ran before its deadline"),
                    || panic!("no inspection should be in flight"),
                ).is_none());
                assert_eq!(observation.retry_at, Some(deadline));
            }
            assert!(observation.needs_observation(key, deadline));
            now = deadline;
        }
        let recovered = sample_at(
            &mut observation, key, now, Ok(Some(test_process(42, 100, "game.exe"))),
        ).unwrap();
        assert_eq!(recovered.exe, "game.exe");
        assert!(!observation.needs_observation(key, now + Duration::from_secs(300)));
    }

    #[test]
    fn foreground_stable_success_only_checks_the_cached_handle_on_watchdog_ticks() {
        let now = Instant::now();
        let key = observation_key(42);
        let mut observation = ForegroundObservation::default();
        let process = test_process(42, 100, "unlisted.exe");
        sample_at(&mut observation, key, now, Ok(Some(process.clone()))).unwrap();
        for tick in 1..=120 {
            let later = now + FOREGROUND_WATCHDOG_INTERVAL * tick;
            assert!(!observation.needs_observation(key, later));
            let cached = observation.sample(
                key, later,
                |_| panic!("a stable process must not be reinspected"),
                || panic!("a cached result has no in-flight observation"),
            ).unwrap();
            assert_eq!(cached.identity, process.identity);
        }
        let same_process_new_hint = ForegroundKey { generation: 2, ..key };
        assert!(!observation.needs_observation(same_process_new_hint, now));
    }

    #[test]
    fn inventory_watchdog_skips_four_idle_ticks_and_deduplicates_unchanged_observations() {
        use crate::display::tests::{monitor, MockDisplay};

        let now = Instant::now();
        let mut schedule = InventoryObservation::new(now);
        let mut controller = HdrController::new(MockDisplay::new(vec![monitor("chosen", 1, false)]));
        controller.refresh_inventory().unwrap();
        schedule.refreshed(now);
        let mut publication = StatusPublication::default();
        let mut payload = HdrStatePayload::unavailable("unchanged policy".into());
        payload.inventory_revision = controller.inventory_revision();
        let first = publication.observe(payload.clone());
        publication.published(first);
        let mut probes = 0;
        for tick in 1..=60 {
            let at = now + FOREGROUND_WATCHDOG_INTERVAL * tick;
            assert_eq!(schedule.due(at), tick % 5 == 0);
            if schedule.due(at) {
                controller.refresh_inventory().unwrap();
                schedule.refreshed(at);
                probes += 1;
                payload.inventory_revision = controller.inventory_revision();
                let observed = publication.observe(payload.clone());
                assert!(!publication.changed(&observed));
            }
        }
        assert_eq!(probes, 12, "the one-second foreground watchdog must not query inventory");
    }

    #[test]
    fn event_refreshes_rearm_inventory_deadline_from_completion_without_catch_up() {
        let now = Instant::now();
        let mut schedule = InventoryObservation::new(now);
        assert!(!schedule.due(now + Duration::from_secs(4)));
        // Foreground/config/manual/explicit refreshes all reset the independent probe.
        schedule.refreshed(now + Duration::from_secs(4));
        assert!(!schedule.due(now + Duration::from_secs(5)));
        assert!(!schedule.due(now + Duration::from_secs(8)));
        assert!(schedule.due(now + Duration::from_secs(9)));
        // A slow successful or failed query receives a complete quiet interval afterwards.
        schedule.refreshed(now + Duration::from_secs(20));
        for tick in 20..25 {
            assert!(!schedule.due(now + Duration::from_secs(tick)));
        }
        assert!(schedule.due(now + Duration::from_secs(25)));
    }

    #[test]
    fn independent_inventory_deadline_wakes_between_foreground_watchdog_ticks() {
        let now = Instant::now();
        let mut schedule = InventoryObservation::new(now);
        schedule.refreshed(now + Duration::from_millis(4500));
        let at = now + Duration::from_secs(9);
        let foreground = now + Duration::from_secs(10);
        assert_eq!(
            actor_wait_timeout(None, foreground.min(schedule.deadline), at),
            Duration::from_millis(500),
        );
    }

    #[test]
    fn foreground_status_config_and_coalesced_hints_cannot_postpone_retry() {
        let now = Instant::now();
        let watchdog_deadline = now + FOREGROUND_WATCHDOG_INTERVAL;
        let key = observation_key(42);
        let mut observation = ForegroundObservation::default();
        sample_at(&mut observation, key, now, Err("Access denied".into()));
        let retry_deadline = observation.retry_at.unwrap();
        for command in 1..=100 {
            let at = now + Duration::from_millis(command * 10);
            assert_eq!(
                actor_wait_timeout(None, watchdog_deadline, at),
                watchdog_deadline.saturating_duration_since(at),
            );
            let key = ForegroundKey { generation: command, ..key };
            if at < retry_deadline {
                assert!(observation.sample(
                    key, at,
                    |_| panic!("config traffic bypassed retry backoff"),
                    || panic!("no inspection should be in flight"),
                ).is_none());
                assert_eq!(observation.retry_at, Some(retry_deadline));
            } else {
                assert!(sample_at(
                    &mut observation, key, at, Ok(Some(test_process(42, 100, "game.exe"))),
                ).is_some());
            }
        }
        assert_eq!(actor_wait_timeout(None, watchdog_deadline, retry_deadline), Duration::ZERO);
    }

    #[test]
    fn foreground_slow_failures_schedule_from_completion_without_catch_up_attempts() {
        let now = Instant::now();
        let finished = now + Duration::from_secs(10);
        let key = observation_key(42);
        let mut observation = ForegroundObservation::default();
        assert!(observation.sample(
            key, now,
            |_| Err("Access denied".into()),
            || (key, finished),
        ).is_none());
        assert_eq!(observation.retry_at, Some(finished + FOREGROUND_RETRY_DELAYS[0]));
        assert!(!observation.needs_observation(key, finished));
    }

    #[test]
    fn foreground_late_success_and_errors_are_fenced_by_pid_and_generation() {
        let now = Instant::now();
        let key = observation_key(42);
        for latest in [
            ForegroundKey { pid: 7, ..key },
            ForegroundKey { generation: 2, ..key },
        ] {
            for success in [false, true] {
                let mut observation = ForegroundObservation::default();
                let result = if success {
                    Ok(Some(test_process(42, 100, "old.exe")))
                } else {
                    Err("old inspection error".into())
                };
                assert!(observation.sample(key, now, |_| result, || (latest, now)).is_none());
                assert!(observation.process.is_none());
                assert!(!observation.completed);
                assert_ne!(observation.warning.as_deref(), Some("old inspection error"));
                let deadline = observation.retry_at.unwrap_or(now);
                let recovered = sample_at(
                    &mut observation, latest, deadline,
                    Ok(Some(test_process(latest.pid, 200, "current.exe"))),
                ).unwrap();
                assert_eq!(recovered.exe, "current.exe");
                assert!(observation.warning.is_none());
            }
        }
    }

    #[test]
    fn foreground_coalesced_round_trip_invalidates_in_flight_success() {
        let fixture = GateFixture::new();
        let (service, receiver) = idle_service(fixture.manager.clone());
        let now = Instant::now();
        let key = ForegroundKey { pid: 42, generation: 0 };
        let mut observation = ForegroundObservation::default();
        assert!(observation.sample(
            key, now,
            |_| {
                service.events.foreground();
                service.events.foreground();
                Ok(Some(test_process(42, 100, "game.exe")))
            },
            || (ForegroundKey {
                generation: service.events.foreground_generation.load(Ordering::Acquire),
                ..key
            }, now),
        ).is_none());
        assert!(matches!(receiver.try_recv(), Ok(Command::ForegroundObserved)));
        assert!(receiver.try_recv().is_err());
        assert!(observation.process.is_none());
        assert_eq!(observation.retry_at, Some(now + FOREGROUND_RETRY_DELAYS[0]));
    }

    #[test]
    fn foreground_pid_reuse_rejects_old_creation_identity_and_inspects_the_new_lifetime() {
        let now = Instant::now();
        let key = observation_key(42);
        let mut observation = ForegroundObservation::default();
        let old = test_process(42, 100, "old.exe");
        sample_at(&mut observation, key, now, Ok(Some(old.clone()))).unwrap();
        unsafe { SetEvent(old.handle.0) }.unwrap();
        assert!(observation.needs_observation(key, now));
        assert!(observation.process.is_none());
        assert!(sample_at(&mut observation, key, now, Ok(Some(old.clone()))).is_none());
        let new = test_process(42, 200, "new.exe");
        let recovered = sample_at(
            &mut observation, key, now + FOREGROUND_RETRY_DELAYS[0], Ok(Some(new.clone())),
        ).unwrap();
        assert_ne!(recovered.identity, old.identity);
        assert_eq!(recovered.identity, new.identity);
        assert_eq!(recovered.exe, "new.exe");
    }

    #[test]
    fn foreground_rejects_a_mismatched_process_identity() {
        let now = Instant::now();
        let mut observation = ForegroundObservation::default();
        assert!(sample_at(
            &mut observation, observation_key(42), now,
            Ok(Some(test_process(7, 100, "wrong.exe"))),
        ).is_none());
        assert!(observation.process.is_none());
        assert!(observation.retry_at.is_some());
    }

    #[test]
    fn foreground_failure_never_reuses_the_previous_app_and_empty_foreground_is_complete() {
        let now = Instant::now();
        let mut observation = ForegroundObservation::default();
        let previous = test_process(42, 100, "game.exe");
        sample_at(
            &mut observation, observation_key(42), now, Ok(Some(previous.clone())),
        ).unwrap();
        assert!(sample_at(
            &mut observation, observation_key(7), now, Err("Access denied".into()),
        ).is_none());
        assert!(previous.is_alive(), "the old activation may still need exit/cleanup tracking");
        assert!(observation.process.is_none(), "it must not become the new foreground identity");
        assert!(sample_at(
            &mut observation, observation_key(0), now, Ok(None),
        ).is_none());
        assert!(observation.warning.is_none());
        assert!(!observation.needs_observation(observation_key(0), now + Duration::from_secs(60)));
    }

    #[test]
    fn foreground_recovery_retires_only_the_scoped_inspection_warning() {
        use crate::display::tests::MockDisplay;

        let now = Instant::now();
        let key = observation_key(42);
        let mut observation = ForegroundObservation::default();
        let mut controller = HdrController::new(MockDisplay::default());
        controller.warn("A persistent display issue");
        sample_at(&mut observation, key, now, Err("Protected process access denied".into()));
        assert_eq!(observation.warnings(controller.warning()), [
            "A persistent display issue", "Protected process access denied",
        ]);
        assert_eq!(controller.warning().as_deref(), Some("A persistent display issue"));
        sample_at(
            &mut observation, key, now + FOREGROUND_RETRY_DELAYS[0],
            Ok(Some(test_process(42, 100, "game.exe"))),
        ).unwrap();
        assert_eq!(observation.warnings(controller.warning()), ["A persistent display issue"]);
        sample_at(
            &mut observation, observation_key(7), now, Err("another process denied".into()),
        );
        sample_at(
            &mut observation, observation_key(8), now,
            Ok(Some(test_process(8, 300, "current.exe"))),
        ).unwrap();
        assert_eq!(observation.warnings(controller.warning()), ["A persistent display issue"]);
    }

    #[test]
    fn foreground_policy_changes_reresolve_a_cached_identity_without_reinspection() {
        let now = Instant::now();
        let key = observation_key(42);
        let mut observation = ForegroundObservation::default();
        let process = test_process(42, 100, "game.exe");
        sample_at(&mut observation, key, now, Ok(Some(process.clone()))).unwrap();
        let mut snapshot = ready_snapshot();
        for restriction in 0..6 {
            let ready = snapshot.clone();
            match restriction {
                0 => snapshot.settings.apps[0].enabled = false,
                1 => snapshot.settings.blacklist.push("GAME.EXE".into()),
                2 => snapshot.settings.switch_method = SwitchMethod::Shortcut,
                3 => snapshot.mode = ConfigMode::RecoveryRequired,
                4 => snapshot.controller_issue = Some(crate::SAFE_TEST_ISSUE.into()),
                _ => snapshot.settings.apps.clear(),
            }
            let cached = observation.sample(
                key, now,
                |_| panic!("a policy change must not require process inspection"),
                || panic!("no inspection should be in flight"),
            ).unwrap();
            assert_eq!(cached.identity, process.identity);
            assert!(automatic_pause(&snapshot, &cached.exe, &snapshot.context_token).is_some());
            snapshot = ready;
            assert!(automatic_pause(&snapshot, &cached.exe, &snapshot.context_token).is_none());
        }
    }

    #[test]
    fn delayed_and_cached_observations_use_latest_path_disable_and_repair_policy() {
        let fixture = GateFixture::new();
        let initial = fixture.ready();
        let now = Instant::now();
        for repair in [false, true] {
            let ready = fixture.manager.mutate(&initial.context_token, None, true, |settings| {
                settings.apps = initial.settings.apps.clone();
                settings.apps[0].path = Some(r"C:\Fixture\game.exe".into());
                Ok(())
            }).unwrap();
            let mut observation = ForegroundObservation::default();
            let key = observation_key(42);
            let process = observation.sample(
                key, now,
                |_| {
                    fixture.manager.mutate(&ready.context_token, None, true, |settings| {
                        if repair {
                            settings.apps[0].exe_name = "user-selected.exe".into();
                            settings.apps[0].path = Some(r"C:\Fixture\user-selected.exe".into());
                            settings.apps[0].alternate_exes.clear();
                        } else {
                            settings.apps[0].enabled = false;
                        }
                        Ok(())
                    }).unwrap();
                    Ok(Some(test_process(42, 100, "game.exe")))
                },
                || (key, now),
            ).unwrap();
            assert!(automatic_pause_for_path(
                &ready, Some(&process.path), &process.exe, &ready.context_token,
            ).is_none());
            let latest = fixture.manager.snapshot().unwrap();
            assert!(automatic_pause_for_path(
                &latest, Some(&process.path), &process.exe, &ready.context_token,
            ).is_some());
            let cached = observation.sample(
                key, now, |_| panic!("policy does not need reinspection"),
                || panic!("cached observation has no in-flight operation"),
            ).unwrap();
            assert!(automatic_pause_for_path(
                &latest, Some(&cached.path), &cached.exe, &ready.context_token,
            ).is_some());
        }
    }

    #[test]
    fn every_nonmatch_reconciles_owned_hdr_without_claiming_user_hdr() {
        use crate::display::tests::{monitor, MockDisplay};
        struct Enable;
        impl WriteAuthority for Enable {
            fn authorize(&mut self, _: &NativeAttempt, issue: &mut dyn FnMut()) -> Result<(), DisplayFailure> {
                issue();
                Ok(())
            }
        }
        let fixture = GateFixture::new();
        fixture.ready();
        for rejection in 0..6 {
            let mut snapshot = ready_snapshot();
            let mut rejected = snapshot.settings.apps[0].clone();
            rejected.exe_name = "other.exe".into();
            snapshot.settings.apps.push(rejected.clone());
            let exe = match rejection {
                0 => "GameLaunchHelper.exe",
                1 => { snapshot.settings.apps[1].enabled = false; "other.exe" }
                2 => { snapshot.settings.apps.push(rejected); "other.exe" }
                3 => "unlisted.exe",
                4 => {
                    snapshot.settings.apps[1].exe_name = "BsSndRpt.exe".into();
                    snapshot.settings.apps[1].alternate_exes.push("other.exe".into());
                    "other.exe"
                }
                _ => {
                    snapshot.settings.apps[1].path = Some(r"D:\Other\other.exe".into());
                    "other.exe"
                }
            };
            assert!(automatic_pause_for_path(
                &snapshot, Some(r"C:\Fixture\other.exe"), exe, "context",
            ).is_some());
            let mut controller = HdrController::new(MockDisplay::new(vec![
                monitor("owned", 1, false), monitor("user", 2, true),
            ]));
            controller.refresh_inventory().unwrap();
            controller.begin(
                ProcessIdentity { pid: 42, created_at: 100 }, "game.exe".into(),
                "context".into(), TargetMonitor::All,
            ).unwrap();
            controller.enable_activation(&mut Enable);
            assert_eq!(unmatched_action(true, true, false, false), UnmatchedAction::Debounce);
            assert_eq!(unmatched_action(true, true, true, true), UnmatchedAction::KeepUntilExit);
            for (eligible, alive, exit_only, expired) in [
                (true, true, false, true), (true, false, true, false), (false, true, true, false),
            ] {
                assert_eq!(unmatched_action(eligible, alive, exit_only, expired), UnmatchedAction::Finish);
            }
            let action = unmatched_action(true, true, false, true);
            let outcomes = match action {
                UnmatchedAction::Finish => controller.end(&mut fixture.authority(OperationKind::Cleanup), usize::MAX),
                _ => panic!("expired nonmatch must follow cleanup"),
            };
            assert_eq!(outcomes.len(), 1);
            assert_eq!(outcomes[0].device_path.as_deref(), Some("owned"));
            assert!(!outcomes[0].requested_hdr);
            controller.refresh_inventory().unwrap();
            assert!(!controller.inventory()[0].is_hdr_enabled);
            assert!(controller.inventory()[1].is_hdr_enabled);
        }
    }

    #[test]
    fn foreground_inspection_failure_does_not_discard_owned_cleanup_or_claim_preexisting_hdr() {
        use crate::display::tests::{monitor, MockDisplay};

        let fixture = GateFixture::new();
        fixture.ready();
        let now = Instant::now();
        let mut observation = ForegroundObservation::default();
        let process = sample_at(
            &mut observation, observation_key(42), now,
            Ok(Some(test_process(42, 100, "game.exe"))),
        ).unwrap();
        let mut controller = HdrController::new(MockDisplay::new(vec![
            monitor("owned", 1, false), monitor("preexisting", 2, true),
        ]));
        controller.refresh_inventory().unwrap();
        controller.begin(
            process.identity, process.exe, "context".into(), TargetMonitor::All,
        ).unwrap();
        struct Enable;
        impl WriteAuthority for Enable {
            fn authorize(
                &mut self, _: &NativeAttempt, issue: &mut dyn FnMut(),
            ) -> Result<(), DisplayFailure> {
                issue();
                Ok(())
            }
        }
        controller.enable_activation(&mut Enable);
        assert!(controller.has_ownership());
        let activation = controller.activation().unwrap().clone();
        assert!(sample_at(
            &mut observation, observation_key(7), now, Err("Access denied".into()),
        ).is_none());
        assert!(controller.matches_activation(activation.generation, activation.process));
        let outcomes = controller.end(&mut fixture.authority(OperationKind::Cleanup), usize::MAX);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].device_path.as_deref(), Some("owned"));
        assert_eq!(outcomes[0].outcome, OutcomeKind::Changed);
        assert!(!outcomes[0].requested_hdr);
        controller.refresh_inventory().unwrap();
        assert!(!controller.inventory()[0].is_hdr_enabled);
        assert!(controller.inventory()[1].is_hdr_enabled);
        assert!(controller.activation().is_none());
        assert!(!controller.has_ownership());
    }

    #[test]
    fn watchdog_wakeup_does_not_expire_a_later_debounce() {
        let now = Instant::now();
        let mut debounce = Some(Debounce {
            deadline: now + Duration::from_secs(5),
            generation: 1,
            process: ProcessIdentity {
                pid: 2,
                created_at: 3,
            },
            seconds: 5,
        });
        assert_eq!(
            actor_wait_timeout(debounce, now + FOREGROUND_WATCHDOG_INTERVAL, now),
            FOREGROUND_WATCHDOG_INTERVAL
        );
        assert!(take_expired_debounce(&mut debounce, now + FOREGROUND_WATCHDOG_INTERVAL).is_none());
        assert!(take_expired_debounce(&mut debounce, now + Duration::from_secs(5)).is_some());
    }

    #[test]
    fn shorter_debounce_precedes_the_watchdog_deadline() {
        let now = Instant::now();
        let debounce = Some(Debounce {
            deadline: now + Duration::from_millis(200),
            generation: 1,
            process: ProcessIdentity {
                pid: 2,
                created_at: 3,
            },
            seconds: 0,
        });
        assert_eq!(
            actor_wait_timeout(debounce, now + FOREGROUND_WATCHDOG_INTERVAL, now),
            Duration::from_millis(200)
        );
    }

    #[test]
    fn current_gate_rejects_retired_history_recovery_withdrawn_consent_and_disabled_apps() {
        let mut snapshot = ready_snapshot();
        assert!(automatic_pause(&snapshot, "game.exe", "context").is_none());
        assert!(automatic_pause(&snapshot, "game.exe", "retired").is_some());
        snapshot.mode = ConfigMode::RecoveryRequired;
        assert!(automatic_pause(&snapshot, "game.exe", "context").is_some());
        snapshot.mode = ConfigMode::Ready;
        snapshot.settings.switch_method = SwitchMethod::Shortcut;
        assert!(automatic_pause(&snapshot, "game.exe", "context").is_some());
        snapshot.settings.switch_method = SwitchMethod::Native;
        snapshot.settings.apps[0].enabled = false;
        assert!(automatic_pause(&snapshot, "game.exe", "context").is_some());
        snapshot.settings.apps[0].enabled = true;
        snapshot.controller_issue = Some("predecessor conflict".into());
        assert!(automatic_pause(&snapshot, "game.exe", "context").is_some());
    }

    #[test]
    fn controller_conflict_blocks_manual_and_cleanup_authorizations_without_native_calls() {
        let fixture = GateFixture::new();
        fixture
            .manager
            .set_controller_issue(Some("predecessor conflict".into()))
            .unwrap();
        for kind in [OperationKind::Manual, OperationKind::Cleanup] {
            let mut authority = fixture.authority(kind);
            let mut issued = false;
            let mut attempt = mock_attempt();
            if kind == OperationKind::Cleanup {
                attempt.purpose = NativePurpose::Cleanup;
                attempt.requested_hdr = false;
            }
            let result = authority.authorize(&attempt, &mut || issued = true);
            assert_eq!(result.unwrap_err().kind, FailureKind::AuthorityDenied);
            assert!(!issued);
        }
    }

    #[test]
    fn closing_admission_blocks_manual_but_allows_only_bounded_cleanup() {
        let fixture = GateFixture::new();
        let mut authority = fixture.authority(OperationKind::Manual);
        authority.admitted.store(false, Ordering::Release);
        let mut issued = 0;
        assert!(authority
            .authorize(&mock_attempt(), &mut || issued += 1)
            .is_err());
        authority.kind = OperationKind::Cleanup;
        authority.shutdown_budget.store(1, Ordering::Release);
        let mut cleanup = mock_attempt();
        cleanup.purpose = NativePurpose::Cleanup;
        cleanup.requested_hdr = false;
        assert!(authority.authorize(&cleanup, &mut || issued += 1).is_ok());
        assert_eq!(
            authority
                .authorize(&cleanup, &mut || issued += 1)
                .unwrap_err()
                .kind,
            FailureKind::AttemptBudgetExhausted,
        );
        assert_eq!(issued, 1);
    }

    #[test]
    fn cleanup_authority_cannot_be_reused_to_enable_hdr() {
        let fixture = GateFixture::new();
        let mut authority = fixture.authority(OperationKind::Cleanup);
        let mut attempt = mock_attempt();
        attempt.purpose = NativePurpose::Cleanup;
        assert_eq!(
            authority
                .authorize(&attempt, &mut || panic!("cleanup enabled HDR"))
                .unwrap_err()
                .kind,
            FailureKind::AuthorityDenied,
        );
    }

    #[test]
    fn issuing_and_gate_publication_share_the_same_short_boundary() {
        let fixture = GateFixture::new();
        let (entered_tx, entered_rx) = channel();
        let (release_tx, release_rx) = channel();
        let (publishing_tx, publishing_rx) = channel();
        let (published_tx, published_rx) = channel();
        let issued = Arc::new(AtomicBool::new(false));
        let gate_manager = fixture.manager.clone();
        let gate_issued = issued.clone();
        let authorization = thread::spawn(move || {
            gate_manager
                .with_control_snapshot(|_| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    gate_issued.store(true, Ordering::Release);
                })
                .unwrap();
        });
        entered_rx.recv().unwrap();
        let publisher_manager = fixture.manager.clone();
        let publisher = thread::spawn(move || {
            publishing_tx.send(()).unwrap();
            publisher_manager
                .set_controller_issue(Some("closed".into()))
                .unwrap();
            published_tx.send(issued.load(Ordering::Acquire)).unwrap();
        });
        publishing_rx.recv().unwrap();
        let published_before_release = published_rx.try_recv().is_ok();
        release_tx.send(()).unwrap();
        authorization.join().unwrap();
        publisher.join().unwrap();
        assert!(!published_before_release);
        assert!(published_rx.recv().unwrap());
        let mut authority = fixture.authority(OperationKind::Manual);
        assert!(authority
            .authorize(&mock_attempt(), &mut || panic!(
                "closed gate issued a request"
            ))
            .is_err());
    }
}
