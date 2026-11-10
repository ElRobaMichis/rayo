//! Paleta, tipografía y estilo de la interfaz.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle, Visuals};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    // Superficies, de la más hundida a la que flota (ver `design.rs`).
    /// Hundida: barra lateral, campos, pozos (vista previa, código), terminal.
    pub sidebar: Color32,
    /// Contenido: listas, pestaña activa, barra de herramientas.
    pub bg: Color32,
    /// Panel: tira de pestañas, barra de estado, panel de vista previa.
    pub chrome: Color32,
    /// Lo que flota: menús, popovers, tooltips, avisos, diálogos.
    pub raised: Color32,
    pub border: Color32,
    /// Contorno de controles y de lo que flota.
    pub border_strong: Color32,
    // Capas de estado: alfa, valen sobre cualquier superficie.
    pub hover: Color32,
    /// Presionado, seleccionado sin foco, ítem activo de navegación.
    pub sel_inactive: Color32,
    // Texto.
    pub text: Color32,
    pub dim: Color32,
    pub faint: Color32,
    pub disabled: Color32,
    /// Texto sobre un relleno de acento o de peligro.
    pub accent_text: Color32,
    // Acento: el azul es «ahora» (foco, selección con foco, acción primaria, lo instantáneo).
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_fill: Color32,
    pub accent_fill_hover: Color32,
    pub accent_fill_pressed: Color32,
    pub accent_soft: Color32,
    /// Selección con foco (tinte de acento; el texto no cambia de color).
    pub sel: Color32,
    pub folder: Color32,
    // Estado: texto y fondo suave.
    pub ok: Color32,
    pub ok_soft: Color32,
    pub warn: Color32,
    pub warn_soft: Color32,
    pub danger: Color32,
    pub danger_soft: Color32,
    pub danger_fill: Color32,
    /// Trama de puntos (firma de Rayo: vacíos, pozo de la vista previa).
    pub dots: Color32,
    pub overlay: Color32,
    // Colores de contenido (no de interfaz): siempre de aquí, nunca literales sueltos.
    /// Etiquetas de color, en el orden de `fsx::tags::COLORS`: rojo, naranja, amarillo, verde,
    /// azul, morado, gris.
    pub tags: [Color32; 7],
    /// Tipos de archivo (glifo grande), los de `icons.rs`: morado (imagen), rosa (video, audio),
    /// rojo (pdf), naranja (código, presentación), ámbar (comprimido), verde (planilla), turquesa
    /// (ejecutable), azul (documento).
    pub kinds: [Color32; 8],
    /// Resaltado de código: palabra clave, cadena, número, tipo.
    pub syntax: [Color32; 4],
    /// Los 16 colores básicos de la terminal: los 8 normales salen de los tokens y los 8
    /// brillantes son los mismos mezclados 15 % hacia `text`.
    pub ansi: [Color32; 16],
}

const fn hex(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// `c` con alfa `a` (0-1).
fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a * 255.0).round() as u8)
}

