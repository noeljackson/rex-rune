use super::detached::TabKey;
use super::*;
use rex_linux::connection::{Command, HostModel};
use rune::{Key, KeyEvent, Mods, Runtime};

fn pair() -> (
    Workspace,
    Workspace,
    Arc<Mutex<Desktop>>,
    flume::Receiver<HostCommand>,
) {
    let (mut source, shared) = super::tests::fixture();
    let (mut child, _) = super::tests::fixture();
    let view = serde_json::from_value(json!({"session_id":"s","revision":1,"active_window_id":"a","windows":[{"window_id":"a","label":"source","focused_block_id":"block:a","layers":[{"kind":"tiled","layer_id":"layer:a","bounds":[0,0,1,1],"layout":{"block_id":"block:a"},"blocks":[{"block_id":"block:a","creator_name":"com.superlogical.terminal"}]}]}]})).unwrap();
    let model = HostModel {
        connected: true,
        visible: true,
        selected: Some("s".into()),
        sessions: vec![rexrs::api::session::Session {
            session_id: "s".into(),
            label: "fixture".into(),
        }],
        views: HashMap::from([("s".into(), view)]),
        ..Default::default()
    };
    // The native fixture starts workers for its nonexistent socket. Give these
    // headless views private channels/models so a worker finishing shutdown
    // cannot overwrite the injected snapshot during sync/clipboard checks.
    for workspace in [&mut source, &mut child] {
        let (commands, _) = flume::unbounded();
        let (_, wake) = flume::unbounded();
        workspace.hosts[0].connection = rex_linux::connection::HostConnection {
            model: Arc::new(Mutex::new(model.clone())),
            commands,
            wake,
        };
        workspace.hosts[0].model = model.clone();
    }
    let mut state = WindowState::new(shared.clone(), 2);
    state.target = Some(("s".into(), "a".into()));
    state.dedicated = Some(TabKey {
        endpoint: source.settings.hosts[0].endpoint.clone(),
        session: "s".into(),
        tab: "a".into(),
    });
    child.desktop = Some(state);
    shared
        .lock()
        .unwrap()
        .windows
        .insert(1, source.handle.clone());
    shared
        .lock()
        .unwrap()
        .windows
        .insert(2, child.handle.clone());
    let (sent, received) = flume::unbounded();
    source.hosts[0].connection.commands = sent;
    (source, child, shared, received)
}

fn key(ws: &Workspace) -> TabKey {
    ws.detach_key(&ws.active_tab().unwrap()).unwrap()
}
fn frame(ws: Workspace) -> Runtime<Workspace> {
    super::super::tests::runtime_at(ws, (1440., 900.))
}
fn rect(rt: &Runtime<Workspace>, key: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing {key}"))
        .rect
}
fn click(rt: &mut Runtime<Workspace>, key: &str) {
    let [x, y, w, h] = rect(rt, key);
    rt.pointer_down(x + w / 2., y + h / 2.);
    rt.pointer_up(x + w / 2., y + h / 2.);
    for _ in 0..4 {
        rt.frame(1. / 60.);
    }
}
fn no_program_mutation(rx: &flume::Receiver<HostCommand>) {
    for command in rx.try_iter() {
        match command {
            HostCommand::Visibility(_)
            | HostCommand::Claim { .. }
            | HostCommand::PrefetchViews(_) => {}
            _ => panic!("detachment/restoration must not mutate programs"),
        }
    }
}

#[test]
fn the_last_tab_becomes_a_placeholder_only_after_a_real_destination_is_registered() {
    let (mut source, child, shared, rx) = pair();
    let before = source.model().view().unwrap().clone();
    let key = key(&source);
    assert!(!source.publish_detached(key.clone(), 99, &mut Tx::default()));
    assert!(!source.detached_here());
    assert!(source.publish_detached(key, 2, &mut Tx::default()));
    assert!(source.detached_here());
    assert!(!child.detached_here());
    assert!(source.focused_block().is_none());
    assert!(source.visible_terminal_keys().is_empty());
    assert!(!source.tab_seen(0, "s", "a"));
    source.claim_visible();
    assert!(source.claimed.is_empty());
    assert_eq!(source.model().view().unwrap(), &before);
    assert_eq!(shared.lock().unwrap().detached.len(), 1);
    let rt = frame(source);
    assert!(rect(&rt, "detached-go")[2] > 0.);
    assert!(rect(&rt, "detached-restore")[2] > 0.);
    assert!(
        !rt.elements()
            .iter()
            .any(|e| !e.exiting && e.key.as_deref() == Some("pane:block:a"))
    );
    no_program_mutation(&rx);
}

