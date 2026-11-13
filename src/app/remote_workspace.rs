//! Capture ordinary UI dispatch and wait for server acknowledgement. Approval is local and
//! bounded by the original request deadline; neither approval nor commands survive reconnect.
use super::{
    actions,
    workspace::{HostId, Workspace},
};
use rex_linux::{
    connection::{
        HostCommand,
        perform::{MAX_WAIT, Outcome, Pending},
    },
    settings::RemoteAccess,
};
use rune::Tx;
use std::time::Instant;

pub struct Operation {
    pub id: u64,
    pub host: HostId,
    pub pending: Pending,
}

pub struct Approval {
    pub host: HostId,
    pub pending: Pending,
    pub session: Option<String>,
    pub block: Option<String>,
}

impl Workspace {
    pub(super) fn run_remote_commands(&mut self, host: usize, pending: Pending, tx: &mut Tx) {
        let approval = Approval {
            host: self.hosts[host].id,
            session: self.model().selected.clone(),
            block: self.focused_block(),
            pending,
        };
        self.remote_dispatch = Some(Vec::new());
        let ran = actions::run(
            self,
            &approval.pending.request.action,
            &approval.pending.request.args,
            tx,
        );
        let commands = self.remote_dispatch.take().unwrap();
        if !ran {
            self.remote_reply(
                host,
                approval.pending,
                Outcome::error(
                    "bad_request",
                    "The action could not run; nothing was dispatched.",
                ),
            );
        } else if self.panel.is_some() {
            // A handler must not send a mutation before asking for approval.
            if !commands.is_empty() {
                self.close_panel(tx);
                self.remote_reply(
                    host,
                    approval.pending,
                    Outcome::error("internal", "Invalid approval flow; nothing was dispatched."),
                );
                return;
            }
            self.keep_remote_approval(approval);
        } else {
            self.dispatch_remote_commands(host, approval.pending, commands);
        }
    }

    fn keep_remote_approval(&mut self, approval: Approval) {
        if let Some(panel) = &mut self.panel {
            panel.remote_call = Some(approval.pending.request.call_id.clone());
        }
        self.inform(format!("{} requested {}. The request expires five seconds after arrival; confirm or cancel in the dialog.",
            approval.pending.request.requester, approval.pending.request.action));
        self.remote_approval = Some(approval);
    }

    fn dispatch_remote_commands(
        &mut self,
        host: usize,
        pending: Pending,
        commands: Vec<(HostId, HostCommand)>,
    ) {
        if commands.iter().any(|(id, _)| *id != self.hosts[host].id) {
            self.remote_reply(
                host,
                pending,
                Outcome::error(
                    "refused",
                    "An action cannot dispatch commands to another host; nothing was dispatched.",
                ),
            );
            return;
        }
        if commands.is_empty() {
            self.remote_reply(
                host,
                pending,
                Outcome::error(
                    "not_found",
                    "No eligible target for this operation; nothing was dispatched.",
                ),
            );
            return;
        }
        let access = self.settings.hosts[host].remote_access();
        let permitted =
            actions::find(&pending.request.action).is_some_and(|spec| match spec.remote() {
                actions::Remote::View { .. } => access != RemoteAccess::Off,
                actions::Remote::Workspace => access == RemoteAccess::Workspace,
                actions::Remote::Never(_) => false,
            });
        if pending.received.elapsed() >= MAX_WAIT
            || !self.hosts[host].model.connected
            || self.hosts[host].model.connection_epoch != pending.epoch
            || !permitted
        {
            self.remote_reply(host, pending, Outcome::error("refused", "The request expired, disconnected or lost remote consent; nothing was dispatched."));
            return;
        }
        self.next_remote_operation += 1;
        let id = self.next_remote_operation;
        self.send(
            host,
            HostCommand::PerformBatch {
                consent: self.hosts[host].consent_generation.clone(),
                generation: self.hosts[host]
                    .consent_generation
                    .load(std::sync::atomic::Ordering::Acquire),
                id,
                epoch: pending.epoch,
                deadline: pending.received + MAX_WAIT,
                commands: commands.into_iter().map(|(_, command)| command).collect(),
            },
        );
        self.remote_operation = Some(Operation {
            id,
            host: self.hosts[host].id,
            pending,
        });
    }