/// `a` mezclado una fracción `k` hacia `b` (colores opacos).
fn mix(a: Color32, b: Color32, k: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

const TAGS: [Color32; 7] = [hex(0xFF5F57), hex(0xFF9F0A), hex(0xFFD60A), hex(0x32D74B), hex(0x0A84FF), hex(0xBF5AF2), hex(0x98989D)];
const KINDS: [Color32; 8] = [hex(0xB57CFF), hex(0xF472B6), hex(0xF0605D), hex(0xF59E42), hex(0xE9B949), hex(0x3FC07A), hex(0x2EC4B6), hex(0x5B9BFF)];

/// Negro `faint`, rojo `danger`, verde `ok`, amarillo `warn`, azul `accent`, magenta `syntax[0]`,
/// cian `syntax[3]`, blanco `text`; después los brillantes.
fn ansi(t: &Theme) -> [Color32; 16] {
    let base = [t.faint, t.danger, t.ok, t.warn, t.accent, t.syntax[0], t.syntax[3], t.text];
    std::array::from_fn(|i| if i < 8 { base[i] } else { mix(base[i - 8], t.text, 0.15) })
}

impl Theme {
    pub fn dark() -> Self {
        let accent = hex(0x4C8DFF);
        let mut t = Theme {
            dark: true,
            sidebar: hex(0x0F0F10),
            bg: hex(0x141415),
            chrome: hex(0x19191B),
            raised: hex(0x1E1E21),
            border: hex(0x232326),
            border_strong: hex(0x303036),
            hover: Color32::from_white_alpha(15),
            sel_inactive: Color32::from_white_alpha(26),
            text: hex(0xEDEDEF),
            dim: hex(0xA8A8B0),
            faint: hex(0x82828C),
            disabled: hex(0x55555E),
            accent_text: hex(0xFFFFFF),
            accent,
            accent_hover: hex(0x6EA4FF),
            accent_fill: hex(0x3366D6),
            accent_fill_hover: hex(0x3D72E6),
            accent_fill_pressed: hex(0x2B59BF),
            accent_soft: alpha(accent, 0.14),
            sel: alpha(accent, 0.24),
            folder: hex(0x5EA6F8),
            ok: hex(0x4CC38A),
            ok_soft: alpha(hex(0x4CC38A), 0.14),
            warn: hex(0xF2B84B),
            warn_soft: alpha(hex(0xF2B84B), 0.14),
            danger: hex(0xFF6B6B),
            danger_soft: alpha(hex(0xFF6B6B), 0.14),
            danger_fill: hex(0xD93D42),
            dots: alpha(hex(0xEDEDEF), 0.12),
            overlay: Color32::from_black_alpha(153),
            tags: TAGS,
            kinds: KINDS,
            syntax: [hex(0xC792EA), hex(0x9ECE6A), hex(0xF7A35C), hex(0x5FC8D8)],
            ansi: [Color32::BLACK; 16],
        };
        t.ansi = ansi(&t);
        t
    }

    pub fn light() -> Self {
        let accent = hex(0x2563EB);
        let mut t = Theme {
            dark: false,
            sidebar: hex(0xF5F5F6),
            bg: hex(0xFFFFFF),
            chrome: hex(0xFAFAFB),
            raised: hex(0xFFFFFF),
            border: hex(0xEBEBEE),
            border_strong: hex(0xDCDCE1),
            hover: Color32::from_black_alpha(10),
            sel_inactive: Color32::from_black_alpha(20),
            text: hex(0x18181B),
            dim: hex(0x52525B),
            faint: hex(0x71717A),
            disabled: hex(0xA1A1AA),
            accent_text: hex(0xFFFFFF),
            accent,
            accent_hover: hex(0x1D4ED8),
            accent_fill: hex(0x2563EB),
            accent_fill_hover: hex(0x1D58D8),
            accent_fill_pressed: hex(0x1A4DBF),
            accent_soft: alpha(accent, 0.10),
            sel: alpha(accent, 0.16),
            folder: hex(0x4A9AF5),
            ok: hex(0x18794E),
            ok_soft: alpha(hex(0x18794E), 0.10),
            warn: hex(0x9A6700),
            warn_soft: alpha(hex(0x9A6700), 0.12),
            danger: hex(0xCE2C31),
            danger_soft: alpha(hex(0xCE2C31), 0.10),
            danger_fill: hex(0xCE2C31),
            dots: alpha(hex(0x18181B), 0.14),
            overlay: Color32::from_black_alpha(82),
            tags: TAGS,
            kinds: KINDS,
            syntax: [hex(0x8B3FD9), hex(0x2E7D32), hex(0xB35C00), hex(0x0B7285)],
            ansi: [Color32::BLACK; 16],
        };
        t.ansi = ansi(&t);
        t
    }

    pub fn get(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn icon_font(size: f32, fill: bool) -> FontId {
    FontId::new(size, FontFamily::Name(if fill { "phosphor-fill" } else { "phosphor" }.into()))
}

/// Hay que dibujar texto en chino, japonés o coreano (las fuentes de Windows se cargan una vez).
static CJK_WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CJK_LOADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// ¿Este texto necesita una fuente CJK? Si sí, se pide cargarla.
pub fn want_cjk(text: &str) {
    if CJK_LOADED.load(std::sync::atomic::Ordering::Relaxed) || text.is_ascii() {
        return;
    }
    let cjk = text.chars().any(|c| matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x1100..=0x11FF));
    if cjk {
        CJK_WANTED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Si apareció texto CJK y todavía no están sus fuentes: agregarlas (Microsoft YaHei y Malgun
/// Gothic, de Windows) como respaldo. Solo entonces: pesan unos 23 MB.
pub fn load_cjk_if_needed(ctx: &egui::Context) {
    use std::sync::atomic::Ordering;
    if CJK_WANTED.load(Ordering::Relaxed) && !CJK_LOADED.swap(true, Ordering::Relaxed) {
        ctx.set_fonts(definitions(true));
        crate::index::mem_log("fuentes CJK cargadas");
    }
}

pub fn setup_fonts(ctx: &egui::Context) {
    ctx.set_fonts(definitions(false));
}

/// Geist y Geist Mono (SIL OFL 1.1, ver `assets/fonts/OFL.txt`), variables: un archivo por familia
/// y el peso se elige con el eje `wght`.
const GEIST: &[u8] = include_bytes!("../../assets/fonts/Geist-Variable.ttf");
const GEIST_MONO: &[u8] = include_bytes!("../../assets/fonts/GeistMono-Variable.ttf");

fn weight(bytes: &'static [u8], w: f32) -> Arc<FontData> {
    let coords = egui::epaint::text::VariationCoords::new([(b"wght", w)]);
    Arc::new(FontData::from_static(bytes).tweak(egui::FontTweak { coords, ..Default::default() }))
}

fn definitions(cjk: bool) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let read = |f: &str| std::fs::read(format!("C:\\Windows\\Fonts\\{f}")).ok();
    fonts.font_data.insert("geist".into(), weight(GEIST, 400.0));
    fonts.font_data.insert("geist-medium".into(), weight(GEIST, 500.0));
    fonts.font_data.insert("geist-mono".into(), weight(GEIST_MONO, 400.0));
    // Respaldo: Segoe UI y Consolas de Windows (cirílico, griego, árabe en nombres de archivo).
    let mut prop = vec!["geist".to_string()];
    let mut medium = vec!["geist-medium".to_string()];
    let mut mono = vec!["geist-mono".to_string()];
    for (name, file, list) in [("segoe", "segoeui.ttf", &mut prop), ("segoe-sb", "seguisb.ttf", &mut medium), ("consolas", "consola.ttf", &mut mono)] {
        if let Some(b) = read(file) {
            fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(b)));
            list.push(name.into());
        }
    }
    let egui_prop = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let egui_mono = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    medium.extend(prop.iter().skip(1).cloned());
    prop.extend(egui_prop.iter().cloned());
    medium.extend(egui_prop.iter().cloned());
    mono.extend(egui_mono.iter().cloned());
    fonts.families.insert(FontFamily::Proportional, prop);
    fonts.families.insert(FontFamily::Monospace, mono);
    fonts.families.insert(FontFamily::Name("semibold".into()), medium);
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    egui_phosphor::add_font_bytes_as_family(&mut fonts, "phosphor-fill", egui_phosphor::bytes::fill::FONT);
    if cjk {
        for (name, file) in [("yahei", "msyh.ttc"), ("malgun", "malgun.ttf")] {
            if let Some(b) = read(file) {
                fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(b)));
                for fam in fonts.families.values_mut() {
                    fam.push(name.into());
                }
            }
        }
    }
    fonts
}

