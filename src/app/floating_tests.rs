use super::*;
use crate::app::{actions, floating};
use rex_linux::connection::{Command, perform::Outcome};
use serde_json::{Value, json};

fn fixture() -> (Workspace, flume::Receiver<HostCommand>) {
    let (mut ws, rx) = tabbed_workspace("split");
    ws.hosts[0].model.views.get_mut("s").unwrap().windows[1] = serde_json::from_value(json!({
        "window_id":"split","label":"dev","focused_block_id":"block:c","layers":[
            {"layer_id":"layer:t","kind":"tiled","bounds":[0,0,1,1],"layout":{"split":{"direction":"horizontal","ratio":0.5,"before":{"block_id":"block:a"},"after":{"block_id":"block:b"}}},"blocks":[{"block_id":"block:a"},{"block_id":"block:b"}]},
            {"layer_id":"layer:c","kind":"floating","bounds":[0.2,0.2,0.5,0.5],"layout":{"block_id":"block:c"},"blocks":[{"block_id":"block:c","label":"floating"}]},
            {"layer_id":"layer:d","kind":"floating","state":"minimized","bounds":[0.3,0.3,0.4,0.4],"layout":{"block_id":"block:d"},"blocks":[{"block_id":"block:d","label":"hidden"}]}
        ]})).unwrap();
    (ws, rx)
}

fn calls(rx: &flume::Receiver<HostCommand>) -> Vec<(String, Value)> {
    rx.try_iter()
        .filter_map(|c| {
            if let HostCommand::Request { method, payload } = c {
                Some((method, payload))
            } else {
                None
            }
        })
        .collect()
}

fn rect(rt: &Runtime<Workspace>, key: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing {key}"))
        .rect
}

fn framed() -> (Runtime<Workspace>, flume::Receiver<HostCommand>) {
    let (mut ws, rx) = fixture();
    let sender = ws.hosts[0].connection.commands.clone();
    for block in ["block:a", "block:b", "block:c", "block:d"] {
        super::pane_verb_tests::framed_terminal(&mut ws, block, &sender, b"fixture text\r\n");
    }
    let mut rt = runtime_at(ws, (1440., 900.));
    for _ in 0..60 {
        rt.frame(1. / 60.);
    }
    rx.try_iter().for_each(drop);
    (rt, rx)
}

#[test]
fn float_and_dock_move_existing_blocks_without_replacing_programs_or_local_geometry() {
    let (mut ws, rx) = fixture();
    let before = ws.model().view().unwrap().clone();
    floating::run(&mut ws, "pane.float", &Value::Null);
    let sent = calls(&rx);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "session.move_block");
    assert_eq!(sent[0].1["block_id"], "block:c");
    assert_eq!(sent[0].1["anchor_block_id"], "block:a");
    assert_eq!(
        ws.model().view().unwrap(),
        &before,
        "only a server snapshot may change the geometry"
    );
    ws.hosts[0].model.views.get_mut("s").unwrap().windows[1].focused_block_id =
        Some("block:a".into());
    floating::run(&mut ws, "pane.float", &Value::Null);
    let sent = calls(&rx);
    assert_eq!(sent[0].0, "session.float_block");
    assert_eq!(sent[0].1["block_id"], "block:a");
}

#[test]
fn the_last_tiled_pane_and_an_empty_docking_destination_are_refused_before_dispatch() {
    let (mut ws, rx) = fixture();
    let window = &mut ws.hosts[0].model.views.get_mut("s").unwrap().windows[1];
    window.layers[0].blocks.truncate(1);
    window.layers[0].layout = Some(serde_json::from_value(json!({"block_id":"block:a"})).unwrap());
    window.focused_block_id = Some("block:a".into());
    assert!(matches!(
        floating::validate(&ws, "pane.float", &Value::Null),
        Err(Outcome::Error {
            code: "not_supported",
            ..
        })
    ));
    floating::run(&mut ws, "pane.float", &Value::Null);
    assert!(calls(&rx).is_empty());
    let window = &mut ws.hosts[0].model.views.get_mut("s").unwrap().windows[1];
    window.layers[0].blocks.clear();
    window.layers[0].layout = None;
    window.focused_block_id = Some("block:c".into());
    assert!(matches!(
        floating::validate(&ws, "pane.float", &Value::Null),
        Err(Outcome::Error {
            code: "not_supported",
            ..
        })
    ));
    assert!(calls(&rx).is_empty());
}

#[test]
fn bounds_are_finite_and_inside_the_tab_and_layer_ids_cannot_cross_tabs() {
    for args in [
        json!({"bounds":[-0.1,0.,0.5,0.5]}),
        json!({"bounds":[0.8,0.,0.5,0.5]}),
        json!({"bounds":[0.,0.,0.,1.]}),
        json!({"bounds":[0.,0.,1.]}),
        json!({"bounds":[0.,0.,1.,1.],"host":"other"}),
    ] {
        assert!(
            !actions::find("pane.floating.bounds")
                .unwrap()
                .accepts(&args)
        );
    }
    assert!(!floating::valid_bounds([0., 0., f64::NAN, 0.5]));
    let (ws, rx) = fixture();
    assert!(
        floating::validate(
            &ws,
            "pane.restore_floating",
            &json!({"layer_id":"other-window-layer"})
        )
        .is_err()
    );
    assert!(calls(&rx).is_empty());
}

