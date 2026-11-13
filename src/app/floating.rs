//! Floating layers belong to the server. Every operation names existing blocks/layers;
//! only the explicit New floating pane action starts a program.
use super::{
    panel::PaneTarget,
    theme::*,
    workspace::{HostId, Workspace},
};
use rex_linux::connection::{HostCommand, perform::Outcome};
use rexrs::api::session::{Layer, Window};
use rune::{Cursor, El, Phase, Tx, el, row, text};
use serde_json::{Value, json};

pub struct DragState {
    host: HostId,
    session: String,
    window: String,
    layer: String,
    block: String,
    epoch: u64,
    original: [f64; 4],
    pub bounds: [f64; 4],
}

pub const DEFAULT_BOUNDS: [f64; 4] = [0.18, 0.15, 0.62, 0.60];

pub fn is_action(name: &str) -> bool {
    matches!(
        name,
        "pane.float"
            | "pane.new_floating"
            | "pane.minimize_floating"
            | "pane.minimize_all_floating"
            | "pane.restore_floating"
            | "pane.floating.bounds"
            | "pane.floating.move"
            | "pane.floating.resize"
            | "pane.floating.raise"
            | "pane.floating.lower"
    )
}

pub fn title(name: &str) -> &'static str {
    match name {
        "pane.float" => "Float or dock pane",
        "pane.new_floating" => "New floating pane",
        "pane.minimize_floating" => "Minimize or restore floating pane",
        "pane.minimize_all_floating" => "Minimize or restore all floating panes",
        "pane.restore_floating" => "Restore floating pane",
        "pane.floating.bounds" => "Set floating pane bounds",
        "pane.floating.move" => "Move floating pane",
        "pane.floating.resize" => "Resize floating pane",
        "pane.floating.raise" => "Raise floating pane",
        "pane.floating.lower" => "Lower floating pane",
        _ => unreachable!(),
    }
}

pub fn bounds(value: &Value) -> Option<[f64; 4]> {
    let values = value.as_array()?;
    if values.len() != 4 {
        return None;
    }
    let rect = [
        values[0].as_f64()?,
        values[1].as_f64()?,
        values[2].as_f64()?,
        values[3].as_f64()?,
    ];
    valid_bounds(rect).then_some(rect)
}

pub fn valid_bounds([x, y, w, h]: [f64; 4]) -> bool {
    [x, y, w, h].iter().all(|n| n.is_finite())
        && x >= 0.
        && y >= 0.
        && w > 0.
        && h > 0.
        && x + w <= 1.
        && y + h <= 1.
}

pub fn schema(name: &str) -> Option<Value> {
    match name {
        "pane.floating.bounds" => Some(
            json!({"type":"object","properties":{"bounds":{"type":"array","items":{"type":"number"},"minItems":4,"maxItems":4}},"required":["bounds"],"additionalProperties":false}),
        ),
        "pane.floating.move" | "pane.floating.resize" => Some(
            json!({"type":"object","properties":{"direction":{"type":"string","enum":["left","right","up","down"]},"amount":{"type":"number","exclusiveMinimum":0,"default":5}},"required":["direction"],"additionalProperties":false}),
        ),
        "pane.restore_floating" => Some(
            json!({"type":"object","properties":{"layer_id":{"type":"string","minLength":1}},"additionalProperties":false}),
        ),
        _ => None,
    }
}

pub fn accepts(name: &str, args: &Value) -> bool {
    let Some(object) = args.as_object() else {
        return args.is_null()
            && !matches!(
                name,
                "pane.floating.bounds" | "pane.floating.move" | "pane.floating.resize"
            );
    };
    match name {
        "pane.floating.bounds" => object.len() == 1 && bounds(&args["bounds"]).is_some(),
        "pane.floating.move" | "pane.floating.resize" => {
            object.keys().all(|k| k == "direction" || k == "amount")
                && matches!(
                    args["direction"].as_str(),
                    Some("left" | "right" | "up" | "down")
                )
                && object
                    .get("amount")
                    .is_none_or(|n| n.as_f64().is_some_and(|n| n.is_finite() && n > 0.))
        }
        "pane.restore_floating" => {
            object.is_empty()
                || object.len() == 1 && args["layer_id"].as_str().is_some_and(|s| !s.is_empty())
        }
        _ => object.is_empty(),
    }
}

