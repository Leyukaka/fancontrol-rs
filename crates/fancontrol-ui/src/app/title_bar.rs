//! Neon theme's own window frame: the native title bar cannot be styled, so in
//! Neon the app draws its title bar (glowing logo and title, window buttons, drag
//! to move, double-click to maximize) and handles resizing from the edges.

use super::*;

const TITLE_BAR_H: f32 = 32.0;
const BUTTON_W: f32 = 46.0;
/// Width of the invisible resize band along the window edges.
const RESIZE_GRIP: f32 = 6.0;

#[derive(Clone, Copy)]
enum FrameButton {
    Minimize,
    Maximize,
    Close,
}

impl FanApp {
    pub(super) fn ui_title_bar(&mut self, ui: &mut egui::Ui) {
        let fill = ui.visuals().panel_fill;
        egui::Panel::top("title_bar")
            .exact_size(TITLE_BAR_H)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(fill))
            .show(ui, |ui| {
                let rect = ui.max_rect();
                let drag_id = ui.id().with("title_drag");
                let drag = ui.interact(rect, drag_id, egui::Sense::click_and_drag());
                let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                if drag.double_clicked() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                } else if drag.drag_started_by(egui::PointerButton::Primary) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                let painter = ui.painter_at(rect);
                let icon_center = egui::pos2(rect.left() + 22.0, rect.center().y);
                for (radius, alpha) in [(14.0, 0.10), (10.0, 0.18)] {
                    let glow = theme::NEON_CYAN.gamma_multiply(alpha);
                    painter.circle_filled(icon_center, radius, glow);
                }
                let icon = self.title_icon(ui.ctx());
                let icon_rect = egui::Rect::from_center_size(icon_center, egui::vec2(18.0, 18.0));
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                painter.image(icon.id(), icon_rect, uv, egui::Color32::WHITE);
                theme::neon_text(
                    &painter,
                    egui::pos2(icon_rect.right() + 12.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    "Fancontrol-RS",
                    egui::FontId::proportional(15.0),
                    theme::NEON_CYAN,
                );

                let mut x = rect.right();
                for button in [
                    FrameButton::Close,
                    FrameButton::Maximize,
                    FrameButton::Minimize,
                ] {
                    x -= BUTTON_W;
                    let r = egui::Rect::from_min_size(
                        egui::pos2(x, rect.top()),
                        egui::vec2(BUTTON_W, rect.height()),
                    );
                    if frame_button(ui, r, button, maximized).clicked() {
                        let cmd = match button {
                            FrameButton::Minimize => egui::ViewportCommand::Minimized(true),
                            FrameButton::Maximize => egui::ViewportCommand::Maximized(!maximized),
                            // Same path as the native close: hides to tray when it exists.
                            FrameButton::Close => egui::ViewportCommand::Close,
                        };
                        ui.ctx().send_viewport_cmd(cmd);
                    }
                }
            });
    }

    /// Without the native frame the OS resize borders are gone: resize from a thin
    /// band along the window edges instead. Call after every panel so the band
    /// gets the pointer (and cursor) over whatever is underneath.
    pub(super) fn handle_frame_resize(&self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
            return;
        }
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let r = ctx.content_rect();
        let west = pos.x - r.left() < RESIZE_GRIP;
        let east = r.right() - pos.x < RESIZE_GRIP;
        let north = pos.y - r.top() < RESIZE_GRIP;
        let south = r.bottom() - pos.y < RESIZE_GRIP;
        use egui::viewport::ResizeDirection as Dir;
        let target = match (north, south, west, east) {
            (true, _, true, _) => Some((Dir::NorthWest, egui::CursorIcon::ResizeNorthWest)),
            (true, _, _, true) => Some((Dir::NorthEast, egui::CursorIcon::ResizeNorthEast)),
            (_, true, true, _) => Some((Dir::SouthWest, egui::CursorIcon::ResizeSouthWest)),
            (_, true, _, true) => Some((Dir::SouthEast, egui::CursorIcon::ResizeSouthEast)),
            (true, ..) => Some((Dir::North, egui::CursorIcon::ResizeNorth)),
            (_, true, ..) => Some((Dir::South, egui::CursorIcon::ResizeSouth)),
            (_, _, true, _) => Some((Dir::West, egui::CursorIcon::ResizeWest)),
            (_, _, _, true) => Some((Dir::East, egui::CursorIcon::ResizeEast)),
            _ => None,
        };
        if let Some((dir, cursor)) = target {
            ctx.set_cursor_icon(cursor);
            if ctx.input(|i| i.pointer.primary_pressed()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
            }
        }
    }

    /// App icon as a texture, decoded once.
    fn title_icon(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        self.title_icon
            .get_or_insert_with(|| {
                let png = include_bytes!("../../../../assets/icon.png");
                let image = match eframe::icon_data::from_png_bytes(png) {
                    Ok(icon) => egui::ColorImage::from_rgba_unmultiplied(
                        [icon.width as usize, icon.height as usize],
                        &icon.rgba,
                    ),
                    Err(_) => egui::ColorImage::filled([1, 1], egui::Color32::TRANSPARENT),
                };
                ctx.load_texture("title_icon", image, egui::TextureOptions::LINEAR)
            })
            .clone()
    }
}

/// One title-bar button: neon glyph, hover highlight (red for close).
fn frame_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    button: FrameButton,
    maximized: bool,
) -> egui::Response {
    let id = ui.id().with(("frame_button", button as u8));
    let resp = ui.interact(rect, id, egui::Sense::click());
    let painter = ui.painter_at(rect);
    if resp.hovered() {
        let hover = match button {
            FrameButton::Close => egui::Color32::from_rgb(196, 30, 58),
            _ => theme::NEON_MAGENTA.gamma_multiply(0.35),
        };
        painter.rect_filled(rect, 0.0, hover);
    }
    let c = rect.center();
    let stroke = egui::Stroke::new(1.2, theme::NEON_CYAN);
    match button {
        FrameButton::Minimize => {
            let (a, b) = (c + egui::vec2(-5.0, 0.0), c + egui::vec2(5.0, 0.0));
            painter.line_segment([a, b], stroke);
        }
        FrameButton::Maximize => {
            let size = if maximized { 8.0 } else { 10.0 };
            let square = egui::Rect::from_center_size(c, egui::vec2(size, size));
            painter.rect_stroke(square, 1.0, stroke, egui::StrokeKind::Inside);
            if maximized {
                // Restore glyph: a second square peeking out behind.
                let back = square.translate(egui::vec2(2.0, -2.0));
                painter.line_segment([back.left_top(), back.right_top()], stroke);
                painter.line_segment([back.right_top(), back.right_bottom()], stroke);
            }
        }
        FrameButton::Close => {
            let d = 5.0;
            painter.line_segment([c + egui::vec2(-d, -d), c + egui::vec2(d, d)], stroke);
            painter.line_segment([c + egui::vec2(-d, d), c + egui::vec2(d, -d)], stroke);
        }
    }
    resp
}