    fn approval_is_current(&self, approval: &Approval) -> bool {
        let Some(host) = self.host_index(approval.host) else {
            return false;
        };
        self.active == host
            && self.settings.hosts[host].remote_access() == RemoteAccess::Workspace
            && self.hosts[host].model.connected
            && self.hosts[host].model.connection_epoch == approval.pending.epoch
            && approval.pending.received.elapsed() < MAX_WAIT
            && self.model().selected == approval.session
            && self.focused_block() == approval.block
            && self.panel.as_ref().and_then(|p| p.remote_call.as_deref())
                == Some(approval.pending.request.call_id.as_str())
    }

    fn approval_failure(approval: &Approval) -> Outcome {
        if approval.pending.received.elapsed() >= MAX_WAIT {
            Outcome::error("timeout", "Approval expired; nothing was dispatched.")
        } else {
            Outcome::error(
                "refused",
                "Approval lost its target, connection or workspace consent; nothing was dispatched.",
            )
        }
    }

    pub(super) fn submit_remote_approval(&mut self, tx: &mut Tx) {
        let approval = self.remote_approval.take().unwrap();
        let Some(host) = self.host_index(approval.host) else {
            return;
        };
        if !self.approval_is_current(&approval) {
            let outcome = Self::approval_failure(&approval);
            self.dismiss_remote_panel(&approval, tx);
            self.remote_reply(host, approval.pending, outcome);
            return;
        }
        self.remote_dispatch = Some(Vec::new());
        self.submit_inner(tx);
        let commands = self.remote_dispatch.take().unwrap();
        if self.panel.is_some() && commands.is_empty() {
            // A validation error or a second explicit choice keeps the original deadline.
            self.keep_remote_approval(approval);
        } else if self.panel.is_some() {
            self.dismiss_remote_panel(&approval, tx);
            self.remote_reply(
                host,
                approval.pending,
                Outcome::error("internal", "Incomplete approval; nothing was dispatched."),
            );
        } else {
            self.dispatch_remote_commands(host, approval.pending, commands);
        }
    }

    fn dismiss_remote_panel(&mut self, approval: &Approval, tx: &mut Tx) {
        if self.panel.as_ref().and_then(|p| p.remote_call.as_deref())
            == Some(approval.pending.request.call_id.as_str())
        {
            self.close_panel(tx);
        }
    }

    pub(super) fn cancel_remote_approval(&mut self) {
        if let Some(approval) = self.remote_approval.take()
            && let Some(host) = self.host_index(approval.host)
        {
            self.remote_reply(
                host,
                approval.pending,
                Outcome::error(
                    "cancelled",
                    "The person cancelled the request; nothing was dispatched.",
                ),
            );
        }
    }

    pub(super) fn poll_remote_workspace(&mut self, now: Instant, tx: &mut Tx) -> bool {
        let mut changed = false;
        if let Some(approval) = self.remote_approval.take() {
            if self.approval_is_current(&approval) {
                self.remote_approval = Some(approval);
            } else {
                let outcome = Self::approval_failure(&approval);
                self.dismiss_remote_panel(&approval, tx);
                if let Some(host) = self.host_index(approval.host)
                    && self.hosts[host].model.connected
                    && self.hosts[host].model.connection_epoch == approval.pending.epoch
                {
                    self.remote_reply(host, approval.pending, outcome);
                }
                changed = true;
            }
        }
        if let Some(operation) = self.remote_operation.take() {
            let Some(host) = self.host_index(operation.host) else {
                return true;
            };
            let model = &self.hosts[host].model;
            if !model.connected || model.connection_epoch != operation.pending.epoch {
                return true;
            }
            let outcome = model
                .remote_result
                .as_ref()
                .filter(|(id, _)| *id == operation.id)
                .map(|(_, result)| result.clone());
            if let Some(outcome) = outcome {
                self.remote_reply(host, operation.pending, outcome);
                changed = true;
            } else if now >= operation.pending.received + MAX_WAIT {
                self.remote_reply(host, operation.pending, Outcome::error("timeout", "The operation did not finish in time; its result may be uncertain. Nothing will be retried."));
                changed = true;
            } else {
                self.remote_operation = Some(operation);
            }
        }
        changed
    }
}
