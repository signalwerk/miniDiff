//! Colours and egui styling for light and dark mode.

use egui::{Color32, CornerRadius, Stroke, Theme};

use crate::highlight::Hl;

mod brand {
    include!("../assets/icon-colors.rs");
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Derive readable theme shades and diff fills from the icon's red and green.
const fn tint(colour: u32, background: u32, percent: u32) -> Color32 {
    const fn channel(colour: u32, background: u32, percent: u32, shift: u32) -> u8 {
        ((((colour >> shift) & 255) * percent + ((background >> shift) & 255) * (100 - percent)) / 100) as u8
    }
    Color32::from_rgb(
        channel(colour, background, percent, 16),
        channel(colour, background, percent, 8),
        channel(colour, background, percent, 0),
    )
}

pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub toolbar: Color32,
    pub border: Color32,
    pub gutter_bg: Color32,
    pub gutter_fg: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_bg: Color32,
    pub hover: Color32,
    pub filler: Color32,
    pub fold_bg: Color32,

    pub add_fg: Color32,
    pub add_bg: Color32,
    pub add_gutter: Color32,
    pub add_emph: Color32,
    pub del_fg: Color32,
    pub del_bg: Color32,
    pub del_gutter: Color32,
    pub del_emph: Color32,
    pub mod_fg: Color32,

    pub local: Color32,
    pub local_bg: Color32,
    pub remote: Color32,
    pub remote_bg: Color32,
    pub conflict: Color32,
    pub conflict_bg: Color32,
    pub base_bg: Color32,
    pub custom_bg: Color32,

    syntax: [Color32; 21],
}

impl Palette {
    pub fn syntax(&self, hl: Hl) -> Color32 {
        self.syntax[hl as usize]
    }
}

// Order must match `Hl`.
const DARK_SYNTAX: [Color32; 21] = [
    hex(0xc9d1d9), // Plain
    hex(0x7d8590), // Comment
    hex(0xff7b72), // Keyword
    hex(0xa5d6ff), // String
    hex(0x79c0ff), // Escape
    hex(0x79c0ff), // Number
    hex(0x79c0ff), // Constant
    hex(0xd2a8ff), // Function
    hex(0x56d4dd), // Macro
    hex(0xffa657), // Type
    hex(0xff7b72), // VariableBuiltin
    hex(0xffa657), // Parameter
    hex(0x79c0ff), // Property
    hex(0xff7b72), // Operator
    hex(0x8b949e), // Punctuation
    hex(0x7ee787), // Tag
    hex(0x79c0ff), // Attribute
    hex(0xffa657), // Module
    hex(0xd2a8ff), // Label
    hex(0x79c0ff), // Heading
    hex(0xa5d6ff), // Link
];

const LIGHT_SYNTAX: [Color32; 21] = [
    hex(0x1f2328), // Plain
    hex(0x6e7781), // Comment
    hex(0xcf222e), // Keyword
    hex(0x0a3069), // String
    hex(0x0550ae), // Escape
    hex(0x0550ae), // Number
    hex(0x0550ae), // Constant
    hex(0x8250df), // Function
    hex(0x1b7c83), // Macro
    hex(0x953800), // Type
    hex(0xcf222e), // VariableBuiltin
    hex(0x953800), // Parameter
    hex(0x0550ae), // Property
    hex(0xcf222e), // Operator
    hex(0x57606a), // Punctuation
    hex(0x116329), // Tag
    hex(0x0550ae), // Attribute
    hex(0x953800), // Module
    hex(0x8250df), // Label
    hex(0x0550ae), // Heading
    hex(0x0a3069), // Link
];

