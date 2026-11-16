use super::*;

impl App {
    /// "@nummer" durch "@Name" ersetzen und für die Hervorhebung markieren.
    pub(crate) fn with_mention_names(&self, text: &str, mentions: &[String]) -> String {
        if mentions.is_empty() || !text.contains('@') {
            return text.to_owned();
        }
        let mut out = text.to_owned();
        for u in mentions {
            let pat = format!("@{u}");
            if !out.contains(&pat) {
                continue;
            }
            let name = if self.me.contains(u) {
                t!("Du", "You").to_owned()
            } else {
                self.names.get(u).map(|n| n.0.clone()).or_else(|| {
                    self.chats.values().find(|c| !c.is_group && c.jid.user.as_str() == u.as_str()).map(|c| self.display_name(c))
                }).unwrap_or_else(|| {
                    // LID: Telefonnummer über die Gruppenmitglieder finden
                    self.members
                        .values()
                        .flat_map(|v| v.iter())
                        .find(|(l, _)| l == u)
                        .and_then(|(_, pn)| self.names.get(pn).map(|n| n.0.clone()).or_else(|| (!pn.is_empty()).then(|| format!("+{pn}"))))
                        .unwrap_or_else(|| u.clone())
                })
            };
            out = out.replace(&pat, &format!("\u{E000}@{name}\u{E001}"));
        }
        out
    }