pub fn layer_for<'a>(window: &'a Window, block: &str) -> Option<&'a Layer> {
    window.layers.iter().find(|l| {
        l.blocks
            .iter()
            .any(|b| b.block_id.as_deref() == Some(block))
    })
}

pub fn is_floating(window: &Window, block: &str) -> bool {
    layer_for(window, block).is_some_and(|l| l.kind == "floating")
}

struct Call {
    method: &'static str,
    payload: Value,
    create: bool,
}
struct Plan {
    host: usize,
    session: String,
    cwd: Option<String>,
    calls: Vec<Call>,
}

fn error(message: &str) -> Outcome {
    Outcome::error("not_found", message)
}

fn plan(
    s: &Workspace,
    target: Option<&PaneTarget>,
    name: &str,
    args: &Value,
) -> Result<Plan, Outcome> {
    let host = target.map_or(s.active, |t| t.host);
    let model = &s
        .hosts
        .get(host)
        .ok_or_else(|| error("The host is no longer saved."))?
        .model;
    if !model.connected {
        return Err(Outcome::error(
            "refused",
            "The host is disconnected; nothing was dispatched.",
        ));
    }
    let session = target
        .map(|t| t.session.as_str())
        .or(model.selected.as_deref())
        .ok_or_else(|| error("No session is selected."))?;
    let block = target.map(|t| t.block.as_str()).or_else(|| model.focused());
    let window = if let Some(target) = target {
        model.views.get(session).and_then(|v| {
            v.windows
                .iter()
                .find(|w| layer_for(w, &target.block).is_some())
        })
    } else {
        model.window()
    }
    .ok_or_else(|| error("The tab is no longer available."))?;
    if target.is_some_and(|target| s.pane_in_detached_window(target))
        || (target.is_none() && s.detached_here())
    {
        return Err(Outcome::error(
            "refused",
            "This tab is open in another window.",
        ));
    }
    let layer = block.and_then(|block| layer_for(window, block));
    let floats: Vec<_> = window
        .layers
        .iter()
        .filter(|l| l.kind == "floating")
        .collect();
    if floats.len() > 1024 {
        return Err(Outcome::error(
            "not_supported",
            "Too many floating layers for one operation.",
        ));
    }
    let mut calls = Vec::new();
    let mut push = |method, payload| {
        calls.push(Call {
            method,
            payload,
            create: false,
        })
    };
    let layer_id = |layer: &Layer| {
        layer
            .layer_id
            .clone()
            .ok_or_else(|| error("The floating layer has no ID."))
    };
    match name {
        "pane.float" => {
            let block = block.ok_or_else(|| error("No pane is focused."))?;
            let layer = layer.ok_or_else(|| error("The pane is no longer in this tab."))?;
            if layer.kind == "floating" {
                let anchor = window.layers.iter().filter(|l| l.kind == "tiled").flat_map(|l| &l.blocks).find_map(|b| b.block_id.as_ref()).ok_or_else(|| Outcome::error("not_supported","This tab has no tiled docking anchor. Move the pane to another existing tab; no replacement shell was created."))?;
                push(
                    "session.move_block",
                    json!({"block_id":block,"anchor_block_id":anchor,"direction":"horizontal","side":"after","ratio":0.5}),
                );
            } else if layer.kind == "tiled" {
                if layer.blocks.len() < 2 {
                    return Err(Outcome::error(
                        "not_supported",
                        "Keep a tiled pane as a docking anchor. API 12 cannot return a pane to an empty tiled layer; nothing was floated.",
                    ));
                }
                push(
                    "session.float_block",
                    json!({"block_id":block,"bounds":DEFAULT_BOUNDS}),
                );
            } else {
                return Err(Outcome::error(
                    "not_supported",
                    "This layer kind is not supported for floating operations.",
                ));
            }
        }
        "pane.new_floating" => {
            let window = window
                .window_id
                .as_ref()
                .ok_or_else(|| error("The tab has no ID."))?;
            calls.push(Call {
                method: "session.new_layer",
                payload: json!({"window_id":window,"bounds":DEFAULT_BOUNDS,"focus":true}),
                create: true,
            });
        }
        "pane.restore_floating" => {
            let layer = if let Some(id) = args["layer_id"].as_str() {
                floats
                    .iter()
                    .find(|l| l.layer_id.as_deref() == Some(id))
                    .copied()
            } else {
                floats
                    .iter()
                    .rev()
                    .find(|l| l.state.as_deref() == Some("minimized"))
                    .copied()
            }
            .ok_or_else(|| error("No matching floating layer in this tab."))?;
            push(
                "session.set_layer_state",
                json!({"layer_id":layer_id(layer)?,"state":"normal","focus":true}),
            );
        }
        "pane.minimize_floating" => {
            if let Some(layer) =
                layer.filter(|l| l.kind == "floating" && l.state.as_deref() != Some("minimized"))
            {
                if !window.layers.iter().any(|l| {
                    l.layer_id != layer.layer_id
                        && l.state.as_deref() != Some("minimized")
                        && !l.blocks.is_empty()
                }) {
                    return Err(Outcome::error(
                        "refused",
                        "Keep at least one visible pane before minimizing this layer.",
                    ));
                }
                push(
                    "session.set_layer_state",
                    json!({"layer_id":layer_id(layer)?,"state":"minimized"}),
                );
            } else if target.is_none() {
                return plan(s, None, "pane.restore_floating", &Value::Null);
            } else {
                return Err(error("This pane is not in a visible floating layer."));
            }
        }
        "pane.minimize_all_floating" => {
            if floats.is_empty() {
                return Err(error("This tab has no floating panes."));
            }
            let hide = floats
                .iter()
                .any(|l| l.state.as_deref() != Some("minimized"));
            if hide
                && !window
                    .layers
                    .iter()
                    .any(|l| l.kind == "tiled" && !l.blocks.is_empty())
            {
                return Err(Outcome::error(
                    "refused",
                    "Keep a visible tiled pane before minimizing all floating layers.",
                ));
            }
            for (i, layer) in floats.iter().enumerate() {
                push(
                    "session.set_layer_state",
                    json!({"layer_id":layer_id(layer)?,"state":if hide {"minimized"} else {"normal"},"focus":!hide && i+1 == floats.len()}),
                );
            }
        }
        "pane.floating.raise"
        | "pane.floating.lower"
        | "pane.floating.bounds"
        | "pane.floating.move"
        | "pane.floating.resize" => {
            let layer = layer
                .filter(|l| l.kind == "floating")
                .ok_or_else(|| error("Focus a floating pane first."))?;
            let id = layer_id(layer)?;
            match name {
                "pane.floating.raise" => push("session.raise_layer", json!({"layer_id":id})),
                "pane.floating.lower" => push("session.lower_layer", json!({"layer_id":id})),
                _ => {
                    let rect = if name == "pane.floating.bounds" {
                        bounds(&args["bounds"]).ok_or_else(|| Outcome::error("bad_request","Bounds must be finite [x,y,width,height] fractions inside the tab."))?
                    } else {
                        let mut r: [f64; 4] =
                            layer.bounds.as_slice().try_into().map_err(|_| {
                                error("The server did not report valid layer bounds.")
                            })?;
                        if !valid_bounds(r) {
                            return Err(error("The server did not report valid layer bounds."));
                        }
                        let amount = args["amount"].as_f64().unwrap_or(5.) / 100.;
                        let direction = args["direction"].as_str().unwrap();
                        let axis = usize::from(matches!(direction, "up" | "down"));
                        let delta = if matches!(direction, "left" | "up") {
                            -amount
                        } else {
                            amount
                        };
                        if name == "pane.floating.move" {
                            r[axis] = (r[axis] + delta).clamp(0., 1. - r[axis + 2]);
                        } else {
                            r[axis + 2] =
                                (r[axis + 2] + delta).clamp(0.05, (1. - r[axis]).max(0.05));
                        }
                        r
                    };
                    if !valid_bounds(rect) {
                        return Err(Outcome::error(
                            "bad_request",
                            "The new bounds would leave the tab.",
                        ));
                    }
                    push("session.resize_layer", json!({"layer_id":id,"bounds":rect}));
                }
            }
        }
        _ => unreachable!(),
    }
    Ok(Plan {
        host,
        session: session.into(),
        cwd: block.map(str::to_owned),
        calls,
    })
}

