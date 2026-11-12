//! Desktop notifications through the freedesktop notification service, on a worker thread
//! so D-Bus never blocks the UI. Clicking one raises the window (with the activation token
//! the server sends just before the click, so Wayland lets it take focus) and shows its tab.
use super::workspace::{HostId, Workspace};
use futures_util::StreamExt;
use rune::AppHandle;
use std::collections::{HashMap, VecDeque};
use zbus::{Connection, Proxy, Variant};

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// Clicks are remembered for this many recent notifications.
const REMEMBERED: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct NoteTarget {
    pub host: HostId,
    pub session: String,
    pub window: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    /// Program messages name their pane, so minimizing it does not hide its notifications.
    pub source_block: Option<String>,
    pub summary: String,
    pub body: String,
    pub target: NoteTarget,
}

pub struct Notifier {
    notes: flume::Sender<Note>,
}

impl Notifier {
    pub fn spawn(handle: AppHandle<Workspace>) -> Self {
        let (notes, receiver) = flume::unbounded();
        let _ = std::thread::Builder::new()
            .name("rex-notifications".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                if let Err(error) = runtime.block_on(run(receiver, handle)) {
                    eprintln!("rex-linux: desktop notifications unavailable: {error}");
                }
            });
        Self { notes }
    }

    pub fn send(&self, note: Note) {
        let _ = self.notes.send(note);
    }
}

async fn run(notes: flume::Receiver<Note>, handle: AppHandle<Workspace>) -> zbus::Result<()> {
    let conn = Connection::session().await?;
    let proxy = Proxy::new(&conn, DEST, PATH, DEST).await?;
    // One stream keeps ActivationToken ahead of the ActionInvoked it belongs to.
    let mut signals = proxy.receive_all_signals().await?;
    let mut targets: HashMap<u32, NoteTarget> = HashMap::new();
    let mut order: VecDeque<u32> = VecDeque::new();
    let mut tokens: HashMap<u32, String> = HashMap::new();
    loop {
        tokio::select! {
            note = notes.recv_async() => {
                let Ok(note) = note else { return Ok(()) };
                let hints = HashMap::from([("desktop-entry", Variant::from("dev.shuv.rex-linux"))]);
                let id: u32 = proxy
                    .call(
                        "Notify",
                        &("Rex", 0u32, "dev.shuv.rex-linux", &note.summary, &note.body,
                          vec!["default", "Show"], hints, -1i32),
                    )
                    .await?;
                targets.insert(id, note.target);
                order.push_back(id);
                while order.len() > REMEMBERED {
                    if let Some(old) = order.pop_front() {
                        targets.remove(&old);
                        tokens.remove(&old);
                    }
                }
            }
            signal = signals.next() => {
                let Some(signal) = signal else { return Ok(()) };
                let header = signal.header();
                match header.member().map(|m| m.as_str()) {
                    Some("ActivationToken") => {
                        if let Ok((id, token)) = signal.body().deserialize::<(u32, String)>()
                            && targets.contains_key(&id)
                        {
                            tokens.insert(id, token);
                        }
                    }
                    Some("ActionInvoked") => {
                        if let Ok((id, _action)) = signal.body().deserialize::<(u32, String)>()
                            && let Some(target) = targets.remove(&id)
                        {
                            let token = tokens.remove(&id);
                            handle.update(move |s, tx| s.open_note(target, token, tx));
                        }
                    }
                    Some("NotificationClosed") => {
                        if let Ok((id, _reason)) = signal.body().deserialize::<(u32, u32)>() {
                            targets.remove(&id);
                            tokens.remove(&id);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