pub static DARK: Palette = Palette {
    dark: true,
    bg: hex(0x0f1216),
    panel: hex(0x161a20),
    toolbar: hex(0x1b2027),
    border: hex(0x2a3038),
    gutter_bg: hex(0x13171c),
    gutter_fg: hex(0x5b636e),
    text: hex(0xd5dbe3),
    text_dim: hex(0x8a939f),
    accent: hex(0x4c9aff),
    accent_bg: hex(0x1d3557),
    hover: Color32::from_rgba_premultiplied(255, 255, 255, 8),
    filler: hex(0x13161b),
    fold_bg: hex(0x182029),

    add_fg: hex(brand::GREEN),
    add_bg: tint(brand::GREEN, 0x0f1216, 12),
    add_gutter: tint(brand::GREEN, 0x0f1216, 20),
    add_emph: tint(brand::GREEN, 0x0f1216, 45),
    del_fg: tint(brand::RED, 0xffffff, 85),
    del_bg: tint(brand::RED, 0x0f1216, 12),
    del_gutter: tint(brand::RED, 0x0f1216, 20),
    del_emph: tint(brand::RED, 0x0f1216, 45),
    mod_fg: hex(0xd29922),

    local: hex(0x4c9aff),
    local_bg: hex(0x142338),
    remote: hex(0xb87fff),
    remote_bg: hex(0x23193a),
    conflict: hex(0xf0883e),
    conflict_bg: hex(0x3a2412),
    base_bg: hex(0x1b1f25),
    custom_bg: hex(0x16302c),

    syntax: DARK_SYNTAX,
};

pub static LIGHT: Palette = Palette {
    dark: false,
    bg: hex(0xffffff),
    panel: hex(0xf6f7f9),
    toolbar: hex(0xeef0f3),
    border: hex(0xd8dce1),
    gutter_bg: hex(0xf6f8fa),
    gutter_fg: hex(0x8c959f),
    text: hex(0x1f2328),
    text_dim: hex(0x656d76),
    accent: hex(0x0969da),
    accent_bg: hex(0xddf4ff),
    hover: Color32::from_rgba_premultiplied(0, 0, 0, 7),
    filler: hex(0xf3f4f6),
    fold_bg: hex(0xeaf2fb),

    add_fg: tint(brand::GREEN, 0x000000, 60),
    add_bg: tint(brand::GREEN, 0xffffff, 10),
    add_gutter: tint(brand::GREEN, 0xffffff, 20),
    add_emph: tint(brand::GREEN, 0xffffff, 35),
    del_fg: tint(brand::RED, 0x000000, 90),
    del_bg: tint(brand::RED, 0xffffff, 10),
    del_gutter: tint(brand::RED, 0xffffff, 20),
    del_emph: tint(brand::RED, 0xffffff, 35),
    mod_fg: hex(0x9a6700),

    local: hex(0x0969da),
    local_bg: hex(0xe4efff),
    remote: hex(0x8250df),
    remote_bg: hex(0xf1eaff),
    conflict: hex(0xbc4c00),
    conflict_bg: hex(0xfff1e5),
    base_bg: hex(0xf3f4f6),
    custom_bg: hex(0xdff7f1),

    syntax: LIGHT_SYNTAX,
};

pub fn palette(ctx: &egui::Context) -> &'static Palette {
    if ctx.global_style().visuals.dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub fn apply(self, ctx: &egui::Context) {
        ctx.set_theme(match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        });
    }


}

/// Install our look into both egui themes.
pub fn install(ctx: &egui::Context) {
    for (theme, pal) in [(Theme::Dark, &DARK), (Theme::Light, &LIGHT)] {
        ctx.style_mut_of(theme, |style| {
            let v = &mut style.visuals;
            v.panel_fill = pal.panel;
            v.window_fill = pal.panel;
            v.extreme_bg_color = pal.bg;
            v.faint_bg_color = pal.toolbar;
            v.override_text_color = None;
            v.hyperlink_color = pal.accent;
            v.selection.bg_fill = pal.accent_bg;
            v.selection.stroke = Stroke::new(1.0, pal.accent);
            v.window_corner_radius = CornerRadius::same(10);
            v.menu_corner_radius = CornerRadius::same(8);
            v.window_stroke = Stroke::new(1.0, pal.border);
            for w in [
                &mut v.widgets.inactive,
                &mut v.widgets.hovered,
                &mut v.widgets.active,
                &mut v.widgets.open,
                &mut v.widgets.noninteractive,
            ] {
                w.corner_radius = CornerRadius::same(6);
                // egui 0.36 includes borders in button layout and subtracts
                // the theme border from padding before explicit overrides.
                // Keep that padding constant across interaction states.
                w.bg_stroke.width = 1.0;
            }
            v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.border);
            v.widgets.noninteractive.fg_stroke.color = pal.text;
            v.widgets.inactive.fg_stroke.color = pal.text;
            v.widgets.inactive.weak_bg_fill = pal.toolbar;
            style.spacing.button_padding = egui::vec2(8.0, 3.0);
            style.spacing.item_spacing = egui::vec2(6.0, 4.0);
            style.interaction.selectable_labels = false;
        });
    }
}
