//! XDG activation on winit's Wayland connection. Observe this surface's real input
//! serials in a private queue: winit 0.30's token request omits the seat/serial.

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::time::{Duration, Instant};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_keyboard, wl_pointer, wl_registry::WlRegistry, wl_seat, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::activation::v1::client::{
    xdg_activation_token_v1::{self, XdgActivationTokenV1},
    xdg_activation_v1::XdgActivationV1,
};
use winit::window::Window;

const LIFETIME: Duration = Duration::from_secs(2);
type Complete = Box<dyn FnOnce(String)>;
struct Pending {
    token: XdgActivationTokenV1,
    started: Instant,
    complete: Complete,
}
struct Seat {
    seat: wl_seat::WlSeat,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
}
struct State {
    surface: WlSurface,
    seats: Vec<Seat>,
    pointer_focus: Vec<ObjectId>,
    keyboard_focus: Vec<ObjectId>,
    input: Option<(wl_seat::WlSeat, u32, Instant)>,
    pending: Option<Pending>,
}
impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<XdgActivationV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &XdgActivationV1,
        _: <XdgActivationV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        s: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            let Some(watch) = s.seats.iter_mut().find(|w| w.seat == *seat) else {
                return;
            };
            if caps.contains(wl_seat::Capability::Pointer) && watch.pointer.is_none() {
                watch.pointer = Some(seat.get_pointer(qh, seat.clone()));
            }
            if caps.contains(wl_seat::Capability::Keyboard) && watch.keyboard.is_none() {
                watch.keyboard = Some(seat.get_keyboard(qh, seat.clone()));
            }
            if !caps.contains(wl_seat::Capability::Pointer)
                && let Some(p) = watch.pointer.take()
            {
                s.pointer_focus.retain(|id| *id != p.id());
                p.release();
            }
            if !caps.contains(wl_seat::Capability::Keyboard)
                && let Some(k) = watch.keyboard.take()
            {
                s.keyboard_focus.retain(|id| *id != k.id());
                k.release();
            }
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, wl_seat::WlSeat> for State {
    fn event(
        s: &mut Self,
        p: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        seat: &wl_seat::WlSeat,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface, .. } if surface == s.surface => {
                if !s.pointer_focus.contains(&p.id()) {
                    s.pointer_focus.push(p.id());
                }
            }
            wl_pointer::Event::Leave { .. } => s.pointer_focus.retain(|id| *id != p.id()),
            wl_pointer::Event::Button {
                serial,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } if s.pointer_focus.contains(&p.id()) => s.input = Some((seat.clone(), serial, Instant::now())),
            _ => {}
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, wl_seat::WlSeat> for State {
    fn event(
        s: &mut Self,
        k: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        seat: &wl_seat::WlSeat,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } if surface == s.surface => {
                if !s.keyboard_focus.contains(&k.id()) {
                    s.keyboard_focus.push(k.id());
                }
            }
            wl_keyboard::Event::Leave { .. } => s.keyboard_focus.retain(|id| *id != k.id()),
            wl_keyboard::Event::Key {
                serial,
                state: WEnum::Value(wl_keyboard::KeyState::Pressed),
                ..
            } if s.keyboard_focus.contains(&k.id()) => s.input = Some((seat.clone(), serial, Instant::now())),
            _ => {}
        }
    }
}
impl Dispatch<XdgActivationTokenV1, ()> for State {
    fn event(
        s: &mut Self,
        token: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token: value } = event
            && s.pending.as_ref().is_some_and(|p| p.token == *token)
            && let Some(p) = s.pending.take()
        {
            p.token.destroy();
            if p.started.elapsed() < LIFETIME {
                (p.complete)(value);
            }
        }
    }
}

/// Created before mapping, so a later local click/key has its real seat and serial.
pub(crate) struct Activator {
    queue: EventQueue<State>,
    activation: XdgActivationV1,
    state: State,
}
impl Activator {
    pub fn new(window: &Window) -> Option<Self> {
        let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(handle)) = (
            window.display_handle().ok()?.as_raw(),
            window.window_handle().ok()?.as_raw(),
        ) else {
            return None;
        };
        // SAFETY: winit owns the display for the event loop's life; a foreign backend does not close it.
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let conn = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<State>(&conn).ok()?;
        let qh = queue.handle();
        let activation = globals.bind::<XdgActivationV1, _, _>(&qh, 1..=1, ()).ok()?;
        // SAFETY: this is winit's live surface; the activator is dropped before its window.
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), handle.surface.as_ptr().cast()) }.ok()?;
        let surface = WlSurface::from_id(&conn, id).ok()?;
        let seats = globals.contents().with_list(|list| {
            list.iter()
                .filter(|g| g.interface == "wl_seat" && g.version >= 5)
                .map(|g| Seat {
                    seat: globals.registry().bind(g.name, g.version.min(8), &qh, ()),
                    pointer: None,
                    keyboard: None,
                })
                .collect()
        });
        let mut this = Self {
            queue,
            activation,
            state: State {
                surface,
                seats,
                pointer_focus: Vec::new(),
                keyboard_focus: Vec::new(),
                input: None,
                pending: None,
            },
        };
        this.dispatch();
        Some(this)
    }
    /// Never reads or waits: winit reads the shared display; pump our pending events each turn.
    pub fn dispatch(&mut self) {
        let _ = self.queue.dispatch_pending(&mut self.state);
        if self
            .state
            .pending
            .as_ref()
            .is_some_and(|p| p.started.elapsed() >= LIFETIME)
            && let Some(p) = self.state.pending.take()
        {
            p.token.destroy();
        }
        let _ = self.queue.flush();
    }
    pub fn request(&mut self, complete: Complete) -> bool {
        self.dispatch();
        if let Some(p) = self.state.pending.take() {
            p.token.destroy();
        }
        let Some((seat, serial, time)) = self.state.input.take().filter(|(_, _, time)| time.elapsed() < LIFETIME)
        else {
            return false;
        };
        let token = self.activation.get_activation_token(&self.queue.handle(), ());
        token.set_serial(serial, &seat);
        token.set_surface(&self.state.surface);
        token.commit();
        self.state.pending = Some(Pending {
            token,
            started: time,
            complete,
        });
        let _ = self.queue.flush();
        true
    }
    pub fn activate(&mut self, token: &str) {
        self.activation.activate(token.to_owned(), &self.state.surface);
        let _ = self.queue.flush();
    }
}
impl Drop for Activator {
    fn drop(&mut self) {
        if let Some(p) = self.state.pending.take() {
            p.token.destroy();
        }
        for watch in &self.state.seats {
            if let Some(p) = &watch.pointer {
                p.release();
            }
            if let Some(k) = &watch.keyboard {
                k.release();
            }
            watch.seat.release();
        }
        self.activation.destroy();
        let _ = self.queue.flush();
    }
}
