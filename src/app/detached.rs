//! Client-only ownership of detached server tabs. Never starts or closes a server program.
use super::*;
use crate::app::{
    tabs::TabTarget,
    theme::*,
    widgets::{Pill, pill},
};
use rune::{Drag, El, Phase, PointerButton, col, el, text};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct TabKey {
    pub endpoint: String,
    pub session: String,
    pub tab: String,
}

pub(super) struct LocalOpening {
    pub key: TabKey,
    pub host: HostId,
    pub epoch: u64,
    pub status: Arc<AtomicU8>,
    pub native: WindowHandle,
    pub child: u64,
    pub deadline: Instant,
}

pub(crate) struct TabDrag {
    target: TabTarget,
    host: HostId,
    epoch: u64,
    origin: (HostId, Option<String>, Option<String>),
    sidebar: bool,
    preview: bool,
}

pub(super) fn broadcast(shared: &Arc<Mutex<Desktop>>) {
    let windows: Vec<_> = shared
        .lock()
        .expect("desktop")
        .windows
        .values()
        .cloned()
        .collect();
    for window in windows {
        window.update(|s, tx| {
            s.refresh_detached_visibility();
            tx.instant();
        });
    }
}

impl Workspace {
    pub(crate) fn terminal_window(&self) -> bool {
        self.desktop.as_ref().is_some_and(|d| d.terminal.is_some())
    }

    pub(crate) fn terminal_contents_available(&self) -> bool {
        self.desktop
            .as_ref()
            .and_then(|d| d.terminal.as_ref())
            .is_none_or(|key| {
                self.settings.hosts[self.active].endpoint == key.endpoint
                    && self.model().selected.as_ref() == Some(&key.session)
                    && self.model().window().and_then(|w| w.window_id.as_ref()) == Some(&key.tab)
            })
    }

    pub(crate) fn terminal_action_allowed(&self, name: &str) -> bool {
        if !self.terminal_window() {
            return true;
        }
        if !self.terminal_contents_available()
            && (name.starts_with("pane.")
                || name.starts_with("client.tab.")
                || name == "client.close")
        {
            return false;
        }
        if name.starts_with("client.tab.") {
            return matches!(
                name,
                "client.tab.rename"
                    | "client.tab.close"
                    | "client.tab.mute"
                    | "client.tab.go_to_window"
                    | "client.tab.restore_to_window"
            );
        }
        !name.starts_with("client.host.")
            && !name.starts_with("client.history.")
            && !name.starts_with("client.sidebar.")
            && !name.starts_with("session.")
    }

    pub(crate) fn terminal_visit_allowed(&self, visit: &crate::app::navigation::Visit) -> bool {
        self.desktop
            .as_ref()
            .and_then(|d| d.terminal.as_ref())
            .is_none_or(|key| {
                self.host_index(visit.host)
                    .is_some_and(|host| self.settings.hosts[host].endpoint == key.endpoint)
                    && visit.session == key.session
                    && visit.window.as_ref() == Some(&key.tab)
            })
    }

    pub(super) fn detach_key(&self, target: &TabTarget) -> Option<TabKey> {
        let host = self.hosts.get(target.host)?;
        if !host.model.connected
            || self
                .tab_target(target.host, &target.session, &target.window)
                .is_none()
        {
            return None;
        }
        Some(TabKey {
            endpoint: self.settings.hosts[target.host].endpoint.clone(),
            session: target.session.clone(),
            tab: target.window.clone(),
        })
    }

    pub(crate) fn can_detach_tab(&self, target: &TabTarget) -> bool {
        let Some(d) = &self.desktop else {
            return false;
        };
        let Some(key) = self.detach_key(target) else {
            return false;
        };
        d.pending.is_none()
            && d.local_opening.is_none()
            && d.shared
                .lock()
                .expect("desktop")
                .detached
                .get(&key)
                .is_none_or(|owner| *owner == d.id)
    }

    fn current_tab_key(&self) -> Option<TabKey> {
        self.detach_key(&self.active_tab()?)
    }

    fn held_tab(&self) -> Option<(TabKey, u64)> {
        let d = self.desktop.as_ref()?;
        let key = self.current_tab_key()?;
        let shared = d.shared.lock().expect("desktop");
        let owner = *shared.detached.get(&key)?;
        (owner != d.id && shared.windows.contains_key(&owner)).then_some((key, owner))
    }

