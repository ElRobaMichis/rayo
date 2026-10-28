//! Paleta, tipografía y estilo de la interfaz.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle, Visuals};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    pub bg: Color32,
    pub sidebar: Color32,
    pub chrome: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub sel: Color32,
    pub sel_inactive: Color32,
    pub folder: Color32,
    pub danger: Color32,
    pub overlay: Color32,
}

const fn hex(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

impl Theme {
    pub fn dark() -> Self {
        Theme {
            dark: true,
            bg: hex(0x16171B),
            sidebar: hex(0x111215),
            chrome: hex(0x1B1C21),
            hover: hex(0x24262C),
            border: hex(0x2A2C33),
            text: hex(0xE8EAEF),
            dim: hex(0x9AA1AD),
            faint: hex(0x626875),
            accent: hex(0x4C8DFF),
            accent_text: hex(0xFFFFFF),
            sel: hex(0x2F5FCC),
            sel_inactive: hex(0x2C2F37),
            folder: hex(0x5EA6F8),
            danger: hex(0xFF6B6B),
            overlay: Color32::from_black_alpha(170),
        }
    }

    pub fn light() -> Self {
        Theme {
            dark: false,
            bg: hex(0xFFFFFF),
            sidebar: hex(0xF3F4F7),
            chrome: hex(0xF8F9FB),
            hover: hex(0xECEEF2),
            border: hex(0xE1E4E9),
            text: hex(0x1D2127),
            dim: hex(0x5D6573),
            faint: hex(0x9AA1AD),
            accent: hex(0x2F6FED),
            accent_text: hex(0xFFFFFF),
            sel: hex(0x2F6FED),
            sel_inactive: hex(0xDDE1E8),
            folder: hex(0x4A9AF5),
            danger: hex(0xE5484D),
            overlay: Color32::from_black_alpha(120),
        }
    }

    pub fn get(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }

    /// Color de la etiqueta de antigüedad: rojo (nuevo) → azul → gris (viejo).
    pub fn age_color(&self, age_secs: u32) -> Color32 {
        let c = match age_secs {
            0..3600 => 0xFF5D5D,
            3600..86_400 => 0xFF9944,
            86_400..604_800 => 0xF2C94C,
            604_800..2_592_000 => 0x4CCB8A,
            2_592_000..15_552_000 => 0x38C2D6,
            15_552_000..31_536_000 => 0x4C8DFF,
            _ => {
                return if self.dark { hex(0x3A3E47) } else { hex(0xD5D9E0) };
            }
        };
        hex(c)
    }
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn icon_font(size: f32, fill: bool) -> FontId {
    FontId::new(size, FontFamily::Name(if fill { "phosphor-fill" } else { "phosphor" }.into()))
}

pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let read = |f: &str| std::fs::read(format!("C:\\Windows\\Fonts\\{f}")).ok();
    if let Some(b) = read("segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), Arc::new(FontData::from_owned(b)));
        fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "segoe".into());
    }
    let mut semi = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(b) = read("seguisb.ttf") {
        fonts.font_data.insert("segoe-sb".into(), Arc::new(FontData::from_owned(b)));
        semi.insert(0, "segoe-sb".into());
    }
    fonts.families.insert(FontFamily::Name("semibold".into()), semi);
    if let Some(b) = read("consola.ttf") {
        fonts.font_data.insert("consolas".into(), Arc::new(FontData::from_owned(b)));
        fonts.families.get_mut(&FontFamily::Monospace).unwrap().insert(0, "consolas".into());
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    egui_phosphor::add_font_bytes_as_family(&mut fonts, "phosphor-fill", egui_phosphor::bytes::fill::FONT);
    ctx.set_fonts(fonts);
}

pub fn apply_style(ctx: &egui::Context, t: &Theme) {
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.bg;
    v.window_fill = t.chrome;
    v.extreme_bg_color = if t.dark { Color32::from_rgb(0x0F, 0x10, 0x13) } else { Color32::WHITE };
    v.faint_bg_color = t.hover;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.selection.bg_fill = t.accent.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.hyperlink_color = t.accent;
    v.override_text_color = Some(t.text);
    let r = CornerRadius::same(6);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.bg_fill = t.hover;
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.active.bg_fill = t.sel_inactive;
    v.widgets.active.weak_bg_fill = t.sel_inactive;
    v.widgets.active.bg_stroke = Stroke::NONE;
    v.widgets.active.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.open.weak_bg_fill = t.hover;
    v.popup_shadow = egui::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(if t.dark { 120 } else { 40 }) };
    v.window_shadow = v.popup_shadow;
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
        s.spacing.menu_margin = Margin::same(6);
        s.spacing.interact_size.y = 26.0;
        s.spacing.scroll = egui::style::ScrollStyle::floating();
        s.spacing.scroll.floating_width = 8.0;
        s.spacing.scroll.bar_width = 8.0;
        s.text_styles = [
            (TextStyle::Heading, semibold(17.0)),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.5)),
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        s.interaction.selectable_labels = false;
        s.animation_time = 0.12;
    });
}
