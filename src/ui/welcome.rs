//! Start screen: two drop slots (A and B), pickers and recent comparisons.

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Painter, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui,
    pos2, vec2,
};

use crate::source::Entry;
use crate::theme::Palette;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WelcomeAction {
    None,
    PickFile(usize),
    PickFolder(usize),
    Clear(usize),
    Compare,
    OpenRecent(usize),
    DemoFolders,
    DemoMerge,
}

/// Paint the same rendered SVG icon used by the native bundle and website.
pub fn paint_logo(p: &Painter, center: Pos2, size: f32) {
    let id = egui::Id::new("minidiff-icon-texture");
    let texture = p.ctx().data_mut(|data| data.get_temp::<egui::TextureHandle>(id)).unwrap_or_else(|| {
        let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon-256.png"))
            .expect("generated icon PNG");
        let image = egui::ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
        let texture = p.ctx().load_texture("minidiff-icon", image, egui::TextureOptions::LINEAR);
        p.ctx().data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
    });
    p.image(texture.id(), Rect::from_center_size(center, vec2(size, size)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

fn dashed_rect(p: &Painter, rect: Rect, stroke: Stroke) {
    let pts = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    p.extend(Shape::dashed_line(&pts, stroke, 6.0, 4.0));
}

pub fn ui(
    ui: &mut Ui,
    slots: &[Option<Entry>; 2],
    recent: &[(String, String)],
    hovering: bool,
) -> WelcomeAction {
    let pal = crate::theme::palette(ui.ctx());
    let mut action = WelcomeAction::None;
    let avail = ui.available_rect_before_wrap();
    ui.painter().rect_filled(avail, 0.0, pal.bg);

    let width = avail.width().min(760.0) - 32.0;
    let top_pad = ((avail.height() - 520.0) / 2.0).clamp(16.0, 120.0);
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add_space(top_pad);
        ui.vertical_centered(|ui| {
            let (logo, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
            paint_logo(ui.painter(), logo.center(), 64.0);
            ui.add_space(6.0);
            ui.label(RichText::new("MiniDiff").size(30.0).strong().color(pal.text));
            ui.label(
                RichText::new("Compare files and folders · resolve merge conflicts")
                    .size(14.0)
                    .color(pal.text_dim),
            );
            ui.add_space(22.0);

            // Two slots side by side.
            let gap = 16.0;
            let slot_w = (width - gap) / 2.0;
            let slot_h = 168.0;
            let (row, _) = ui.allocate_exact_size(vec2(width, slot_h), Sense::hover());
            for (i, slot) in slots.iter().enumerate() {
                let rect = Rect::from_min_size(
                    pos2(row.left() + i as f32 * (slot_w + gap), row.top()),
                    vec2(slot_w, slot_h),
                );
                if let Some(a) = slot_ui(ui, rect, i, slot.as_ref(), hovering, pal) {
                    action = a;
                }
            }
            // Arrow between slots.
            let c = row.center();
            ui.painter().circle_filled(c, 15.0, pal.panel);
            ui.painter().circle_stroke(c, 15.0, Stroke::new(1.0, pal.border));
            ui.painter()
                .text(c, Align2::CENTER_CENTER, "⇄", FontId::proportional(15.0), pal.text_dim);

            ui.add_space(18.0);
            let ready = slots[0].is_some() && slots[1].is_some();
            let b = egui::Button::new(RichText::new("Compare").size(15.0).strong().color(if ready {
                Color32::WHITE
            } else {
                pal.text_dim
            }))
            .fill(if ready { pal.accent } else { pal.panel })
            .min_size(vec2(160.0, 34.0))
            .corner_radius(CornerRadius::same(8));
            if ui.add_enabled(ready, b).clicked() {
                action = WelcomeAction::Compare;
            }

            ui.add_space(18.0);
            let tip = "Drop two files or two folders anywhere in this window or on the app icon.\nDrop a single file with conflict markers to resolve it.";
            ui.label(RichText::new(tip).color(pal.text_dim).size(12.5));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let w = 290.0;
                ui.add_space((ui.available_width() - w).max(0.0) / 2.0);
                ui.label(RichText::new("No files at hand? Try the").color(pal.text_dim).size(12.5));
                if ui.link(RichText::new("folder demo").size(12.5)).clicked() {
                    action = WelcomeAction::DemoFolders;
                }
                ui.label(RichText::new("or").color(pal.text_dim).size(12.5));
                if ui.link(RichText::new("merge demo").size(12.5)).clicked() {
                    action = WelcomeAction::DemoMerge;
                }
            });

            if !recent.is_empty() {
                ui.add_space(22.0);
                ui.label(RichText::new("RECENT").size(11.0).color(pal.text_dim).strong());
                ui.add_space(4.0);
                for (i, (a, b)) in recent.iter().enumerate().take(8) {
                    let name = |p: &str| {
                        std::path::Path::new(p)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| p.to_owned())
                    };
                    let text = format!("{}  ⇄  {}", name(a), name(b));
                    let r = ui
                        .add(egui::Button::new(RichText::new(text).color(pal.text)).frame_when_inactive(false))
                        .on_hover_text(format!("{a}\n{b}"));
                    if r.clicked() {
                        action = WelcomeAction::OpenRecent(i);
                    }
                }
            }
            ui.add_space(24.0);
        });
    });
    action
}