#[test]
fn restore_keeps_the_original_tab_blocks_and_claims_its_size_again() {
    let (mut source, _, shared, rx) = pair();
    let before = source.model().view().unwrap().clone();
    source
        .claimed
        .insert((source.hosts[0].id, "block:a".into()));
    assert!(source.publish_detached(key(&source), 2, &mut Tx::default()));
    rx.try_iter().for_each(drop);
    let mut rt = frame(source);
    click(&mut rt, "detached-restore");
    assert_eq!(
        shared.lock().unwrap().detached.get(&key(&rt.state)),
        Some(&1)
    );
    assert!(!rt.state.detached_here());
    assert_eq!(rt.state.focused_block().as_deref(), Some("block:a"));
    assert_eq!(rt.state.model().view().unwrap(), &before);
    assert!(
        rx.try_iter()
            .any(|c| matches!(c,HostCommand::Claim { blocks,.. } if blocks==vec!["block:a"]))
    );
}

#[test]
fn closing_the_owner_restores_other_views_and_closing_the_source_preserves_the_owner() {
    for close_owner in [false, true] {
        let (mut source, mut child, shared, rx) = pair();
        let original = source.model().view().unwrap().clone();
        assert!(source.publish_detached(key(&source), 2, &mut Tx::default()));
        if close_owner {
            closed(&mut child, &mut Tx::default());
            source.poll_detached(Instant::now(), &mut Tx::default());
            assert!(!source.detached_here());
            assert!(shared.lock().unwrap().windows.contains_key(&1));
        } else {
            closed(&mut source, &mut Tx::default());
            assert!(!child.detached_here());
            assert_eq!(shared.lock().unwrap().detached.get(&key(&child)), Some(&2));
            assert!(shared.lock().unwrap().windows.contains_key(&2));
        }
        assert_eq!(source.model().view().unwrap(), &original);
        assert_eq!(child.model().view().unwrap(), &original);
        no_program_mutation(&rx);
    }
}

#[test]
fn ownership_can_move_again_without_an_old_restore_control_stealing_it_back() {
    let (mut source, mut child, shared, rx) = pair();
    let tab = key(&source);
    assert!(source.publish_detached(tab.clone(), 2, &mut Tx::default()));
    shared
        .lock()
        .unwrap()
        .windows
        .insert(3, child.handle.clone());
    assert!(child.publish_detached(tab.clone(), 3, &mut Tx::default()));
    source.restore_detached_target(tab.clone(), 2, &mut Tx::default());
    assert_eq!(shared.lock().unwrap().detached.get(&tab), Some(&3));
    source.restore_detached(&mut Tx::default());
    assert_eq!(shared.lock().unwrap().detached.get(&tab), Some(&1));
    no_program_mutation(&rx);
}

#[test]
fn server_tab_removal_clears_ownership_but_an_unattached_session_does_not() {
    let (mut source, _, shared, rx) = pair();
    let tab = key(&source);
    assert!(source.publish_detached(tab.clone(), 2, &mut Tx::default()));
    let view = source.hosts[0].model.views.remove("s").unwrap();
    source.poll_detached(Instant::now(), &mut Tx::default());
    assert!(shared.lock().unwrap().detached.contains_key(&tab));
    source.hosts[0].model.views.insert("s".into(), view);
    source.hosts[0]
        .model
        .views
        .get_mut("s")
        .unwrap()
        .windows
        .clear();
    source.poll_detached(Instant::now(), &mut Tx::default());
    assert!(shared.lock().unwrap().detached.is_empty());
    assert!(!source.detached_here());
    no_program_mutation(&rx);
}