    /// Umfrage, Standort, Kontaktkarte oder Link-Vorschau in einer Blase.
    pub(crate) fn extra_block(&self, ui: &mut egui::Ui, m: &ChatMessage, k: &backend::Extra, w: f32, pal: &Palette, msg_act: &mut Option<MsgAct>) {
        use backend::Extra;
        match k {
            Extra::Poll { name, options, multi, secret } => {
                ui.label(RichText::new(name).size(15.0).family(theme::bold()));
                ui.label(RichText::new(if *multi { t!("Wähle eine oder mehrere Optionen", "Select one or more options") } else { t!("Wähle eine Option", "Select one option") }).size(12.0).color(pal.dim));
                ui.add_space(4.0);
                let votes = self.votes.get(&m.id);
                let mine: Vec<String> = votes.and_then(|v| v.get("")).cloned().unwrap_or_default();
                let total = votes.map(|v| v.values().filter(|o| !o.is_empty()).count()).unwrap_or(0).max(1);
                for opt in options {
                    let n = votes.map(|v| v.values().filter(|o| o.contains(opt)).count()).unwrap_or(0);
                    let on = mine.contains(opt);
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w.max(220.0), 38.0), egui::Sense::click());
                    let p = ui.painter();
                    let c = rect.left_center() + egui::vec2(10.0, -5.0);
                    if on {
                        p.circle_filled(c, 8.0, pal.accent);
                        p.line_segment([c + egui::vec2(-3.5, 0.0), c + egui::vec2(-1.0, 2.5)], egui::Stroke::new(1.6_f32, pal.on_accent));
                        p.line_segment([c + egui::vec2(-1.0, 2.5), c + egui::vec2(3.5, -2.5)], egui::Stroke::new(1.6_f32, pal.on_accent));
                    } else {
                        p.circle_stroke(c, 8.0, egui::Stroke::new(1.5_f32, pal.dim));
                    }
                    p.text(c + egui::vec2(16.0, 0.0), egui::Align2::LEFT_CENTER, opt, egui::FontId::proportional(14.0), pal.text);
                    p.text(egui::pos2(rect.right() - 4.0, c.y), egui::Align2::RIGHT_CENTER, n.to_string(), egui::FontId::proportional(13.0), pal.dim);
                    let bar = egui::Rect::from_min_size(egui::pos2(rect.left() + 26.0, rect.bottom() - 9.0), egui::vec2(rect.width() - 30.0, 5.0));
                    p.rect_filled(bar, 3.0, pal.border);
                    let frac = n as f32 / total as f32;
                    p.rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac, 5.0)), 3.0, pal.accent);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !secret.is_empty() {
                        let mut sel = mine.clone();
                        if *multi {
                            if on {
                                sel.retain(|o| o != opt);
                            } else {
                                sel.push(opt.clone());
                            }
                        } else {
                            sel = if on { Vec::new() } else { vec![opt.clone()] };
                        }
                        *msg_act = Some(MsgAct::Vote(m.id.clone(), sel));
                    }
                }
                if secret.is_empty() {
                    ui.label(RichText::new(t!("Abstimmen ist bei dieser Umfrage hier nicht möglich.", "Voting isn't possible for this poll here.")).size(11.0).color(pal.dim));
                }
            }
            Extra::Location { lat, lon, name, address, live } => {
                ui.label(RichText::new(if *live { t!("📍 Live-Standort", "📍 Live location") } else { t!("📍 Standort", "📍 Location") }).size(14.0).family(theme::bold()));
                if !name.is_empty() {
                    ui.label(RichText::new(name).size(14.0));
                }
                if !address.is_empty() {
                    ui.label(RichText::new(address).size(12.5).color(pal.dim));
                }
                if ui.link(RichText::new(t!("In Karte öffnen", "Open in map")).size(13.0)).clicked() {
                    let url = format!("https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=16/{lat}/{lon}");
                    safety::spawn_opener(std::ffi::OsStr::new(&url));
                }
            }
            Extra::Contact { name, vcard } => {
                ui.label(RichText::new(format!("👤 {name}")).size(14.0).family(theme::bold()));
                let tel = vcard
                    .lines()
                    .find(|l| l.starts_with("TEL") || l.contains(".TEL"))
                    .and_then(|l| l.split(':').next_back())
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                if !tel.is_empty() {
                    ui.label(RichText::new(&tel).size(13.0).color(pal.dim));
                    let digits: String = tel.chars().filter(|c| c.is_ascii_digit()).collect();
                    if digits.len() >= 7 && ui.button(t!("Chat starten", "Start chat")).clicked() {
                        *msg_act = Some(MsgAct::OpenNumber(digits));
                    }
                }
            }
            Extra::Link { url, title, description } => {
                let resp = egui::Frame::new()
                    .fill(if self.settings.dark { Color32::from_black_alpha(60) } else { Color32::from_black_alpha(12) })
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        ui.set_min_width((w - 20.0).max(160.0));
                        ui.label(RichText::new(title).size(13.5).family(theme::bold()));
                        if !description.is_empty() {
                            let d: String = description.chars().take(160).collect();
                            ui.label(RichText::new(d).size(12.5).color(pal.dim));
                        }
                        let host = url.split("//").nth(1).unwrap_or(url).split('/').next().unwrap_or("");
                        ui.label(RichText::new(host).size(11.5).color(pal.dim));
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    let u = if url.starts_with("http") { url.clone() } else { format!("https://{url}") };
                    safety::spawn_opener(std::ffi::OsStr::new(&u));
                }
                ui.add_space(3.0);
            }
        }
    }

    /// Übersicht der mit Stern markierten Nachrichten.
    pub(crate) fn starred_view(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let mut jump = None;
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.label(RichText::new(t!("Markierte Nachrichten", "Starred messages")).size(22.0).family(theme::bold()));
        });
        ui.add_space(10.0);
        let mut list: Vec<(&Chat, &ChatMessage)> = self
            .chats
            .values()
            .flat_map(|c| c.messages.iter().filter(|m| self.starred.contains(&m.id)).map(move |m| (c, m)))
            .collect();
        list.sort_by_key(|(_, m)| std::cmp::Reverse(m.ts));
        if list.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(RichText::new(t!("Noch keine markierten Nachrichten. Rechtsklick auf eine Nachricht → Mit Stern markieren.", "No starred messages yet. Right-click a message → Star.")).color(pal.dim));
            });
        }
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for (c, m) in list {
                let who = if m.from_me { t!("Du", "You").to_owned() } else if c.is_group { m.sender_name.clone() } else { self.display_name(c) };
                let resp = egui::Frame::new()
                    .fill(pal.bubble_them)
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(14, 8))
                    .outer_margin(egui::Margin::symmetric(24, 4))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{who} ▸ {}", self.display_name(c))).size(12.5).color(pal.accent));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(format!("{} {}", day_label(m.ts), fmt_time(m.ts))).size(11.5).color(pal.dim));
                            });
                        });
                        let t = match (&m.media, &m.x.kind) {
                            (Some(md), _) if m.text.is_empty() => md.label(),
                            (_, Some(k)) if m.text.is_empty() => k.label(),
                            _ => m.text.clone(),
                        };
                        ui.label(RichText::new(t).size(14.0));
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    jump = Some(m.id.clone());
                }
            }
        });
        if let Some(id) = jump {
            self.run_msg_act(MsgAct::Jump(id));
        }
    }

    /// Sprachnachricht: Play/Pause, Wellenform zum Spulen, Zeit.
    pub(crate) fn audio_block(&self, ui: &mut egui::Ui, m: &ChatMessage, md: &backend::MediaRef, max_w: f32, pal: &Palette, audio_req: &mut Option<(String, backend::MediaRef, Option<f32>)>) {
        let playing = self.player.as_ref().filter(|p| p.id == m.id);
        let dec = self.decoded.get(&m.id);
        let dur = dec.map(|(pcm, ch, _)| pcm.len() as f32 / *ch as f32 / audio::RATE as f32).unwrap_or(md.seconds as f32).max(0.1);
        let pos = playing.map(|p| p.position()).or_else(|| self.paused_at.get(&m.id).copied()).unwrap_or(0.0);
        let loading = self.want_play.as_ref().map(|w| w.0 == m.id).unwrap_or(false);
        let w = 260.0_f32.min(max_w - 20.0);
        ui.horizontal(|ui| {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::click());
            let c = r.center();
            let p = ui.painter();
            p.circle_filled(c, 18.0, if m.from_me { Color32::from_black_alpha(40) } else { pal.hover });
            if loading {
                p.text(c, egui::Align2::CENTER_CENTER, "…", egui::FontId::proportional(16.0), pal.text);
            } else if playing.is_some() {
                p.rect_filled(egui::Rect::from_center_size(c + egui::vec2(-4.0, 0.0), egui::vec2(4.0, 14.0)), 1.0, pal.text);
                p.rect_filled(egui::Rect::from_center_size(c + egui::vec2(4.0, 0.0), egui::vec2(4.0, 14.0)), 1.0, pal.text);
            } else {
                let pts = vec![c + egui::vec2(-5.0, -8.0), c + egui::vec2(9.0, 0.0), c + egui::vec2(-5.0, 8.0)];
                p.add(egui::Shape::convex_polygon(pts, pal.text, egui::Stroke::NONE));
            }
            if resp.on_hover_text(if playing.is_some() { t!("Pause", "Pause") } else { t!("Abspielen", "Play") }).clicked() {
                *audio_req = Some((m.id.clone(), md.clone(), None));
            }
            ui.vertical(|ui| {
                let (wr, wresp) = ui.allocate_exact_size(egui::vec2(w - 50.0, 26.0), egui::Sense::click());
                let bars: Vec<u8> = dec.map(|d| d.2.clone()).unwrap_or_else(|| {
                    // Platzhalter, bis die Datei geladen ist
                    (0..40).map(|i| (30 + ((i * 37 + m.id.len() * 11) % 50)) as u8).collect()
                });
                let n = bars.len().max(1);
                let bw = wr.width() / n as f32;
                let frac = (pos / dur).clamp(0.0, 1.0);
                for (i, b) in bars.iter().enumerate() {
                    let h = (*b as f32 / 100.0 * wr.height()).max(3.0);
                    let x = wr.left() + i as f32 * bw + bw / 2.0;
                    let played = (i as f32 + 0.5) / n as f32 <= frac;
                    let col = if played { pal.tick_read } else { pal.dim.gamma_multiply(0.7) };
                    ui.painter().line_segment([egui::pos2(x, wr.center().y - h / 2.0), egui::pos2(x, wr.center().y + h / 2.0)], egui::Stroke::new((bw * 0.6).clamp(1.5, 3.0), col));
                }
                if wresp.clicked() {
                    if let Some(pt) = wresp.interact_pointer_pos() {
                        let f = ((pt.x - wr.left()) / wr.width()).clamp(0.0, 1.0);
                        *audio_req = Some((m.id.clone(), md.clone(), Some(f)));
                    }
                }
                let shown = if playing.is_some() || pos > 0.0 { pos } else { dur };
                let label = if md.kind == "ptt" { "" } else { t!("  🎵 Audio", "  🎵 Audio") };
                let shown = shown.round() as u32;
                ui.label(RichText::new(format!("{}:{:02}{label}", shown / 60, shown % 60)).size(11.0).color(pal.dim));
            });
        });
        if playing.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Bild, Sticker oder Anhang in einer Blase.
    /// Bild holen und als „gerade gezeigt“ vermerken.
    pub(crate) fn tex(&self, id: &str) -> Option<&(egui::TextureHandle, bool)> {
        let t = self.images.get(id)?;
        self.touch(id);
        Some(t)
    }

    pub(crate) fn touch(&self, id: &str) {
        let mut seen = self.img_seen.borrow_mut();
        match seen.get_mut(id) {
            Some(at) => *at = std::time::Instant::now(),
            None => {
                seen.insert(id.to_owned(), std::time::Instant::now());
            }
        }
    }

    /// Bilder und Sticker-Animationen, die seit einer Weile nicht zu sehen waren, freigeben.
    /// Die Dateien liegen auf der Platte und werden beim nächsten Anzeigen neu eingelesen.
    pub(crate) fn sweep_images(&mut self) {
        if self.img_sweep.elapsed().as_secs() < 10 {
            return;
        }
        self.img_sweep = std::time::Instant::now();
        if self.images.len() + self.anims.len() <= 40 {
            return;
        }
        let seen = self.img_seen.get_mut();
        let old: Vec<String> = self
            .images
            .keys()
            .chain(self.anims.keys())
            .filter(|id| seen.get(*id).map(|t| t.elapsed().as_secs() >= 45).unwrap_or(true))
            .cloned()
            .collect();
        for id in old {
            // Erst nach dem zweiten Durchlauf ohne Sichtung entfernen (frisch geladene haben noch keinen Vermerk).
            if seen.remove(&id).is_none() && !self.img_stale.remove(&id) {
                self.img_stale.insert(id);
                continue;
            }
            self.img_stale.remove(&id);
            self.images.remove(&id);
            self.anims.remove(&id);
            self.img_requested.remove(&id);
        }
    }

    /// Prüfwert über alles, was die Höhe einer Nachricht ändern kann.
    pub(crate) fn height_sig(&self, m: &ChatMessage, first: bool, width: f32) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        width.to_bits().hash(&mut h);
        self.settings.font_size.to_bits().hash(&mut h);
        first.hash(&mut h);
        m.text.hash(&mut h);
        (m.x.deleted, m.x.edited, m.x.view_once, m.quote.is_some(), m.media.is_some()).hash(&mut h);
        self.reactions.get(&m.id).map(|l| l.len()).unwrap_or(0).hash(&mut h);
        self.images.get(&m.id).map(|(t, full)| (t.size(), *full)).hash(&mut h);
        self.anims.contains_key(&m.id).hash(&mut h);
        self.votes.get(&m.id).map(|v| v.len()).unwrap_or(0).hash(&mut h);
        h.finish()
    }

    pub(crate) fn media_block(
        &self,
        ui: &mut egui::Ui,
        m: &ChatMessage,
        md: &backend::MediaRef,
        max_w: f32,
        pal: &Palette,
        media_req: &mut Vec<(String, backend::MediaRef)>,
        open_req: &mut Vec<(String, backend::MediaRef)>,
        audio_req: &mut Option<(String, backend::MediaRef, Option<f32>)>,
    ) {
        if md.kind == "ptt" || md.kind == "audio" {
            self.audio_block(ui, m, md, max_w, pal, audio_req);
            return;
        }
        if let Some((frames, delays)) = self.anims.get(&m.id) {
            self.touch(&m.id);
            // animierter Sticker: Bild nach Zeit auswählen
            let total: u32 = delays.iter().sum::<u32>().max(1);
            let t = (ui.input(|i| i.time) * 1000.0) as u64 % total as u64;
            let mut acc = 0u64;
            let mut idx = 0;
            for (i, d) in delays.iter().enumerate() {
                acc += *d as u64;
                if t < acc {
                    idx = i;
                    break;
                }
            }
            let tex = &frames[idx];
            let size = media_size(md, Some(&(tex.clone(), true)), max_w);
            let resp = ui.add(egui::Image::new((tex.id(), size)).sense(egui::Sense::click()));
            if resp.on_hover_text(t!("Öffnen", "Open")).clicked() {
                open_req.push((m.id.clone(), md.clone()));
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(delays[idx].max(30) as u64));
            return;
        }
        let tex = self.tex(&m.id);
        let auto = self.settings.autoload || m.from_me || self.img_requested.contains(&format!("{}#ok", m.id));
        // Nur anfordern, was in Sichtweite ist: sonst lädt ein langer Chat beim Öffnen alles auf einmal.
        let near = {
            let (y, c) = (ui.cursor().top(), ui.clip_rect());
            y > c.top() - 900.0 && y < c.bottom() + 900.0
        };
        let failed = self.img_failed.get(&m.id).map(|t| t.elapsed().as_secs() < 60).unwrap_or(false);
        if auto && near && !failed && !self.img_requested.contains(&m.id) && tex.map(|t| !t.1).unwrap_or(true) {
            media_req.push((m.id.clone(), md.clone()));
        }
        if !auto && tex.is_none() && md.is_picture() {
            let size = media_size(md, None, max_w);
            let (r, resp) = ui.allocate_exact_size(size, egui::Sense::click());
            ui.painter().rect_filled(r, 6.0, pal.hover);
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, tf!("⬇ {} laden", "⬇ Load {}", md.label()), egui::FontId::proportional(13.0), pal.text);
            if resp.clicked() {
                media_req.push((format!("{}#ok", m.id), md.clone()));
            }
            return;
        }
        if md.is_picture() {
            let size = media_size(md, tex, max_w);
            let resp = match tex {
                Some((t, _)) => ui.add(
                    egui::Image::new((t.id(), size))
                        .corner_radius(if md.kind == "sticker" { 0.0 } else { 6.0 })
                        .sense(egui::Sense::click()),
                ),
                None => {
                    let (r, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                    ui.painter().rect_filled(r, 6.0, pal.hover);
                    let label = if failed { t!("⟳ Laden fehlgeschlagen – klicken", "⟳ Loading failed – click") } else { t!("Lädt …", "Loading …") };
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(13.0), pal.dim);
                    if failed && resp.clicked() {
                        media_req.push((m.id.clone(), md.clone()));
                        return;
                    }
                    resp
                }
            };
            if resp.on_hover_text(t!("Öffnen", "Open")).clicked() {
                open_req.push((m.id.clone(), md.clone()));
            }
            return;
        }
        // Video/GIF mit Vorschaubild
        if let (Some((t, _)), true) = (tex, md.kind == "video" || md.kind == "gif") {
            let size = media_size(md, Some(&(t.clone(), false)), max_w);
            let resp = ui.add(egui::Image::new((t.id(), size)).corner_radius(6.0).sense(egui::Sense::click()));
            let c = resp.rect.center();
            ui.painter().circle_filled(c, 22.0, Color32::from_black_alpha(150));
            let pts = vec![c + egui::vec2(-6.0, -9.0), c + egui::vec2(10.0, 0.0), c + egui::vec2(-6.0, 9.0)];
            ui.painter().add(egui::Shape::convex_polygon(pts, Color32::WHITE, egui::Stroke::NONE));
            if md.kind == "gif" {
                ui.painter().text(resp.rect.left_bottom() + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, "GIF", egui::FontId::new(12.0, theme::bold()), Color32::WHITE);
            }
            if resp.on_hover_text(t!("Im Standardprogramm abspielen", "Play in default app")).clicked() {
                open_req.push((m.id.clone(), md.clone()));
            }
            return;
        }
        // Dokument, Audio, Sprachnachricht, Video ohne Vorschau
        let resp = egui::Frame::new()
            .fill(pal.hover.gamma_multiply(0.9))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(240.0_f32.min(max_w - 20.0));
                ui.horizontal(|ui| {
                    ui.label(RichText::new(md.label()).size(14.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if md.len > 0 {
                            ui.label(RichText::new(fmt_size(md.len)).size(11.0).color(pal.dim));
                        }
                    });
                });
            })
            .response
            .interact(egui::Sense::click());
        if resp.on_hover_text(t!("Herunterladen und öffnen", "Download and open")).clicked() {
            open_req.push((m.id.clone(), md.clone()));
        }
        ui.add_space(2.0);
    }
}
