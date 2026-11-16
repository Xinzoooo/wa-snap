use super::*;

impl App {
    pub(crate) fn input_bar(&mut self, ctx: &egui::Context, pal: &Palette) {
        let input_id = egui::Id::new("wa_input");
        let mut do_send = false;
        let mut attach = false;
        let mut make_sticker = false;
        let mut paste_now = false;
        let mut gif_send: Option<backend::GifItem> = None;
        let mut rec_action: u8 = 0; // 1 starten, 2 senden, 3 abbrechen
        let mut paste_send: Option<std::path::PathBuf> = None;
        let mut typed = false;
        let mut sticker_send: Option<backend::MediaRef> = None;
        egui::TopBottomPanel::bottom("input")
            .frame(egui::Frame::new().fill(pal.chat).inner_margin(egui::Margin { left: 16, right: 16, top: 6, bottom: 12 }))
            .show(ctx, |ui| {
                let enabled = self.conn == Conn::Online;
                if enabled && ctx.memory(|m| m.has_focus(input_id)) {
                    let send_mods = if self.settings.enter_sends { egui::Modifiers::NONE } else { egui::Modifiers::COMMAND };
                    if ctx.input_mut(|i| i.consume_key(send_mods, egui::Key::Enter)) {
                        do_send = true;
                    }
                }
                if enabled && ctx.memory(|m| m.has_focus(input_id)) && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::V)) {
                    self.try_paste_image(ctx);
                }
                if let Some((path, tex)) = self.pending_paste.clone() {
                    egui::Frame::new()
                        .fill(pal.bubble_them)
                        .corner_radius(12.0)
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let sz = tex.size_vec2();
                                let k = (96.0 / sz.y).min(1.0);
                                ui.add(egui::Image::new((tex.id(), sz * k)).corner_radius(6.0));
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(t!("Bild aus der Zwischenablage senden?", "Send image from clipboard?")).family(theme::bold()));
                                    ui.label(RichText::new(t!("Text im Eingabefeld wird zur Bildunterschrift.", "Text in the input field becomes the caption.")).small().color(pal.dim));
                                    ui.horizontal(|ui| {
                                        if ui.button(t!("Senden", "Send")).clicked() {
                                            paste_send = Some(path.clone());
                                        }
                                        if ui.button(t!("Abbrechen", "Cancel")).clicked() {
                                            self.discard_paste();
                                        }
                                    });
                                });
                            });
                        });
                    ui.add_space(4.0);
                }
                if self.editing.as_ref().map(|(k, _)| Some(k) == self.selected.as_ref()).unwrap_or(false) {
                    egui::Frame::new()
                        .fill(pal.bubble_them)
                        .corner_radius(12.0)
                        .inner_margin(egui::Margin { left: 14, right: 10, top: 8, bottom: 8 })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(t!("✏ Nachricht bearbeiten", "✏ Edit message")).size(13.0).family(theme::bold()).color(pal.accent));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("×").on_hover_text(t!("Bearbeiten abbrechen", "Cancel editing")).clicked() {
                                        self.editing = None;
                                        self.input.clear();
                                    }
                                });
                            });
                        });
                    ui.add_space(4.0);
                }
                // @Erwähnung: Vorschläge, solange das letzte Wort mit @ beginnt
                let group_user = self
                    .selected
                    .as_ref()
                    .and_then(|k| self.chats.get(k))
                    .filter(|c| c.is_group)
                    .map(|c| c.jid.user.to_string());
                if let Some(g) = group_user {
                    let last = self.input.rsplit([' ', '\n']).next().unwrap_or("").to_owned();
                    if let Some(q) = last.strip_prefix('@') {
                        let q = q.to_lowercase();
                        let mut cands: Vec<(String, String)> = self
                            .members
                            .get(&g)
                            .map(|ms| {
                                ms.iter()
                                    .filter(|(u, _)| !self.me.contains(u))
                                    .map(|(u, pn)| {
                                        let tag = if pn.is_empty() { u.clone() } else { pn.clone() };
                                        let name = self.names.get(pn).or_else(|| self.names.get(u)).map(|n| n.0.clone()).unwrap_or_else(|| format!("+{tag}"));
                                        (tag, name)
                                    })
                                    .filter(|(t, n)| q.is_empty() || n.to_lowercase().contains(&q) || t.contains(&q))
                                    .collect()
                            })
                            .unwrap_or_default();
                        cands.sort_by(|a, b| a.1.cmp(&b.1));
                        if !cands.is_empty() {
                            egui::Frame::new()
                                .fill(pal.bubble_them)
                                .corner_radius(12.0)
                                .inner_margin(egui::Margin::same(6))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                                        for (tag, name) in cands.into_iter().take(30) {
                                            if ui.selectable_label(false, RichText::new(format!("{name}   ")).size(14.0)).clicked() {
                                                let cut = self.input.len() - last.len();
                                                self.input.truncate(cut);
                                                self.input.push_str(&format!("@{tag} "));
                                                ctx.memory_mut(|m| m.request_focus(input_id));
                                            }
                                        }
                                    });
                                });
                            ui.add_space(4.0);
                        }
                    }
                }
                let reply = self.reply_to.clone().filter(|(k, _, _)| Some(k) == self.selected.as_ref());
                if let Some((_, q, _)) = reply {
                    egui::Frame::new()
                        .fill(pal.bubble_them)
                        .corner_radius(12.0)
                        .inner_margin(egui::Margin { left: 14, right: 10, top: 8, bottom: 8 })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    ui.label(RichText::new(tf!("Antwort an {}", "Reply to {}", q.name)).size(12.5).family(theme::bold()).color(pal.accent));
                                    let t: String = q.text.replace('\n', " ").chars().take(120).collect();
                                    ui.add(egui::Label::new(RichText::new(t).size(13.0).color(pal.dim)).truncate());
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("×").on_hover_text(t!("Antwort abbrechen", "Cancel reply")).clicked() {
                                        self.reply_to = None;
                                    }
                                });
                            });
                        });
                    ui.add_space(4.0);
                }
                egui::Frame::new()
                    .fill(pal.bubble_them)
                    .corner_radius(26.0)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let clip = icon_button(ui, "📎", 19.0, pal.dim, pal, t!("Anhängen", "Attach"));
                            let attach_id = egui::Id::new("attach_menu");
                            if clip.clicked() && enabled {
                                ui.memory_mut(|m| m.toggle_popup(attach_id));
                            }
                            egui::popup::popup_above_or_below_widget(ui, attach_id, &clip, egui::AboveOrBelow::Above, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                                ui.set_min_width(230.0);
                                if ui.button(t!("📄  Foto, Video oder Datei", "📄  Photo, video or file")).clicked() {
                                    attach = true;
                                }
                                if ui.button(t!("🖼  Sticker aus Bild", "🖼  Sticker from image")).clicked() {
                                    make_sticker = true;
                                }
                                if ui.button(t!("📋  Bild aus Zwischenablage", "📋  Image from clipboard")).clicked() {
                                    paste_now = true;
                                }
                            });
                            let emoji_btn = icon_button(ui, "", 19.0, pal.dim, pal, t!("Emoji", "Emoji"));
                            {
                                // Smiley als Umriss, wie das Symbol bei WhatsApp
                                let c = emoji_btn.rect.center();
                                let st = egui::Stroke::new(1.6_f32, pal.dim);
                                let p = ui.painter();
                                p.circle_stroke(c, 9.5, st);
                                p.circle_filled(c + egui::vec2(-3.2, -2.5), 1.4, pal.dim);
                                p.circle_filled(c + egui::vec2(3.2, -2.5), 1.4, pal.dim);
                                let smile: Vec<egui::Pos2> = (0..=8)
                                    .map(|i| {
                                        let a = std::f32::consts::PI * (0.2 + 0.6 * i as f32 / 8.0);
                                        c + egui::vec2(-a.cos() * 5.0, a.sin() * 4.5 + 0.5)
                                    })
                                    .collect();
                                p.add(egui::Shape::line(smile, st));
                            }
                            let popup = egui::Id::new("emoji_popup");
                            if emoji_btn.clicked() {
                                ui.memory_mut(|m| m.toggle_popup(popup));
                            }
                            egui::popup::popup_above_or_below_widget(
                                ui,
                                popup,
                                &emoji_btn,
                                egui::AboveOrBelow::Above,
                                egui::PopupCloseBehavior::CloseOnClickOutside,
                                |ui| {
                                    ui.set_width(400.0);
                                    ui.horizontal(|ui| {
                                        for (i, l) in [t!("Emoji", "Emoji"), "GIF", t!("Sticker", "Stickers")].iter().enumerate() {
                                            if chip(ui, l, self.picker_tab == i as u8, pal) {
                                                self.picker_tab = i as u8;
                                            }
                                        }
                                    });
                                    ui.add_space(4.0);
                                    if self.picker_tab == 1 {
                                        if self.settings.giphy_key.trim().is_empty() {
                                            ui.label(RichText::new(t!("Für die GIF-Suche brauchst du einen kostenlosen API-Schlüssel von Giphy (developers.giphy.com → Create an API Key). Hier eintragen:", "GIF search needs a free Giphy API key (developers.giphy.com → Create an API Key). Enter it here:")).small().color(pal.dim));
                                            if ui.add(egui::TextEdit::singleline(&mut self.settings.giphy_key).hint_text(t!("Giphy-API-Schlüssel", "Giphy API key")).desired_width(340.0)).lost_focus() {
                                                self.save_settings();
                                            }
                                            return;
                                        }
                                        let r = ui.add(egui::TextEdit::singleline(&mut self.gif_query).hint_text(t!("GIFs suchen (Enter) …", "Search GIFs (Enter) …")).desired_width(340.0));
                                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.gif_query.trim().is_empty() {
                                            self.gif_loading = true;
                                            let _ = self.cmd.send(Cmd::GifSearch { query: self.gif_query.trim().to_owned(), key: self.settings.giphy_key.trim().to_owned() });
                                        }
                                        if self.gif_loading {
                                            ui.spinner();
                                        }
                                        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                                            ui.horizontal_wrapped(|ui| {
                                                for (g, tex) in &self.gif_results {
                                                    let clicked = match tex {
                                                        Some(t) => {
                                                            let sz = t.size_vec2();
                                                            let k = 108.0 / sz.x.max(1.0);
                                                            ui.add(egui::Button::image(egui::Image::new((t.id(), sz * k)).corner_radius(4.0)).frame(false)).on_hover_text(&g.title).clicked()
                                                        }
                                                        None => ui.button(&g.title).clicked(),
                                                    };
                                                    if clicked {
                                                        gif_send = Some(g.clone());
                                                        ui.memory_mut(|m| m.close_popup());
                                                    }
                                                }
                                            });
                                            ui.label(RichText::new("Powered by GIPHY").size(10.0).color(pal.dim));
                                        });
                                        return;
                                    }
                                    if self.picker_tab == 2 {
                                        if ui.button(t!("➕ Eigenen Sticker aus Bild erstellen", "➕ Create your own sticker from an image")).clicked() {
                                            make_sticker = true;
                                            ui.memory_mut(|m| m.close_popup());
                                        }
                                        ui.add_space(4.0);
                                        let list = self.known_stickers();
                                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                                            if list.is_empty() {
                                                ui.label(RichText::new(t!("Noch keine Sticker. Sticker, die du bekommst, erscheinen hier und können weitergeschickt werden.", "No stickers yet. Stickers you receive show up here and can be sent again.")).small().color(pal.dim));
                                            }
                                            ui.horizontal_wrapped(|ui| {
                                                for (id, md) in list {
                                                    match self.tex(&id) {
                                                        Some((t, _)) => {
                                                            if ui.add(egui::Button::image(egui::Image::new((t.id(), egui::vec2(72.0, 72.0)))).frame(false)).clicked() {
                                                                sticker_send = Some(md);
                                                                ui.memory_mut(|m| m.close_popup());
                                                            }
                                                        }
                                                        None => {
                                                            ui.add_sized([72.0, 72.0], egui::Spinner::new());
                                                            if !self.img_requested.contains(&id) {
                                                                self.img_requested.insert(id.clone());
                                                                let _ = self.cmd.send(Cmd::LoadMedia { id, media: md });
                                                            }
                                                        }
                                                    }
                                                }
                                            });
                                        });
                                        return;
                                    }
                                    self.emoji_grid(ui, pal);
                                },
                            );
                            let has_text = !self.input.trim().is_empty();
                            let send_w = 44.0;
                            if let Some(rec) = &self.recording {
                                // Aufnahme läuft: Zeit, Abbrechen, Senden
                                let secs = rec.seconds() as u32;
                                ui.add_space(6.0);
                                let blink = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
                                ui.label(RichText::new(if blink { "●" } else { " " }).color(pal.danger).size(16.0));
                                ui.label(RichText::new(format!("{}:{:02}", secs / 60, secs % 60)).size(15.0));
                                ui.label(RichText::new(t!("Sprachnachricht wird aufgenommen", "Recording voice message")).size(13.0).color(pal.dim));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
                                    let c = rect.center();
                                    ui.painter().circle_filled(c, 20.0, pal.accent);
                                    let pts = vec![c + egui::vec2(-6.0, -7.0), c + egui::vec2(8.0, 0.0), c + egui::vec2(-6.0, 7.0), c + egui::vec2(-3.0, 0.0)];
                                    ui.painter().add(egui::Shape::convex_polygon(pts, pal.on_accent, egui::Stroke::NONE));
                                    if resp.on_hover_text(t!("Senden", "Send")).clicked() {
                                        rec_action = 2;
                                    }
                                    if icon_button(ui, "🗑", 18.0, pal.danger, pal, t!("Verwerfen", "Discard")).clicked() {
                                        rec_action = 3;
                                    }
                                });
                                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
                                if secs >= 15 * 60 {
                                    rec_action = 2;
                                }
                                return;
                            }
                            typed = ui.add_enabled(
                                enabled,
                                egui::TextEdit::multiline(&mut self.input)
                                    .id(input_id)
                                    .frame(false)
                                    .hint_text(if enabled { t!("Gib eine Nachricht ein.", "Type a message") } else { t!("Nicht verbunden …", "Not connected …") })
                                    .desired_rows(1)
                                    .margin(egui::vec2(6.0, 9.0))
                                    .desired_width(ui.available_width() - send_w),
                            ).changed();
                            let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
                            let c = rect.center();
                            if has_text || self.editing.is_some() {
                                ui.painter().circle_filled(c, 20.0, pal.accent);
                                let pts = vec![c + egui::vec2(-6.0, -7.0), c + egui::vec2(8.0, 0.0), c + egui::vec2(-6.0, 7.0), c + egui::vec2(-3.0, 0.0)];
                                ui.painter().add(egui::Shape::convex_polygon(pts, pal.on_accent, egui::Stroke::NONE));
                                if resp.on_hover_text(t!("Senden (Enter)", "Send (Enter)")).clicked() {
                                    do_send = true;
                                }
                            } else {
                                // Mikrofon zeichnen
                                if resp.hovered() {
                                    ui.painter().circle_filled(c, 20.0, pal.hover);
                                }
                                let st = egui::Stroke::new(1.8_f32, pal.dim);
                                let p = ui.painter();
                                p.rect_stroke(egui::Rect::from_center_size(c + egui::vec2(0.0, -3.0), egui::vec2(8.0, 13.0)), 4.0, st, egui::StrokeKind::Middle);
                                let arc: Vec<egui::Pos2> = (0..=10)
                                    .map(|i| {
                                        let a = std::f32::consts::PI * i as f32 / 10.0;
                                        c + egui::vec2(-a.cos() * 7.0, a.sin() * 6.0 + 1.0)
                                    })
                                    .collect();
                                p.add(egui::Shape::line(arc, st));
                                p.line_segment([c + egui::vec2(0.0, 7.0), c + egui::vec2(0.0, 10.0)], st);
                                if resp.on_hover_text(t!("Sprachnachricht aufnehmen", "Record voice message")).clicked() && enabled {
                                    rec_action = 1;
                                }
                            }
                        });
                    });
            });
        if do_send {
            self.send_current();
            ctx.memory_mut(|m| m.request_focus(input_id));
        }
        if paste_now {
            self.try_paste_image(ctx);
            if self.pending_paste.is_none() && self.info.is_none() {
                self.info = Some(t!("In der Zwischenablage ist kein Bild.", "There's no image in the clipboard.").into());
            }
        }
        // Dateien per Ziehen-und-Ablegen oder über 📎 senden
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if let Some(jid) = self.selected.as_ref().and_then(|k| self.chats.get(k)).map(|c| c.jid.clone()) {
            if self.conn == Conn::Online {
                for (i, path) in dropped.into_iter().enumerate() {
                    let caption = if i == 0 { std::mem::take(&mut self.input).trim().to_owned() } else { String::new() };
                    let _ = self.cmd.send(Cmd::SendFile { chat: jid.clone(), path: Some(path), caption });
                }
                if let Some(path) = paste_send {
                    self.pending_paste = None;
                    let caption = std::mem::take(&mut self.input).trim().to_owned();
                    let _ = self.cmd.send(Cmd::SendFile { chat: jid.clone(), path: Some(path), caption });
                }
                // Eigenen Tippstatus senden (höchstens alle 8 Sekunden)
                let key = jid.to_string();
                if typed && !self.input.trim().is_empty() && self.settings.send_typing {
                    let fresh = matches!(&self.my_typing, Some((k, t)) if *k == key && t.elapsed().as_secs() < 8);
                    if !fresh {
                        let _ = self.cmd.send(Cmd::Typing { chat: jid.clone(), on: true });
                        self.my_typing = Some((key, std::time::Instant::now()));
                    }
                } else if (typed || do_send) && self.my_typing.is_some() {
                    let _ = self.cmd.send(Cmd::Typing { chat: jid.clone(), on: false });
                    self.my_typing = None;
                }
                if make_sticker {
                    let _ = self.cmd.send(Cmd::MakeSticker { chat: jid.clone(), path: None });
                }
                if let Some(gif) = gif_send.take() {
                    let _ = self.cmd.send(Cmd::SendGif { chat: jid.clone(), gif });
                }
                match rec_action {
                    1 => match audio::start_recording(&data_dir().join("tmp")) {
                        Ok(r) => {
                            if let Some(p) = self.player.take() {
                                p.stop();
                            }
                            self.recording = Some(r);
                        }
                        Err(e) => self.info = Some(tf!("Aufnahme nicht möglich: {e}", "Recording not possible: {e}")),
                    },
                    2 => {
                        if let Some(r) = self.recording.take() {
                            let secs = r.seconds().round().max(1.0) as u32;
                            match r.finish().and_then(|pcm| {
                                if pcm.len() < (audio::RATE / 2) as usize {
                                    return Err(t!("Aufnahme zu kurz oder kein Mikrofon-Signal", "Recording too short or no microphone signal").into());
                                }
                                let wf = audio::waveform(&pcm, 1);
                                audio::encode_ogg_opus(&pcm).map(|ogg| (ogg, wf))
                            }) {
                                Ok((ogg, waveform)) => {
                                    let _ = self.cmd.send(Cmd::SendVoice { chat: jid.clone(), ogg, seconds: secs, waveform });
                                }
                                Err(e) => self.info = Some(tf!("Sprachnachricht: {e}", "Voice message: {e}")),
                            }
                        }
                    }
                    3 => {
                        if let Some(r) = self.recording.take() {
                            r.cancel();
                        }
                    }
                    _ => {}
                }
                if attach {
                    let caption = std::mem::take(&mut self.input).trim().to_owned();
                    let _ = self.cmd.send(Cmd::SendFile { chat: jid.clone(), path: None, caption });
                }
                if let Some(media) = sticker_send {
                    let _ = self.cmd.send(Cmd::SendSticker { chat: jid, media });
                }
            }
        }
    }
}