    pub(crate) fn pane_in_detached_window(&self, target: &crate::app::panel::PaneTarget) -> bool {
        let Some(d) = &self.desktop else {
            return false;
        };
        let Some(tab) = self.pane_window(target).and_then(|w| w.window_id.as_ref()) else {
            return false;
        };
        let key = TabKey {
            endpoint: self.settings.hosts[target.host].endpoint.clone(),
            session: target.session.clone(),
            tab: tab.clone(),
        };
        let shared = d.shared.lock().expect("desktop");
        shared
            .detached
            .get(&key)
            .is_some_and(|owner| *owner != d.id && shared.windows.contains_key(owner))
    }

    pub(crate) fn detached_here(&self) -> bool {
        self.held_tab().is_some()
    }

    pub(crate) fn refresh_detached_visibility(&mut self) -> bool {
        let held = self.detached_here();
        let Some(d) = &mut self.desktop else {
            return false;
        };
        if d.placeholder == held {
            return false;
        }
        d.placeholder = held;
        self.bindings.sequence.clear();
        self.cancel_tab_drag();
        self.floating_drag = None;
        self.drag = None;
        self.update_visibility();
        self.claimed.clear();
        if held {
            self.forwarded.clear();
            self.bindings.consumed.clear();
        } else {
            self.claim_visible();
        }
        self.record_desktop_focus();
        true
    }

    fn valid_detached_key(&self, key: &TabKey) -> bool {
        self.settings
            .hosts
            .iter()
            .position(|h| h.endpoint == key.endpoint)
            .is_some_and(|host| {
                self.hosts[host].model.connected
                    && self.tab_target(host, &key.session, &key.tab).is_some()
            })
    }

    pub(super) fn publish_detached(&mut self, key: TabKey, owner: u64, tx: &mut Tx) -> bool {
        if !self.valid_detached_key(&key) {
            return false;
        }
        let Some(d) = &self.desktop else {
            return false;
        };
        let shared = d.shared.clone();
        {
            let mut desktop = shared.lock().expect("desktop");
            if !desktop.windows.contains_key(&owner)
                || desktop.detached.get(&key).is_some_and(|old| *old != d.id)
            {
                return false;
            }
            desktop.detached.insert(key, owner);
        }
        self.refresh_detached_visibility();
        broadcast(&shared);
        tx.instant();
        true
    }

    pub(super) fn poll_detached(&mut self, now: Instant, tx: &mut Tx) -> bool {
        let Some(d) = &self.desktop else {
            return false;
        };
        let shared = d.shared.clone();
        let current = self.current_tab_key();
        if let Some(d) = &mut self.desktop
            && d.dedicated
                .as_ref()
                .is_some_and(|original| current.as_ref().is_some_and(|current| current != original))
        {
            d.dedicated = None;
        }
        let removed = {
            let mut desktop = shared.lock().expect("desktop");
            let before = desktop.detached.len();
            desktop.detached.retain(|key, _| {
                let Some(index) = self
                    .settings
                    .hosts
                    .iter()
                    .position(|h| h.endpoint == key.endpoint)
                else {
                    return false;
                };
                let model = &self.hosts[index].model;
                // Unknown views are not evidence of closure: another session may not be attached here.
                !model.connected
                    || (model.sessions.iter().any(|s| s.session_id == key.session)
                        && model.views.get(&key.session).is_none_or(|view| {
                            view.windows
                                .iter()
                                .any(|w| w.window_id.as_deref() == Some(&key.tab))
                        }))
            });
            before != desktop.detached.len()
        };
        if removed {
            broadcast(&shared);
        }
        let cancelled = self
            .tab_drag
            .as_ref()
            .is_some_and(|drag| !self.tab_drag_valid(drag));
        if cancelled {
            self.cancel_tab_drag();
        }
        let mut changed = removed | cancelled | self.refresh_detached_visibility();
        let opening = self.desktop.as_mut().and_then(|d| d.local_opening.take());
        if let Some(opening) = opening {
            let valid = self.host_index(opening.host).is_some_and(|h| {
                self.hosts[h].model.connected
                    && self.hosts[h].model.connection_epoch == opening.epoch
            }) && self.valid_detached_key(&opening.key);
            if !valid || now >= opening.deadline {
                opening.native.close();
                self.inform("Tab detachment did not finish; its original view and programs remain available.");
                changed = true;
            } else if opening.status.load(Ordering::Acquire) == 3 {
                if !self.publish_detached(opening.key, opening.child, tx) {
                    opening.native.close();
                }
                changed = true;
            } else {
                self.desktop.as_mut().unwrap().local_opening = Some(opening);
                tx.wake_after(std::time::Duration::from_millis(30));
            }
        }
        changed
    }