#[test]
fn minimizing_all_preserves_a_visible_pane_and_restoration_names_the_original_layer() {
    let (mut ws, rx) = fixture();
    floating::run(&mut ws, "pane.minimize_floating", &Value::Null);
    let sent = calls(&rx);
    assert_eq!(sent[0].1["layer_id"], "layer:c");
    assert_eq!(sent[0].1["state"], "minimized");
    floating::run(
        &mut ws,
        "pane.restore_floating",
        &json!({"layer_id":"layer:d"}),
    );
    let sent = calls(&rx);
    assert_eq!(sent[0].1["layer_id"], "layer:d");
    assert_eq!(sent[0].1["focus"], true);
    ws.hosts[0].model.views.get_mut("s").unwrap().windows[1]
        .layers
        .remove(0);
    assert!(matches!(
        floating::validate(&ws, "pane.minimize_all_floating", &Value::Null),
        Err(Outcome::Error {
            code: "refused",
            ..
        })
    ));
    assert!(calls(&rx).is_empty());
}

#[test]
fn new_floating_is_an_explicit_creation_and_waits_in_the_acknowledged_pipeline() {
    let (mut ws, rx) = fixture();
    ws.settings.hosts[0].set_remote_access(rex_linux::settings::RemoteAccess::Workspace);
    ws.remote_perform(
        0,
        super::remote_tests::pending("pane.new_floating", Value::Null),
        &mut rune::Tx::default(),
    );
    let sent: Vec<_> = rx.try_iter().collect();
    assert_eq!(sent.len(), 1, "no success before the server result");
    let HostCommand::PerformBatch { commands, .. } = &sent[0] else {
        panic!()
    };
    assert!(
        matches!(&commands[..],[HostCommand::NewTerminal{method,cwd_from,payload}] if method=="session.new_layer" && cwd_from=="block:c" && payload["window_id"]=="split")
    );
}

#[test]
fn a_floating_pane_receives_pointer_input_above_a_tiled_divider() {
    let (mut rt, rx) = framed();
    let pane = rect(&rt, "pane:block:c");
    let tiled = rect(&rt, "pane:block:a");
    let x = tiled[0] + tiled[2] + 4.;
    let y = pane[1] + pane[3] / 2.;
    rt.pointer_down(x, y);
    rt.pointer_move(x + 30., y);
    rt.pointer_up(x + 30., y);
    for _ in 0..4 {
        rt.frame(1. / 60.);
    }
    assert!(rt.state.drag.is_none());
    let sent = calls(&rx);
    assert!(
        sent.iter()
            .any(|(method, payload)| method == "session.raise_layer"
                && payload["layer_id"] == "layer:c"),
        "the top floating pane received the press"
    );
    assert!(
        !sent
            .iter()
            .any(|(method, _)| method == "session.resize_pane")
    );
}

#[test]
fn header_drag_previews_then_sends_one_resize_and_cancels_when_its_epoch_changes() {
    for interrupted in ["none", "epoch", "bounds", "focus", "block"] {
        let (mut rt, rx) = framed();
        let h = rect(&rt, "pane-header:block:c");
        let (x, y) = (h[0] + 20., h[1] + 12.);
        rt.pointer_down(x, y);
        rt.pointer_move(x + 70., y + 30.);
        rt.frame(1. / 60.);
        assert!(rt.state.floating_drag.is_some());
        assert!(!calls(&rx).iter().any(|(m, _)| m == "session.resize_layer"));
        if interrupted != "none" {
            rt.update(|s, tx| {
                match interrupted {
                    "epoch" => s.hosts[0].model.connection_epoch += 1,
                    "bounds" => {
                        s.hosts[0].model.views.get_mut("s").unwrap().windows[1].layers[1].bounds
                            [0] += 0.01
                    }
                    "focus" => s.window_focus(false),
                    "block" => s.hosts[0].model.views.get_mut("s").unwrap().windows[1].layers[1]
                        .blocks
                        .clear(),
                    _ => unreachable!(),
                }
                crate::app::poll(s, 0., tx);
            });
        }
        rt.pointer_up(x + 70., y + 30.);
        for _ in 0..4 {
            rt.frame(1. / 60.);
        }
        let sent = calls(&rx);
        assert_eq!(
            sent.iter()
                .filter(|(m, _)| m == "session.resize_layer")
                .count(),
            usize::from(interrupted == "none")
        );
        assert!(rt.state.floating_drag.is_none());
    }
}

