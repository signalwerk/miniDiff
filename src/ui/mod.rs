pub mod code;
pub mod file_view;
pub mod folder_view;
pub mod merge_view;
pub mod welcome;

use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Color32, CornerRadius, Margin, Response, RichText, Stroke, Ui};

use crate::diff::DiffOptions;
use crate::theme::Palette;

/// Persisted view preferences shared by all diff views.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ViewSettings {
    pub unified: bool,
    pub collapse: bool,
    pub context: usize,
    pub font_size: f32,
    pub diff: DiffOptions,
    pub show_base: bool,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            unified: false,
            collapse: false,
            context: 3,
            font_size: 13.0,
            diff: DiffOptions::default(),
            show_base: false,
        }
    }
}

pub fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The bar at the top of every view.
pub fn toolbar_frame(pal: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(pal.toolbar)
        .inner_margin(Margin::symmetric(10, 6))
        .stroke(Stroke::NONE)
}

/// A two-or-more option segmented control. Returns true if the value changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str, &str)]) -> bool {
    let mut changed = false;
    let pal = crate::theme::palette(ui.ctx());
    egui::Frame::new()
        .fill(pal.bg)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            // Inside a right-to-left toolbar, `horizontal` lays out right to left too.
            let rtl = ui.layout().prefer_right_to_left();
            ui.horizontal(|ui| {
                let ordered: Vec<_> = if rtl { options.iter().rev().collect() } else { options.iter().collect() };
                for (v, label, tip) in ordered {
                    let selected = *value == *v;
                    let text = if selected {
                        RichText::new(*label).color(pal.text).strong()
                    } else {
                        RichText::new(*label).color(pal.text_dim)
                    };
                    let b = egui::Button::new(text)
                        .fill(if selected { pal.toolbar } else { Color32::TRANSPARENT })
                        .stroke(if selected {
                            Stroke::new(1.0, pal.border)
                        } else {
                            Stroke::NONE
                        })
                        .corner_radius(CornerRadius::same(5));
                    if ui.add(b).on_hover_text(*tip).clicked() && !selected {
                        *value = *v;
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// A small toggle button that looks pressed when on.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str, tip: &str) -> Response {
    let pal = crate::theme::palette(ui.ctx());
    let text = if *on {
        RichText::new(label).color(pal.accent)
    } else {
        RichText::new(label).color(pal.text_dim)
    };
    let r = ui
        .add(
            egui::Button::new(text)
                .fill(if *on { pal.accent_bg } else { Color32::TRANSPARENT })
                .stroke(Stroke::new(1.0, if *on { pal.accent } else { pal.border })),
        )
        .on_hover_text(tip);
    if r.clicked() {
        *on = !*on;
    }
    r
}

/// Icon-ish button without a frame until hovered.
pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str, enabled: bool) -> Response {
    ui.add_enabled(enabled, egui::Button::new(RichText::new(icon).size(15.0)).frame_when_inactive(false))
        .on_hover_text(tip)
}

/// Platform specific name of the command key.
pub fn cmd() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl+"
    }
}

/// Platform specific name of the option / alt key.
pub fn alt() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌥"
    } else {
        "Alt+"
    }
}

/// Should global single-key shortcuts fire (no text field focused)?
pub fn keys_free(ctx: &egui::Context) -> bool {
    !ctx.egui_wants_keyboard_input()
}