    pub(crate) fn go_to_detached(&mut self, tx: &mut Tx) {
        let Some((key, owner)) = self.held_tab() else {
            return;
        };
        self.go_to_detached_target(key, owner, tx);
    }

    fn go_to_detached_target(&mut self, key: TabKey, owner: u64, tx: &mut Tx) {
        if self.held_tab().as_ref() != Some(&(key.clone(), owner)) {
            return;
        }
        let d = self.desktop.as_ref().unwrap();
        let source = d.native.clone();
        let target = d
            .shared
            .lock()
            .expect("desktop")
            .windows
            .get(&owner)
            .cloned();
        if let Some(target) = target {
            target.update(move |s, tx| {
                if !s.desktop.as_ref().is_some_and(|d| {
                    let desktop = d.shared.lock().expect("desktop");
                    desktop.detached.get(&key) == Some(&owner) && d.id == owner
                }) || !s.valid_detached_key(&key)
                {
                    return;
                }
                let Some(host) = s
                    .settings
                    .hosts
                    .iter()
                    .position(|h| h.endpoint == key.endpoint)
                else {
                    return;
                };
                if s.valid_detached_key(&key) && s.panel.is_none() {
                    s.select_window(host, key.session.clone(), key.tab.clone(), tx);
                }
                if let Some(native) = s.desktop.as_ref().and_then(|d| d.native.as_ref()) {
                    if let Some(source) = source {
                        native.focus_from(&source);
                    } else {
                        native.focus();
                    }
                }
            });
        }
        tx.blur();
    }

    pub(crate) fn restore_detached(&mut self, tx: &mut Tx) {
        let Some((key, owner)) = self.held_tab() else {
            return;
        };
        self.restore_detached_target(key, owner, tx);
    }

    pub(super) fn restore_detached_target(&mut self, key: TabKey, owner: u64, tx: &mut Tx) {
        if self.held_tab().as_ref() != Some(&(key.clone(), owner)) {
            return;
        }
        let restored_to = self.desktop.as_ref().unwrap().id;
        let shared = self.desktop.as_ref().unwrap().shared.clone();
        let child = {
            let mut d = shared.lock().expect("desktop");
            if d.detached.get(&key) != Some(&owner) {
                return;
            }
            d.detached.insert(key.clone(), restored_to);
            d.windows.get(&owner).cloned()
        };
        self.refresh_detached_visibility();
        broadcast(&shared);
        if let Some(child) = child {
            child.update(move |s, tx| {
                // Close only the dedicated view still showing this tab, never unrelated work
                // or a window with an active form/confirmation.
                if s.close_restored_destination(&key, restored_to) {
                    tx.close_window();
                }
            });
        }
        tx.blur();
        tx.instant();
    }

    pub(super) fn close_restored_destination(&self, key: &TabKey, restored_to: u64) -> bool {
        self.panel.is_none()
            && self.navigation.pending.is_none()
            && self.remote_operation.is_none()
            && self.remote_navigation.is_none()
            && self.remote_approval.is_none()
            && self.current_tab_key().as_ref() == Some(key)
            && self.desktop.as_ref().is_some_and(|d| {
                d.id != restored_to
                    && d.dedicated.as_ref() == Some(key)
                    && d.pending.is_none()
                    && d.local_opening.is_none()
                    && d.shared.lock().expect("desktop").detached.get(key) == Some(&restored_to)
            })
    }

    fn drag_origin(&self) -> (HostId, Option<String>, Option<String>) {
        (
            self.hosts[self.active].id,
            self.model().selected.clone(),
            self.model().window().and_then(|w| w.window_id.clone()),
        )
    }

    fn tab_drag_valid(&self, drag: &TabDrag) -> bool {
        self.window_focused
            && self.panel.is_none()
            && self.drag_origin() == drag.origin
            && self.host_index(drag.host).is_some_and(|h| {
                h == drag.target.host
                    && self.hosts[h].model.connected
                    && self.hosts[h].model.connection_epoch == drag.epoch
            })
            && self.detach_key(&drag.target).is_some()
    }

