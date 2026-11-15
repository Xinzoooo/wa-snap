//! Bilder und Videos direkt in der App ansehen: Overlay über dem ganzen Fenster mit
//! Zoom, Verschieben, Blättern durch die Medien des Chats und einem Videoplayer.

use std::path::PathBuf;

use eframe::egui::{self, Color32, RichText};

use crate::backend::{Cmd, MediaRef};
use crate::video;
use crate::App;

const BAR_H: f32 = 48.0;
const CTRL_H: f32 = 52.0;
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 12.0;

/// Zustand des geöffneten Betrachters.
pub struct Viewer {
    /// Schlüssel des Chats, durch dessen Medien geblättert wird.
    pub chat: String,
    /// Nachricht, die gerade gezeigt wird.
    pub id: String,
    zoom: f32,
    pan: egui::Vec2,
    /// Bild in voller Auflösung (bis dahin wird die Vorschau gezeigt).
    tex: Option<egui::TextureHandle>,
    player: Option<video::Player>,
    vtex: Option<egui::TextureHandle>,
    /// Datei auf der Platte, sobald sie geladen ist.
    path: Option<PathBuf>,
    error: Option<String>,
    fullscreen: bool,
    /// Position des Reglers, solange er gezogen wird.
    scrub: Option<f32>,
    volume: f32,
}

/// Was der Betrachter anzeigen kann.
pub fn viewable(m: &MediaRef) -> bool {
    matches!(m.kind.as_str(), "image" | "sticker" | "video" | "gif")
}

fn is_video(m: &MediaRef) -> bool {
    matches!(m.kind.as_str(), "video" | "gif")
}

/// Größe, mit der ein Inhalt (w×h) ganz in `area` passt (kleine Bilder werden nicht vergrößert).
pub fn fit_size(content: egui::Vec2, area: egui::Vec2) -> egui::Vec2 {
    if content.x <= 0.0 || content.y <= 0.0 {
        return egui::Vec2::ZERO;
    }
    let s = (area.x / content.x).min(area.y / content.y).min(1.0);
    content * s
}

