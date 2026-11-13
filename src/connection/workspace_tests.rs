//! Remote workspace completion follows actual RPC results and refreshed server state.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

fn dispatch(
    host: &HostConnection,
    id: u64,
    consent: Arc<AtomicU64>,
    generation: u64,
    deadline: Instant,
    commands: Vec<HostCommand>,
) {
    host.commands
        .send(HostCommand::PerformBatch {
            id,
            epoch: host.model.lock().unwrap().connection_epoch,
            consent,
            generation,
            deadline,
            commands,
        })
        .unwrap();
}

fn rename() -> HostCommand {
    HostCommand::Request {
        method: "session.set_label".into(),
        payload: json!({"session_id":SESSION,"label":"new"}),
    }
}

#[tokio::test]
async fn desktop_close_waits_for_the_reply_ack_and_revocation_refuses_it() {
    for allowed in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (seen, waiting) = tokio::sync::oneshot::channel();
        let (release, resume) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut ws = accept(&listener, "rex.control.v1").await;
            welcome(&mut ws, 12).await;
            let bootstrap = recv(&mut ws).await;
            reply(
                &mut ws,
                &bootstrap,
                1,
                serde_json::to_value(boot()).unwrap(),
            )
            .await;
            let request = recv(&mut ws).await;
            assert_eq!(request["method"], "client.reply");
            assert_eq!(request["payload"]["error"].is_null(), allowed);
            seen.send(()).unwrap();
            resume.await.unwrap();
            reply(&mut ws, &request, 2, json!({})).await;
            assert!(matches!(ws.next().await, Some(Ok(Message::Close(_)))));
        });
        let host = HostConnection::spawn(endpoint, None, None, None, false, None);
        connected(&host).await;
        let epoch = host.model.lock().unwrap().connection_epoch;
        host.commands
            .send(HostCommand::DesktopCloseReply {
                epoch,
                call_id: "close".into(),
                consent: Arc::new(AtomicU64::new(if allowed { 1 } else { 2 })),
                generation: 1,
                deadline: Instant::now() + Duration::from_secs(5),
            })
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), waiting)
            .await
            .unwrap()
            .unwrap();
        assert!(host.model.lock().unwrap().desktop_reply.is_none());
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if host.model.lock().unwrap().desktop_reply.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            host.model.lock().unwrap().desktop_reply,
            Some(("close".into(), allowed))
        );
        drop(host);
        server.await.unwrap();
    }
}

#[test]
fn pinned_tab_survives_another_clients_focus_and_falls_back_when_closed() {
    let mut model = HostModel {
        selected: Some(SESSION.into()),
        pinned_tabs: Some(HashMap::from([(SESSION.into(), "one".into())])),
        ..Default::default()
    };
    let view: session::ViewResponse = serde_json::from_value(json!({"session_id":SESSION,"revision":1,"active_window_id":"two","windows":[{"window_id":"one","focused_block_id":"block:a","layers":[]},{"window_id":"two","focused_block_id":"block:b","layers":[]}]})).unwrap();
    model.views.insert(SESSION.into(), view);
    model.pin_current();
    assert_eq!(model.focused(), Some("block:a"));
    model.views.get_mut(SESSION).unwrap().windows.remove(0);
    model.pin_current();
    assert_eq!(model.focused(), Some("block:b"));
    assert_eq!(model.pinned_tabs.as_ref().unwrap()[SESSION], "two");
}

