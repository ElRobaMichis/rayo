//! Audio y video en la Vista rápida: carátula, póster, barra de transporte. Reproduce con
//! `fsx::media::Player` (el video lo dibuja Windows en una ventana hija; aquí solo se le dice
//! dónde va).
//!
//! Piezas nuevas que viven aquí hasta promoverse a `design.rs`: `transport`, `scrubber`,
//! `play_plate`, `eq_bars`.
// El permiso se va al integrar (hasta entonces solo lo usa la tarjeta vieja).
#![allow(dead_code)]

use super::config::QuickLookCfg;
use super::design::{self, IconBtn, h, icon, radius, sp, stroke};
use super::preview::{PreviewItem, Textures};
use super::ql::{ART, ART_NARROW, OPEN_DELAY, PLAY_PLATE, QL_NARROW};
use super::theme::Theme;
use crate::fsx::media::{Phase, Player, State};
use crate::fsx::{self, props};
use egui::{Align2, Color32, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as ic;

/// Qué reproduce el cuerpo.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Media {
    Audio,
    Video,
}

/// Alto de la barra de transporte y de las zonas sensibles de sus medidores.
const BAR_H: f32 = 44.0;
const ZONE_H: f32 = 20.0;
/// Medidor de volumen: 8 puntos.
const VOL_W: f32 = 8.0 * 5.0 - 2.0;
/// Cada cuánto se manda un salto mientras se arrastra el avance (s).
const SEEK_EVERY: f64 = 0.08;
/// La rueda sobre el volumen se guarda cuando para (s).
const VOL_SETTLE: f64 = 0.4;
/// Mientras suena, la interfaz se repinta sola cada tanto (la posición no la despierta).
const TICK: std::time::Duration = std::time::Duration::from_millis(250);

/// Estado del reproductor en la ventana (arrastre del avance, volumen pendiente…).
#[derive(Default)]
pub struct MediaState {
    /// El archivo de este estado (`PreviewItem::source`).
    path: String,
    /// Desde cuándo se ve (para la demora antes de abrir).
    since: f64,
    /// Ya se le pidió al reproductor.
    opened: bool,
    /// Va a sonar solo al abrir (`cfg.autoplay`, salvo que se haya pausado antes, o solo en la nube).
    auto: bool,
    /// Se pausó antes de que abriera: abre callado.
    held: bool,
    /// Ya hubo un cuadro: el video tapa al póster.
    played: bool,
    /// Arrastrando el avance: fracción en vivo y cuándo fue el último salto.
    drag: Option<f32>,
    seek_at: f64,
    /// Volumen mientras se arrastra o se gira la rueda; se guarda al soltar (o al parar la rueda).
    vol: Option<f32>,
    vol_commit: Option<f64>,
}

/// ¿Se reproduce en la Vista rápida? (extensión sin punto, en minúsculas). Lo que Media
/// Foundation puede abrir; si falta el códec (en LTSC: Vorbis, Opus, VP9, HEVC, AV1), el
/// reproductor queda en `Failed` y se ve el póster o la carátula.
pub fn kind(ext: &str) -> Option<Media> {
    match ext {
        "mp3" | "wav" | "flac" | "m4a" | "aac" | "adts" | "wma" | "aif" | "aiff" | "aifc" | "ogg" | "oga" | "opus" | "mka" | "ac3" | "ec3" | "amr" | "3ga" => Some(Media::Audio),
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "asf" | "3gp" | "3g2" | "mpg" | "mpeg" | "m2ts" | "mts" | "ts" => Some(Media::Video),
        _ => None,
    }
}