fn slot_ui(
    ui: &mut Ui,
    rect: Rect,
    i: usize,
    entry: Option<&Entry>,
    hovering: bool,
    pal: &Palette,
) -> Option<WelcomeAction> {
    let mut action = None;
    let (tag, color) = if i == 0 { ("A", pal.del_fg) } else { ("B", pal.add_fg) };
    let p = ui.painter().clone();
    let resp = ui.interact(rect, ui.id().with(("slot", i)), Sense::hover());
    let hot = hovering || resp.hovered();
    let fill = if entry.is_some() {
        pal.panel
    } else if hot {
        color.gamma_multiply(0.08)
    } else {
        pal.bg
    };
    p.rect_filled(rect, 12.0, fill);
    if entry.is_some() {
        p.rect_stroke(rect, 12.0, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
    } else {
        dashed_rect(
            &p,
            rect.shrink(1.0),
            Stroke::new(1.5, if hot { color } else { pal.border }),
        );
    }
    let badge = Rect::from_min_size(rect.min + vec2(14.0, 14.0), vec2(24.0, 22.0));
    p.rect_filled(badge, 6.0, color);
    p.text(badge.center(), Align2::CENTER_CENTER, tag, FontId::proportional(13.0), Color32::WHITE);

    match entry {
        Some(e) => {
            let icon = if e.is_dir() { "🗀" } else { "🗋" };
            p.text(rect.center() - vec2(0.0, 26.0), Align2::CENTER_CENTER, icon, FontId::proportional(30.0), color);
            let clip = p.with_clip_rect(rect.shrink(10.0));
            clip.text(rect.center() + vec2(0.0, 10.0), Align2::CENTER_CENTER, e.name(), FontId::proportional(15.0), pal.text);
            clip.text(
                rect.center() + vec2(0.0, 30.0),
                Align2::CENTER_CENTER,
                e.display_path(),
                FontId::proportional(11.0),
                pal.text_dim,
            );
            let x = Rect::from_min_size(pos2(rect.right() - 34.0, rect.top() + 12.0), vec2(22.0, 22.0));
            let xr = ui
                .interact(x, ui.id().with(("slot-clear", i)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text("Remove");
            if xr.hovered() {
                p.circle_filled(x.center(), 11.0, pal.hover);
            }
            p.text(x.center(), Align2::CENTER_CENTER, "✕", FontId::proportional(12.0), pal.text_dim);
            if xr.clicked() {
                action = Some(WelcomeAction::Clear(i));
            }
        }
        None => {
            p.text(
                rect.center() - vec2(0.0, 22.0),
                Align2::CENTER_CENTER,
                "Drop a file or folder",
                FontId::proportional(15.0),
                if hot { color } else { pal.text_dim },
            );
            let bw = 84.0;
            let brow = Rect::from_center_size(rect.center() + vec2(0.0, 22.0), vec2(bw * 2.0 + 8.0, 28.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(brow).layout(egui::Layout::left_to_right(egui::Align::Center)));
            if child.add(egui::Button::new("File…").min_size(vec2(bw, 26.0))).clicked() {
                action = Some(WelcomeAction::PickFile(i));
            }
            if child.add(egui::Button::new("Folder…").min_size(vec2(bw, 26.0))).clicked() {
                action = Some(WelcomeAction::PickFolder(i));
            }
        }
    }
    action
}