async fn completed(host: &HostConnection, id: u64) -> perform::Outcome {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some((got, result)) = &host.model.lock().unwrap().remote_result
                && *got == id
            {
                return result.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn completion_waits_for_the_rpc_and_refreshed_view_without_replaying() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (observed, waiting) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut ws = accept(&listener, "rex.control.v1").await;
        welcome(&mut ws, 12).await;
        let bootstrap = recv(&mut ws).await;
        reply(
            &mut ws,
            &bootstrap,
            1,
            serde_json::to_value(boot()).unwrap(),
        )
        .await;
        let request = recv(&mut ws).await;
        assert_eq!(request["method"], "session.set_label");
        observed.send(()).unwrap();
        resume.await.unwrap();
        reply(&mut ws, &request, 2, json!({"revision":2})).await;
        let view = recv(&mut ws).await;
        assert_eq!(view["method"], "session.view");
        reply(
            &mut ws,
            &view,
            3,
            json!({"session_id":SESSION,"label":"new","revision":2,"windows":[]}),
        )
        .await;
        assert!(matches!(ws.next().await, Some(Ok(Message::Close(_)))));
    });
    let host = HostConnection::spawn(endpoint, None, None, None, false, None);
    connected(&host).await;
    dispatch(
        &host,
        1,
        Arc::new(AtomicU64::new(1)),
        1,
        Instant::now() + Duration::from_secs(5),
        vec![rename()],
    );
    tokio::time::timeout(Duration::from_secs(3), waiting)
        .await
        .unwrap()
        .unwrap();
    assert!(
        host.model.lock().unwrap().remote_result.is_none(),
        "no success on dispatch"
    );
    release.send(()).unwrap();
    assert_eq!(completed(&host, 1).await, perform::Outcome::Performed);
    assert_eq!(host.model.lock().unwrap().views[SESSION].revision, 2);
    drop(host);
    server.await.unwrap();
}

#[tokio::test]
async fn expired_revoked_and_invalid_batches_send_no_mutations() {
    for hazard in ["expired", "revoked", "invalid"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut ws = accept(&listener, "rex.control.v1").await;
            welcome(&mut ws, 12).await;
            let bootstrap = recv(&mut ws).await;
            reply(
                &mut ws,
                &bootstrap,
                1,
                serde_json::to_value(boot()).unwrap(),
            )
            .await;
            assert!(
                matches!(ws.next().await, Some(Ok(Message::Close(_)))),
                "{hazard}: no command before close"
            );
        });
        let host = HostConnection::spawn(endpoint, None, None, None, false, None);
        connected(&host).await;
        let consent = Arc::new(AtomicU64::new(1));
        if hazard == "revoked" {
            consent.fetch_add(1, Ordering::AcqRel);
        }
        let deadline = if hazard == "expired" {
            Instant::now() - Duration::from_secs(1)
        } else {
            Instant::now() + Duration::from_secs(5)
        };
        let mut commands = vec![rename()];
        if hazard == "invalid" {
            commands.push(HostCommand::Theme(None));
        }
        dispatch(&host, 2, consent, 1, deadline, commands);
        let result = completed(&host, 2).await;
        assert!(
            matches!(result, perform::Outcome::Error { .. }),
            "{hazard}: {result:?}"
        );
        assert!(
            host.model.lock().unwrap().connected,
            "a refusal is not a transport failure"
        );
        drop(host);
        server.await.unwrap();
    }
}

#[tokio::test]
async fn a_server_refusal_is_reported_without_retry_or_reconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut ws = accept(&listener, "rex.control.v1").await;
        welcome(&mut ws, 12).await;
        let bootstrap = recv(&mut ws).await;
        reply(
            &mut ws,
            &bootstrap,
            1,
            serde_json::to_value(boot()).unwrap(),
        )
        .await;
        let request = recv(&mut ws).await;
        send(&mut ws, json!({"type":"response","id":request["id"],"seq":2,"error":{"code":"not_found","message":"target vanished","status":404}})).await;
        assert!(matches!(ws.next().await, Some(Ok(Message::Close(_)))));
    });
    let host = HostConnection::spawn(endpoint, None, None, None, false, None);
    connected(&host).await;
    let epoch = host.model.lock().unwrap().connection_epoch;
    dispatch(
        &host,
        3,
        Arc::new(AtomicU64::new(1)),
        1,
        Instant::now() + Duration::from_secs(5),
        vec![rename()],
    );
    let result = completed(&host, 3).await;
    assert!(
        matches!(result, perform::Outcome::Error { code: "not_found", ref message } if message.contains("target vanished"))
    );
    assert_eq!(host.model.lock().unwrap().connection_epoch, epoch);
    assert!(host.model.lock().unwrap().connected);
    drop(host);
    server.await.unwrap();
}