/// `m:ss`, o `h:mm:ss` si `long`.
fn clock(ms: u64, long: bool) -> String {
    let s = ms / 1000;
    if long { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// Dibuja el cuerpo de un audio o video en `rect` (a sangre, con su barra de transporte).
/// Some = rectángulo (puntos) donde va la ventana nativa del video.
#[allow(clippy::too_many_arguments)]
pub fn body(ui: &mut Ui, t: &Theme, rect: Rect, item: &PreviewItem, tex: &mut Textures, player: &Player, cfg: &mut QuickLookCfg, st: &mut MediaState) -> Option<Rect> {
    let e = &item.entry;
    let media = kind(&e.ext)?;
    let path = item.source();
    let now = ui.input(|i| i.time);
    if st.path != path {
        // Otro archivo sin pasar por `stop`: el anterior se corta ya, no al abrir el nuevo.
        if st.opened {
            player.close();
        }
        *st = MediaState { path: path.to_string(), since: now, ..Default::default() };
    }
    // Demora antes de abrir: bajar con ↓ por veinte archivos no abre veinte. Solo en la nube:
    // abrirlo lo descargaría; espera a que se pida reproducir.
    let cloud = e.cloud == fsx::Cloud::OnlineOnly;
    if !st.opened {
        st.auto = cfg.autoplay && !cloud && !st.held;
    }
    if !st.opened && !cloud {
        let wait = st.since + OPEN_DELAY - now;
        if wait <= 0.0 {
            player.open(path, st.auto, cfg.volume, cfg.muted);
            st.opened = true;
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
    }
    let s = player.state();
    let mine = st.opened && s.path == path;
    let phase = if mine { s.phase } else { Phase::Idle };
    if phase == Phase::Playing {
        st.played = true;
        ui.ctx().request_repaint_after(TICK);
    }
    let failed = phase == Phase::Failed;
    let narrow = rect.width() < QL_NARROW;
    let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - BAR_H), rect.max);
    let well = if failed { rect } else { Rect::from_min_max(rect.min, pos2(rect.right(), bar.top())) };
    let p = ui.painter_at(rect);
    let mut video = None;
    match media {
        Media::Audio => {
            p.rect_filled(well, 0.0, t.sidebar);
            design::dot_grid(&p, t, well);
            audio_hero(ui, t, well, item, tex, phase == Phase::Playing, narrow);
        }
        Media::Video => {
            p.rect_filled(well, 0.0, t.sidebar);
            let ppp = ui.ctx().pixels_per_point();
            let side = ((well.width().max(well.height()) * ppp) as u32).clamp(256, 1024).next_multiple_of(256);
            // Solo un cuadro de verdad (opaco): sin miniatura, Windows da el ícono del programa.
            let poster = tex.get_now(path, side).and_then(|te| te.pic.as_ref().filter(|_| te.opaque && !te.icon).cloned());
            // El cuadro encajado, sin aire: con la medida del video si ya se sabe, si no la del póster.
            let aspect = s.video.filter(|_| mine).map(|(w, h)| vec2(w as f32, h as f32)).or(poster.as_ref().map(|tx| tx.size_vec2())).unwrap_or(vec2(16.0, 9.0));
            let k = (well.width() / aspect.x).min(well.height() / aspect.y);
            let frame = Rect::from_center_size(well.center(), aspect * k);
            // Píxeles enteros: la ventana nativa no tiene medio píxel.
            let frame = Rect::from_min_max((frame.min.to_vec2() * ppp).round().to_pos2() / ppp, (frame.max.to_vec2() * ppp).round().to_pos2() / ppp);
            match &poster {
                Some(pic) => pic.paint(&p, frame, None, Color32::WHITE),
                None => {
                    p.text(frame.center(), Align2::CENTER_CENTER, ic::FILM_STRIP, design::icon_font(56.0), t.kinds[1]);
                }
            }
            if !failed {
                if !st.played {
                    // Antes del primer cuadro: clic en el póster = reproducir.
                    let r = ui.interact(well, ui.id().with("ql-poster"), Sense::click());
                    let hov = r.hovered();
                    play_plate(&p, t, frame.center(), hov);
                    if r.clicked() {
                        play(player, st, cfg);
                    }
                }
                video = Some(frame);
            }
        }
    }
    if failed {
        // Sin códec: queda el póster o la carátula; la barra se va y abajo, la línea de estado.
        let text = t!("rapida.no_reproduce");
        let w = icon::M + sp::S + design::text_width(&p, text, design::text::small()) + 2.0;
        let r = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - sp::M - h::HEAD / 2.0), vec2(w, h::HEAD));
        let mut sl = ui.new_child(egui::UiBuilder::new().id_salt("ql-media-fail").max_rect(r));
        design::status_line(&mut sl, t, design::Tone::Neutral, "", text);
    } else {
        transport(ui, t, bar, player, cfg, st, &s, mine, narrow, video.is_some());
    }
    video
}

