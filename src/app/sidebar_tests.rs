//! Mac-style sidebar capsules and the row-specific close affordance.
use super::*;

fn info(rt: &Runtime<Workspace>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing {key}"))
}

fn rest(rt: &mut Runtime<Workspace>) {
    for _ in 0..90 {
        rt.frame(1. / 60.);
    }
}

#[test]
fn sidebar_capsules_reveal_close_without_moving_the_foreground() {
    for vertical in [false, true] {
        for (scale, size) in [(1., (1024., 700.)), (1.3, (832., 624.))] {
            let (mut ws, _rx) = sidebar_tabs_workspace();
            ws.settings.vertical_tabs = vertical;
            ws.settings.ui_scale = scale;
            let mut rt = runtime_at(ws, size);
            rest(&mut rt);
            let (row, close, label) = (
                "sidebar-tab:0:s:single",
                "sidebar-tab-close:0:s:single",
                "sidebar-tab-label",
            );
            let row_info = info(&rt, row);
            assert!((row_info.rect[3] - 30. * scale).abs() < 0.01);
            assert!(
                (row_info.radius - 15.).abs() < 0.01,
                "capsule: {:?}",
                row_info.rect
            );
            let foreground = |rt: &Runtime<Workspace>| {
                rt.elements()
                    .into_iter()
                    .filter(|e| {
                        !e.exiting
                            && e.path.contains(&format!("{row}/"))
                            && (e.key.as_deref() == Some(label)
                                || e.role == "icon" && !e.path.contains("-close:"))
                    })
                    .map(|e| (e.path, e.rect, e.opacity))
                    .collect::<Vec<_>>()
            };
            let before = foreground(&rt);
            assert_eq!(before.len(), 2);
            let hidden = info(&rt, close);
            if scale == 1. {
                assert!(
                    (hidden.rect[0] + hidden.rect[2] / 2. - 213.).abs() < 0.01,
                    "native close x: {:?}",
                    hidden.rect
                );
                assert!(
                    (hidden.rect[1] + hidden.rect[3] / 2. - 131.).abs() < 0.01,
                    "native close y: {:?}",
                    hidden.rect
                );
            }
            assert!(hidden.opacity < 0.01, "close at rest: {}", hidden.opacity);
            rt.pointer_move(
                row_info.rect[0] + 70. * scale,
                row_info.rect[1] + row_info.rect[3] / 2.,
            );
            rest(&mut rt);
            let shown = info(&rt, close);
            assert!(shown.opacity > 0.99, "close on hover: {}", shown.opacity);
            assert_eq!(shown.rect, hidden.rect);
            assert!(
                shown.rect[0] >= row_info.rect[0]
                    && shown.rect[0] + shown.rect[2] <= row_info.rect[0] + row_info.rect[2]
            );
            assert_eq!(
                foreground(&rt),
                before,
                "hover must not shift or remount icon/label"
            );
            rt.pointer_move(700. * scale, 400. * scale);
            rest(&mut rt);
            assert!(info(&rt, close).opacity < 0.01);
            assert_eq!(foreground(&rt), before);
            rt.update(|_, tx| tx.focus(close, true));
            rest(&mut rt);
            assert!(
                info(&rt, close).opacity > 0.99,
                "focus must expose the close control too"
            );
            std::fs::remove_file(&rt.state.settings_path).ok();
        }
    }
}

#[test]
fn closing_the_last_tab_confirms_its_session_and_keeps_running_by_default() {
    let (mut ws, rx) = sidebar_tabs_workspace();
    ws.hosts[0]
        .model
        .views
        .get_mut("s")
        .unwrap()
        .windows
        .truncate(1);
    let mut rt = runtime(ws);
    rest(&mut rt);
    rx.try_iter().for_each(drop);
    click(&mut rt, "sidebar-tab-close:0:s:single");
    let panel = rt.state.panel.as_ref().unwrap();
    assert!(
        matches!(&panel.kind,PanelKind::Confirm{method,description,..} if method=="session.destroy" && description.contains("last tab"))
    );
    assert_eq!(panel.tab.as_ref().unwrap().session, "s");
    assert!(requests(&rx).is_empty());
    press(&mut rt, Key::Enter);
    assert!(rt.state.panel.is_none());
    assert!(requests(&rx).is_empty());
    click(&mut rt, "sidebar-tab-close:0:s:single");
    press(&mut rt, Key::Tab);
    press(&mut rt, Key::Text(" ".into()));
    let sent = requests(&rx);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        (sent[0].0.as_str(), sent[0].1.as_str()),
        ("session.destroy", "s")
    );
    std::fs::remove_file(&rt.state.settings_path).ok();
}

