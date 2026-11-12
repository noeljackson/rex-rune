//! Native window identities, shared preference deltas and window-local lifecycle.
use super::{
    fonts::Fonts,
    workspace::{HostId, Workspace},
};
use rex_linux::{
    connection::{
        HostCommand, Options,
        perform::{MAX_WAIT, Outcome, Pending},
    },
    settings::{RemoteAccess, Settings},
};
use rune::{AppHandle, Tx, WindowHandle, WindowSpec};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    time::Instant,
};

type NoteKey = (String, String, Option<String>, String, String);
type Foreground = (u64, String, Option<String>, Option<String>);

pub struct Desktop {
    pub settings: Settings,
    pub fonts: Fonts,
    pub launch_font: String,
    pub saved_font: String,
    next: u64,
    windows: HashMap<u64, AppHandle<Workspace>>,
    pub grants: Vec<(String, Weak<AtomicU64>)>,
    revision: u64,
    base_label: String,
    foreground: Option<Foreground>,
    notes: HashMap<NoteKey, (u64, Instant)>,
}

impl Desktop {
    pub fn new(settings: Settings, fonts: Fonts) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            base_label: settings.client_label(),
            launch_font: settings.font_family.clone(),
            saved_font: settings.font_family.clone(),
            settings,
            fonts,
            next: 2,
            windows: HashMap::new(),
            grants: Vec::new(),
            revision: 0,
            foreground: None,
            notes: HashMap::new(),
        }))
    }
}

pub struct WindowState {
    pub shared: Arc<Mutex<Desktop>>,
    pub id: u64,
    pub label: String,
    pub initially_focused: bool,
    baseline: Value,
    baseline_shortcuts: rex_linux::keymap::Overrides,
    target: Option<(String, String)>,
    revision: u64,
    status: Arc<AtomicU8>,
    native: Option<WindowHandle>,
    pub pending: Option<Opening>,
    pub close_reply: Option<(HostId, u64, String)>,
}

pub struct Opening {
    host: HostId,
    pending: Pending,
    status: Arc<AtomicU8>,
    native: WindowHandle,
    detach: bool,
}

impl WindowState {
    pub(super) fn initial_tabs(&self) -> HashMap<String, String> {
        self.target.clone().into_iter().collect()
    }
    pub fn first(shared: Arc<Mutex<Desktop>>) -> Self {
        Self::new(shared, 1)
    }
    fn new(shared: Arc<Mutex<Desktop>>, id: u64) -> Self {
        let d = shared.lock().expect("desktop");
        let label = if id == 1 {
            d.base_label.clone()
        } else {
            format!("{}/window-{id}", d.base_label)
        };
        let baseline = serde_json::to_value(&d.settings).expect("settings");
        let baseline_shortcuts = d.settings.shortcut_file.overrides.clone();
        let revision = d.revision;
        drop(d);
        Self {
            shared,
            id,
            label,
            initially_focused: true,
            baseline,
            baseline_shortcuts,
            target: None,
            revision,
            status: Arc::new(AtomicU8::new(0)),
            native: None,
            pending: None,
            close_reply: None,
        }
    }
}

pub fn window(
    options: Options,
    settings: Settings,
    path: PathBuf,
    mut state: WindowState,
    tab: Option<(String, String)>,
    inactive: bool,
) -> WindowSpec<Workspace> {
    state.target = tab.clone();
    state.initially_focused = !inactive;
    let id = state.id;
    let shared = state.shared.clone();
    let app = super::build_app(options, settings, path, state);
    let handle = app.handle();
    shared
        .lock()
        .expect("desktop")
        .windows
        .insert(id, handle.clone());
    let spec = WindowSpec::from_app(format!("window-{id}"), app).on_created(move |native| {
        handle.update(move |s, _| {
            let d = s.desktop.as_mut().expect("desktop window");
            d.native = Some(native);
            d.status.fetch_or(1, Ordering::AcqRel);
        });
    });
    if inactive { spec.inactive() } else { spec }
}