    pub(crate) fn cancel_tab_drag(&mut self) {
        self.tab_drag = None;
        // The native preview is cleared on release/blur too, including a stale captured row.
        if let Some(native) = self.desktop.as_ref().and_then(|d| d.native.as_ref()) {
            native.clear_drag_preview();
        }
    }

    pub(crate) fn drag_tab(&mut self, target: &TabTarget, gesture: &Drag, tx: &mut Tx) {
        self.drag_tab_from(target, gesture, false, tx);
    }

    pub(crate) fn drag_sidebar_tab(&mut self, target: &TabTarget, gesture: &Drag, tx: &mut Tx) {
        self.drag_tab_from(target, gesture, true, tx);
    }

    fn drag_tab_from(&mut self, target: &TabTarget, gesture: &Drag, sidebar: bool, tx: &mut Tx) {
        if gesture.phase == Phase::Down {
            self.tab_press_suppressed = false;
            self.cancel_tab_drag();
            if tx.button() != PointerButton::Primary
                || self.panel.is_some()
                || !self.can_detach_tab(target)
            {
                return;
            }
            self.tab_drag = Some(TabDrag {
                target: target.clone(),
                host: self.hosts[target.host].id,
                epoch: self.hosts[target.host].model.connection_epoch,
                origin: self.drag_origin(),
                sidebar,
                preview: false,
            });
        }
        if gesture.dx.hypot(gesture.dy) / self.ui_scale() >= 16. {
            self.tab_press_suppressed = true;
        }
        let Some(drag) = &self.tab_drag else {
            return;
        };
        if !self.tab_drag_valid(drag) || gesture.cancelled() {
            self.cancel_tab_drag();
            return;
        }
        let moved = gesture.dx.hypot(gesture.dy) / self.ui_scale() >= 16.;
        if gesture.phase == Phase::Move && moved && !drag.preview {
            let heading = if drag.sidebar {
                format!(
                    "sidebar-tab:{}:{}:{}",
                    target.host, target.session, target.window
                )
            } else {
                format!("tab:{}", target.window)
            };
            // Capture terminal pixels only when this is the tab actually on screen.
            let content =
                (self.current_tab_key() == self.detach_key(target)).then(|| "panes".to_owned());
            if let Some(native) = self.desktop.as_ref().and_then(|d| d.native.as_ref()) {
                native.drag_preview(content, heading);
            }
            self.tab_drag.as_mut().unwrap().preview = true;
        }
        if gesture.phase == Phase::Up {
            let drag = self.tab_drag.take().unwrap();
            self.cancel_tab_drag();
            let [x, y] = gesture.window_point().map(|v| v / self.ui_scale());
            let [width, height] = self.viewport.map(|v| v / self.ui_scale());
            let outside = if drag.sidebar {
                x > self.sidebar.width() + 12. || x < -12. || y < -12. || y > height + 12.
            } else {
                y > TOOLBAR + 12. || y < -12. || x < -12. || x > width + 12.
            };
            if outside && moved {
                self.open_desktop_target(Some(drag.target), None, tx);
            }
        }
    }
}

pub(crate) fn placeholder(s: &Workspace) -> Option<El> {
    let (key, owner) = s.held_tab()?;
    let (go_key, restore_key) = (key.clone(), key);
    Some(
        el().key("detached-placeholder")
            .grow(1.)
            .h_full()
            .min_w(0.)
            .center()
            .child(
                col()
                    .max_w(440.)
                    .px(28.)
                    .py(28.)
                    .gap(16.)
                    .center()
                    .child(
                        text("This tab is open in another window")
                            .font(medium(18.))
                            .color(rgb(ink())),
                    )
                    .child(
                        text("Its programs are still running.")
                            .font(ui(12.))
                            .color(rgb(muted())),
                    )
                    .child(pill(
                        "detached-go",
                        "Go to window",
                        Pill::Primary,
                        34.,
                        move |s, tx| s.go_to_detached_target(go_key.clone(), owner, tx),
                    ))
                    .child(pill(
                        "detached-restore",
                        "← Restore to this window",
                        Pill::Neutral,
                        30.,
                        move |s, tx| s.restore_detached_target(restore_key.clone(), owner, tx),
                    )),
            ),
    )
}