/// Reproducir (abre si todavía no se abrió o si el reproductor lo soltó).
fn play(player: &Player, st: &mut MediaState, cfg: &QuickLookCfg) {
    st.auto = true;
    st.held = false;
    if !st.opened || player.state().path != st.path {
        player.open(&st.path, true, cfg.volume, cfg.muted);
        st.opened = true;
    } else {
        player.play();
    }
}

/// Placa de reproducir sobre el póster: 48, radio `L`, `raised`, `border_strong`, sombra de
/// popover, `PLAY` de 20.
fn play_plate(p: &egui::Painter, t: &Theme, center: Pos2, hovered: bool) {
    let r = Rect::from_center_size(center, vec2(PLAY_PLATE, PLAY_PLATE));
    p.add(design::popover_shadow(t).as_shape(r, radius::L));
    p.rect_filled(r, radius::L, t.raised);
    if hovered {
        p.rect_filled(r, radius::L, t.hover);
    }
    p.rect_stroke(r, radius::L, Stroke::new(stroke::HAIR, t.border_strong), StrokeKind::Inside);
    p.text(center, Align2::CENTER_CENTER, ic::PLAY, design::icon_font(icon::L), t.text);
}

/// Placa (carátula o glifo), título, artista · álbum y formato. Angosta: placa arriba y el
/// texto centrado debajo.
fn audio_hero(ui: &Ui, t: &Theme, well: Rect, item: &PreviewItem, tex: &mut Textures, playing: bool, narrow: bool) {
    let p = ui.painter_at(well);
    let e = &item.entry;
    let info = props::info(item.source());
    let stem = e.name.rsplit_once('.').map_or(e.name.as_str(), |(s, _)| s);
    let title = info.as_ref().map(|i| i.title.trim()).filter(|s| !s.is_empty()).unwrap_or(stem).to_string();
    let row = |f: props::Field| info.as_ref().and_then(|i| i.rows.iter().find(|(k, _)| *k == f).map(|(_, v)| v.clone()));
    let byline = [row(props::Field::Artist), row(props::Field::Album)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let fmt = audio_format(&e.ext, info.as_ref().and_then(|i| i.sample_rate), info.as_ref().and_then(|i| i.bitrate), row(props::Field::Bitrate));
    let art = if narrow { ART_NARROW } else { ART };
    let pad = sp::L;
    // Ancho del texto: lo que queda; el bloque entero se centra.
    let text_max = if narrow { well.width() - 2.0 * pad } else { (well.width() - 2.0 * pad - art - sp::XL).max(40.0) };
    let tg = super::views::elided(&p, &title, design::text::title(), t.text, text_max - if playing { 4.0 * 5.0 + sp::S } else { 0.0 });
    let bg = (!byline.is_empty()).then(|| super::views::elided(&p, &byline, design::text::small(), t.dim, text_max));
    let mg = p.layout_job(design::micro_job(&fmt, t.faint));
    let eq_w = if playing { sp::S + 4.0 * 5.0 } else { 0.0 };
    let text_w = (tg.size().x + eq_w).max(bg.as_ref().map_or(0.0, |g| g.size().x)).max(mg.size().x.min(text_max));
    let text_h = tg.size().y + bg.as_ref().map_or(0.0, |g| sp::XS + g.size().y) + sp::M + mg.size().y;
    let (plate, text_top_left, centered) = if narrow {
        let total_h = art + sp::M + text_h;
        let top = well.center().y - total_h / 2.0;
        (Rect::from_min_size(pos2(well.center().x - art / 2.0, top), vec2(art, art)), pos2(well.center().x, top + art + sp::M), true)
    } else {
        let total_w = art + sp::XL + text_w;
        let left = (well.center().x - total_w / 2.0).max(well.left() + pad);
        let plate = Rect::from_min_size(pos2(left, well.center().y - art / 2.0), vec2(art, art));
        (plate, pos2(plate.right() + sp::XL, well.center().y - text_h / 2.0), false)
    };
    // Placa: la carátula si Windows la tiene (su miniatura); si no, el glifo del tipo.
    // Opaca: sin carátula, Windows da el ícono del programa (con transparencia).
    let cover = tex.get_now(item.source(), 256).and_then(|te| te.tex.as_ref().filter(|_| te.opaque && !te.icon).cloned());
    match cover {
        Some(tx) => {
            // Recortada al cuadrado, como una tapa.
            let sz = tx.size_vec2();
            let (u, v) = if sz.x > sz.y { ((1.0 - sz.y / sz.x) / 2.0, 0.0) } else { (0.0, (1.0 - sz.x / sz.y) / 2.0) };
            let uv = Rect::from_min_max(pos2(u, v), pos2(1.0 - u, 1.0 - v));
            p.add(egui::epaint::RectShape::filled(plate, radius::L, Color32::WHITE).with_texture(tx.id(), uv));
            p.rect_stroke(plate, radius::L, Stroke::new(stroke::HAIR, t.border), StrokeKind::Inside);
        }
        None => {
            p.rect_filled(plate, radius::L, t.raised);
            p.rect_stroke(plate, radius::L, Stroke::new(stroke::HAIR, t.border), StrokeKind::Inside);
            let (glyph, color) = super::icons::for_entry(t, false, &e.ext);
            p.text(plate.center(), Align2::CENTER_CENTER, glyph, design::icon_font(if narrow { 40.0 } else { 56.0 }), color);
        }
    }
    // Texto.
    let line_x = |w: f32| if centered { text_top_left.x - w / 2.0 } else { text_top_left.x };
    let mut y = text_top_left.y;
    let title_w = tg.size().x + eq_w;
    let tx0 = line_x(title_w);
    let th = tg.size().y;
    p.galley(pos2(tx0, y), tg.clone(), t.text);
    if playing {
        eq_bars(&p, t, pos2(tx0 + tg.size().x + sp::S, y + th / 2.0 + 6.0), ui.input(|i| i.time));
    }
    y += th;
    if let Some(g) = bg {
        y += sp::XS;
        let w = g.size().x;
        p.galley(pos2(line_x(w), y), g.clone(), t.dim);
        y += g.size().y;
    }
    y += sp::M;
    let mw = mg.size().x;
    p.galley(pos2(line_x(mw), y), mg, t.faint);
}

/// Micro-etiqueta del formato: «WAV · 48 kHz · 1411 kbps» (en mayúsculas al dibujarla). Número y
/// unidad separados; si Windows no da el número de la calidad, su texto con el espacio puesto.
fn audio_format(ext: &str, rate_hz: Option<u64>, bps: Option<u64>, bitrate_text: Option<String>) -> String {
    let mut parts = vec![ext.to_uppercase()];
    if let Some(hz) = rate_hz.filter(|&h| h > 0) {
        // Hasta dos decimales, sin ceros de más: 48, 44.1, 22.05.
        let k = format!("{:.2}", hz as f64 / 1000.0);
        parts.push(format!("{} kHz", k.trim_end_matches('0').trim_end_matches('.')));
    }
    if let Some(b) = bps.filter(|&b| b > 0) {
        parts.push(format!("{} kbps", (b as f64 / 1000.0).round() as u64));
    } else if let Some(text) = bitrate_text {
        // «352kbps» → «352 kbps».
        let digits: String = text.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
        let unit = text[digits.len()..].trim();
        parts.push(if digits.is_empty() || unit.is_empty() { text.clone() } else { format!("{digits} {unit}") });
    }
    parts.join(" · ")
}

/// «Sonando»: cuatro barras de 3 en `accent` que suben y bajan a saltos (12 de alto como
/// máximo, apoyadas en `base`).
fn eq_bars(p: &egui::Painter, t: &Theme, base: Pos2, time: f64) {
    for i in 0..4 {
        // Cinco escalones por ciclo de 0,9 s, desfasados como en la maqueta.
        let ph = ((time / 0.9 + [0.0, 0.33, 0.67, 0.17][i]) % 2.0) as f32;
        let k = if ph < 1.0 { ph } else { 2.0 - ph };
        let hgt = 3.0 + ((k * 4.0).round() / 4.0) * 9.0;
        let x = base.x + i as f32 * 5.0;
        p.rect_filled(Rect::from_min_max(pos2(x, base.y - hgt), pos2(x + 3.0, base.y)), 0.0, t.accent);
    }
}

/// Barra de transporte: alto 44, `panel`, hairline arriba.
/// `[8][▶ 28][8][0:17][8][ avance ][8][0:42][8]|[8][🔊 28][4][volumen][12]`
/// `native`: arriba está la ventana del video, que taparía los tooltips (abren hacia arriba): los
/// de los botones van a un costado, dentro de la barra, y el del avance no se abre (el tiempo bajo
/// el puntero se ve en el reloj de la izquierda, como al arrastrar).
#[allow(clippy::too_many_arguments)]
fn transport(ui: &mut Ui, t: &Theme, bar: Rect, player: &Player, cfg: &mut QuickLookCfg, st: &mut MediaState, s: &State, mine: bool, narrow: bool, native: bool) {
    let tip = |r: egui::Response, text: &str, side: egui::RectAlign| if native { design::tip_keys_beside(r, t, text, "", side) } else { design::tip(r, t, text) };
    let now = ui.input(|i| i.time);
    let p = ui.painter_at(bar);
    p.rect_filled(bar, 0.0, t.chrome);
    p.line_segment([bar.left_top(), bar.right_top()], Stroke::new(stroke::HAIR, t.border));
    let cy = bar.center().y;
    let phase = if mine { s.phase } else { Phase::Idle };
    let dur = if mine { s.dur_ms } else { 0 };
    let pos = if mine { s.pos_ms } else { 0 };
    // Volumen pendiente de la rueda: se guarda cuando para.
    if let Some(at) = st.vol_commit {
        if now >= at {
            commit_volume(st, cfg);
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(at - now));
        }
    }
    // Reproducir / pausa.
    let mut x = bar.left() + sp::S;
    let pr = Rect::from_center_size(pos2(x + h::CTL / 2.0, cy), vec2(h::CTL, h::CTL));
    // Antes de abrir, el botón ya dice lo que va a pasar.
    let playing = phase == Phase::Playing || (st.auto && (!st.opened || phase == Phase::Opening));
    let r = design::icon_button_at(ui, t, pr, ui.id().with("ql-play"), if playing { ic::PAUSE } else { ic::PLAY }, IconBtn::default());
    let r = tip(r, if playing { t!("rapida.pausar") } else { t!("rapida.reproducir") }, egui::RectAlign::RIGHT);
    if r.clicked() {
        if playing {
            player.pause();
            // Todavía abriendo (o esperando la demora): que abra callado.
            st.held = true;
            st.auto = false;
        } else {
            play(player, st, cfg);
        }
    }
    x = pr.right() + sp::S;
    // Tiempos: ancho fijo según el total, para que el avance no baile.
    let long = dur >= 3_600_000;
    let total = clock(dur, long);
    let tw = design::text_width(&p, &total, design::text::mono());
    // El reloj se dibuja después del avance (puede mostrar el tiempo bajo el puntero).
    let clock_at = pos2(x + tw, cy);
    x += tw + sp::S;
    // Lado derecho, de afuera hacia adentro.
    let mut right = bar.right() - sp::M;
    let vol_r = (!narrow).then(|| {
        let r = Rect::from_min_max(pos2(right - VOL_W, cy - ZONE_H / 2.0), pos2(right, cy + ZONE_H / 2.0));
        right = r.left() - sp::XS;
        r
    });
    let spk = Rect::from_min_max(pos2(right - h::CTL, cy - h::CTL / 2.0), pos2(right, cy + h::CTL / 2.0));
    right = spk.left() - sp::S;
    p.line_segment([pos2(right, cy - 8.0), pos2(right, cy + 8.0)], Stroke::new(stroke::HAIR, t.border));
    right -= sp::S;
    p.text(pos2(right, cy), Align2::RIGHT_CENTER, &total, design::text::mono(), t.dim);
    right -= tw + sp::S;
    // Avance.
    let scrub = Rect::from_min_max(pos2(x, cy - ZONE_H / 2.0), pos2(right.max(x + 1.0), cy + ZONE_H / 2.0));
    let frac = st.drag.unwrap_or(if dur > 0 { pos as f32 / dur as f32 } else { 0.0 });
    let (scrubbed, hover) = scrubber(ui, t, scrub, "ql-scrub", frac, dur > 0 && mine, !native, |f| clock((f as f64 * dur as f64) as u64, long));
    let shown = st.drag.or(hover.filter(|_| native));
    let shown_ms = shown.map_or(pos, |f| (f as f64 * dur as f64) as u64);
    p.text(clock_at, Align2::RIGHT_CENTER, clock(shown_ms, long), design::text::mono(), t.text);
    if let Some(f) = scrubbed {
        match f {
            Scrub::Live(f) => {
                st.drag = Some(f);
                if now - st.seek_at >= SEEK_EVERY {
                    st.seek_at = now;
                    player.seek((f as f64 * dur as f64) as u64);
                }
            }
            Scrub::Done(f) => {
                st.drag = None;
                player.seek((f as f64 * dur as f64) as u64);
            }
        }
    }
    // Volumen: botón (silencio) y medidor de 8 puntos.
    let vol = st.vol.unwrap_or(cfg.volume);
    let muted = cfg.muted && st.vol.is_none();
    let glyph = if muted || vol <= 0.001 {
        ic::SPEAKER_X
    } else if vol < 0.5 {
        ic::SPEAKER_LOW
    } else {
        ic::SPEAKER_HIGH
    };
    let r = design::icon_button_at(ui, t, spk, ui.id().with("ql-mute"), glyph, IconBtn::default());
    let r = tip(r, if muted { t!("rapida.activar_sonido") } else { t!("rapida.silenciar") }, egui::RectAlign::LEFT);
    if r.clicked() {
        commit_volume(st, cfg);
        cfg.muted = !cfg.muted;
        if !cfg.muted && cfg.volume <= 0.001 {
            cfg.volume = QuickLookCfg::default().volume;
        }
        player.set_volume(cfg.volume, cfg.muted);
    }
    if let Some(vr) = vol_r {
        let id = ui.id().with("ql-vol");
        let r = ui.interact(vr, id, Sense::click_and_drag());
        let hov = design::hover_amount(ui.ctx(), id, r.hovered() || r.dragged());
        if hov > 0.01 {
            p.rect_filled(vr.expand2(vec2(sp::XS, 0.0)), radius::S, t.hover.gamma_multiply(hov));
        }
        design::dot_meter(&p, t, vr, if muted { 0.0 } else { vol }, t.accent);
        let at = |x: f32| ((x - vr.left() + 1.0) / vr.width()).clamp(0.0, 1.0);
        if (r.dragged() || r.drag_started()) && let Some(pp) = r.interact_pointer_pos() {
            let v = at(pp.x);
            st.vol = Some(v);
            st.vol_commit = None;
            player.set_volume(v, false);
        }
        if (r.drag_stopped() || r.clicked()) && let Some(pp) = r.interact_pointer_pos() {
            st.vol = Some(at(pp.x));
            commit_volume(st, cfg);
            player.set_volume(cfg.volume, cfg.muted);
        }
        if r.hovered() {
            // De a una muesca (la rueda cruda, no el desplazamiento suavizado).
            let dy: f32 = ui.input(|i| i.raw.events.iter().map(|e| if let egui::Event::MouseWheel { delta, .. } = e { delta.y } else { 0.0 }).sum());
            if dy != 0.0 {
                let base = if muted { 0.0 } else { vol };
                let v = ((base + 0.1 * dy.signum()) * 10.0).round() / 10.0;
                let v = v.clamp(0.0, 1.0);
                st.vol = Some(v);
                st.vol_commit = Some(now + VOL_SETTLE);
                player.set_volume(v, false);
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(VOL_SETTLE));
            }
        }
        tip(r, &format!("{} · {}%", t!("rapida.volumen"), (if muted { 0.0 } else { vol } * 100.0).round() as u32), egui::RectAlign::LEFT);
    }
}

