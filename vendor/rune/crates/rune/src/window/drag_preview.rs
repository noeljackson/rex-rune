//! A bounded, per-gesture thumbnail carried by the native pointer grab. Unlike
//! an extra Wayland toplevel, a cursor image can follow that grab outside its source.
use super::*;
use image::{Rgba, RgbaImage, imageops};

impl<S: 'static> Host<S> {
    pub(super) fn set_drag_preview(
        &mut self,
        event_loop: &ActiveEventLoop,
        keys: Option<(Option<String>, String)>,
    ) {
        let Some((content, heading)) = keys else {
            if let Some(w) = &mut self.win {
                w.clear_drag_preview();
            }
            return;
        };
        if !self.rt.pointer_captured() {
            return;
        }
        let Some(w) = self.win.as_ref().filter(|w| w.window.has_focus()) else {
            return;
        };
        let Some((gpu, renderer)) = &self.gpu else {
            return;
        };
        let elements = self.rt.elements();
        let snapshot = |key: &str| {
            let rect = elements
                .iter()
                .find(|e| !e.exiting && e.key.as_deref() == Some(key))?
                .rect;
            let image = crate::drag_out::snapshot(
                gpu,
                renderer,
                (w.config.width, w.config.height),
                w.window.scale_factor() as f32,
                self.rt.clear_color(),
                rect,
            )?;
            image::load_from_memory_with_format(image.png_data(), image::ImageFormat::Png)
                .ok()
                .map(|i| i.into_rgba8())
        };
        let Some(heading) = snapshot(&heading) else {
            return;
        };
        let content = content.as_deref().and_then(snapshot);
        let image = thumbnail(&heading, content.as_ref());
        let scale = w.window.scale_factor() as f32;
        // This is a one-gesture cursor, never inserted in the process's image cache.
        let width = (image.width() as f32 * scale).round().clamp(1., 1024.) as u32;
        let height = (image.height() as f32 * scale).round().clamp(1., 1024.) as u32;
        let image = imageops::resize(&image, width, height, imageops::FilterType::Triangle);
        let Ok(source) = winit::window::CustomCursor::from_rgba(
            image.into_raw(),
            width as u16,
            height as u16,
            (width / 2) as u16,
            (height / 2) as u16,
        ) else {
            return;
        };
        let cursor = event_loop.create_custom_cursor(source);
        w.window.set_cursor(cursor.clone());
        self.win.as_mut().unwrap().drag_cursor = Some(cursor);
    }
}

impl Win {
    pub(super) fn clear_drag_preview(&mut self) {
        if self.drag_cursor.take().is_some() {
            self.cursor.invalidate();
            self.window.set_cursor(winit::window::CursorIcon::Default);
        }
    }
}

/// Keep the heading at readable size and fit the frozen terminal into a small card.
/// An unrendered tab has a heading only; never substitute another tab's output.
fn thumbnail(heading: &RgbaImage, content: Option<&RgbaImage>) -> RgbaImage {
    let mut out = RgbaImage::from_pixel(256, 224, Rgba([24, 28, 34, 250]));
    let h = heading.height().clamp(1, 32);
    let heading = imageops::resize(
        heading,
        ((heading.width() as f32 * h as f32 / heading.height() as f32).round() as u32).max(1),
        h,
        imageops::FilterType::Triangle,
    );
    let heading = imageops::crop_imm(&heading, 0, 0, heading.width().min(240), h).to_image();
    imageops::overlay(&mut out, &heading, 8, 8);
    if let Some(content) = content {
        let terminal = imageops::thumbnail(content, 240, 168);
        imageops::overlay(&mut out, &terminal, 8, 48);
    }
    for (x, y, pixel) in out.enumerate_pixels_mut() {
        let dx = if x < 8 { 8 - x } else { x.saturating_sub(247) };
        let dy = if y < 8 { 8 - y } else { y.saturating_sub(215) };
        if dx * dx + dy * dy > 64 {
            pixel.0[3] = 0;
        }
    }
    out
}