#[test]
fn a_placeholder_discards_clipboard_events_and_terminal_keys() {
    use rex_linux::connection::clipboard::ClipboardCopy;
    let (mut source, _, _, rx) = pair();
    assert!(source.publish_detached(key(&source), 2, &mut Tx::default()));
    let model = source.hosts[0].model.clone();
    *source.hosts[0].connection.model.lock().unwrap() = model;
    let mut rt = frame(source);
    let clipboard = rune::clipboard::memory();
    clipboard.set_text("sentinel");
    rt.use_clipboard(&clipboard);
    rt.update(|s, tx| {
        s.hosts[0].connection.model.lock().unwrap().clipboard = Some(ClipboardCopy {
            session: "s".into(),
            block: "block:a".into(),
            text: "hidden copy".into(),
        });
        s.sync(0, tx);
    });
    rt.key(KeyEvent {
        key: Key::Text("x".into()),
        mods: Mods::default(),
    });
    rt.frame(1. / 60.);
    assert_eq!(
        clipboard.text().as_deref().map(String::as_str),
        Some("sentinel")
    );
    assert!(!rx.try_iter().any(|c| matches!(
        c,
        HostCommand::Terminal {
            command: Command::Key(_),
            ..
        }
    )));
}

#[test]
fn a_tab_drag_cancels_on_epoch_focus_session_or_target_changes() {
    for interruption in ["epoch", "focus", "session", "tab"] {
        let (source, _, shared, _) = pair();
        let mut rt = frame(source);
        let [x, y, w, h] = rect(&rt, "tab:a");
        rt.pointer_down(x + w / 2., y + h / 2.);
        rt.pointer_move(x + w / 2. + 20., y + h / 2. + 70.);
        rt.frame(1. / 60.);
        assert!(rt.state.tab_drag.is_some());
        rt.update(|s, tx| {
            match interruption {
                "epoch" => s.hosts[0].model.connection_epoch += 1,
                "focus" => s.window_focus(false),
                "session" => s.hosts[0].model.selected = None,
                "tab" => s.hosts[0].model.views.get_mut("s").unwrap().windows.clear(),
                _ => unreachable!(),
            };
            s.poll_detached(Instant::now(), tx);
        });
        assert!(rt.state.tab_drag.is_none());
        rt.pointer_up(x + w / 2. + 20., y + h / 2. + 70.);
        rt.frame(1. / 60.);
        assert!(rt.state.desktop.as_ref().unwrap().local_opening.is_none());
        assert!(shared.lock().unwrap().detached.is_empty());
    }
}

fn unopened_native() -> WindowHandle {
    rune::open_window(rune::WindowSpec::new(
        "never-mapped-test-view",
        rune::AppSpec::new("test", (800., 600.), || (), |_, _| rune::el()),
    ))
}

#[test]
fn an_unfinished_open_never_hides_the_source_and_failure_is_not_retried() {
    for failure in ["pending", "timeout", "epoch", "removed", "unregistered"] {
        let (mut source, _, shared, rx) = pair();
        let tab = key(&source);
        let before = source.model().view().unwrap().clone();
        let now = Instant::now();
        let deadline = now + MAX_WAIT;
        source.desktop.as_mut().unwrap().local_opening = Some(super::detached::LocalOpening {
            key: tab,
            host: source.hosts[0].id,
            epoch: source.hosts[0].model.connection_epoch,
            status: Arc::new(AtomicU8::new(if failure == "pending" { 0 } else { 3 })),
            native: unopened_native(),
            child: 2,
            deadline,
        });
        match failure {
            "epoch" => source.hosts[0].model.connection_epoch += 1,
            "removed" => source.hosts[0]
                .model
                .views
                .get_mut("s")
                .unwrap()
                .windows
                .clear(),
            "unregistered" => {
                shared.lock().unwrap().windows.remove(&2);
            }
            _ => {}
        }
        source.poll_detached(
            if failure == "timeout" { deadline } else { now },
            &mut Tx::default(),
        );
        assert!(!source.detached_here());
        assert!(shared.lock().unwrap().detached.is_empty());
        if failure == "pending" {
            assert!(source.desktop.as_ref().unwrap().local_opening.is_some());
            source.poll_detached(deadline, &mut Tx::default());
        }
        assert!(source.desktop.as_ref().unwrap().local_opening.is_none());
        source.poll_detached(deadline + MAX_WAIT, &mut Tx::default());
        assert!(source.desktop.as_ref().unwrap().local_opening.is_none());
        if failure != "removed" {
            assert_eq!(source.model().view().unwrap(), &before);
        }
        no_program_mutation(&rx);
    }
}

