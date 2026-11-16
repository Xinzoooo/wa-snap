use super::*;

impl App {
    pub(crate) fn channel_create_dialog(&mut self, ctx: &egui::Context) {
        let Some((mut name, mut desc)) = self.channel_new.take() else { return };
        let mut open = true;
        let mut create = false;
        egui::Window::new(t!("Kanal erstellen", "Create channel"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.add(egui::TextEdit::singleline(&mut name).hint_text(t!("Name des Kanals", "Channel name")).desired_width(f32::INFINITY));
                ui.add(egui::TextEdit::multiline(&mut desc).hint_text(t!("Beschreibung (optional)", "Description (optional)")).desired_rows(3).desired_width(f32::INFINITY));
                ui.label(RichText::new(t!("Kanäle sind öffentlich auffindbar. Abonnenten sehen deine Beiträge, aber nicht deine Nummer.", "Channels are publicly discoverable. Followers see your posts but not your number.")).small());
                if ui.add_enabled(!name.trim().is_empty() && self.conn == Conn::Online, egui::Button::new(t!("Erstellen", "Create"))).clicked() {
                    create = true;
                }
            });
        if create {
            let _ = self.cmd.send(Cmd::ChannelCreate { name: name.trim().to_owned(), description: desc.trim().to_owned() });
        } else if open {
            self.channel_new = Some((name, desc));
        }
    }

    /// Empfänger für eigenen Status: alle Kontakte mit Einzelchat (Telefonnummer).
    pub(crate) fn status_recipients(&self) -> Vec<Jid> {
        self.chats.values().filter(|c| !c.is_group && c.jid.server == whatsapp_rust::Server::Pn).map(|c| c.jid.clone()).collect()
    }