pub fn closed(s: &mut Workspace, _: &mut Tx) {
    if let Some(d) = &s.desktop {
        d.status.store(0, Ordering::Release);
        let mut shared = d.shared.lock().expect("desktop");
        shared.windows.remove(&d.id);
        if shared.foreground.as_ref().is_some_and(|f| f.0 == d.id) {
            shared.foreground = None;
        }
    }
}

/// Three-way preference merge: unchanged fields never overwrite another window's save.
fn merge(base: &Value, local: &Value, latest: &mut Value) {
    if base == local {
        return;
    }
    if let (Some(base), Some(local), Some(latest)) =
        (base.as_object(), local.as_object(), latest.as_object_mut())
    {
        for key in base.keys().chain(local.keys()) {
            let before = base.get(key).unwrap_or(&Value::Null);
            let after = local.get(key).unwrap_or(&Value::Null);
            if before == after {
                continue;
            }
            if key == "hosts" {
                merge_hosts(
                    before,
                    after,
                    latest.entry(key.clone()).or_insert(Value::Null),
                );
            } else if key == "muted_tabs" {
                let base = before.as_array().cloned().unwrap_or_default();
                let local = after.as_array().cloned().unwrap_or_default();
                let value = latest.entry(key.clone()).or_insert(json!([]));
                let mut values = value.as_array().cloned().unwrap_or_default();
                values.retain(|v| !base.contains(v) || local.contains(v));
                for v in local {
                    if !base.contains(&v) && !values.contains(&v) {
                        values.push(v);
                    }
                }
                *value = Value::Array(values);
            } else if after.is_null() {
                latest.remove(key);
            } else {
                merge(
                    before,
                    after,
                    latest.entry(key.clone()).or_insert(Value::Null),
                );
            }
        }
    } else {
        *latest = local.clone();
    }
}

fn merge_hosts(base: &Value, local: &Value, latest: &mut Value) {
    let (Some(base), Some(local), Some(latest)) =
        (base.as_array(), local.as_array(), latest.as_array_mut())
    else {
        *latest = local.clone();
        return;
    };
    for old in base {
        if !local.iter().any(|h| h["endpoint"] == old["endpoint"]) {
            latest.retain(|h| h["endpoint"] != old["endpoint"]);
        }
    }
    for host in local {
        let old = base.iter().find(|h| h["endpoint"] == host["endpoint"]);
        if let Some(target) = latest
            .iter_mut()
            .find(|h| h["endpoint"] == host["endpoint"])
        {
            merge(old.unwrap_or(&Value::Null), host, target);
        } else if old.is_none() {
            latest.push(host.clone());
        }
    }
}

impl Workspace {
    pub(super) fn record_desktop_focus(&self) {
        let Some(d) = &self.desktop else {
            return;
        };
        let mut shared = d.shared.lock().expect("desktop");
        if self.window_focused {
            shared.foreground = Some((
                d.id,
                self.settings.hosts[self.active].endpoint.clone(),
                self.model().selected.clone(),
                self.model().window().and_then(|w| w.window_id.clone()),
            ));
        } else if shared.foreground.as_ref().is_some_and(|f| f.0 == d.id) {
            shared.foreground = None;
        }
    }

    pub(super) fn allow_desktop_note(&self, note: &super::notify::Note) -> bool {
        let Some(d) = &self.desktop else {
            return true;
        };
        let Some(host) = self.host_index(note.target.host) else {
            return false;
        };
        let endpoint = &self.settings.hosts[host].endpoint;
        let mut shared = d.shared.lock().expect("desktop");
        if shared.foreground.as_ref().is_some_and(|(_, e, s, w)| {
            e == endpoint && s.as_ref() == Some(&note.target.session) && w == &note.target.window
        }) {
            return false;
        }
        let key = (
            endpoint.clone(),
            note.target.session.clone(),
            note.target.window.clone(),
            note.summary.clone(),
            note.body.clone(),
        );
        shared
            .notes
            .retain(|_, (_, at)| at.elapsed() < std::time::Duration::from_secs(1));
        if shared
            .notes
            .get(&key)
            .is_some_and(|(window, _)| *window != d.id)
        {
            return false;
        }
        if shared.notes.len() >= 128 {
            let oldest = shared
                .notes
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest {
                shared.notes.remove(&key);
            }
        }
        shared.notes.insert(key, (d.id, Instant::now()));
        true
    }