#[tokio::test]
async fn an_operation_from_a_replaced_epoch_is_discarded_before_dispatch() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut ws = accept(&listener, "rex.control.v1").await;
        welcome(&mut ws, 12).await;
        let bootstrap = recv(&mut ws).await;
        reply(
            &mut ws,
            &bootstrap,
            1,
            serde_json::to_value(boot()).unwrap(),
        )
        .await;
        assert!(matches!(ws.next().await, Some(Ok(Message::Close(_)))));
    });
    let host = HostConnection::spawn(endpoint, None, None, None, false, None);
    connected(&host).await;
    let epoch = host.model.lock().unwrap().connection_epoch;
    host.commands
        .send(HostCommand::PerformBatch {
            id: 5,
            epoch: epoch + 1,
            consent: Arc::new(AtomicU64::new(1)),
            generation: 1,
            deadline: Instant::now() + Duration::from_secs(5),
            commands: vec![rename()],
        })
        .unwrap();
    let visible = !host.model.lock().unwrap().visible;
    host.commands
        .send(HostCommand::Visibility(visible))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while host.model.lock().unwrap().visible != visible {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(host.model.lock().unwrap().remote_result.is_none());
    drop(host);
    server.await.unwrap();
}

#[tokio::test]
async fn disconnect_after_dispatch_does_not_replay_the_mutation_on_reconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut first = accept(&listener, "rex.control.v1").await;
        welcome(&mut first, 12).await;
        let bootstrap = recv(&mut first).await;
        reply(
            &mut first,
            &bootstrap,
            1,
            serde_json::to_value(boot()).unwrap(),
        )
        .await;
        let request = recv(&mut first).await;
        assert_eq!(request["method"], "session.set_label");
        first.close(None).await.unwrap();
        let mut second = accept(&listener, "rex.control.v1").await;
        welcome(&mut second, 12).await;
        let bootstrap = recv(&mut second).await;
        assert_eq!(bootstrap["method"], "session.bootstrap");
        reply(
            &mut second,
            &bootstrap,
            1,
            serde_json::to_value(boot()).unwrap(),
        )
        .await;
        assert!(
            matches!(second.next().await, Some(Ok(Message::Close(_)))),
            "reconnect must not repeat session.set_label"
        );
    });
    let host = HostConnection::spawn(endpoint, None, None, None, false, None);
    connected(&host).await;
    let epoch = host.model.lock().unwrap().connection_epoch;
    dispatch(
        &host,
        6,
        Arc::new(AtomicU64::new(1)),
        1,
        Instant::now() + Duration::from_secs(5),
        vec![rename()],
    );
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let reconnected = {
                let model = host.model.lock().unwrap();
                model.connected && model.connection_epoch > epoch
            };
            if reconnected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        host.model.lock().unwrap().remote_result.is_none(),
        "old completion does not cross epochs"
    );
    drop(host);
    server.await.unwrap();
}

#[tokio::test]
async fn directory_lookup_cannot_resurrect_an_expired_creation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (observed, waiting) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut ws = accept(&listener, "rex.control.v1").await;
        welcome(&mut ws, 12).await;
        let bootstrap = recv(&mut ws).await;
        reply(
            &mut ws,
            &bootstrap,
            1,
            serde_json::to_value(boot()).unwrap(),
        )
        .await;
        let process = recv(&mut ws).await;
        assert!(process["method"].as_str().unwrap().ends_with(".process"));
        observed.send(()).unwrap();
        resume.await.unwrap();
        reply(&mut ws, &process, 2, json!({})).await;
        assert!(
            matches!(ws.next().await, Some(Ok(Message::Close(_)))),
            "no new split after expiry"
        );
    });
    let host = HostConnection::spawn(endpoint, None, None, None, false, None);
    connected(&host).await;
    let deadline = Instant::now() + Duration::from_millis(100);
    dispatch(
        &host,
        4,
        Arc::new(AtomicU64::new(1)),
        1,
        deadline,
        vec![HostCommand::NewTerminal {
            method: "session.new_split".into(),
            payload: json!({"session_id":SESSION,"layout":crate::layout::shell_layout(None,None)}),
            cwd_from: "b".into(),
        }],
    );
    tokio::time::timeout(Duration::from_secs(3), waiting)
        .await
        .unwrap()
        .unwrap();
    tokio::time::sleep_until(deadline.into()).await;
    release.send(()).unwrap();
    assert!(matches!(
        completed(&host, 4).await,
        perform::Outcome::Error {
            code: "refused",
            ..
        }
    ));
    drop(host);
    server.await.unwrap();
}