pub fn validate(s: &Workspace, name: &str, args: &Value) -> Result<(), Outcome> {
    if is_action(name) {
        plan(s, None, name, args).map(|_| ())
    } else {
        Ok(())
    }
}

fn dispatch(s: &mut Workspace, plan: Plan) {
    for call in plan.calls {
        if call.create {
            let mut payload = call.payload;
            payload["layout"] = s.new_terminal(plan.host);
            s.open_terminal(
                plan.host,
                call.method,
                payload,
                &plan.session,
                plan.cwd.as_deref(),
            );
        } else {
            let mut payload = call.payload;
            payload["session_id"] = json!(plan.session);
            s.send(
                plan.host,
                HostCommand::Request {
                    method: call.method.into(),
                    payload,
                },
            );
        }
    }
}

pub fn run(s: &mut Workspace, name: &str, args: &Value) {
    match plan(s, None, name, args) {
        Ok(plan) => dispatch(s, plan),
        Err(Outcome::Error { message, .. }) => {
            s.inform(message);
        }
        Err(Outcome::Performed) => unreachable!(),
    }
}

pub fn target_available(s: &Workspace, target: &PaneTarget, name: &str) -> bool {
    plan(s, Some(target), name, &Value::Null).is_ok()
}

pub fn target(s: &mut Workspace, target: &PaneTarget, name: &str) {
    match plan(s, Some(target), name, &Value::Null) {
        Ok(plan) => dispatch(s, plan),
        Err(Outcome::Error { message, .. }) => {
            s.inform(message);
        }
        Err(Outcome::Performed) => unreachable!(),
    }
}