/// Guarda en `cfg` el volumen pendiente (al soltar, al parar la rueda): fuera del silencio.
fn commit_volume(st: &mut MediaState, cfg: &mut QuickLookCfg) {
    st.vol_commit = None;
    if let Some(v) = st.vol.take() {
        cfg.volume = v;
        cfg.muted = false;
    }
}

enum Scrub {
    /// Arrastrando: fracción en vivo.
    Live(f32),
    /// Soltó o hizo clic: fracción final.
    Done(f32),
}

/// Avance arrastrable sobre el medidor de puntos (3 × 3 cada 5) en `accent`, a todo `zone`.
/// Hover: capa `hover` radio `S`, raya de 1.5 × 14 donde caería el clic y tooltip con ese valor
/// (`label`; sin él si `!tips`). Clic o arrastre: salta ahí. Devuelve también la fracción bajo el
/// puntero.
#[allow(clippy::too_many_arguments)]
fn scrubber(ui: &mut Ui, t: &Theme, zone: Rect, id: &str, frac: f32, enabled: bool, tips: bool, label: impl Fn(f32) -> String) -> (Option<Scrub>, Option<f32>) {
    let id = ui.id().with(id);
    let r = ui.interact(zone, id, if enabled { Sense::click_and_drag() } else { Sense::hover() });
    let p = ui.painter_at(zone.expand(1.0));
    let meter = zone.shrink2(vec2(sp::XS, 0.0));
    let at = |x: f32| ((x - meter.left()) / meter.width().max(1.0)).clamp(0.0, 1.0);
    let active = enabled && (r.hovered() || r.dragged());
    let hov = design::hover_amount(ui.ctx(), id, active);
    if hov > 0.01 {
        p.rect_filled(zone, radius::S, t.hover.gamma_multiply(hov));
    }
    design::dot_meter(&p, t, meter, frac, t.accent);
    let mut out = None;
    let mut hover = None;
    if active && let Some(pp) = ui.input(|i| i.pointer.hover_pos()).or(r.interact_pointer_pos()) {
        let x = pp.x.clamp(meter.left(), meter.right());
        p.rect_filled(Rect::from_center_size(pos2(x, zone.center().y), vec2(1.5, 14.0)), 0.0, t.accent);
        let f = at(x);
        hover = Some(f);
        if tips {
            let tip = label(f);
            let mut tt = egui::Tooltip::for_widget(&r).at_pointer().gap(sp::M);
            tt.popup = tt.popup.frame(design::tooltip_frame(t));
            tt.show(|ui| ui.label(egui::RichText::new(tip).font(design::text::mono()).color(t.text)));
        }
    }
    if enabled && let Some(pp) = r.interact_pointer_pos() {
        if r.drag_stopped() || r.clicked() {
            out = Some(Scrub::Done(at(pp.x)));
        } else if r.dragged() || r.drag_started() {
            out = Some(Scrub::Live(at(pp.x)));
        }
    }
    (out, hover)
}

