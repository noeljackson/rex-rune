use super::remote_tests::{code, pending, reply};
use super::*;
use crate::app::actions::{self, Remote};
use crate::app::panel::Panel;
use rex_linux::{connection::perform::Outcome, settings::RemoteAccess};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn enabled() -> (Workspace, flume::Receiver<HostCommand>) {
    let (mut ws, rx) = tabbed_workspace("split");
    ws.settings.hosts[0].set_remote_access(RemoteAccess::Workspace);
    (ws, rx)
}

fn batch(rx: &flume::Receiver<HostCommand>) -> (u64, Vec<HostCommand>) {
    let mut sent: Vec<_> = rx
        .try_iter()
        .filter(|command| {
            !matches!(
                command,
                HostCommand::Visibility(_) | HostCommand::PrefetchViews(_)
            )
        })
        .collect();
    assert_eq!(
        sent.len(),
        1,
        "dispatch once, without an early successful reply"
    );
    match sent.pop().unwrap() {
        HostCommand::PerformBatch {
            id,
            epoch,
            deadline,
            commands,
            ..
        } => {
            assert_eq!(epoch, 0);
            assert!(deadline <= Instant::now() + Duration::from_secs(5));
            (id, commands)
        }
        _ => panic!("expected a correlated operation"),
    }
}

#[test]
fn workspace_permission_is_explicit_and_the_exact_view_set_stays_unchanged() {
    let mut names: Vec<_> = actions::SPECS
        .iter()
        .filter(|s| matches!(s.remote(), Remote::Workspace))
        .map(|s| s.name)
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "client.close",
            "client.tab.close",
            "client.tab.close_others",
            "client.tab.move",
            "client.tab.move.backward",
            "client.tab.move.forward",
            "client.tab.move_to_new_window",
            "client.tab.new",
            "client.tab.rename",
            "client.window.close",
            "client.window.fullscreen.toggle",
            "client.window.new",
            "pane.balance",
            "pane.close",
            "pane.close_others",
            "pane.focus.down",
            "pane.focus.left",
            "pane.focus.right",
            "pane.focus.up",
            "pane.focus_next",
            "pane.focus_previous",
            "pane.move",
            "pane.move.down",
            "pane.move.left",
            "pane.move.right",
            "pane.move.up",
            "pane.move_to_tab",
            "pane.rename",
            "pane.resize",
            "pane.split",
            "pane.split.auto",
            "pane.split.down",
            "pane.split.left",
            "pane.split.right",
            "pane.split.up",
            "session.close",
            "session.new",
            "session.next",
            "session.previous",
            "session.rename",
        ]
    );
    for access in [RemoteAccess::Off, RemoteAccess::View] {
        let (mut ws, rx) = enabled();
        ws.settings.hosts[0].set_remote_access(access);
        ws.remote_perform(
            0,
            pending("pane.split.right", Value::Null),
            &mut rune::Tx::default(),
        );
        code(reply(&rx), "refused");
        assert!(ws.remote_operation.is_none());
        assert!(ws.panel.is_none());
    }
    let (mut ws, rx) = enabled();
    for name in [
        "pane.copy",
        "pane.paste",
        "pane.send_key",
        "client.settings",
        "client.host.add",
        "client.config.copy_path",
    ] {
        ws.remote_perform(0, pending(name, Value::Null), &mut rune::Tx::default());
        code(reply(&rx), "not_supported");
    }
}

#[test]
fn workspace_dispatch_waits_for_the_matching_server_result_and_replies_once() {
    let (mut ws, rx) = enabled();
    ws.remote_perform(
        0,
        pending("pane.resize", json!({"direction":"right","amount":5})),
        &mut rune::Tx::default(),
    );
    let (id, commands) = batch(&rx);
    assert!(
        matches!(&commands[..], [HostCommand::Request { method, .. }] if method == "session.resize_pane")
    );
    assert!(ws.notices.is_empty());
    assert!(!ws.poll_remote_workspace(Instant::now(), &mut rune::Tx::default()));
    ws.hosts[0].model.remote_result = Some((id + 1, Outcome::Performed));
    assert!(!ws.poll_remote_workspace(Instant::now(), &mut rune::Tx::default()));
    assert!(rx.is_empty());
    ws.hosts[0].model.remote_result = Some((id, Outcome::Performed));
    assert!(ws.poll_remote_workspace(Instant::now(), &mut rune::Tx::default()));
    assert_eq!(reply(&rx), Outcome::Performed);
    assert_eq!(ws.notices.len(), 1);
    assert!(!ws.poll_remote_workspace(Instant::now(), &mut rune::Tx::default()));
    assert!(rx.is_empty());
}

#[test]
fn view_only_tab_navigation_finishes_before_the_next_cli_action() {
    let (mut ws, rx) = tabbed_workspace("single");
    ws.settings.hosts[0].set_remote_access(RemoteAccess::View);
    ws.remote_perform(
        0,
        pending("client.tab.goto", json!({"index":2})),
        &mut rune::Tx::default(),
    );
    let (id, commands) = batch(&rx);
    let navigation = commands
        .iter()
        .find_map(|command| match command {
            HostCommand::Navigate { id, .. } => Some(*id),
            _ => None,
        })
        .expect("the existing navigation handler is captured, not sent early");
    assert!(ws.navigation.pending.is_some());
    ws.hosts[0]
        .model
        .views
        .get_mut("s")
        .unwrap()
        .active_window_id = Some("split".into());
    ws.hosts[0].model.navigation_result = Some((navigation, true));
    ws.hosts[0].model.remote_result = Some((id, Outcome::Performed));
    ws.observe_navigation();
    ws.poll_remote_workspace(Instant::now(), &mut rune::Tx::default());
    assert_eq!(reply(&rx), Outcome::Performed);
    assert!(ws.navigation.pending.is_none());
    ws.remote_perform(
        0,
        pending("pane.zoom", Value::Null),
        &mut rune::Tx::default(),
    );
    let (_, commands) = batch(&rx);
    assert!(
        matches!(&commands[..], [HostCommand::Request { method, .. }] if method == "session.zoom_block")
    );
    std::fs::remove_file(&ws.settings_path).ok();
}