    pub(super) fn remote_access(&self, host: usize) -> RemoteAccess {
        let endpoint = &self.settings.hosts[host].endpoint;
        self.desktop.as_ref().map_or_else(
            || self.settings.hosts[host].remote_access(),
            |d| {
                d.shared
                    .lock()
                    .expect("desktop")
                    .settings
                    .hosts
                    .iter()
                    .find(|h| &h.endpoint == endpoint)
                    .map_or(RemoteAccess::Off, |h| h.remote_access())
            },
        )
    }

    pub(super) fn share_preferences(&mut self) {
        let Some(d) = &mut self.desktop else {
            return;
        };
        let local = serde_json::to_value(&self.settings).expect("settings");
        let mut shared = d.shared.lock().expect("desktop");
        let mut latest = serde_json::to_value(&shared.settings).expect("settings");
        merge(&d.baseline, &local, &mut latest);
        let mut settings: Settings =
            serde_json::from_value(latest.clone()).expect("merged settings");
        settings.shortcut_file = if self.settings.shortcut_file.overrides != d.baseline_shortcuts {
            self.settings.shortcut_file.clone()
        } else {
            shared.settings.shortcut_file.clone()
        };
        let revoked: Vec<_> = shared
            .settings
            .hosts
            .iter()
            .filter(|old| {
                settings
                    .hosts
                    .iter()
                    .find(|h| h.endpoint == old.endpoint)
                    .is_none_or(|new| new.remote_access() != old.remote_access())
            })
            .map(|h| h.endpoint.clone())
            .collect();
        shared.grants.retain(|(_, grant)| grant.strong_count() > 0);
        for (endpoint, grant) in &shared.grants {
            if revoked.contains(endpoint)
                && let Some(grant) = grant.upgrade()
            {
                grant.fetch_add(1, Ordering::AcqRel);
            }
        }
        shared.settings = settings.clone();
        shared.revision += 1;
        let revision = shared.revision;
        let listeners: Vec<_> = shared
            .windows
            .iter()
            .filter(|(id, _)| **id != d.id)
            .map(|(_, h)| h.clone())
            .collect();
        drop(shared);
        d.baseline = latest;
        d.revision = revision;
        self.apply_preferences(settings, revision);
        for listener in listeners {
            let settings = self.settings.clone();
            listener.update(move |s, _| s.apply_preferences(settings, revision));
        }
    }

    fn apply_preferences(&mut self, settings: Settings, revision: u64) {
        let Some(d) = &mut self.desktop else {
            return;
        };
        if revision < d.revision {
            return;
        }
        d.revision = revision;
        d.baseline = serde_json::to_value(&settings).expect("settings");
        d.baseline_shortcuts = settings.shortcut_file.overrides.clone();
        let active = self
            .settings
            .hosts
            .get(self.active)
            .map(|h| h.endpoint.clone());
        let old_endpoints: Vec<_> = self
            .settings
            .hosts
            .iter()
            .map(|h| h.endpoint.clone())
            .collect();
        let hosts_changed = old_endpoints
            != settings
                .hosts
                .iter()
                .map(|h| h.endpoint.clone())
                .collect::<Vec<_>>();
        let keymap_changed =
            self.settings.shortcut_file.overrides != settings.shortcut_file.overrides;
        let theme_changed = self.settings.terminal_theme != settings.terminal_theme
            || self.settings.chrome_theme != settings.chrome_theme;
        self.settings = settings;
        if hosts_changed {
            self.navigation.cancel();
            self.panel = None;
            self.drag = None;
            let mut states: HashMap<_, _> = old_endpoints
                .into_iter()
                .zip(std::mem::take(&mut self.hosts))
                .collect();
            self.active = active
                .and_then(|endpoint| {
                    self.settings
                        .hosts
                        .iter()
                        .position(|h| h.endpoint == endpoint)
                })
                .unwrap_or(0);
            for index in 0..self.settings.hosts.len() {
                let state = states
                    .remove(&self.settings.hosts[index].endpoint)
                    .unwrap_or_else(|| self.spawn_host(index, false));
                self.hosts.push(state);
            }
            for (_, state) in states {
                self.forget_host(state);
            }
            self.update_visibility();
        }
        if keymap_changed {
            self.local_keymap = super::actions::resolve(&self.settings.shortcut_file.overrides);
            self.keymap_generation += 1;
            self.bindings.sequence.clear();
            let declaration = self.keymap_declaration();
            for index in 0..self.hosts.len() {
                self.send(index, HostCommand::Keymap(Box::new(declaration.clone())));
            }
        }
        if theme_changed {
            if let Ok(theme) = self
                .theme_dirs
                .resolve(&self.settings.terminal_theme, self.settings.chrome_theme)
            {
                self.term_theme = theme;
            }
            self.declare_theme();
        }
        self.sync_prefetch();
    }