/// ← → = ∓5 s. ¿La consumió?
pub fn key(st: &mut MediaState, player: &Player, key: Key) -> bool {
    let d: i64 = match key {
        Key::ArrowLeft => -5000,
        Key::ArrowRight => 5000,
        _ => return false,
    };
    let s = player.state();
    if !st.opened || s.path != st.path || !matches!(s.phase, Phase::Playing | Phase::Paused | Phase::Ended) || s.dur_ms == 0 {
        return false;
    }
    let to = (s.pos_ms as i64 + d).clamp(0, s.dur_ms as i64) as u64;
    player.seek(to);
    st.drag = None;
    true
}

/// Corta el sonido y suelta el archivo (al cerrar o cambiar de archivo).
pub fn stop(st: &mut MediaState, player: &Player) {
    player.close();
    *st = MediaState::default();
}

#[cfg(test)]
mod tests {
    use super::audio_format;

    #[test]
    fn audio_format_separates_number_and_unit() {
        assert_eq!(audio_format("wav", Some(48_000), Some(1_411_200), None), "WAV · 48 kHz · 1411 kbps");
        assert_eq!(audio_format("mp3", Some(44_100), Some(320_000), None), "MP3 · 44.1 kHz · 320 kbps");
        assert_eq!(audio_format("wav", Some(22_050), Some(352_800), None), "WAV · 22.05 kHz · 353 kbps");
        // Sin el número: el texto de Windows con el espacio puesto.
        assert_eq!(audio_format("wav", None, None, Some("352kbps".into())), "WAV · 352 kbps");
        assert_eq!(audio_format("flac", None, None, None), "FLAC");
    }
}