#[test]
fn a_tab_arriving_during_last_tab_confirmation_keeps_its_session() {
    let (mut ws, rx) = sidebar_tabs_workspace();
    ws.hosts[0]
        .model
        .views
        .get_mut("s")
        .unwrap()
        .windows
        .truncate(1);
    let mut rt = runtime(ws);
    rest(&mut rt);
    rx.try_iter().for_each(drop);
    click(&mut rt, "sidebar-tab-close:0:s:single");
    rt.update(|s, _| {
        let mut new = s.hosts[0].model.views["other"].windows[0].clone();
        new.window_id = Some("arrived".into());
        s.hosts[0]
            .model
            .views
            .get_mut("s")
            .unwrap()
            .windows
            .push(new);
    });
    rt.state.submit(&mut rune::Tx::default());
    let sent = requests(&rx);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        (sent[0].0.as_str(), sent[0].1.as_str()),
        ("session.close_window", "s")
    );
    assert_eq!(sent[0].2["window_id"], "single");
    std::fs::remove_file(&rt.state.settings_path).ok();
}

#[test]
fn sidebar_tab_close_confirms_the_inactive_sessions_tab_without_navigation() {
    let (ws, rx) = sidebar_tabs_workspace();
    let mut rt = runtime(ws);
    rest(&mut rt);
    rx.try_iter().for_each(drop);
    click(&mut rt, "sidebar-tab-close:0:other:other-second");
    assert!(
        matches!(&rt.state.panel.as_ref().unwrap().kind,PanelKind::Confirm{method,payload,..} if method=="session.close_window" && payload["window_id"]=="other-second")
    );
    assert_eq!(
        rt.state
            .panel
            .as_ref()
            .unwrap()
            .tab
            .as_ref()
            .unwrap()
            .session,
        "other"
    );
    assert_eq!(rt.state.hosts[0].model.selected.as_deref(), Some("s"));
    assert!(
        rx.try_iter().next().is_none(),
        "close click must not select or destroy before confirmation"
    );
    rt.state.submit(&mut rune::Tx::default());
    let sent = requests(&rx);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        (sent[0].0.as_str(), sent[0].1.as_str()),
        ("session.close_window", "other")
    );
    assert_eq!(sent[0].2["window_id"], "other-second");
    std::fs::remove_file(&rt.state.settings_path).ok();
}

#[test]
fn closing_the_final_pane_ends_the_session_but_late_siblings_survive() {
    use crate::app::panel::PaneAction;
    for added in 0..3 {
        let (mut ws, rx) = sidebar_tabs_workspace();
        ws.hosts[0]
            .model
            .views
            .get_mut("s")
            .unwrap()
            .windows
            .truncate(1);
        let target = ws.target("block:a").unwrap();
        let mut rt = runtime(ws);
        rest(&mut rt);
        rx.try_iter().for_each(drop);
        rt.update(|s, tx| s.perform(Intent::Pane(target.clone(), PaneAction::Close), tx));
        if added == 1 {
            rt.update(|s, _| {
                let w = &mut s.hosts[0].model.views.get_mut("s").unwrap().windows[0];
                let mut block = w.layers[0].blocks[0].clone();
                block.block_id = Some("late-pane".into());
                w.layers[0].blocks.push(block);
            });
        } else if added == 2 {
            rt.update(|s, _| {
                let mut w = s.hosts[0].model.views["other"].windows[0].clone();
                w.window_id = Some("late-tab".into());
                s.hosts[0].model.views.get_mut("s").unwrap().windows.push(w);
            });
        }
        rt.state.submit(&mut rune::Tx::default());
        let sent = requests(&rx);
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].0,
            if added == 0 {
                "session.destroy"
            } else {
                "block.close"
            }
        );
        assert_eq!(sent[0].1, "s");
        if added != 0 {
            assert_eq!(sent[0].2["block_id"], "block:a");
        }
        std::fs::remove_file(&rt.state.settings_path).ok();
    }
}

#[test]
fn legacy_empty_or_loading_sessions_never_offer_new_tab_or_mutate_the_server() {
    for loaded in [false, true] {
        let mut ws = workspace();
        let rx = capture_host(&mut ws, 0);
        ws.sidebar.set_open(true);
        ws.hosts[0].model.connected = true;
        ws.hosts[0].model.sessions = vec![session("legacy")];
        ws.hosts[0].model.selected = Some("legacy".into());
        if loaded {
            ws.hosts[0].model.views.insert(
                "legacy".into(),
                rexrs::api::session::ViewResponse::default(),
            );
        }
        let mut rt = runtime(ws);
        rest(&mut rt);
        assert!(!rt.elements().iter().any(|e| matches!(
            e.text.as_deref(),
            Some("This session has no tabs" | "Create a tab to start a terminal.")
        )));
        if loaded {
            assert!(!has(&rt, "session-group:0:legacy"));
        } else {
            assert!(has(&rt, "session-loading"));
        }
        assert!(
            !rx.try_iter().any(|c| matches!(
                c,
                HostCommand::CreateSession { .. }
                    | HostCommand::NewTerminal { .. }
                    | HostCommand::Request { .. }
            )),
            "render/attachment must not repair or destroy sessions"
        );
        std::fs::remove_file(&rt.state.settings_path).ok();
    }
}