    pub(super) fn open_desktop(
        &mut self,
        detach: bool,
        remote: Option<(usize, Pending)>,
        tx: &mut Tx,
    ) {
        let Some(desktop) = &self.desktop else {
            return;
        };
        let shared = desktop.shared.clone();
        let mut options = self.options.clone();
        options.new = None;
        options.block = None;
        options.rune.clear();
        options.font_size = None;
        options.endpoint = self.settings.hosts[self.active].endpoint.clone();
        options.session = self.model().selected.clone();
        let tab = options
            .session
            .clone()
            .zip(self.model().window().and_then(|w| w.window_id.clone()));
        let (id, settings) = {
            let mut d = shared.lock().expect("desktop");
            let id = d.next;
            d.next += 1;
            (id, d.settings.clone())
        };
        let child = WindowState::new(shared, id);
        let status = child.status.clone();
        let native = rune::open_window(window(
            options,
            settings,
            self.settings_path.clone(),
            child,
            tab,
            remote.is_some(),
        ));
        if let Some((host, pending)) = remote {
            self.desktop.as_mut().unwrap().pending = Some(Opening {
                host: self.hosts[host].id,
                pending,
                status,
                native,
                detach,
            });
            tx.wake_after(std::time::Duration::from_millis(30));
        } else if detach {
            self.leave_detached_tab(tx);
        }
    }

    fn leave_detached_tab(&mut self, tx: &mut Tx) {
        let current = self.model().window().and_then(|w| w.window_id.as_ref());
        let other = self.model().view().and_then(|v| {
            v.windows.iter().find_map(|w| {
                w.window_id
                    .as_ref()
                    .filter(|id| Some(*id) != current)
                    .cloned()
            })
        });
        if let Some(other) = other {
            self.perform(super::panel::Intent::Tab(other), tx);
        }
        // The final tab remains a second view: detaching never creates or destroys programs.
    }