#[test]
fn close_waits_for_local_confirmation_then_server_acknowledgement() {
    let (mut ws, rx) = enabled();
    ws.remote_perform(
        0,
        pending("pane.close", Value::Null),
        &mut rune::Tx::default(),
    );
    assert!(matches!(
        ws.panel.as_ref().unwrap().kind,
        PanelKind::Confirm { .. }
    ));
    assert!(
        rx.is_empty(),
        "no mutation or successful reply before confirmation"
    );
    ws.submit(&mut rune::Tx::default());
    let (id, commands) = batch(&rx);
    assert!(
        matches!(&commands[..], [HostCommand::Request { method, .. }] if method == "block.close")
    );
    assert!(ws.panel.is_none());
    ws.hosts[0].model.remote_result = Some((
        id,
        Outcome::error("not_found", "The pane ended before the close."),
    ));
    ws.poll_remote_workspace(Instant::now(), &mut rune::Tx::default());
    code(reply(&rx), "not_found");
    assert!(!ws.notices.iter().any(|n| n.text.contains("performed")));
}

#[test]
fn cancellation_expiry_revocation_and_changed_targets_never_send_a_close() {
    for hazard in [
        "cancel", "expiry", "consent", "session", "pane", "epoch", "panel",
    ] {
        let (mut ws, rx) = enabled();
        ws.remote_perform(
            0,
            pending("pane.close", Value::Null),
            &mut rune::Tx::default(),
        );
        match hazard {
            "cancel" => ws.cancel_panel(&mut rune::Tx::default()),
            "expiry" => {
                ws.remote_approval.as_mut().unwrap().pending.received -= Duration::from_secs(6)
            }
            "consent" => ws.settings.hosts[0].set_remote_access(RemoteAccess::View),
            "session" => ws.hosts[0].model.selected = Some("other".into()),
            "pane" => {
                ws.hosts[0]
                    .model
                    .views
                    .get_mut("s")
                    .unwrap()
                    .windows
                    .iter_mut()
                    .find(|w| w.window_id.as_deref() == Some("split"))
                    .unwrap()
                    .focused_block_id = Some("block:c".into())
            }
            "epoch" => ws.hosts[0].model.connection_epoch += 1,
            "panel" => ws.panel = Some(Panel::new(PanelKind::Settings, "Settings", &[])),
            _ => unreachable!(),
        }
        if hazard != "cancel" {
            ws.poll_remote_workspace(Instant::now(), &mut rune::Tx::default());
        }
        let sent: Vec<_> = rx.try_iter().collect();
        assert!(
            sent.iter().all(|c| matches!(
                c,
                HostCommand::PerformReply {
                    outcome: Outcome::Error { .. },
                    ..
                }
            )),
            "{hazard}"
        );
        assert_eq!(
            sent.len(),
            usize::from(hazard != "epoch"),
            "{hazard}: replies stay on their original epoch"
        );
        assert!(ws.remote_approval.is_none());
        if hazard == "panel" {
            assert!(
                matches!(ws.panel.as_ref().unwrap().kind, PanelKind::Settings),
                "an unrelated panel is preserved"
            );
        } else {
            assert!(ws.panel.is_none());
        }
    }
}

#[test]
fn a_late_submit_cannot_bypass_expiry_and_operations_do_not_cross_reconnects() {
    let (mut ws, rx) = enabled();
    ws.remote_perform(
        0,
        pending("pane.close", Value::Null),
        &mut rune::Tx::default(),
    );
    ws.remote_approval.as_mut().unwrap().pending.received -= Duration::from_secs(6);
    ws.submit(&mut rune::Tx::default());
    code(reply(&rx), "timeout");
    assert!(ws.remote_operation.is_none());
    assert!(ws.panel.is_none());
    ws.remote_perform(
        0,
        pending("pane.split.right", Value::Null),
        &mut rune::Tx::default(),
    );
    let (id, _) = batch(&rx);
    ws.hosts[0].model.connection_epoch += 1;
    ws.hosts[0].model.remote_result = Some((id, Outcome::Performed));
    assert!(ws.poll_remote_workspace(Instant::now(), &mut rune::Tx::default()));
    assert!(rx.is_empty());
    assert!(ws.remote_operation.is_none());
}

#[test]
fn toggling_remote_control_never_upgrades_view_consent_and_clears_workspace_on_off() {
    let (mut ws, _) = enabled();
    ws.toggle_remote_control(0);
    assert_eq!(ws.settings.hosts[0].remote_access(), RemoteAccess::Off);
    assert!(!ws.settings.hosts[0].workspace_control);
    ws.toggle_remote_control(0);
    assert_eq!(ws.settings.hosts[0].remote_access(), RemoteAccess::View);
    std::fs::remove_file(&ws.settings_path).ok();
}