#[test]
fn a_ready_remote_detach_cannot_publish_after_consent_expiry_or_reconnect() {
    for failure in ["consent", "expiry", "epoch"] {
        let (mut source, _, shared, rx) = pair();
        let now = Instant::now();
        let pending = Pending {
            epoch: source.hosts[0].model.connection_epoch,
            received: now,
            request: serde_json::from_value(
                json!({"call_id":"once", "action":"client.tab.move_to_new_window"}),
            )
            .unwrap(),
        };
        source.desktop.as_mut().unwrap().pending = Some(Opening {
            host: source.hosts[0].id,
            pending,
            status: Arc::new(AtomicU8::new(3)),
            native: unopened_native(),
            detach: Some(key(&source)),
            child: 2,
        });
        match failure {
            "consent" => {
                shared.lock().unwrap().settings.hosts[0].set_remote_access(RemoteAccess::Off)
            }
            "epoch" => source.hosts[0].model.connection_epoch += 1,
            _ => {}
        }
        source.poll_desktop(
            if failure == "expiry" {
                now + MAX_WAIT
            } else {
                now
            },
            &mut Tx::default(),
        );
        assert!(source.desktop.as_ref().unwrap().pending.is_none());
        assert!(shared.lock().unwrap().detached.is_empty());
        assert!(!source.detached_here());
        let replies: Vec<_> = rx.try_iter().collect();
        assert_eq!(replies.len(), 1);
        assert!(
            matches!(&replies[0], HostCommand::PerformReply { epoch: 0, call_id, outcome: Outcome::Error { .. } } if call_id == "once")
        );
        source.poll_desktop(now + MAX_WAIT + MAX_WAIT, &mut Tx::default());
        no_program_mutation(&rx);
    }
}

#[test]
fn a_queued_restore_close_rechecks_ownership_dialogs_and_dedication() {
    let (mut source, mut child, shared, _) = pair();
    let tab = key(&source);
    assert!(source.publish_detached(tab.clone(), 2, &mut Tx::default()));
    source.restore_detached(&mut Tx::default());
    assert!(child.close_restored_destination(&tab, 1));
    child.picker(false);
    assert!(!child.close_restored_destination(&tab, 1));
    child.panel = None;
    assert!(child.close_restored_destination(&tab, 1));
    shared.lock().unwrap().detached.insert(tab.clone(), 2);
    assert!(
        !child.close_restored_destination(&tab, 1),
        "a late callback must not close a window that became owner again"
    );
    shared.lock().unwrap().detached.insert(tab.clone(), 1);
    child.desktop.as_mut().unwrap().dedicated = None;
    assert!(
        !child.close_restored_destination(&tab, 1),
        "a destination used for other work stays open"
    );
}

#[test]
fn a_detached_terminal_has_pane_controls_and_no_workspace_navigation() {
    let (_, mut child, _, _) = pair();
    child.desktop.as_mut().unwrap().terminal = Some(key(&child));
    child.sidebar.set_open(true);
    child.settings.vertical_tabs = true;
    let rt = frame(child);
    assert!(rect(&rt, "terminal-window-bar")[2] > 0.);
    assert!(rect(&rt, "close-terminal-window")[2] > 0.);
    for direction in ["left", "right", "up", "down"] {
        assert!(rect(&rt, &format!("pane-split-{direction}:block:a"))[2] > 0.);
    }
    assert!(!rt.elements().iter().any(|e| !e.exiting
        && e.key.as_deref().is_some_and(|k| {
            k == "toolbar" || k == "tabs" || k == "identity" || k.starts_with("sidebar-tab:")
        })));
}

#[test]
fn detached_navigation_is_consumed_locally_and_cannot_create_another_tab() {
    use crate::app::{actions, navigation::Visit, panel::Intent};
    let (_, mut child, _, _) = pair();
    child.desktop.as_mut().unwrap().terminal = Some(key(&child));
    let (sent, rx) = flume::unbounded();
    child.hosts[0].connection.commands = sent;
    let original = child.model().view().unwrap().clone();
    for action in [
        "client.tab.new",
        "client.tab.next",
        "client.tab.previous",
        "session.new",
        "session.switch",
        "client.sidebar.toggle",
    ] {
        assert!(!child.terminal_action_allowed(action));
        assert!(actions::run(
            &mut child,
            action,
            &Value::Null,
            &mut Tx::default()
        ));
    }
    child.perform(Intent::NewWindow, &mut Tx::default());
    child.perform(Intent::CreateOn(0, None), &mut Tx::default());
    child.perform(Intent::ChooseHost(String::new()), &mut Tx::default());
    assert!(child.panel.is_none());
    child.navigate_to(
        Visit {
            host: child.hosts[0].id,
            session: "s".into(),
            window: Some("other".into()),
        },
        &mut Tx::default(),
    );
    assert!(child.navigation.pending.is_none());
    assert_eq!(child.model().view().unwrap(), &original);
    assert!(rx.try_iter().next().is_none());
    for direction in ["left", "right", "up", "down"] {
        assert!(child.terminal_action_allowed(&format!("pane.split.{direction}")));
    }
}