    pub(super) fn poll_desktop(&mut self, now: Instant, tx: &mut Tx) -> bool {
        self.record_desktop_focus();
        if let Some(d) = &self.desktop {
            let connected = self.hosts.get(self.active).is_some_and(|h| {
                h.model.connected
                    && (d.target.is_none()
                        || h.model.window().is_some_and(|w| {
                            d.target.as_ref().is_none_or(|(session, tab)| {
                                h.model.selected.as_ref() == Some(session)
                                    && w.window_id.as_ref() == Some(tab)
                            })
                        }))
            });
            if connected {
                d.status.fetch_or(2, Ordering::AcqRel);
            } else {
                d.status.fetch_and(!2, Ordering::AcqRel);
            }
        }
        let Some(d) = &mut self.desktop else {
            return false;
        };
        if let Some((host, epoch, call)) = &d.close_reply {
            let result = self
                .hosts
                .iter()
                .find(|h| h.id == *host)
                .and_then(|h| {
                    (h.model.connection_epoch == *epoch).then_some(&h.model.desktop_reply)
                })
                .and_then(Option::as_ref);
            if result.is_some_and(|(id, ok)| id == call && *ok) {
                tx.close_window();
                d.close_reply = None;
                return true;
            }
            if result.is_some_and(|(id, _)| id == call)
                || !self.hosts.iter().any(|h| {
                    h.id == *host && h.model.connected && h.model.connection_epoch == *epoch
                })
            {
                d.close_reply = None;
                self.inform(
                    "The close reply was not acknowledged. This desktop window remains open.",
                );
                return true;
            }
        }
        let Some(opening) = d.pending.take() else {
            return false;
        };
        let host = self.host_index(opening.host);
        let valid = host.is_some_and(|h| {
            self.remote_access(h) == RemoteAccess::Workspace
                && self.hosts[h].model.connection_epoch == opening.pending.epoch
                && self.hosts[h].model.connected
        });
        let ready = opening.status.load(Ordering::Acquire) == 3;
        if !valid || now >= opening.pending.received + MAX_WAIT {
            opening.native.close();
            if let Some(host) = host {
                self.remote_reply(host, opening.pending, Outcome::error("timeout", "Window opening expired or lost its connection/consent; the new view was closed. Server programs keep running."));
            }
            return true;
        }
        if ready {
            if opening.detach {
                self.remote_dispatch = Some(Vec::new());
                self.leave_detached_tab(tx);
                let commands = self.remote_dispatch.take().unwrap();
                if !commands.is_empty() {
                    self.dispatch_remote_commands(host.unwrap(), opening.pending, commands);
                    return true;
                }
            }
            self.remote_reply(host.unwrap(), opening.pending, Outcome::Performed);
            return true;
        }
        self.desktop.as_mut().unwrap().pending = Some(opening);
        tx.wake_after(std::time::Duration::from_millis(30));
        false
    }

    pub(super) fn remote_desktop(&mut self, host: usize, pending: Pending, tx: &mut Tx) {
        match pending.request.action.as_str() {
            "client.window.new" => self.open_desktop(false, Some((host, pending)), tx),
            "client.tab.move_to_new_window" => self.open_desktop(true, Some((host, pending)), tx),
            "client.window.close" | "client.close" => {
                self.desktop.as_mut().unwrap().close_reply = Some((
                    self.hosts[host].id,
                    pending.epoch,
                    pending.request.call_id.clone(),
                ));
                self.send(
                    host,
                    HostCommand::DesktopCloseReply {
                        epoch: pending.epoch,
                        call_id: pending.request.call_id,
                        consent: self.hosts[host].consent_generation.clone(),
                        generation: self.hosts[host].consent_generation.load(Ordering::Acquire),
                        deadline: pending.received + MAX_WAIT,
                    },
                );
            }
            "client.window.fullscreen.toggle" => {
                tx.toggle_full_screen();
                self.remote_reply(host, pending, Outcome::Performed);
            }
            _ => unreachable!(),
        }
    }