    /// Anruf-Anzeige oben in der Mitte: klingelnd oder laufend.
    pub(crate) fn call_overlay(&mut self, ctx: &egui::Context, pal: &Palette) {
        if self.call_in.is_none() && self.call.is_none() {
            return;
        }
        let mut cmd: Option<Cmd> = None;
        let mut clear_in = false;
        egui::Area::new(egui::Id::new("call_overlay"))
            .anchor(egui::Align2::CENTER_TOP, [0.0, 16.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.bubble_them)
                    .stroke(egui::Stroke::new(1.0_f32, pal.border))
                    .corner_radius(16.0)
                    .inner_margin(egui::Margin::symmetric(18, 12))
                    .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 18, spread: 0, color: Color32::from_black_alpha(90) })
                    .show(ui, |ui| {
                        ui.set_min_width(320.0);
                        if let Some((id, from, name, video)) = self.call_in.clone() {
                            let title = self.name_for_user(&from).or_else(|| (!name.is_empty()).then(|| name.clone())).unwrap_or_else(|| format!("+{from}"));
                            ui.horizontal(|ui| {
                                draw_avatar(ui, self.avatars.get(&from), &title, 48.0, self.settings.dark);
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&title).size(17.0).family(theme::bold()));
                                    ui.label(RichText::new(if video { t!("Videoanruf (wird als Sprachanruf angenommen)", "Video call (will be answered as a voice call)") } else { t!("WhatsApp-Sprachanruf", "WhatsApp voice call") }).color(pal.dim));
                                });
                            });
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui.add(egui::Button::new(RichText::new(t!("✆  Annehmen", "✆  Accept")).color(Color32::WHITE)).fill(pal.accent).corner_radius(18.0).min_size(egui::vec2(140.0, 36.0))).clicked() {
                                    cmd = Some(Cmd::AcceptCall(id.clone()));
                                    self.call = Some((from.clone(), t!("Verbinde …", "Connecting …").into(), std::time::Instant::now(), false));
                                    clear_in = true;
                                }
                                if ui.add(egui::Button::new(RichText::new(t!("Ablehnen", "Decline")).color(Color32::WHITE)).fill(pal.danger).corner_radius(18.0).min_size(egui::vec2(140.0, 36.0))).clicked() {
                                    cmd = Some(Cmd::RejectCall(id.clone()));
                                    clear_in = true;
                                }
                            });
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                        } else if let Some((peer, text, start, muted)) = self.call.clone() {
                            let title = self.name_for_user(&peer).unwrap_or_else(|| format!("+{peer}"));
                            ui.horizontal(|ui| {
                                draw_avatar(ui, self.avatars.get(&peer), &title, 48.0, self.settings.dark);
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&title).size(17.0).family(theme::bold()));
                                    let t = if text == t!("Verbunden", "Connected") {
                                        let s = start.elapsed().as_secs();
                                        format!("{}:{:02}", s / 60, s % 60)
                                    } else {
                                        text.clone()
                                    };
                                    ui.label(RichText::new(t).color(pal.dim));
                                });
                            });
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                let ml = if muted { t!("🎤 Stumm aus", "🎤 Unmute") } else { t!("🎤 Stumm", "🎤 Mute") };
                                if ui.add(egui::Button::new(ml).corner_radius(18.0).min_size(egui::vec2(140.0, 36.0)).selected(muted)).clicked() {
                                    cmd = Some(Cmd::MuteCall(!muted));
                                    if let Some(c) = self.call.as_mut() {
                                        c.3 = !muted;
                                    }
                                }
                                if ui.add(egui::Button::new(RichText::new(t!("Auflegen", "Hang up")).color(Color32::WHITE)).fill(pal.danger).corner_radius(18.0).min_size(egui::vec2(140.0, 36.0))).clicked() {
                                    cmd = Some(Cmd::Hangup);
                                }
                            });
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                        }
                    });
            });
        if clear_in {
            self.call_in = None;
        }
        if let Some(c) = cmd {
            let _ = self.cmd.send(c);
        }
    }

    pub(crate) fn new_group_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some((mut name, mut sel, mut nums)) = self.new_group.take() else { return };
        let mut open = true;
        let mut create = false;
        egui::Window::new(t!("Neue Gruppe", "New group"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(360.0);
                ui.add(egui::TextEdit::singleline(&mut name).hint_text(t!("Gruppenname", "Group name")).desired_width(f32::INFINITY).margin(egui::vec2(10.0, 7.0)));
                ui.add_space(6.0);
                ui.label(RichText::new(t!("Mitglieder auswählen", "Select members")).small().color(pal.dim));
                let mut people: Vec<(String, String)> = self
                    .chats
                    .values()
                    .filter(|c| !c.is_group && c.jid.server == whatsapp_rust::Server::Pn)
                    .map(|c| (c.jid.user.to_string(), self.display_name(c)))
                    .collect();
                people.sort_by_key(|a| a.1.to_lowercase());
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    for (u, n) in people {
                        let mut on = sel.contains(&u);
                        if ui.checkbox(&mut on, n).changed() {
                            if on {
                                sel.insert(u);
                            } else {
                                sel.remove(&u);
                            }
                        }
                    }
                });
                ui.add(egui::TextEdit::singleline(&mut nums).hint_text(t!("Weitere Nummern (mit Komma getrennt)", "More numbers (comma-separated)")).desired_width(f32::INFINITY));
                ui.add_space(6.0);
                if ui.add_enabled(!name.trim().is_empty() && self.conn == Conn::Online, egui::Button::new(t!("Gruppe erstellen", "Create group"))).clicked() {
                    create = true;
                }
            });
        if create {
            let mut numbers: Vec<String> = sel.iter().cloned().collect();
            numbers.extend(nums.split([',', ';', ' ']).map(|n| n.chars().filter(|c| c.is_ascii_digit()).collect::<String>()).filter(|n| n.len() >= 7));
            let _ = self.cmd.send(Cmd::CreateGroup { name: name.trim().to_owned(), numbers });
        } else if open {
            self.new_group = Some((name, sel, nums));
        }
    }

    /// Tastenkürzel: Strg+F Suche, Strg+N neuer Chat, Esc schließen, Alt+↑/↓ Chat wechseln.
    pub(crate) fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let (find, new, esc, up, down) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::F),
                i.consume_key(Modifiers::COMMAND, Key::N),
                i.key_pressed(Key::Escape),
                i.consume_key(Modifiers::ALT, Key::ArrowUp),
                i.consume_key(Modifiers::ALT, Key::ArrowDown),
            )
        });
        if find || new {
            self.show_settings = false;
            self.show_archive = false;
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("list_search")));
            if new {
                self.info = Some(t!("Telefonnummer mit Ländervorwahl in die Suche tippen, z. B. +49 151 …", "Type a phone number with country code into the search, e.g. +49 151 …").into());
            }
        }
        if esc && !ctx.memory(|m| m.any_popup_open()) {
            if self.forward.is_some() {
                self.forward = None;
            } else if self.pending_paste.is_some() {
                self.discard_paste();
            } else if self.reply_to.is_some() {
                self.reply_to = None;
            } else if self.chat_search.is_some() {
                self.chat_search = None;
            } else if self.show_settings {
                self.show_settings = false;
            } else if !self.search.is_empty() {
                self.search.clear();
            } else if let Some(k) = self.selected.clone() {
                let d = std::mem::take(&mut self.input);
                if !d.trim().is_empty() {
                    self.drafts.insert(k, d);
                }
                self.selected = None;
            }
        }
        if (up || down) && !self.row_keys.is_empty() {
            let pos = self.selected.as_ref().and_then(|k| self.row_keys.iter().position(|r| r == k));
            let next = match (pos, down) {
                (None, _) => 0,
                (Some(p), true) => (p + 1).min(self.row_keys.len() - 1),
                (Some(p), false) => p.saturating_sub(1),
            };
            let k = self.row_keys[next].clone();
            self.select_chat(k);
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("wa_input")));
        }
    }

    pub(crate) fn forward_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some((id, mut q)) = self.forward.take() else { return };
        let mut open = true;
        let mut target: Option<String> = None;
        egui::Window::new(t!("Weiterleiten an …", "Forward to …"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(340.0);
                let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text(t!("Chat suchen", "Search chat")).desired_width(f32::INFINITY).margin(egui::vec2(10.0, 7.0)));
                r.request_focus();
                ui.add_space(6.0);
                let needle = q.to_lowercase();
                let mut list: Vec<(String, String, i64)> = self
                    .chats
                    .iter()
                    .map(|(k, c)| (k.clone(), self.display_name(c), c.last_ts))
                    .filter(|(_, n, _)| needle.is_empty() || n.to_lowercase().contains(&needle))
                    .collect();
                list.sort_by_key(|x| std::cmp::Reverse(x.2));
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for (k, n, _) in list.into_iter().take(80) {
                        let user = self.chats[&k].jid.user.to_string();
                        let resp = ui
                            .horizontal(|ui| {
                                draw_avatar(ui, self.avatars.get(&user), &n, 32.0, self.settings.dark);
                                ui.label(RichText::new(&n).size(15.0));
                            })
                            .response
                            .interact(egui::Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(resp.rect, 6.0, pal.hover.gamma_multiply(0.5));
                        }
                        if resp.clicked() {
                            target = Some(k);
                        }
                    }
                });
            });
        match target {
            Some(k) => {
                let msg = self
                    .msg_index
                    .get(&id)
                    .and_then(|ck| self.chats.get(ck))
                    .and_then(|c| c.messages.iter().rev().find(|m| m.id == id).cloned());
                if let (Some(msg), Some(c)) = (msg, self.chats.get(&k)) {
                    let name = self.display_name(c);
                    let _ = self.cmd.send(Cmd::Forward { chat: c.jid.clone(), msg });
                    self.info = Some(tf!("Weitergeleitet an {name}", "Forwarded to {name}"));
                }
            }
            None if open => self.forward = Some((id, q)),
            None => {}
        }
    }

    /// Strg+V mit Bild in der Zwischenablage: Bild übernehmen und Vorschau zeigen.
    /// Eingefügtes Bild verwerfen und die Temp-Datei gleich mit löschen.
    pub(crate) fn discard_paste(&mut self) {
        if let Some((path, _)) = self.pending_paste.take() {
            let _ = std::fs::remove_file(path);
        }
    }

    pub(crate) fn try_paste_image(&mut self, ctx: &egui::Context) {
        let img = match arboard::Clipboard::new().and_then(|mut c| c.get_image()) {
            Ok(i) => i,
            Err(e) => {
                backend::log(format!("Zwischenablage: {e}"));
                return;
            }
        };
        let Some(rgba) = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned()) else { return };
        // PNG-Kodieren dauert bei großen Bildschirmfotos spürbar: nicht im Fenster-Thread.
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = (|| -> Result<(std::path::PathBuf, [usize; 2], Vec<u8>), String> {
                let dir = data_dir().join("tmp");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = dir.join(format!("Zwischenablage-{}.png", now_ts()));
                rgba.save(&path).map_err(|e| e.to_string())?;
                let thumb = image::DynamicImage::ImageRgba8(rgba).thumbnail(240, 240).to_rgba8();
                Ok((path, [thumb.width() as usize, thumb.height() as usize], thumb.into_raw()))
            })();
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.paste_rx = Some(rx);
    }

    /// Fertig kodiertes Bild aus der Zwischenablage übernehmen.
    pub(crate) fn poll_paste(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.paste_rx else { return };
        match rx.try_recv() {
            Ok(Ok((path, size, rgba))) => {
                let ci = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                let tex = ctx.load_texture("paste", ci, egui::TextureOptions::LINEAR);
                if let Some((old, _)) = self.pending_paste.replace((path, tex)) {
                    let _ = std::fs::remove_file(old);
                }
                self.paste_rx = None;
            }
            Ok(Err(e)) => {
                self.info = Some(tf!("Bild aus der Zwischenablage: {e}", "Image from clipboard: {e}"));
                self.paste_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.paste_rx = None,
        }
    }
}