pub fn apply_style(ctx: &egui::Context, t: &Theme) {
    use super::design::{dur, radius, text};
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.bg;
    v.window_fill = t.raised;
    v.extreme_bg_color = t.sidebar;
    v.text_edit_bg_color = Some(t.sidebar);
    v.faint_bg_color = t.hover;
    v.window_stroke = Stroke::new(1.0, t.border_strong);
    v.window_corner_radius = CornerRadius::same(radius::XL as u8);
    v.menu_corner_radius = CornerRadius::same(radius::L as u8);
    v.selection.bg_fill = t.sel;
    // La selección no cambia el color del texto (4.0).
    v.selection.stroke = Stroke::new(1.0, t.text);
    v.text_cursor.stroke = Stroke::new(super::design::stroke::FOCUS, t.accent);
    v.hyperlink_color = t.accent;
    v.override_text_color = Some(t.text);
    let r = CornerRadius::same(radius::M as u8);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = r;
    }
    // ui.separator()
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    // El botón de egui por defecto es el «secundario» de Rayo: transparente con contorno (los menús
    // de egui le quitan el contorno solos).
    v.widgets.inactive.bg_fill = t.hover;
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.border_strong);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.border_strong);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.active.bg_fill = t.sel_inactive;
    v.widgets.active.weak_bg_fill = t.sel_inactive;
    v.widgets.active.bg_stroke = Stroke::new(1.0, t.border_strong);
    v.widgets.active.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.open.weak_bg_fill = t.sel_inactive;
    v.widgets.open.bg_stroke = Stroke::new(1.0, t.border_strong);
    v.popup_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(if t.dark { 110 } else { 36 }) };
    v.window_shadow = egui::Shadow { offset: [0, 16], blur: 48, spread: 0, color: Color32::from_black_alpha(if t.dark { 140 } else { 48 }) };
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 4.0);
        s.spacing.button_padding = egui::vec2(12.0, 4.0);
        s.spacing.menu_margin = Margin::same(4);
        s.spacing.window_margin = Margin::same(24);
        s.spacing.interact_size.y = 28.0;
        s.spacing.scroll = egui::style::ScrollStyle::floating();
        s.spacing.scroll.floating_width = 4.0;
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.handle_min_length = 24.0;
        s.spacing.scroll.dormant_handle_opacity = 0.20;
        s.spacing.scroll.active_handle_opacity = 0.45;
        s.spacing.scroll.interact_handle_opacity = 0.70;
        s.text_styles = [
            (TextStyle::Heading, text::title()),
            (TextStyle::Body, text::body()),
            (TextStyle::Button, text::body()),
            (TextStyle::Small, text::small()),
            (TextStyle::Monospace, text::mono()),
        ]
        .into();
        s.interaction.selectable_labels = false;
        s.animation_time = dur::BASE;
    });
}