    /// Resolve helpers against this window's pinned tab; server active-tab state can
    /// change between requests from independently connected desktop windows.
    pub(super) fn desktop_focus(&self, method: &str, payload: &Value) -> Option<(String, Value)> {
        self.desktop.as_ref()?;
        let window = self.model().window()?;
        if matches!(
            method,
            "session.focus_next_window" | "session.focus_previous_window"
        ) {
            let view = self.model().view()?;
            let index = view
                .windows
                .iter()
                .position(|w| w.window_id == window.window_id)?;
            let next = if method == "session.focus_next_window" {
                (index + 1).min(view.windows.len() - 1)
            } else {
                index.saturating_sub(1)
            };
            return Some((
                "session.focus_window".into(),
                json!({"window_id": view.windows[next].window_id}),
            ));
        }
        let (panes, _) = rex_linux::layout::placements(window, None);
        let block = self.focused_block()?;
        let index = panes.iter().position(|p| p.block == block)?;
        let wrap = payload["wrap"].as_bool().unwrap_or(false);
        let target = if matches!(
            method,
            "session.focus_next_pane" | "session.focus_previous_pane"
        ) {
            let count = panes.len();
            let next = if method == "session.focus_next_pane" {
                if index + 1 < count {
                    index + 1
                } else if wrap {
                    0
                } else {
                    index
                }
            } else if index > 0 {
                index - 1
            } else if wrap {
                count - 1
            } else {
                index
            };
            panes[next].block.clone()
        } else if method == "session.focus_direction" {
            let rect = panes[index].rect;
            let center = [rect[0] + rect[2] / 2., rect[1] + rect[3] / 2.];
            let direction = payload["direction"].as_str()?;
            let axis = usize::from(matches!(direction, "up" | "down"));
            let sign = if matches!(direction, "left" | "up") {
                -1.
            } else {
                1.
            };
            panes
                .iter()
                .filter(|p| p.block != block)
                .filter_map(|p| {
                    let c = [p.rect[0] + p.rect[2] / 2., p.rect[1] + p.rect[3] / 2.];
                    let mut distance = sign * (c[axis] - center[axis]);
                    if distance <= 0. {
                        if wrap {
                            distance += 1.;
                        } else {
                            return None;
                        }
                    }
                    Some((distance + (c[1 - axis] - center[1 - axis]).abs() * 2., p))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map_or(block.clone(), |(_, p)| p.block.clone())
        } else {
            return None;
        };
        Some(("session.focus_block".into(), json!({"block_id":target})))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn fixture() -> (Workspace, Arc<Mutex<Desktop>>) {
        let options = Options::parse_from([
            "rex-linux",
            "--endpoint",
            "unix:///nonexistent/desktop-test.sock",
        ]);
        let mut settings = Settings::default();
        settings.hosts = vec![rex_linux::settings::Host {
            label: "test".into(),
            endpoint: options.endpoint.clone(),
            remote_control: true,
            workspace_control: true,
            collapsed: false,
            shell: None,
        }];
        let fonts = Fonts {
            files: &[],
            paths: &[],
            families: ("sans-serif", "monospace"),
            terminal: None,
        };
        let shared = Desktop::new(settings.clone(), fonts);
        let app = rune::App::new(rune::AppSpec::new(
            "test",
            (800., 600.),
            super::super::placeholder,
            super::super::view,
        ));
        let state = WindowState::first(shared.clone());
        let workspace = Workspace::new_desktop(
            options,
            settings,
            std::env::temp_dir().join("desktop-unsaved.json"),
            app.handle(),
            None,
            Some(state),
        );
        (workspace, shared)
    }

    #[test]
    fn stale_unrelated_save_keeps_revocation_and_each_windows_changes() {
        let base = json!({"hosts":[{"endpoint":"a", "remote_control":true,"workspace_control":true}], "font_size":14, "sessions":{"a":"one"}});
        let local = json!({"hosts":[{"endpoint":"a", "remote_control":true,"workspace_control":true}], "font_size":18, "sessions":{"a":"one"}});
        let mut latest = json!({"hosts":[{"endpoint":"a", "remote_control":false}], "font_size":14, "sessions":{"a":"two"}});
        merge(&base, &local, &mut latest);
        assert_eq!(
            latest["hosts"][0],
            json!({"endpoint":"a", "remote_control":false})
        );
        assert_eq!(latest["font_size"], 18);
        assert_eq!(latest["sessions"]["a"], "two");
    }

    #[test]
    fn independent_mute_changes_merge_and_unmuting_does_not_erase_new_mutes() {
        let base = json!({"muted_tabs":["a"]});
        let local = json!({"muted_tabs":[]});
        let mut latest = json!({"muted_tabs":["a","b"]});
        merge(&base, &local, &mut latest);
        assert_eq!(latest["muted_tabs"], json!(["b"]));
    }

    #[test]
    fn a_stale_window_cannot_overwrite_another_windows_reloaded_shortcuts() {
        let (mut ws, shared) = fixture();
        let overrides = rex_linux::keymap::Overrides::parse(
            &json!([{"keys":"ctrl+f9","action":"client.window.new"}]),
        );
        shared.lock().unwrap().settings.shortcut_file.overrides = overrides.clone();
        ws.settings.font_size = 19.;
        ws.share_preferences();
        assert_eq!(ws.settings.shortcut_file.overrides, overrides);
        assert_eq!(
            shared.lock().unwrap().settings.shortcut_file.overrides,
            overrides
        );
    }

    #[test]
    fn notifications_are_deduplicated_across_windows_and_suppressed_in_the_foreground_view() {
        let (first, shared) = fixture();
        let (mut second, _) = fixture();
        second.desktop = Some(WindowState::new(shared.clone(), 2));
        let note = super::super::notify::Note {
            summary: "Done".into(),
            body: "fixture".into(),
            target: super::super::notify::NoteTarget {
                host: first.hosts[0].id,
                session: "s".into(),
                window: Some("tab".into()),
            },
        };
        assert!(first.allow_desktop_note(&note));
        assert!(!second.allow_desktop_note(&note));
        assert!(
            first.allow_desktop_note(&note),
            "distinct repeats from the same source are still delivered"
        );
        shared.lock().unwrap().foreground = Some((
            2,
            first.settings.hosts[0].endpoint.clone(),
            Some("s".into()),
            Some("tab".into()),
        ));
        assert!(!first.allow_desktop_note(&note));
        assert!(!second.allow_desktop_note(&note));
    }

    #[test]
    fn host_removal_wins_over_stale_host_edit_without_resurrection() {
        let base = json!({"hosts":[{"endpoint":"a", "label":"old"},{"endpoint":"b"}]});
        let local = json!({"hosts":[{"endpoint":"a", "label":"new"},{"endpoint":"b"}]});
        let mut latest = json!({"hosts":[{"endpoint":"b"},{"endpoint":"c"}]});
        merge(&base, &local, &mut latest);
        assert_eq!(latest, json!({"hosts":[{"endpoint":"b"},{"endpoint":"c"}]}));
    }

    #[test]
    fn consent_revocation_invalidates_other_windows_before_their_ui_update() {
        let (mut ws, shared) = fixture();
        let other = Arc::new(AtomicU64::new(5));
        shared.lock().unwrap().grants.push((
            ws.settings.hosts[0].endpoint.clone(),
            Arc::downgrade(&other),
        ));
        ws.settings.hosts[0].set_remote_access(RemoteAccess::Off);
        ws.share_preferences();
        assert_eq!(other.load(Ordering::Acquire), 6);
        // A different workspace can still have its old UI snapshot; checks use the shared grant.
        ws.settings.hosts[0].set_remote_access(RemoteAccess::Workspace);
        assert_eq!(ws.remote_access(0), RemoteAccess::Off);
    }

    #[test]
    fn labels_are_stable_and_closing_one_window_does_not_remove_another() {
        let (mut ws, shared) = fixture();
        let second = WindowState::new(shared.clone(), 2);
        assert_eq!(
            second.label,
            format!("{}/window-2", ws.settings.client_label())
        );
        shared.lock().unwrap().windows.insert(1, ws.handle.clone());
        shared.lock().unwrap().windows.insert(2, ws.handle.clone());
        closed(&mut ws, &mut Tx::default());
        assert!(!shared.lock().unwrap().windows.contains_key(&1));
        assert!(shared.lock().unwrap().windows.contains_key(&2));
    }

    #[test]
    fn an_older_broadcast_cannot_restore_old_consent_or_preferences() {
        let (mut ws, _) = fixture();
        let stale = ws.settings.clone();
        ws.settings.hosts[0].set_remote_access(RemoteAccess::Off);
        ws.settings.font_size = 20.;
        ws.share_preferences();
        ws.apply_preferences(stale, 0);
        assert_eq!(ws.settings.hosts[0].remote_access(), RemoteAccess::Off);
        assert_eq!(ws.settings.font_size, 20.);
    }

    #[test]
    fn an_inactive_window_starts_without_claiming_panes_before_native_focus() {
        let (ws, shared) = fixture();
        let mut state = WindowState::new(shared, 2);
        state.initially_focused = false;
        let mut inactive = Workspace::new_desktop(
            ws.options.clone(),
            ws.settings.clone(),
            ws.settings_path.clone(),
            ws.handle.clone(),
            None,
            Some(state),
        );
        assert!(!inactive.window_focused);
        inactive.claim_visible();
        assert!(inactive.claimed.is_empty());
        inactive.window_focus(true);
        assert!(inactive.window_focused);
    }
}
