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
            let (row, close, label) = if vertical {
                (
                    "sidebar-tab:0:s:single",
                    "sidebar-tab-close:0:s:single",
                    "sidebar-tab-label",
                )
            } else {
                ("session:0:s", "session-close:0:s", "session-label")
            };
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
                    (hidden.rect[1] + hidden.rect[3] / 2. - if vertical { 209. } else { 183. })
                        .abs()
                        < 0.01,
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
fn sidebar_session_close_confirms_its_own_host_without_selecting_the_row() {
    let mut ws = workspace_hosts(2);
    ws.sidebar.set_open(true);
    let current = capture_host(&mut ws, 0);
    let other = capture_host(&mut ws, 1);
    for (host, id) in ws.hosts.iter_mut().zip(["current", "other"]) {
        host.model.connected = true;
        host.model.sessions = vec![session(id)];
        host.model.selected = Some(id.into());
    }
    let mut rt = runtime(ws);
    rest(&mut rt);
    click(&mut rt, "session-close:1:other");
    let panel = rt.state.panel.as_ref().unwrap();
    assert!(
        matches!(&panel.kind,PanelKind::Confirm{method,payload,..} if method=="session.destroy" && payload["session_id"]=="other")
    );
    assert_eq!(panel.session.as_ref().unwrap().host, 1);
    assert_eq!(rt.state.active, 0);
    assert_eq!(rt.state.hosts[0].model.selected.as_deref(), Some("current"));
    assert!(requests(&other).is_empty() && requests(&current).is_empty());
    press(&mut rt, Key::Escape);
    assert!(rt.state.panel.is_none());
    assert!(requests(&other).is_empty());
    click(&mut rt, "session-close:1:other");
    press(&mut rt, Key::Enter); // Keep running remains the default.
    assert!(rt.state.panel.is_none());
    assert!(requests(&other).is_empty());
    click(&mut rt, "session-close:1:other");
    assert!(focused(&rt).unwrap().ends_with("cancel-close"));
    press(&mut rt, Key::Tab);
    assert!(focused(&rt).unwrap().ends_with("confirm-close"));
    press(&mut rt, Key::Text(" ".into())); // Space activates the focused button.
    let sent = requests(&other);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        (sent[0].0.as_str(), sent[0].1.as_str()),
        ("session.destroy", "other")
    );
    assert!(requests(&current).is_empty());
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