#[test]
fn minimized_panes_receive_neither_keys_nor_size_claims_but_keep_their_notifications() {
    let (mut ws, rx) = fixture();
    ws.hosts[0].model.views.get_mut("s").unwrap().windows[1].focused_block_id =
        Some("block:d".into());
    assert!(ws.focused_block().is_none());
    ws.claim_visible();
    let claims: Vec<_> = rx
        .try_iter()
        .filter_map(|c| {
            if let HostCommand::Claim { blocks, .. } = c {
                Some(blocks)
            } else {
                None
            }
        })
        .flatten()
        .collect();
    assert!(!claims.contains(&"block:d".to_owned()));
    assert!(claims.contains(&"block:c".to_owned()));
    let sender = ws.hosts[0].connection.commands.clone();
    super::pane_verb_tests::framed_terminal(&mut ws, "block:d", &sender, b"hidden live terminal");
    let mut rt = runtime_at(ws, (1440., 900.));
    press(&mut rt, Key::Text("x".into()));
    assert!(!rx.try_iter().any(|c| matches!(
        c,
        HostCommand::Terminal {
            command: Command::Key(_),
            ..
        }
    )));
    rt.state.settings.notifications.system_notifications = true;
    rt.state.settings.notifications.program_messages = true;
    rt.state.deliver_program_notices(
        0,
        vec![rex_linux::connection::activity::DesktopNotice {
            session: "s".into(),
            block: "block:d".into(),
            title: "hidden".into(),
            body: "alert".into(),
        }],
    );
    assert_eq!(rt.state.outbox.len(), 1);
    assert_eq!(rt.state.outbox[0].source_block.as_deref(), Some("block:d"));
}

#[test]
fn the_minimized_capsule_restores_the_captured_layer_without_a_new_program() {
    let (mut rt, rx) = framed();
    click(&mut rt, "floating-restore:layer:d");
    let sent = calls(&rx);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "session.set_layer_state");
    assert_eq!(sent[0].1["layer_id"], "layer:d");
    assert_eq!(sent[0].1["state"], "normal");
    assert!(rect(&rt, "floating-tray")[3] > 0.);
}

#[test]
fn the_corner_handle_resizes_the_layer_without_moving_its_origin() {
    let (mut rt, rx) = framed();
    let r = rect(&rt, "floating-resize:layer:c");
    let (x, y) = (r[0] + r[2] / 2., r[1] + r[3] / 2.);
    rt.pointer_down(x, y);
    rt.pointer_move(x + 40., y + 25.);
    rt.frame(1. / 60.);
    assert!(rt.state.floating_drag.is_some());
    rt.pointer_up(x + 40., y + 25.);
    for _ in 0..4 {
        rt.frame(1. / 60.);
    }
    let sent: Vec<_> = calls(&rx)
        .into_iter()
        .filter(|(m, _)| m == "session.resize_layer")
        .collect();
    assert_eq!(sent.len(), 1);
    let bounds = floating::bounds(&sent[0].1["bounds"]).unwrap();
    assert_eq!(&bounds[..2], &[0.2, 0.2]);
    assert!(bounds[2] > 0.5 && bounds[3] > 0.5);
}

#[test]
fn floating_headers_keep_dock_and_minimize_controls_in_compact_and_hidden_modes() {
    let (mut rt, _) = framed();
    for density in [
        rex_linux::settings::PaneDensity::Comfortable,
        rex_linux::settings::PaneDensity::Compact,
    ] {
        rt.update(|s, _| {
            s.settings.pane_density = density;
            s.settings.pane_header = rex_linux::settings::PaneHeader::Hidden;
        });
        rt.frame(1. / 60.);
        assert_eq!(
            rect(&rt, "pane-header:block:c")[3],
            crate::app::theme::CARD_HEADER
        );
        assert!(rect(&rt, "pane-dock:block:c")[2] > 0.);
        assert!(rect(&rt, "pane-minimize:block:c")[2] > 0.);
    }
}

#[test]
fn clipboard_events_from_minimized_panes_are_discarded_even_if_server_focus_is_stale() {
    use rex_linux::connection::clipboard::ClipboardCopy;
    for shown in [false, true] {
        let (mut ws, _) = fixture();
        ws.hosts[0].model.visible = true;
        let window = &mut ws.hosts[0].model.views.get_mut("s").unwrap().windows[1];
        window.focused_block_id = Some("block:d".into());
        if shown {
            window.layers[2].state = None;
        }
        *ws.hosts[0].connection.model.lock().unwrap() = ws.hosts[0].model.clone();
        let mut rt = runtime(ws);
        let clipboard = rune::clipboard::memory();
        clipboard.set_text("sentinel");
        rt.use_clipboard(&clipboard);
        rt.update(|s, tx| {
            s.hosts[0].connection.model.lock().unwrap().clipboard = Some(ClipboardCopy {
                session: "s".into(),
                block: "block:d".into(),
                text: "program copy".into(),
            });
            s.sync(0, tx);
        });
        rt.frame(1. / 60.);
        assert_eq!(
            clipboard.text().as_deref().map(String::as_str),
            Some(if shown { "program copy" } else { "sentinel" })
        );
        assert!(
            rt.state.hosts[0]
                .connection
                .model
                .lock()
                .unwrap()
                .clipboard
                .is_none()
        );
        std::fs::remove_file(&rt.state.settings_path).ok();
    }
}