#[test]
fn detached_remote_navigation_replies_once_without_changing_the_terminal() {
    let (_, mut child, _, _) = pair();
    child.desktop.as_mut().unwrap().terminal = Some(key(&child));
    child.settings.hosts[0].set_remote_access(RemoteAccess::Workspace);
    let (sent, rx) = flume::unbounded();
    child.hosts[0].connection.commands = sent;
    let original = child.model().view().unwrap().clone();
    for (action, args) in [
        ("client.tab.new", Value::Null),
        ("client.tab.next", Value::Null),
        ("session.select", json!({"session_id":"other"})),
        ("client.sidebar.toggle", Value::Null),
    ] {
        let pending = Pending {
            epoch: child.hosts[0].model.connection_epoch,
            received: Instant::now(),
            request: serde_json::from_value(
                json!({"call_id":"isolated", "action":action, "args":args}),
            )
            .unwrap(),
        };
        child.remote_perform(0, pending, &mut Tx::default());
        let commands: Vec<_> = rx.try_iter().collect();
        assert_eq!(commands.len(), 1, "{action} must send only a reply");
        let HostCommand::PerformReply { outcome, .. } = commands.into_iter().next().unwrap() else {
            panic!()
        };
        assert!(matches!(outcome, Outcome::Error { code, .. } if code == "refused"));
        assert_eq!(child.model().view().unwrap(), &original);
        assert!(child.navigation.pending.is_none());
    }
}

#[test]
fn removal_of_the_owned_tab_never_shows_or_targets_an_unrelated_terminal() {
    let (_, mut child, _, _) = pair();
    child.desktop.as_mut().unwrap().terminal = Some(key(&child));
    let view = child.hosts[0].model.views.get_mut("s").unwrap();
    view.windows[0].window_id = Some("other".into());
    view.active_window_id = Some("other".into());
    assert!(!child.terminal_contents_available());
    assert!(child.focused_block().is_none());
    assert!(child.visible_terminal_keys().is_empty());
    assert!(!child.terminal_action_allowed("pane.split.right"));
    assert!(!child.terminal_action_allowed("client.tab.close"));
    let rt = frame(child);
    assert!(rect(&rt, "terminal-unavailable")[2] > 0.);
    assert!(
        !rt.elements()
            .iter()
            .any(|e| !e.exiting && e.key.as_deref() == Some("panes"))
    );
}

#[test]
fn sidebar_tearoff_captures_another_cached_session_without_switching_the_source() {
    let (mut source, _, _, _) = pair();
    source.sidebar.set_open(true);
    source.settings.vertical_tabs = true;
    let mut other = source.model().view().unwrap().clone();
    other.session_id = Some("other-session".into());
    other.windows[0].window_id = Some("other-tab".into());
    other.active_window_id = Some("other-tab".into());
    source.hosts[0]
        .model
        .sessions
        .push(rexrs::api::session::Session {
            session_id: "other-session".into(),
            label: "other".into(),
        });
    source.hosts[0]
        .model
        .views
        .insert("other-session".into(), other);
    let mut rt = frame(source);
    let [x, y, w, h] = rect(&rt, "sidebar-tab:0:other-session:other-tab");
    rt.pointer_down(x + w / 2., y + h / 2.);
    rt.pointer_move(x + w / 2. + 40., y + h / 2.);
    rt.frame(1. / 60.);
    assert!(rt.state.tab_drag.is_some());
    assert_eq!(rt.state.model().selected.as_deref(), Some("s"));
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    rt.frame(1. / 60.);
    assert!(rt.state.tab_drag.is_none());
    rt.pointer_up(x + w / 2. + 40., y + h / 2.);
    rt.frame(1. / 60.);
    assert!(rt.state.desktop.as_ref().unwrap().local_opening.is_none());
    assert!(rt.state.navigation.pending.is_none());
    assert_eq!(rt.state.model().selected.as_deref(), Some("s"));
}