impl Workspace {
    pub(super) fn raise_floating(&mut self, block: &str) {
        let id = self
            .model()
            .window()
            .and_then(|w| layer_for(w, block))
            .filter(|l| l.kind == "floating")
            .and_then(|l| l.layer_id.clone());
        if let Some(id) = id {
            self.request("session.raise_layer", json!({"layer_id":id}));
        }
    }

    fn floating_drag_current(&self, drag: &DragState) -> bool {
        self.window_focused
            && self.hosts[self.active].id == drag.host
            && self.model().connected
            && self.model().connection_epoch == drag.epoch
            && self.model().selected.as_ref() == Some(&drag.session)
            && self.model().window().is_some_and(|w| {
                w.window_id.as_ref() == Some(&drag.window)
                    && w.zoomed_block_id.is_none()
                    && w.layers.iter().any(|l| {
                        l.layer_id.as_ref() == Some(&drag.layer)
                            && l.kind == "floating"
                            && l.state.as_deref() != Some("minimized")
                            && l.bounds.as_slice() == drag.original
                            && l.blocks
                                .iter()
                                .any(|b| b.block_id.as_ref() == Some(&drag.block))
                    })
            })
    }

    pub(super) fn poll_floating_drag(&mut self) -> bool {
        if self
            .floating_drag
            .as_ref()
            .is_some_and(|drag| !self.floating_drag_current(drag))
        {
            self.floating_drag = None;
            return true;
        }
        false
    }

    pub(super) fn drag_floating(
        &mut self,
        block: &str,
        gesture: &rune::Drag,
        resize: bool,
        tx: &mut Tx,
    ) {
        if gesture.phase == Phase::Down {
            if self.panel.is_some() {
                return;
            }
            self.floating_drag = None;
            let Some(window) = self
                .model()
                .window()
                .filter(|w| w.zoomed_block_id.is_none())
            else {
                return;
            };
            let Some(layer) = layer_for(window, block)
                .filter(|l| l.kind == "floating" && l.state.as_deref() != Some("minimized"))
            else {
                return;
            };
            let Ok(rect) = <[f64; 4]>::try_from(layer.bounds.as_slice()) else {
                return;
            };
            if !valid_bounds(rect) || !self.model().connected {
                return;
            }
            let (Some(window), Some(layer), Some(session)) = (
                window.window_id.clone(),
                layer.layer_id.clone(),
                self.model().selected.clone(),
            ) else {
                return;
            };
            self.floating_drag = Some(DragState {
                host: self.hosts[self.active].id,
                session,
                window,
                layer,
                block: block.into(),
                epoch: self.model().connection_epoch,
                original: rect,
                bounds: rect,
            });
            self.raise_floating(block);
        }
        if self.poll_floating_drag() {
            return;
        }
        let [_, _, width, height] = super::panes::inner(self);
        let scale = self.ui_scale() as f64;
        let Some(drag) = &mut self.floating_drag else {
            return;
        };
        if width <= 0. || height <= 0. {
            self.floating_drag = None;
            return;
        }
        let dx = gesture.dx as f64 / (width as f64 * scale);
        let dy = gesture.dy as f64 / (height as f64 * scale);
        let mut rect = drag.original;
        if resize {
            rect[2] = (rect[2] + dx).clamp((160. / width as f64).min(1. - rect[0]), 1. - rect[0]);
            rect[3] = (rect[3] + dy).clamp((90. / height as f64).min(1. - rect[1]), 1. - rect[1]);
        } else {
            rect[0] = (rect[0] + dx).clamp(0., 1. - rect[2]);
            rect[1] = (rect[1] + dy).clamp(0., 1. - rect[3]);
        }
        drag.bounds = rect;
        tx.instant();
        if gesture.phase == Phase::Up {
            let drag = self.floating_drag.take().unwrap();
            if !gesture.cancelled() && valid_bounds(drag.bounds) && drag.bounds != drag.original {
                self.request(
                    "session.resize_layer",
                    json!({"layer_id":drag.layer,"bounds":drag.bounds}),
                );
            }
        }
    }
}

