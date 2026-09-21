use crate::config::TargetMonitor;
use crate::display::{
    self, DisplayBackend, DisplayFailure, FailureKind, MonitorInfo, MonitorOutcome, NativeAttempt,
    NativePurpose, OutcomeKind, ScopeHdrState, TargetStatus,
};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessIdentity {
    pub pid: u32,
    pub created_at: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct Activation {
    pub generation: u64,
    pub process: ProcessIdentity,
    pub exe: String,
    pub context_token: String,
    pub target: TargetMonitor,
    pub members: Vec<String>,
    partial: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlState {
    Unowned,
    Owned,
    ManuallyOverridden,
    OutcomeUnknown,
}

#[derive(Debug, Clone)]
pub(crate) struct OperationRecord {
    pub device_path: String,
    pub requested_hdr: bool,
    pub activation_generation: Option<u64>,
    pub context_token: Option<String>,
    pub attempts: Vec<NativeAttempt>,
}

#[derive(Debug, Clone)]
struct ControlRecord {
    device_path: String,
    state: ControlState,
    operations: Vec<OperationRecord>,
    expected_hdr: Option<bool>,
}

pub(crate) trait WriteAuthority {
    /// Implementations publish `mark_issued` within the same short lock used to publish the gate.
    fn authorize(
        &mut self,
        attempt: &NativeAttempt,
        mark_issued: &mut dyn FnMut(),
    ) -> Result<(), DisplayFailure>;
}

struct Selection {
    members: Vec<String>,
    skipped: Vec<MonitorOutcome>,
}

struct ManualWarning {
    scope: TargetMonitor,
    message: String,
}

fn same_inventory(left: &[MonitorInfo], right: &[MonitorInfo]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut unmatched: Vec<_> = right.iter().collect();
    for monitor in left {
        let Some(index) = unmatched.iter().position(|other| {
            monitor.id == other.id
                && monitor.device_path == other.device_path
                && monitor.identity_status == other.identity_status
                && monitor.identity_error == other.identity_error
                && monitor.name == other.name
                && monitor.address() == other.address()
                && monitor.is_hdr_supported == other.is_hdr_supported
                && monitor.is_hdr_enabled == other.is_hdr_enabled
                && monitor.hdr_state_known == other.hdr_state_known
                && monitor.state_error == other.state_error
                && monitor.is_primary == other.is_primary
        }) else {
            return false;
        };
        unmatched.swap_remove(index);
    }
    true
}

fn contains_identity(identities: &[String], identity: &str) -> bool {
    identities
        .iter()
        .any(|other| display::identity_eq(other, identity))
}

pub(crate) fn same_target(left: &TargetMonitor, right: &TargetMonitor) -> bool {
    match (left, right) {
        (TargetMonitor::All, TargetMonitor::All) => true,
        (
            TargetMonitor::Monitor {
                device_path: left, ..
            },
            TargetMonitor::Monitor {
                device_path: right, ..
            },
        ) => display::identity_eq(left, right),
        (
            TargetMonitor::NeedsConfirmation {
                legacy_runtime_id: left,
            },
            TargetMonitor::NeedsConfirmation {
                legacy_runtime_id: right,
            },
        ) => left == right,
        _ => false,
    }
}

fn select_scope(
    monitors: &[MonitorInfo],
    scope: &TargetMonitor,
    enable: bool,
    identify_only: bool,
) -> Result<Selection, DisplayFailure> {
    match scope {
        TargetMonitor::NeedsConfirmation { .. } => Err(DisplayFailure::new(
            FailureKind::NeedsConfirmation,
            "Confirm a durable monitor target before controlling HDR",
        )),
        TargetMonitor::Monitor { device_path, .. } => {
            let monitor = if identify_only {
                display::resolve_identity(monitors, device_path)?
            } else {
                display::resolve_monitor(monitors, device_path)?
            };
            Ok(Selection {
                members: vec![monitor.device_path.clone().expect("resolved identity")],
                skipped: Vec::new(),
            })
        }
        TargetMonitor::All => {
            let mut members = Vec::new();
            let mut considered = Vec::new();
            let mut skipped = Vec::new();
            for monitor in monitors {
                let Some(identity) = &monitor.device_path else {
                    skipped.push(MonitorOutcome::failed(
                        None,
                        enable,
                        DisplayFailure::new(
                            FailureKind::IdentityUnavailable,
                            format!(
                                "{} has no durable monitor identity and was skipped",
                                monitor.name
                            ),
                        ),
                    ));
                    continue;
                };
                if contains_identity(&considered, identity) {
                    continue;
                }
                considered.push(identity.clone());
                let resolved = if identify_only {
                    display::resolve_identity(monitors, identity).and_then(|monitor| {
                        if monitor.hdr_state_known && !monitor.is_hdr_supported {
                            Err(DisplayFailure::new(
                                FailureKind::NotHdrCapable,
                                "HDR is not supported",
                            ))
                        } else {
                            Ok(monitor)
                        }
                    })
                } else {
                    display::resolve_monitor(monitors, identity)
                };
                match resolved {
                    Ok(_) => members.push(identity.clone()),
                    Err(error) if error.kind == FailureKind::NotHdrCapable => {}
                    Err(error) => skipped.push(MonitorOutcome::failed(
                        Some(identity.clone()),
                        enable,
                        error,
                    )),
                }
            }
            if members.is_empty() && skipped.is_empty() {
                return Err(DisplayFailure::new(
                    if monitors.is_empty() {
                        FailureKind::Disconnected
                    } else {
                        FailureKind::NotHdrCapable
                    },
                    "No identifiable HDR-capable monitors are connected",
                ));
            }
            Ok(Selection { members, skipped })
        }
    }
}

pub(crate) struct HdrController<B: DisplayBackend> {
    backend: B,
    activation: Option<Activation>,
    next_generation: u64,
    records: Vec<ControlRecord>,
    inventory: Vec<MonitorInfo>,
    inventory_error: Option<String>,
    inventory_revision: u64,
    warnings: VecDeque<String>,
    manual_warnings: Vec<ManualWarning>,
}

impl<B: DisplayBackend> HdrController<B> {
    pub(crate) fn new(backend: B) -> Self {
        Self {
            backend,
            activation: None,
            next_generation: 0,
            records: Vec::new(),
            inventory: Vec::new(),
            inventory_error: None,
            inventory_revision: 0,
            warnings: VecDeque::new(),
            manual_warnings: Vec::new(),
        }
    }

    pub(crate) fn activation(&self) -> Option<&Activation> {
        self.activation.as_ref()
    }

    pub(crate) fn inventory(&self) -> &[MonitorInfo] {
        &self.inventory
    }

    pub(crate) fn inventory_error(&self) -> Option<&str> {
        self.inventory_error.as_deref()
    }

    pub(crate) fn inventory_revision(&self) -> String {
        self.inventory_revision.to_string()
    }

    pub(crate) fn warning(&self) -> Option<String> {
        let mut warnings: Vec<String> = self.warnings.iter().cloned().collect();
        for warning in &self.manual_warnings {
            if !warnings.contains(&warning.message) {
                warnings.push(warning.message.clone());
            }
        }
        if self.has_uncertainty() {
            warnings.push(
                "HDR may have changed. Automatic writes to uncertain monitors are blocked until an explicitly scoped manual On/Off request is verified".into(),
            );
        }
        if warnings.is_empty() {
            None
        } else {
            Some(warnings.join("\n"))
        }
    }

    pub(crate) fn warn(&mut self, warning: impl Into<String>) {
        let warning = warning.into();
        if !self.warnings.contains(&warning) {
            self.warnings.push_back(warning);
            if self.warnings.len() > 8 {
                self.warnings.pop_front();
            }
        }
    }

    fn record_index(&mut self, identity: &str) -> usize {
        if let Some(index) = self
            .records
            .iter()
            .position(|record| display::identity_eq(&record.device_path, identity))
        {
            return index;
        }
        self.records.push(ControlRecord {
            device_path: identity.to_string(),
            state: ControlState::Unowned,
            operations: Vec::new(),
            expected_hdr: None,
        });
        self.records.len() - 1
    }

    pub(crate) fn refresh_inventory(&mut self) -> Result<Vec<MonitorInfo>, String> {
        match self.backend.inventory() {
            Ok(inventory) => {
                self.observe(&inventory);
                if self.inventory_revision == 0
                    || self.inventory_error.is_some()
                    || !same_inventory(&self.inventory, &inventory)
                {
                    self.inventory_revision += 1;
                }
                self.inventory = inventory.clone();
                self.inventory_error = None;
                Ok(inventory)
            }
            Err(error) => {
                if self.inventory_error.as_ref() != Some(&error) {
                    self.inventory_revision += 1;
                }
                self.inventory_error = Some(error.clone());
                Err(error)
            }
        }
    }

    fn observe(&mut self, inventory: &[MonitorInfo]) {
        let mut drifted = Vec::new();
        for record in &mut self.records {
            if !matches!(record.state, ControlState::Owned | ControlState::Unowned)
                || record.expected_hdr != Some(true)
            {
                continue;
            }
            if let Ok(monitor) = display::resolve_monitor(inventory, &record.device_path) {
                if !monitor.is_hdr_enabled {
                    record.state = ControlState::ManuallyOverridden;
                    record.operations.clear();
                    record.expected_hdr = None;
                    drifted.push(monitor.name.clone());
                }
            }
        }
        for name in drifted {
            self.warn(format!(
                "{name}: an external HDR change was observed; automatic control is suppressed for this activation"
            ));
        }
    }

    pub(crate) fn begin(
        &mut self,
        process: ProcessIdentity,
        exe: String,
        context_token: String,
        target: TargetMonitor,
    ) -> Result<Vec<MonitorOutcome>, DisplayFailure> {
        if self.activation.is_some() {
            return Err(DisplayFailure::new(
                FailureKind::AuthorityDenied,
                "An activation is already in progress",
            ));
        }
        if let Some(error) = &self.inventory_error {
            return Err(DisplayFailure::new(
                FailureKind::EnumerationFailed,
                error.clone(),
            ));
        }
        let selection = select_scope(&self.inventory, &target, true, false)?;
        if selection.members.is_empty() {
            let failure = selection
                .skipped
                .first()
                .expect("empty selection has a reason");
            return Err(DisplayFailure::new(
                failure.failure.unwrap_or(FailureKind::IdentityUnavailable),
                failure
                    .message
                    .clone()
                    .unwrap_or_else(|| "The selected scope is unavailable".into()),
            ));
        }
        for record in &mut self.records {
            if record.state == ControlState::ManuallyOverridden {
                record.state = ControlState::Unowned;
                record.operations.clear();
                record.expected_hdr = None;
            }
        }
        self.next_generation = self.next_generation.saturating_add(1);
        self.activation = Some(Activation {
            generation: self.next_generation,
            process,
            exe,
            context_token,
            target,
            members: selection.members,
            partial: !selection.skipped.is_empty(),
        });
        for skipped in &selection.skipped {
            if let Some(message) = &skipped.message {
                self.warn(message.clone());
            }
        }
        Ok(selection.skipped)
    }

    pub(crate) fn transfer(&mut self, process: ProcessIdentity, exe: String) {
        if let Some(activation) = &mut self.activation {
            if activation.process != process {
                self.next_generation = self.next_generation.saturating_add(1);
                activation.generation = self.next_generation;
                activation.process = process;
                activation.exe = exe;
            }
        }
    }

    pub(crate) fn matches_activation(&self, generation: u64, process: ProcessIdentity) -> bool {
        self.activation
            .as_ref()
            .is_some_and(|active| active.generation == generation && active.process == process)
    }

    fn execute(
        &mut self,
        identity: &str,
        enable: bool,
        purpose: NativePurpose,
        authority: &mut impl WriteAuthority,
        remaining_attempts: &mut usize,
    ) -> (MonitorOutcome, OperationRecord) {
        let mut record = OperationRecord {
            device_path: identity.into(),
            requested_hdr: enable,
            activation_generation: self
                .activation
                .as_ref()
                .map(|activation| activation.generation),
            context_token: self
                .activation
                .as_ref()
                .map(|activation| activation.context_token.clone()),
            attempts: Vec::with_capacity(2),
        };
        let outcome = display::set_hdr(&mut self.backend, identity, enable, purpose, |attempt| {
            if *remaining_attempts == 0 {
                return Err(DisplayFailure::new(
                    FailureKind::AttemptBudgetExhausted,
                    "The bounded HDR cleanup attempt budget has been exhausted",
                ));
            }
            authority.authorize(attempt, &mut || {
                *remaining_attempts -= 1;
                record.attempts.push(attempt.clone());
            })
        });
        if purpose != NativePurpose::Manual && outcome.outcome != OutcomeKind::OutcomeUnknown {
            if let Some(message) = &outcome.message {
                self.warn(message.clone());
            }
        }
        (outcome, record)
    }

    pub(crate) fn enable_activation(
        &mut self,
        authority: &mut impl WriteAuthority,
    ) -> Vec<MonitorOutcome> {
        let Some(activation) = &self.activation else {
            return Vec::new();
        };
        let members = activation.members.clone();
        let mut outcomes = Vec::new();
        let mut budget = members.len().saturating_mul(2);
        for identity in members {
            let index = self.record_index(&identity);
            if self.records[index].state != ControlState::Unowned
                || self.records[index].expected_hdr == Some(true)
            {
                continue;
            }
            let (outcome, operation) = self.execute(
                &identity,
                true,
                NativePurpose::AutomaticEnable,
                authority,
                &mut budget,
            );
            match outcome.outcome {
                OutcomeKind::Changed
                    if outcome.previous_hdr == Some(false)
                        && outcome.previous_hdr_user_enabled == Some(false) =>
                {
                    self.records[index].state = ControlState::Owned;
                    self.records[index].operations = vec![operation];
                    self.records[index].expected_hdr = Some(true);
                }
                OutcomeKind::AlreadyInDesiredState | OutcomeKind::Changed => {
                    self.records[index].expected_hdr = Some(true);
                }
                OutcomeKind::OutcomeUnknown => {
                    self.records[index].state = ControlState::OutcomeUnknown;
                    self.records[index].operations.push(operation);
                }
                _ => {}
            }
            outcomes.push(outcome);
        }
        outcomes
    }

    pub(crate) fn manual_set(
        &mut self,
        scope: &TargetMonitor,
        enable: bool,
        authority: &mut impl WriteAuthority,
    ) -> Vec<MonitorOutcome> {
        let outcomes = self.manual_set_inner(scope, enable, authority);
        self.record_manual_outcomes(scope, &outcomes);
        outcomes
    }

    fn record_manual_outcomes(&mut self, scope: &TargetMonitor, outcomes: &[MonitorOutcome]) {
        for outcome in outcomes {
            let outcome_scope = match &outcome.device_path {
                Some(identity) => TargetMonitor::Monitor {
                    device_path: identity.clone(),
                    display_name: outcome.display_name.clone().unwrap_or_else(|| identity.clone()),
                },
                None => scope.clone(),
            };
            if outcome.is_verified() {
                self.manual_warnings
                    .retain(|warning| !same_target(&warning.scope, &outcome_scope));
            } else if outcome.outcome == OutcomeKind::Failed {
                let message = format!(
                    "The last manual HDR request failed: {}",
                    outcome.message.as_deref().unwrap_or("The requested HDR state was not verified"),
                );
                let message = match &outcome_scope {
                    TargetMonitor::Monitor { display_name, .. } => format!("{display_name}: {message}"),
                    _ => message,
                };
                if let Some(warning) = self.manual_warnings.iter_mut()
                    .find(|warning| same_target(&warning.scope, &outcome_scope))
                {
                    warning.message = message;
                } else {
                    self.manual_warnings.push(ManualWarning { scope: outcome_scope, message });
                }
            }
        }
        // A success for one endpoint must not dismiss a failed All request or another display.
        if !outcomes.is_empty() && outcomes.iter().all(MonitorOutcome::is_verified) {
            self.manual_warnings.retain(|warning| !same_target(&warning.scope, scope));
        }
    }

    fn manual_set_inner(
        &mut self,
        scope: &TargetMonitor,
        enable: bool,
        authority: &mut impl WriteAuthority,
    ) -> Vec<MonitorOutcome> {
        let monitors = match self.refresh_inventory() {
            Ok(monitors) => monitors,
            Err(error) => {
                return vec![MonitorOutcome::failed(
                    None,
                    enable,
                    DisplayFailure::new(FailureKind::EnumerationFailed, error),
                )];
            }
        };
        let selection = match select_scope(&monitors, scope, enable, true) {
            Ok(selection) => selection,
            Err(error) => {
                let identity = match scope {
                    TargetMonitor::Monitor { device_path, .. } => Some(device_path.clone()),
                    _ => None,
                };
                return vec![MonitorOutcome::failed(identity, enable, error)];
            }
        };
        // Relinquish ownership for the complete resolved scope before the first native attempt,
        // including endpoints that will fail or already have the requested state.
        for identity in &selection.members {
            let index = self.record_index(identity);
            if self.records[index].state != ControlState::OutcomeUnknown {
                self.records[index].state = ControlState::ManuallyOverridden;
                self.records[index].operations.clear();
                self.records[index].expected_hdr = None;
            }
        }
        let mut outcomes = selection.skipped;
        let mut budget = selection.members.len().saturating_mul(2);
        for identity in selection.members {
            let index = self.record_index(&identity);
            let (outcome, operation) = self.execute(
                &identity,
                enable,
                NativePurpose::Manual,
                authority,
                &mut budget,
            );
            if outcome.is_verified() {
                self.records[index].state = ControlState::ManuallyOverridden;
                self.records[index].operations.clear();
                self.records[index].expected_hdr = None;
            } else if outcome.outcome == OutcomeKind::OutcomeUnknown {
                self.records[index].state = ControlState::OutcomeUnknown;
                self.records[index].operations.push(operation);
            }
            outcomes.push(outcome);
        }
        outcomes
    }

    pub(crate) fn end(
        &mut self,
        authority: &mut impl WriteAuthority,
        max_attempts: usize,
    ) -> Vec<MonitorOutcome> {
        let mut remaining = max_attempts;
        let owned: Vec<String> = self
            .records
            .iter()
            .filter(|record| record.state == ControlState::Owned)
            .map(|record| record.device_path.clone())
            .collect();
        let mut outcomes = Vec::new();
        for identity in owned {
            let index = self.record_index(&identity);
            let (outcome, operation) = self.execute(
                &identity,
                false,
                NativePurpose::Cleanup,
                authority,
                &mut remaining,
            );
            if outcome.outcome == OutcomeKind::OutcomeUnknown {
                self.records[index].state = ControlState::OutcomeUnknown;
                self.records[index].operations.push(operation);
            } else {
                self.records[index].state = ControlState::Unowned;
                self.records[index].operations.clear();
                self.records[index].expected_hdr = None;
                if !outcome.is_verified() {
                    self.warn(format!(
                        "{identity}: HDR restoration was skipped and will not be replayed on reconnect"
                    ));
                }
            }
            outcomes.push(outcome);
        }
        self.activation = None;
        for record in &mut self.records {
            if record.state == ControlState::ManuallyOverridden {
                record.state = ControlState::Unowned;
                record.operations.clear();
                record.expected_hdr = None;
            }
        }
        self.records
            .retain(|record| record.state != ControlState::Unowned);
        outcomes
    }

    pub(crate) fn has_ownership(&self) -> bool {
        self.records
            .iter()
            .any(|record| record.state == ControlState::Owned)
    }

    pub(crate) fn has_uncertainty(&self) -> bool {
        self.records
            .iter()
            .any(|record| record.state == ControlState::OutcomeUnknown)
    }

    pub(crate) fn uncertain_targets(&self) -> Vec<String> {
        self.records
            .iter()
            .filter(|record| record.state == ControlState::OutcomeUnknown)
            .map(|record| record.device_path.clone())
            .collect()
    }

    pub(crate) fn target_status(&self, target: &TargetMonitor) -> TargetStatus {
        if self.inventory_error.is_some() {
            return TargetStatus::EnumerationFailed;
        }
        match select_scope(&self.inventory, target, true, false) {
            Err(error) => display::status_for_failure(error.kind),
            Ok(selection) => {
                if self.records.iter().any(|record| {
                    record.state == ControlState::OutcomeUnknown
                        && contains_identity(&selection.members, &record.device_path)
                }) {
                    TargetStatus::OutcomeUnknown
                } else if let Some(failed) = selection.skipped.first() {
                    display::status_for_failure(
                        failed.failure.unwrap_or(FailureKind::IdentityUnavailable),
                    )
                } else {
                    TargetStatus::Ready
                }
            }
        }
    }

    pub(crate) fn scope_hdr_state(&self, target: &TargetMonitor) -> ScopeHdrState {
        if self.inventory_error.is_some() {
            return ScopeHdrState::Unknown;
        }
        let members = match &self.activation {
            Some(activation) if !activation.partial => activation.members.clone(),
            Some(_) => return ScopeHdrState::Unknown,
            None => match select_scope(&self.inventory, target, true, false) {
                Ok(selection) if selection.skipped.is_empty() => selection.members,
                _ => return ScopeHdrState::Unknown,
            },
        };
        if members.is_empty() {
            return ScopeHdrState::Unknown;
        }
        let mut hdr_count = 0;
        for identity in &members {
            if self.records.iter().any(|record| {
                display::identity_eq(identity, &record.device_path)
                    && record.state == ControlState::OutcomeUnknown
            }) {
                return ScopeHdrState::Unknown;
            }
            match display::resolve_monitor(&self.inventory, identity) {
                Ok(monitor) => hdr_count += usize::from(monitor.is_hdr_enabled),
                Err(_) => return ScopeHdrState::Unknown,
            }
        }
        match hdr_count {
            0 => ScopeHdrState::Sdr,
            count if count == members.len() => ScopeHdrState::Hdr,
            _ => ScopeHdrState::Mixed,
        }
    }

    pub(crate) fn scope_is_active(&self, target: &TargetMonitor) -> bool {
        self.scope_hdr_state(target) == ScopeHdrState::Hdr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::tests::{monitor, MockDisplay};
    use crate::display::NativeError;
    use std::cell::Cell;
    use std::rc::Rc;

    struct Authority {
        allowed: Rc<Cell<bool>>,
        issued: usize,
    }

    impl Default for Authority {
        fn default() -> Self {
            Self {
                allowed: Rc::new(Cell::new(true)),
                issued: 0,
            }
        }
    }

    impl WriteAuthority for Authority {
        fn authorize(
            &mut self,
            _: &NativeAttempt,
            issue: &mut dyn FnMut(),
        ) -> Result<(), DisplayFailure> {
            if !self.allowed.get() {
                return Err(DisplayFailure::new(
                    FailureKind::AuthorityDenied,
                    "gate closed",
                ));
            }
            issue();
            self.issued += 1;
            Ok(())
        }
    }

    fn process(pid: u32) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            created_at: u64::from(pid) * 100,
        }
    }
    fn chosen() -> TargetMonitor {
        TargetMonitor::Monitor {
            device_path: "chosen".into(),
            display_name: "Chosen".into(),
        }
    }

    fn active(monitors: Vec<MonitorInfo>, target: TargetMonitor) -> HdrController<MockDisplay> {
        let mut engine = HdrController::new(MockDisplay::new(monitors));
        engine.refresh_inventory().unwrap();
        engine
            .begin(process(1), "a.exe".into(), "history".into(), target)
            .unwrap();
        engine
    }

    fn state(engine: &HdrController<MockDisplay>, path: &str) -> ControlState {
        engine
            .records
            .iter()
            .find(|record| record.device_path == path)
            .map(|record| record.state)
            .unwrap_or(ControlState::Unowned)
    }

    #[test]
    fn inventory_revision_tracks_metadata_and_topology_without_aggregate_hdr_changes() {
        let mut engine = HdrController::new(MockDisplay::new(vec![
            monitor("chosen", 1, false), monitor("other", 2, false),
        ]));
        engine.refresh_inventory().unwrap();
        assert_eq!(engine.inventory_revision(), "1");
        engine.backend.monitors.reverse();
        engine.refresh_inventory().unwrap();
        assert_eq!(engine.inventory_revision(), "1", "enumeration order is not a change");
        engine.backend.monitors.reverse();

        let changes: [fn(&mut Vec<MonitorInfo>); 9] = [
            |monitors| monitors.push(monitor("new", 3, false)),
            |monitors| monitors[1].name = "Renamed display".into(),
            |monitors| monitors[1].is_hdr_supported = false,
            |monitors| monitors[1].is_primary = true,
            |monitors| monitors[1].target_id = 20,
            |monitors| monitors[1].hdr_state_known = false,
            |monitors| monitors[1].state_error = Some("Query failed".into()),
            |monitors| monitors[1].identity_status = TargetStatus::Ambiguous,
            |monitors| { monitors.pop(); },
        ];
        for (index, change) in changes.into_iter().enumerate() {
            change(&mut engine.backend.monitors);
            engine.refresh_inventory().unwrap();
            let revision = (index + 2).to_string();
            assert_eq!(engine.inventory_revision(), revision);
            assert_eq!(engine.scope_hdr_state(&chosen()), ScopeHdrState::Sdr);
            assert!(engine.inventory().iter().all(|monitor| !monitor.is_hdr_enabled));
            engine.refresh_inventory().unwrap();
            assert_eq!(engine.inventory_revision(), revision, "identical polls stay quiet");
        }
        assert!(engine.backend.writes.is_empty(), "inventory probes never write HDR");
    }

    #[test]
    fn inventory_failure_and_recovery_have_revisions_even_for_empty_inventory() {
        let mut engine = HdrController::new(MockDisplay::new(Vec::new()));
        engine.refresh_inventory().unwrap();
        assert_eq!(engine.inventory_revision(), "1");
        engine.backend.enumeration_error = Some("Read failed".into());
        assert!(engine.refresh_inventory().is_err());
        assert_eq!(engine.inventory_revision(), "2");
        assert!(engine.refresh_inventory().is_err());
        assert_eq!(engine.inventory_revision(), "2");
        engine.backend.enumeration_error = None;
        engine.refresh_inventory().unwrap();
        assert_eq!(engine.inventory_revision(), "3");
        engine.refresh_inventory().unwrap();
        assert_eq!(engine.inventory_revision(), "3");
        assert!(engine.inventory_error().is_none());
    }

    #[test]
    fn inventory_comparison_preserves_duplicate_endpoint_multiplicity() {
        let first = monitor("first", 1, false);
        let second = monitor("second", 2, false);
        assert!(!same_inventory(
            &[first.clone(), first.clone()], &[first.clone(), second.clone()],
        ));
        assert!(same_inventory(
            &[first.clone(), second.clone()], &[second, first],
        ));
    }

    #[test]
    fn verified_manual_recovery_retires_only_its_failure_warning_without_repeating_it() {
        let mut engine = HdrController::new(MockDisplay::new(vec![monitor("chosen", 1, false)]));
        let mut denied = Authority::default();
        denied.allowed.set(false);
        let outcomes = engine.manual_set(&chosen(), true, &mut denied);
        assert_eq!(outcomes[0].failure, Some(FailureKind::AuthorityDenied));
        let warning = engine.warning().unwrap();
        assert_eq!(warning.matches("gate closed").count(), 1);
        engine.manual_set(&chosen(), true, &mut denied);
        assert_eq!(engine.warning().as_deref(), Some(warning.as_str()));
        // Verifying the opposite state is also an explicit recovery of this display.
        let recovered = engine.manual_set(&chosen(), false, &mut Authority::default());
        assert_eq!(recovered[0].outcome, OutcomeKind::AlreadyInDesiredState);
        assert!(engine.warning().is_none());
        assert!(engine.backend.writes.is_empty());
    }

    #[test]
    fn manual_success_preserves_other_display_failures_persistent_issues_and_uncertainty() {
        let mut engine = active(
            vec![monitor("chosen", 1, false), monitor("other", 2, false)], chosen(),
        );
        engine.backend.update_state = false;
        engine.enable_activation(&mut Authority::default());
        assert!(engine.has_uncertainty());
        engine.warn("Unresolved controller conflict");
        let other = TargetMonitor::Monitor {
            device_path: "other".into(), display_name: "Other".into(),
        };
        let mut denied = Authority::default();
        denied.allowed.set(false);
        engine.manual_set(&chosen(), true, &mut denied);
        engine.manual_set(&other, true, &mut denied);
        let recovered = engine.manual_set(&other, false, &mut Authority::default());
        assert!(recovered[0].is_verified());
        let warnings = engine.warning().unwrap();
        assert!(warnings.contains("Unresolved controller conflict"));
        assert!(warnings.contains("gate closed"), "the other failed scope remains unresolved");
        assert!(warnings.contains("HDR may have changed"));
        assert!(engine.has_uncertainty());
        engine.manual_set(&chosen(), false, &mut Authority::default());
        assert!(!engine.has_uncertainty());
        assert_eq!(engine.warning().as_deref(), Some("Unresolved controller conflict"));
    }

    #[test]
    fn failed_scope_selection_and_enumeration_retire_only_after_verified_scope_recovery() {
        let mut engine = HdrController::new(MockDisplay::new(Vec::new()));
        engine.manual_set(&chosen(), true, &mut Authority::default());
        assert!(engine.warning().is_some());
        engine.backend.monitors.push(monitor("chosen", 1, false));
        engine.manual_set(&chosen(), false, &mut Authority::default());
        assert!(engine.warning().is_none());

        engine.backend.enumeration_error = Some("Inventory unavailable".into());
        engine.manual_set(&TargetMonitor::All, true, &mut Authority::default());
        assert!(engine.warning().unwrap().contains("Inventory unavailable"));
        engine.backend.enumeration_error = None;
        engine.manual_set(&chosen(), false, &mut Authority::default());
        assert!(engine.warning().is_some(), "a single-display request is not All recovery");
        engine.manual_set(&TargetMonitor::All, false, &mut Authority::default());
        assert!(engine.warning().is_none());
    }

    #[test]
    fn only_verified_sdr_to_hdr_changes_are_owned() {
        let mut engine = active(
            vec![monitor("chosen", 1, false), monitor("already", 2, true)],
            TargetMonitor::All,
        );
        engine.enable_activation(&mut Authority::default());
        assert_eq!(state(&engine, "chosen"), ControlState::Owned);
        assert_eq!(state(&engine, "already"), ControlState::Unowned);
        engine.end(&mut Authority::default(), 8);
        assert_eq!(engine.backend.writes.len(), 2);
        assert!(engine
            .backend
            .writes
            .iter()
            .all(|(address, _, _)| address.target_id == 1));
    }

    #[test]
    fn manual_already_satisfied_relinquishes_ownership_and_keeps_zero_owned_activation() {
        let mut engine = active(vec![monitor("chosen", 1, false)], chosen());
        engine.enable_activation(&mut Authority::default());
        let outcomes = engine.manual_set(&chosen(), true, &mut Authority::default());
        assert_eq!(outcomes[0].outcome, OutcomeKind::AlreadyInDesiredState);
        assert!(!engine.has_ownership());
        assert!(engine.activation().is_some());
        engine.backend.monitors[0].is_hdr_enabled = false;
        engine.enable_activation(&mut Authority::default());
        engine.end(&mut Authority::default(), 8);
        assert_eq!(engine.backend.writes.len(), 1);
    }

    #[test]
    fn failed_manual_action_still_suppresses_automatic_control() {
        let mut engine = active(vec![monitor("chosen", 1, false)], chosen());
        engine.enable_activation(&mut Authority::default());
        let mut denied = Authority::default();
        denied.allowed.set(false);
        let outcomes = engine.manual_set(&chosen(), false, &mut denied);
        assert_eq!(outcomes[0].failure, Some(FailureKind::AuthorityDenied));
        assert_eq!(state(&engine, "chosen"), ControlState::ManuallyOverridden);
        engine.backend.monitors[0].is_hdr_enabled = false;
        engine.enable_activation(&mut Authority::default());
        engine.end(&mut Authority::default(), 8);
        assert_eq!(engine.backend.writes.len(), 1);
    }

    #[test]
    fn manual_scope_transfers_authority_even_when_the_hdr_state_query_fails() {
        let mut engine = active(vec![monitor("chosen", 1, false)], chosen());
        engine.enable_activation(&mut Authority::default());
        engine.backend.monitors[0].hdr_state_known = false;
        engine.backend.monitors[0].state_error = Some("state query failed".into());
        let outcomes = engine.manual_set(&chosen(), false, &mut Authority::default());
        assert_eq!(outcomes[0].failure, Some(FailureKind::StateUnavailable));
        assert_eq!(state(&engine, "chosen"), ControlState::ManuallyOverridden);
        engine.backend.monitors[0].hdr_state_known = true;
        engine.end(&mut Authority::default(), 8);
        assert_eq!(engine.backend.writes.len(), 1);
    }

    #[test]
    fn preexisting_hdr_is_neither_owned_nor_reasserted_after_external_change() {
        let mut engine = active(vec![monitor("chosen", 1, true)], chosen());
        engine.enable_activation(&mut Authority::default());
        engine.backend.monitors[0].is_hdr_enabled = false;
        engine.enable_activation(&mut Authority::default());
        engine.refresh_inventory().unwrap();
        assert_eq!(state(&engine, "chosen"), ControlState::ManuallyOverridden);
        engine.end(&mut Authority::default(), 8);
        assert!(engine.backend.writes.is_empty());
    }

    #[test]
    fn unknown_outcomes_survive_refresh_and_activation_end_without_retroactive_ownership() {
        let mut engine = active(vec![monitor("chosen", 1, false)], chosen());
        engine.backend.update_state = false;
        engine.enable_activation(&mut Authority::default());
        assert_eq!(state(&engine, "chosen"), ControlState::OutcomeUnknown);
        assert_eq!(engine.records[0].operations[0].attempts.len(), 1);
        engine.backend.monitors[0].is_hdr_enabled = true;
        engine.refresh_inventory().unwrap();
        assert_eq!(engine.scope_hdr_state(&chosen()), ScopeHdrState::Unknown);
        assert!(!engine.scope_is_active(&chosen()));
        engine.end(&mut Authority::default(), 8);
        engine
            .begin(process(2), "b.exe".into(), "history".into(), chosen())
            .unwrap();
        engine.enable_activation(&mut Authority::default());
        assert_eq!(engine.backend.writes.len(), 1);
        assert!(engine.has_uncertainty());
        let outcome = engine.manual_set(&chosen(), true, &mut Authority::default());
        assert!(outcome[0].is_verified());
        assert!(!engine.has_uncertainty());
        assert!(!engine.has_ownership());
        assert_eq!(state(&engine, "chosen"), ControlState::ManuallyOverridden);
    }

    #[test]
    fn failed_manual_resolution_does_not_clear_uncertainty() {
        let mut engine = active(vec![monitor("chosen", 1, false)], chosen());
        engine.backend.update_state = false;
        engine.enable_activation(&mut Authority::default());
        let mut denied = Authority::default();
        denied.allowed.set(false);
        engine.manual_set(&chosen(), true, &mut denied);
        assert!(engine.has_uncertainty());
        assert_eq!(engine.records[0].operations.len(), 1);
    }

    #[test]
    fn a_to_b_transfer_preserves_ownership_membership_and_manual_suppression() {
        let mut engine = active(
            vec![monitor("chosen", 1, false), monitor("manual", 2, false)],
            TargetMonitor::All,
        );
        engine.enable_activation(&mut Authority::default());
        let old_generation = engine.activation().unwrap().generation;
        let manual = TargetMonitor::Monitor {
            device_path: "manual".into(),
            display_name: "Manual".into(),
        };
        engine.manual_set(&manual, false, &mut Authority::default());
        engine.backend.monitors.push(monitor("new", 3, false));
        engine.refresh_inventory().unwrap();
        engine.transfer(process(2), "b.exe".into());
        assert!(!engine.matches_activation(old_generation, process(1)));
        engine.enable_activation(&mut Authority::default());
        assert_eq!(engine.backend.writes.len(), 3);
        assert_eq!(engine.activation().unwrap().members.len(), 2);
        assert_eq!(state(&engine, "manual"), ControlState::ManuallyOverridden);
        assert_eq!(state(&engine, "chosen"), ControlState::Owned);
    }

    #[test]
    fn pid_reuse_and_stale_debounce_cannot_match_current_generation() {
        let mut engine = active(vec![monitor("chosen", 1, true)], chosen());
        let old = engine.activation().unwrap().clone();
        let reused = ProcessIdentity {
            pid: 1,
            created_at: 9999,
        };
        engine.transfer(reused, "new.exe".into());
        assert!(!engine.matches_activation(old.generation, old.process));
        assert!(!engine.matches_activation(engine.activation().unwrap().generation, old.process));
        assert!(engine.matches_activation(engine.activation().unwrap().generation, reused));
    }

    #[test]
    fn all_freezes_members_and_reresolves_each_endpoint_after_topology_change() {
        let mut engine = active(
            vec![monitor("chosen", 1, false), monitor("second", 2, false)],
            TargetMonitor::All,
        );
        engine.backend.after_set = Some(Box::new(|monitors| {
            monitors[1].target_id = 88;
            if monitors.len() == 2 {
                monitors.push(monitor("new", 3, false));
            }
        }));
        engine.enable_activation(&mut Authority::default());
        assert_eq!(engine.backend.writes.len(), 2);
        assert_eq!(engine.backend.writes[1].0.target_id, 88);
        engine.refresh_inventory().unwrap();
        engine.enable_activation(&mut Authority::default());
        assert_eq!(engine.backend.writes.len(), 2);
    }

    #[test]
    fn all_partial_results_own_only_successful_endpoints() {
        let mut engine = active(
            vec![monitor("chosen", 1, false), monitor("second", 2, false)],
            TargetMonitor::All,
        );
        engine
            .backend
            .results
            .extend([Ok(()), Err(NativeError(31))]);
        let outcomes = engine.enable_activation(&mut Authority::default());
        assert_eq!(outcomes[0].outcome, OutcomeKind::Changed);
        assert_eq!(outcomes[1].outcome, OutcomeKind::OutcomeUnknown);
        assert_eq!(state(&engine, "chosen"), ControlState::Owned);
        assert_eq!(state(&engine, "second"), ControlState::OutcomeUnknown);
        engine.end(&mut Authority::default(), 8);
        assert_eq!(engine.backend.writes.len(), 3);
        assert_eq!(engine.backend.writes[2].0.target_id, 1);
    }

    #[test]
    fn closing_gate_during_issued_call_retains_its_effect_but_blocks_next_endpoint() {
        let mut authority = Authority::default();
        let allowed = authority.allowed.clone();
        let mut engine = active(
            vec![monitor("chosen", 1, false), monitor("second", 2, false)],
            TargetMonitor::All,
        );
        engine.backend.after_set = Some(Box::new(move |_| allowed.set(false)));
        engine.enable_activation(&mut authority);
        assert_eq!(authority.issued, 1);
        assert_eq!(state(&engine, "chosen"), ControlState::Owned);
        assert_eq!(state(&engine, "second"), ControlState::Unowned);
        let operation = &engine.records[0].operations[0];
        assert_eq!(operation.device_path, "chosen");
        assert_eq!(operation.context_token.as_deref(), Some("history"));
        assert_eq!(operation.activation_generation, Some(1));
        assert!(operation.requested_hdr);
        engine.end(&mut Authority::default(), 8);
        assert_eq!(engine.backend.writes.len(), 2);
    }

    #[test]
    fn controller_conflict_blocks_cleanup_and_abandons_obligations() {
        let mut engine = active(vec![monitor("chosen", 1, false)], chosen());
        engine.enable_activation(&mut Authority::default());
        let mut conflict = Authority::default();
        conflict.allowed.set(false);
        engine.end(&mut conflict, 8);
        assert_eq!(engine.backend.writes.len(), 1);
        assert!(!engine.has_ownership());
        engine.end(&mut Authority::default(), 8);
        assert_eq!(engine.backend.writes.len(), 1);
    }

    #[test]
    fn disconnected_cleanup_is_terminal_not_a_hotplug_request() {
        let mut engine = active(vec![monitor("chosen", 1, false)], chosen());
        engine.enable_activation(&mut Authority::default());
        engine.backend.monitors.clear();
        let outcomes = engine.end(&mut Authority::default(), 8);
        assert_eq!(outcomes[0].failure, Some(FailureKind::Disconnected));
        engine.backend.monitors.push(monitor("chosen", 90, true));
        engine.refresh_inventory().unwrap();
        engine.end(&mut Authority::default(), 8);
        assert_eq!(engine.backend.writes.len(), 1);
        assert!(engine.warning().unwrap().contains("will not be replayed"));
    }

    #[test]
    fn observed_external_drift_relinquishes_ownership_and_suppresses_reassertion() {
        let mut engine = active(vec![monitor("chosen", 1, false)], chosen());
        engine.enable_activation(&mut Authority::default());
        engine.backend.monitors[0].is_hdr_enabled = false;
        engine.refresh_inventory().unwrap();
        assert_eq!(state(&engine, "chosen"), ControlState::ManuallyOverridden);
        engine.enable_activation(&mut Authority::default());
        engine.backend.monitors[0].is_hdr_enabled = true;
        engine.end(&mut Authority::default(), 8);
        assert_eq!(engine.backend.writes.len(), 1);
    }

    #[test]
    fn target_change_does_not_retarget_existing_cleanup() {
        let mut engine = active(
            vec![monitor("chosen", 1, false), monitor("next", 2, false)],
            chosen(),
        );
        engine.enable_activation(&mut Authority::default());
        engine.refresh_inventory().unwrap();
        let next = TargetMonitor::Monitor {
            device_path: "next".into(),
            display_name: "Next".into(),
        };
        assert_eq!(engine.target_status(&next), TargetStatus::Ready);
        assert!(engine.scope_is_active(&next));
        let missing = TargetMonitor::Monitor {
            device_path: "missing".into(),
            display_name: "Absent".into(),
        };
        assert_eq!(engine.target_status(&missing), TargetStatus::Disconnected);
        assert!(engine.scope_is_active(&missing));
        engine.end(&mut Authority::default(), 8);
        engine.refresh_inventory().unwrap();
        assert!(!engine.scope_is_active(&next));
        assert!(!engine.scope_is_active(&missing));
        assert!(engine
            .backend
            .writes
            .iter()
            .all(|(address, _, _)| address.target_id == 1));
    }

    #[test]
    fn shutdown_cleanup_has_a_strict_native_attempt_budget() {
        let mut engine = active(
            vec![monitor("chosen", 1, false), monitor("second", 2, false)],
            TargetMonitor::All,
        );
        engine.enable_activation(&mut Authority::default());
        let outcomes = engine.end(&mut Authority::default(), 1);
        assert_eq!(engine.backend.writes.len(), 3);
        assert_eq!(
            outcomes[1].failure,
            Some(FailureKind::AttemptBudgetExhausted)
        );
        assert!(!engine.has_ownership());
        assert!(engine.activation().is_none());
    }

    #[test]
    fn enumeration_failure_keeps_stale_inventory_but_never_reuses_its_addresses() {
        let mut engine = active(vec![monitor("chosen", 1, false)], chosen());
        engine.backend.enumeration_error = Some("mock failure".into());
        assert!(engine.refresh_inventory().is_err());
        assert_eq!(engine.inventory().len(), 1);
        let outcomes = engine.enable_activation(&mut Authority::default());
        assert_eq!(outcomes[0].failure, Some(FailureKind::EnumerationFailed));
        assert!(engine.backend.writes.is_empty());
        assert!(!engine.scope_is_active(&chosen()));
    }

    #[test]
    fn partial_all_never_claims_the_complete_scope_is_hdr_active() {
        let mut unidentifiable = monitor("unknown", 2, true);
        unidentifiable.id.clear();
        unidentifiable.device_path = None;
        unidentifiable.identity_status = TargetStatus::IdentityUnavailable;
        let mut engine = active(
            vec![monitor("chosen", 1, false), unidentifiable],
            TargetMonitor::All,
        );
        engine.enable_activation(&mut Authority::default());
        engine.refresh_inventory().unwrap();
        assert_eq!(engine.backend.writes.len(), 1);
        assert!(!engine.scope_is_active(&TargetMonitor::All));
        assert_eq!(
            engine.target_status(&TargetMonitor::All),
            TargetStatus::IdentityUnavailable
        );
    }

    #[test]
    fn cached_name_and_path_case_do_not_change_the_automatic_scope() {
        let renamed = TargetMonitor::Monitor {
            device_path: "CHOSEN".into(),
            display_name: "New label".into(),
        };
        assert!(same_target(&chosen(), &renamed));
        assert!(!same_target(&chosen(), &TargetMonitor::All));
    }

    #[test]
    fn idle_all_distinguishes_mixed_hdr_and_sdr_instead_of_reporting_sdr() {
        let mut engine = HdrController::new(MockDisplay::new(vec![
            monitor("chosen", 1, true),
            monitor("second", 2, false),
        ]));
        engine.refresh_inventory().unwrap();
        assert_eq!(
            engine.scope_hdr_state(&TargetMonitor::All),
            ScopeHdrState::Mixed
        );
        assert!(!engine.scope_is_active(&TargetMonitor::All));
        engine
            .begin(
                process(1),
                "a.exe".into(),
                "history".into(),
                TargetMonitor::All,
            )
            .unwrap();
        assert_eq!(engine.scope_hdr_state(&chosen()), ScopeHdrState::Mixed);
        engine.backend.monitors[0].is_hdr_enabled = false;
        engine.refresh_inventory().unwrap();
        assert_eq!(
            engine.scope_hdr_state(&TargetMonitor::All),
            ScopeHdrState::Sdr
        );
        for monitor in &mut engine.backend.monitors {
            monitor.is_hdr_enabled = true;
        }
        engine.refresh_inventory().unwrap();
        assert_eq!(
            engine.scope_hdr_state(&TargetMonitor::All),
            ScopeHdrState::Hdr
        );
        assert!(engine.scope_is_active(&TargetMonitor::All));
    }

    #[test]
    fn a_single_target_ignores_unrelated_hdr_and_unknown_monitors() {
        let mut unknown = monitor("unknown", 3, false);
        unknown.hdr_state_known = false;
        let mut engine = HdrController::new(MockDisplay::new(vec![
            monitor("chosen", 1, false),
            monitor("other", 2, true),
            unknown,
        ]));
        engine.refresh_inventory().unwrap();
        assert_eq!(engine.scope_hdr_state(&chosen()), ScopeHdrState::Sdr);
        assert_eq!(
            engine.scope_hdr_state(&TargetMonitor::All),
            ScopeHdrState::Unknown
        );
    }

    #[test]
    fn active_all_state_uses_frozen_members_not_new_displays_or_the_saved_next_target() {
        let mut engine = active(vec![monitor("chosen", 1, true)], TargetMonitor::All);
        let mut unknown = monitor("unknown", 3, false);
        unknown.hdr_state_known = false;
        engine
            .backend
            .monitors
            .extend([monitor("new", 2, false), unknown]);
        engine.refresh_inventory().unwrap();
        assert_eq!(
            engine.scope_hdr_state(&TargetMonitor::All),
            ScopeHdrState::Hdr
        );
        let next = TargetMonitor::Monitor {
            device_path: "new".into(),
            display_name: "Next".into(),
        };
        assert_eq!(engine.scope_hdr_state(&next), ScopeHdrState::Hdr);
        engine.backend.monitors[0].is_hdr_enabled = false;
        engine.backend.monitors[1].is_hdr_enabled = true;
        engine.refresh_inventory().unwrap();
        assert_eq!(engine.scope_hdr_state(&next), ScopeHdrState::Sdr);
        engine.backend.monitors.remove(0);
        engine.refresh_inventory().unwrap();
        assert_eq!(
            engine.scope_hdr_state(&TargetMonitor::All),
            ScopeHdrState::Unknown
        );
    }

    #[test]
    fn empty_missing_and_unobserved_scopes_are_unknown_not_sdr() {
        let mut engine = HdrController::new(MockDisplay::new(Vec::new()));
        engine.refresh_inventory().unwrap();
        assert_eq!(
            engine.scope_hdr_state(&TargetMonitor::All),
            ScopeHdrState::Unknown
        );
        assert_eq!(engine.scope_hdr_state(&chosen()), ScopeHdrState::Unknown);
        engine.backend.monitors.push(monitor("chosen", 1, false));
        engine.backend.monitors[0].hdr_state_known = false;
        engine.refresh_inventory().unwrap();
        assert_eq!(engine.scope_hdr_state(&chosen()), ScopeHdrState::Unknown);
        engine.backend.monitors[0].hdr_state_known = true;
        engine.backend.enumeration_error = Some("enumeration failed".into());
        assert!(engine.refresh_inventory().is_err());
        assert_eq!(engine.scope_hdr_state(&chosen()), ScopeHdrState::Unknown);
    }
}