/// Neuer Zoom und Versatz, wenn um `factor` gezoomt wird und der Punkt `anchor`
/// (relativ zur Bildmitte ohne Versatz) unter dem Mauszeiger bleiben soll.
pub fn zoom_at(zoom: f32, pan: egui::Vec2, factor: f32, anchor: egui::Vec2) -> (f32, egui::Vec2) {
    let new = (zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    let real = new / zoom;
    (new, anchor - (anchor - pan) * real)
}

fn fmt_pos(secs: f32) -> String {
    let s = secs.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

impl App {
    /// Medien des Chats in zeitlicher Reihenfolge, durch die geblättert werden kann.
    fn viewer_items(&self, chat: &str) -> Vec<(String, MediaRef)> {
        self.chats
            .get(chat)
            .map(|c| {
                c.messages
                    .iter()
                    .filter(|m| !m.x.view_once && !m.id.is_empty())
                    .filter_map(|m| m.media.as_ref().filter(|md| viewable(md)).map(|md| (m.id.clone(), md.clone())))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Betrachter für eine Nachricht öffnen (oder zu ihr wechseln).
    pub fn open_viewer(&mut self, ctx: &egui::Context, chat: String, id: String, media: MediaRef) {
        let (fullscreen, volume) = self.viewer.as_ref().map(|v| (v.fullscreen, v.volume)).unwrap_or((false, 1.0));
        self.viewer = Some(Viewer {
            chat,
            id: id.clone(),
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            tex: None,
            player: None,
            vtex: None,
            path: None,
            error: None,
            fullscreen,
            scrub: None,
            volume,
        });
        // Eigene Tastenkürzel für Strg +/−: die eingebaute Oberflächen-Skalierung solange abschalten.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let max_side = ctx.input(|i| i.max_texture_side).min(3840) as u32;
        let _ = self.cmd.send(Cmd::ViewMedia { id, media, max_side });
    }

    pub fn close_viewer(&mut self, ctx: &egui::Context) {
        if let Some(v) = self.viewer.take() {
            if v.fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
        }
        ctx.options_mut(|o| o.zoom_with_keyboard = true);
    }

    /// Antwort des Backends: Datei liegt vor, Bild ist dekodiert bzw. Video vermessen.
    pub fn viewer_ready(&mut self, ctx: &egui::Context, id: String, path: PathBuf, image: Option<([usize; 2], Vec<u8>)>, video: Option<Result<video::Info, String>>) {
        let Some(v) = self.viewer.as_mut().filter(|v| v.id == id) else { return };
        v.path = Some(path.clone());
        if let Some((size, rgba)) = image {
            let img = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
            v.tex = Some(ctx.load_texture(format!("view-{id}"), img, egui::TextureOptions::LINEAR));
        }
        match video {
            Some(Ok(info)) => {
                let mut p = video::Player::with_info(&path, info);
                p.volume = v.volume;
                p.play_from(0.0);
                v.error = p.error.clone();
                v.player = Some(p);
            }
            Some(Err(e)) => v.error = Some(e),
            None => {}
        }
    }

    pub fn viewer_failed(&mut self, id: &str, err: String) {
        if let Some(v) = self.viewer.as_mut().filter(|v| v.id == id) {
            v.error = Some(err);
        }
    }

    /// Overlay zeichnen und bedienen. Nur aufrufen, wenn ein Betrachter offen ist.
    pub fn viewer_ui(&mut self, ctx: &egui::Context) {
        let Some(v) = self.viewer.as_ref() else { return };
        let items = self.viewer_items(&v.chat);
        let Some(index) = items.iter().position(|(id, _)| *id == v.id) else {
            self.close_viewer(ctx);
            return;
        };
        let media = items[index].1.clone();
        let video = is_video(&media);
        let (chat, id) = (v.chat.clone(), v.id.clone());
        // Kopfzeile: Absender und Zeit
        let title = self
            .chats
            .get(&chat)
            .and_then(|c| c.messages.iter().find(|m| m.id == id))
            .map(|m| {
                let who = if m.from_me { t!("Du", "You").to_owned() } else { m.sender_name.clone() };
                format!("{who} · {} {}", crate::day_label(m.ts), crate::fmt_time(m.ts))
            })
            .unwrap_or_default();
        let preview = self.tex(&id).map(|t| (t.0.id(), t.0.size_vec2()));

        // Tasten
        use egui::{Key, Modifiers};
        let (mut close, mut step, mut zoom_key, mut reset, mut toggle_play, mut toggle_full, mut jump) = (false, 0i32, 0i32, false, false, false, 0.0f32);
        ctx.input_mut(|i| {
            close = i.consume_key(Modifiers::NONE, Key::Escape);
            if i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                step = -1;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowRight) {
                step = 1;
            }
            for m in [Modifiers::COMMAND, Modifiers::NONE] {
                if i.consume_key(m, Key::Plus) || i.consume_key(m, Key::Equals) {
                    zoom_key = 1;
                }
                if i.consume_key(m, Key::Minus) {
                    zoom_key = -1;
                }
                if i.consume_key(m, Key::Num0) {
                    reset = true;
                }
            }
            toggle_play = i.consume_key(Modifiers::NONE, Key::Space);
            toggle_full = i.consume_key(Modifiers::NONE, Key::F) || i.consume_key(Modifiers::NONE, Key::F11);
            if i.consume_key(Modifiers::NONE, Key::Comma) {
                jump = -5.0;
            }
            if i.consume_key(Modifiers::NONE, Key::Period) {
                jump = 5.0;
            }
        });

        let (mut save, mut external) = (false, false);
        let screen = ctx.screen_rect();
        let v = self.viewer.as_mut().expect("oben geprüft");
        egui::Area::new(egui::Id::new("media_viewer")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (_, bg) = ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
            ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(240));
            let stage = egui::Rect::from_min_max(
                screen.min + egui::vec2(0.0, BAR_H),
                screen.max - egui::vec2(0.0, if video { CTRL_H } else { 0.0 }),
            );

            if video {
                // Neues Bild vom Dekoder übernehmen
                if let Some(p) = v.player.as_mut() {
                    if let Some(f) = p.poll() {
                        let img = egui::ColorImage::from_rgba_premultiplied([p.size.0 as usize, p.size.1 as usize], &f.rgba);
                        match v.vtex.as_mut() {
                            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                            None => v.vtex = Some(ctx.load_texture(format!("video-{id}"), img, egui::TextureOptions::LINEAR)),
                        }
                    }
                    if media.kind == "gif" && !p.playing() && !p.seeking() && p.position() >= p.info.duration - 0.1 && p.error.is_none() {
                        p.play_from(0.0);
                    }
                    if p.playing() || p.seeking() {
                        ctx.request_repaint();
                    }
                }
                let shown = v.vtex.as_ref().map(|t| (t.id(), v.player.as_ref().map(|p| egui::vec2(p.info.width as f32, p.info.height as f32)).unwrap_or(t.size_vec2()))).or(preview);
                if let Some((tid, size)) = shown {
                    // Videos füllen die Bühne (auch kleine), Seitenverhältnis bleibt.
                    let s = (stage.width() / size.x).min(stage.height() / size.y);
                    let r = egui::Rect::from_center_size(stage.center(), size * s);
                    ui.painter().image(tid, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    if bg.clicked() && bg.interact_pointer_pos().map(|p| r.contains(p)).unwrap_or(false) {
                        toggle_play = true;
                    }
                    if !v.player.as_ref().map(|p| p.playing() || p.seeking()).unwrap_or(false) && v.player.is_some() {
                        let c = r.center();
                        ui.painter().circle_filled(c, 34.0, Color32::from_black_alpha(150));
                        ui.painter().add(egui::Shape::convex_polygon(vec![c + egui::vec2(-10.0, -15.0), c + egui::vec2(16.0, 0.0), c + egui::vec2(-10.0, 15.0)], Color32::WHITE, egui::Stroke::NONE));
                    }
                }
            } else {
                let shown = v.tex.as_ref().map(|t| (t.id(), t.size_vec2())).or(preview);
                if let Some((tid, size)) = shown {
                    // Die Vorschau ist kleiner als das Original: Größe aus der Nachricht nehmen, falls bekannt.
                    let natural = if v.tex.is_some() || media.w == 0 || media.h == 0 { size } else { egui::vec2(media.w as f32, media.h as f32) };
                    let base = fit_size(natural, stage.size() - egui::vec2(24.0, 24.0));
                    // Zoomen mit dem Mausrad, um den Mauszeiger herum
                    let (wheel, pinch, pointer) = ctx.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta(), i.pointer.hover_pos()));
                    let mut factor = pinch;
                    if wheel != 0.0 {
                        factor *= (wheel * 0.004).exp();
                    }
                    if zoom_key != 0 {
                        factor *= if zoom_key > 0 { 1.25 } else { 0.8 };
                    }
                    if factor != 1.0 {
                        let anchor = if zoom_key != 0 { egui::Vec2::ZERO } else { pointer.map(|p| p - stage.center()).unwrap_or(egui::Vec2::ZERO) };
                        (v.zoom, v.pan) = zoom_at(v.zoom, v.pan, factor, anchor);
                    }
                    if reset {
                        (v.zoom, v.pan) = (1.0, egui::Vec2::ZERO);
                    }
                    if bg.dragged() {
                        v.pan += bg.drag_delta();
                    }
                    if bg.double_clicked() {
                        (v.zoom, v.pan) = if v.zoom > 1.01 { (1.0, egui::Vec2::ZERO) } else { (2.5, egui::Vec2::ZERO) };
                    }
                    // Bild nicht aus dem Fenster schieben lassen
                    let half = (base * v.zoom - stage.size()).max(egui::Vec2::ZERO) / 2.0 + egui::vec2(40.0, 40.0);
                    v.pan = v.pan.clamp(-half, half);
                    if v.zoom <= 1.0 {
                        v.pan = egui::Vec2::ZERO;
                    }
                    let r = egui::Rect::from_center_size(stage.center() + v.pan, base * v.zoom);
                    ui.painter().with_clip_rect(stage).image(tid, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    if v.zoom > 1.0 {
                        ctx.set_cursor_icon(if bg.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
                    }
                }
            }
            if let Some(e) = &v.error {
                ui.painter().text(stage.center(), egui::Align2::CENTER_CENTER, e, egui::FontId::proportional(15.0), Color32::from_rgb(0xff, 0x8a, 0x80));
            } else if v.path.is_none() {
                ui.painter().text(stage.center_bottom() - egui::vec2(0.0, 24.0), egui::Align2::CENTER_CENTER, t!("Lädt …", "Loading …"), egui::FontId::proportional(14.0), Color32::LIGHT_GRAY);
            }

            // Kopfzeile
            let bar = egui::Rect::from_min_size(screen.min, egui::vec2(screen.width(), BAR_H));
            ui.painter().rect_filled(bar, 0.0, Color32::from_black_alpha(160));
            let mut top = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(egui::vec2(12.0, 6.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            top.visuals_mut().override_text_color = Some(Color32::WHITE);
            top.label(RichText::new(tf!("{} / {}", "{} / {}", index + 1, items.len())).size(13.0).color(Color32::LIGHT_GRAY));
            top.add_space(10.0);
            top.label(RichText::new(&title).size(14.0));
            top.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(RichText::new("×").size(20.0)).on_hover_text(t!("Schließen (Esc)", "Close (Esc)")).clicked() {
                    close = true;
                }
                if ui.button(t!("Vollbild", "Fullscreen")).on_hover_text("F").clicked() {
                    toggle_full = true;
                }
                if ui.add_enabled(v.path.is_some(), egui::Button::new(t!("Extern öffnen", "Open externally"))).clicked() {
                    external = true;
                }
                if ui.add_enabled(v.path.is_some(), egui::Button::new(t!("Speichern unter …", "Save as …"))).clicked() {
                    save = true;
                }
                if !video {
                    if ui.button("1:1").on_hover_text(t!("Zoom zurücksetzen (0)", "Reset zoom (0)")).clicked() {
                        (v.zoom, v.pan) = (1.0, egui::Vec2::ZERO);
                    }
                    if ui.button(RichText::new("+").size(16.0)).on_hover_text(t!("Vergrößern (+ oder Mausrad)", "Zoom in (+ or mouse wheel)")).clicked() {
                        (v.zoom, v.pan) = zoom_at(v.zoom, v.pan, 1.25, egui::Vec2::ZERO);
                    }
                    if ui.button(RichText::new("–").size(16.0)).on_hover_text(t!("Verkleinern (−)", "Zoom out (−)")).clicked() {
                        (v.zoom, v.pan) = zoom_at(v.zoom, v.pan, 0.8, egui::Vec2::ZERO);
                    }
                    ui.label(RichText::new(format!("{} %", (v.zoom * 100.0).round())).size(12.0).color(Color32::LIGHT_GRAY));
                }
            });

            // Blättern
            for (dir, x, glyph, tip) in [
                (-1, screen.left() + 34.0, "‹", t!("Vorheriges (←)", "Previous (←)")),
                (1, screen.right() - 34.0, "›", t!("Nächstes (→)", "Next (→)")),
            ] {
                let there = if dir < 0 { index > 0 } else { index + 1 < items.len() };
                if !there {
                    continue;
                }
                let r = egui::Rect::from_center_size(egui::pos2(x, stage.center().y), egui::vec2(44.0, 72.0));
                let resp = ui.interact(r, egui::Id::new(("viewer_nav", dir)), egui::Sense::click()).on_hover_text(tip);
                ui.painter().rect_filled(r, 10.0, Color32::from_black_alpha(if resp.hovered() { 200 } else { 110 }));
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(34.0), Color32::WHITE);
                if resp.clicked() {
                    step = dir;
                }
            }

            // Videosteuerung
            if video {
                let bar = egui::Rect::from_min_max(egui::pos2(screen.left(), screen.bottom() - CTRL_H), screen.max);
                ui.painter().rect_filled(bar, 0.0, Color32::from_black_alpha(190));
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(egui::vec2(14.0, 8.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                c.visuals_mut().override_text_color = Some(Color32::WHITE);
                match v.player.as_mut() {
                    Some(p) => {
                        if c.button(RichText::new(if p.playing() { "⏸" } else { "▶" }).size(18.0)).on_hover_text(t!("Abspielen/Pause (Leertaste)", "Play/pause (space)")).clicked() {
                            toggle_play = true;
                        }
                        let dur = p.info.duration.max(0.1);
                        let mut pos = v.scrub.unwrap_or_else(|| p.position());
                        c.label(RichText::new(format!("{} / {}", fmt_pos(pos), fmt_pos(dur))).size(12.5).monospace());
                        let vol_w = 150.0;
                        c.spacing_mut().slider_width = (c.available_width() - vol_w - 40.0).max(80.0);
                        let s = c.add(egui::Slider::new(&mut pos, 0.0..=dur).show_value(false));
                        if s.dragged() {
                            v.scrub = Some(pos);
                        } else if s.drag_stopped() || s.changed() {
                            v.scrub = None;
                            p.seek(pos);
                        }
                        c.label(RichText::new("🔊").size(14.0));
                        c.spacing_mut().slider_width = 90.0;
                        let vs = c.add(egui::Slider::new(&mut v.volume, 0.0..=1.5).show_value(false)).on_hover_text(t!("Lautstärke", "Volume"));
                        if vs.drag_stopped() || (vs.changed() && !vs.dragged()) {
                            p.set_volume(v.volume);
                        }
                        if toggle_play {
                            p.toggle();
                        }
                        if jump != 0.0 {
                            let at = p.position() + jump;
                            p.seek(at);
                        }
                        if p.error.is_some() {
                            v.error = p.error.clone();
                        }
                    }
                    None => {
                        c.label(RichText::new(t!("Video wird geladen …", "Loading video …")).size(13.0).color(Color32::LIGHT_GRAY));
                    }
                }
            }
        });

        if toggle_full {
            if let Some(v) = self.viewer.as_mut() {
                v.fullscreen = !v.fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(v.fullscreen));
            }
        }
        if save {
            let _ = self.cmd.send(Cmd::SaveMedia { id: id.clone(), media: media.clone() });
        }
        if external {
            if let Some(p) = self.viewer.as_mut().and_then(|v| v.player.as_mut()) {
                p.pause();
            }
            let _ = self.cmd.send(Cmd::OpenMedia { id: id.clone(), media: media.clone() });
        }
        if close {
            self.close_viewer(ctx);
        } else if step != 0 {
            let next = index as i64 + step as i64;
            if next >= 0 && (next as usize) < items.len() {
                let (nid, nmedia) = items[next as usize].clone();
                self.open_viewer(ctx, chat, nid, nmedia);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    #[test]
    fn fit_shrinks_but_never_enlarges() {
        assert_eq!(fit_size(vec2(4000.0, 2000.0), vec2(1000.0, 1000.0)), vec2(1000.0, 500.0));
        assert_eq!(fit_size(vec2(200.0, 100.0), vec2(1000.0, 1000.0)), vec2(200.0, 100.0));
        assert_eq!(fit_size(vec2(0.0, 100.0), vec2(1000.0, 1000.0)), egui::Vec2::ZERO);
    }

    #[test]
    fn zoom_keeps_point_under_cursor() {
        // Punkt 100 px rechts der Mitte: nach dem Zoom muss derselbe Bildpunkt dort liegen.
        let anchor = vec2(100.0, 0.0);
        let (z, pan) = zoom_at(1.0, egui::Vec2::ZERO, 2.0, anchor);
        assert_eq!(z, 2.0);
        // Bildpunkt unter dem Zeiger vorher: (anchor - pan0) / zoom0 = 100; nachher: (anchor - pan) / z
        assert!(((anchor.x - pan.x) / z - 100.0).abs() < 1e-3);
    }

    #[test]
    fn zoom_is_clamped() {
        assert_eq!(zoom_at(10.0, egui::Vec2::ZERO, 100.0, egui::Vec2::ZERO).0, MAX_ZOOM);
        assert_eq!(zoom_at(0.5, egui::Vec2::ZERO, 0.001, egui::Vec2::ZERO).0, MIN_ZOOM);
    }

    #[test]
    fn only_pictures_and_videos_are_viewable() {
        let m = |k: &str| MediaRef { kind: k.into(), ..Default::default() };
        assert!(viewable(&m("image")) && viewable(&m("video")) && viewable(&m("gif")) && viewable(&m("sticker")));
        assert!(!viewable(&m("document")) && !viewable(&m("ptt")));
    }
}