pub fn resize_handle(s: &Workspace, window: &Window, layer: &Layer) -> Option<El> {
    if window.zoomed_block_id.is_some()
        || layer.kind != "floating"
        || layer.state.as_deref() == Some("minimized")
    {
        return None;
    }
    let block = layer.blocks.last()?.block_id.clone()?;
    let rect: [f64; 4] = layer.bounds.as_slice().try_into().ok()?;
    let [x, y, w, h] = super::panes::card_rect(s, rect);
    Some(
        el().key(format!(
            "floating-resize:{}",
            layer.layer_id.as_deref().unwrap_or(&block)
        ))
        .label("Resize floating pane")
        .absolute(x + w - 16., y + h - 16.)
        .size(16., 16.)
        .cursor(Cursor::ResizeEdge(rune::WindowEdge::SouthEast))
        .preserve_focus_on_press(true)
        .on_drag(move |s: &mut Workspace, g, tx| s.drag_floating(&block, g, true, tx))
        .child(
            el().pin(None, None, Some(4.), Some(4.))
                .size(6., 6.)
                .border(1., rgb(muted()).alpha(0.6)),
        ),
    )
}

pub fn preview(s: &Workspace) -> Option<El> {
    let drag = s.floating_drag.as_ref()?;
    let [x, y, w, h] = super::panes::card_rect(s, drag.bounds);
    Some(
        el().key("floating-preview")
            .absolute(x, y)
            .size(w, h)
            .radius(CARD_RADIUS)
            .border(1.5, rgb(accent()).alpha(0.8))
            .instant_changes()
            .motion(STILL),
    )
}

pub fn tray(s: &Workspace, window: &Window) -> Option<El> {
    let mut buttons = row().gap(6.).h(30.);
    let host = s.hosts[s.active].id;
    let session = s.model().selected.clone()?;
    let mut count = 0;
    for layer in window
        .layers
        .iter()
        .filter(|l| l.kind == "floating" && l.state.as_deref() == Some("minimized"))
    {
        let Some(id) = layer.layer_id.clone() else {
            continue;
        };
        let label = layer
            .blocks
            .first()
            .and_then(|b| b.block_id.as_deref())
            .map_or_else(|| "Floating pane".into(), |b| s.pane_name(b));
        let (key, session) = (format!("floating-restore:{id}"), session.clone());
        buttons = buttons.child(
            el().key(key)
                .role("button")
                .label(format!("Restore {label}"))
                .h(28.)
                .max_w(180.)
                .px(12.)
                .center_y()
                .radius(10.)
                .fill(rgb(popover()))
                .border(1., white(0.15))
                .cursor(Cursor::Pointer)
                .preserve_focus_on_press(true)
                .on_press_tx(move |s: &mut Workspace, tx| {
                    if s.hosts[s.active].id == host
                        && s.model().selected.as_ref() == Some(&session)
                        && validate(s, "pane.restore_floating", &json!({"layer_id":id})).is_ok()
                    {
                        run(s, "pane.restore_floating", &json!({"layer_id":id}));
                    }
                    tx.blur();
                })
                .child(text(label).font(medium(11.5)).color(rgb(ink())).truncate()),
        );
        count += 1;
    }
    (count > 0).then(|| {
        row()
            .key("floating-tray")
            .pin(Some(8.), None, Some(8.), Some(8.))
            .h(30.)
            .scroll_x()
            .scroll_edge_fade(12.)
            .child(buttons)
    })
}
