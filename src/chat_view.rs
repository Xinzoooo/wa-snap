use super::*;

impl App {
    pub(crate) fn chat_view(&mut self, ctx: &egui::Context, pal: &Palette) {
        let mut act: Option<Act> = None;
        let mut chat_hits = self.chat_search_cache.take();
        let mut heights = std::mem::take(&mut self.heights);
        let mut ecache = std::mem::take(&mut self.emoji);
        let mut media_req: Vec<(String, backend::MediaRef)> = Vec::new();
        let mut open_req: Vec<(String, backend::MediaRef)> = Vec::new();
        let mut msg_act: Option<MsgAct> = None;
        let mut jump_req: Option<String> = None;
        let mut older_req = false;
        let mut audio_req: Option<(String, backend::MediaRef, Option<f32>)> = None;
        let mut chat_cmd: Option<Cmd> = None;
        let mut toggle_info = false;
        let mut refetch = false;
        let mut export_chat = false;
        let mut jumped = false;
        let mut scroll_out: Option<(f32, egui::Rect)> = None;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(pal.chat)).show(ctx, |ui| {
            if self.show_settings {
                self.settings_view(ui, ctx, pal);
                return;
            }
            if self.show_starred {
                self.starred_view(ui, pal);
                return;
            }
            let Some(key) = self.selected.clone() else {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.35);
                    ui.label(RichText::new(t!("WhatsSnap für Linux", "WhatsSnap for Linux")).size(28.0).family(theme::bold()));
                    ui.add_space(6.0);
                    ui.label(RichText::new(t!("Nachrichten senden und empfangen, ohne Browser.", "Send and receive messages without a browser.")).color(pal.dim));
                    ui.add_space(24.0);
                    ui.label(RichText::new(t!("🔒 Deine privaten Nachrichten sind Ende-zu-Ende-verschlüsselt.", "🔒 Your personal messages are end-to-end encrypted.")).small().color(pal.dim));
                });
                return;
            };
            let Some(chat) = self.chats.get(&key) else { return };
            let title = self.display_name(chat);
            let user = chat.jid.user.to_string();
            let muted = Self::has(&self.settings.muted, &key);
            let pinned = Self::has(&self.settings.pinned, &key);
            let archived = Self::has(&self.settings.archived, &key);
            let is_group = chat.is_group;
            let chat_jid = chat.jid.clone();
            let dark = self.settings.dark;

            // Kopfzeile
            egui::Frame::new()
                .fill(pal.bar)
                .inner_margin(egui::Margin::symmetric(16, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let av = ui.scope(|ui| draw_avatar(ui, self.avatars.get(&user), &title, 40.0, dark)).response;
                        if av.interact(egui::Sense::click()).on_hover_text(t!("Kontakt-/Gruppeninfo", "Contact/group info")).clicked() {
                            toggle_info = true;
                        }
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add_space(2.0);
                            ui.label(RichText::new(&title).size(16.0).family(theme::bold()));
                            let typing = self.typing.get(user.as_str()).filter(|t| t.2.elapsed().as_secs() < 12);
                            let (sub, col) = if let Some((from, rec, _)) = typing {
                                let what = if *rec { t!("nimmt Sprachnachricht auf …", "recording audio …") } else { t!("schreibt …", "typing …") };
                                let t = if is_group {
                                    format!("{} {what}", self.names.get(from).map(|n| n.0.clone()).unwrap_or_else(|| format!("+{from}")))
                                } else {
                                    what.to_owned()
                                };
                                (t, pal.accent)
                            } else if is_group {
                                (t!("Gruppe", "Group").to_owned(), pal.dim)
                            } else if let Some((online, seen)) = self.presence.get(user.as_str()) {
                                if *online {
                                    ("online".to_owned(), pal.dim)
                                } else if let Some(t) = seen {
                                    (tf!("zuletzt online {}", "last seen {}", fmt_last_seen(*t)), pal.dim)
                                } else {
                                    (format!("+{user}"), pal.dim)
                                }
                            } else if chat.jid.server == whatsapp_rust::Server::Pn {
                                (format!("+{user}"), pal.dim)
                            } else {
                                (t!("Kontakt", "Contact").to_owned(), pal.dim)
                            };
                            ui.label(RichText::new(sub).size(12.5).color(col));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let dots = icon_button(ui, "", 17.0, pal.dim, pal, t!("Menü", "Menu"));
                            for dy in [-5.0, 0.0, 5.0] {
                                ui.painter().circle_filled(dots.rect.center() + egui::vec2(0.0, dy), 1.8, pal.dim);
                            }
                            let menu_id = egui::Id::new("chat_menu");
                            if dots.clicked() {
                                ui.memory_mut(|m| m.toggle_popup(menu_id));
                            }
                            egui::popup::popup_below_widget(ui, menu_id, &dots, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                                ui.set_min_width(240.0);
                                if muted {
                                    if ui.button(t!("Stummschaltung aufheben", "Unmute")).clicked() {
                                        act = Some(Act::Mute(key.clone(), None));
                                    }
                                } else {
                                    ui.label(RichText::new(t!("Stummschalten für", "Mute for")).small().color(pal.dim));
                                    for (l, d) in mute_opts().iter() {
                                        if ui.button(format!("   {l}")).clicked() {
                                            act = Some(Act::Mute(key.clone(), *d));
                                        }
                                    }
                                }
                                if ui.button(if pinned { t!("Chat loslösen", "Unpin chat") } else { t!("Chat anheften", "Pin chat") }).clicked() {
                                    act = Some(Act::Pin(key.clone()));
                                    ui.close_menu();
                                }
                                if ui.button(if archived { t!("Aus Archiv holen", "Unarchive") } else { t!("Chat archivieren", "Archive chat") }).clicked() {
                                    act = Some(Act::Archive(key.clone()));
                                    ui.close_menu();
                                }
                                ui.separator();
                                ui.label(RichText::new(t!("Selbstlöschende Nachrichten", "Disappearing messages")).small().color(pal.dim));
                                ui.horizontal(|ui| {
                                    for (secs, l) in [(0u32, t!("Aus", "Off")), (86_400, t!("24 Std.", "24 hrs")), (604_800, t!("7 Tage", "7 days")), (7_776_000, t!("90 Tage", "90 days"))] {
                                        if ui.small_button(l).clicked() {
                                            chat_cmd = Some(Cmd::SetChatDisappearing { chat: chat_jid.clone(), secs });
                                        }
                                    }
                                });
                                if !is_group {
                                    let blocked = self.blocklist.iter().any(|b| user_of(b) == user);
                                    if ui.button(if blocked { t!("Blockierung aufheben", "Unblock") } else { t!("Blockieren", "Block") }).clicked() {
                                        chat_cmd = Some(Cmd::Block { jid: chat_jid.clone(), on: !blocked });
                                    }
                                }
                                ui.separator();
                                if ui.button(t!("Chat exportieren (.txt)", "Export chat (.txt)")).clicked() {
                                    export_chat = true;
                                }
                                if ui
                                    .add_enabled(self.conn == Conn::Online, egui::Button::new(t!("Verlauf vom Handy nachladen", "Reload history from phone")))
                                    .on_hover_text(t!("Holt die letzten rund 300 Nachrichten dieses Chats erneut. Fehlende Fotos, Videos und Sprachnachrichten werden ergänzt.", "Fetches the last 300 or so messages of this chat again. Missing photos, videos and voice messages are added."))
                                    .clicked()
                                {
                                    refetch = true;
                                }
                                if ui.button(t!("Chat schließen", "Close chat")).clicked() {
                                    self.selected = None;
                                }
                            });
                            if !is_group && self.conn == Conn::Online && self.call.is_none() && icon_button(ui, "📞", 17.0, pal.dim, pal, t!("Sprachanruf", "Voice call")).clicked() {
                                chat_cmd = Some(Cmd::Call(chat_jid.clone()));
                                self.call = Some((user.clone(), t!("Rufe an …", "Calling …").into(), std::time::Instant::now(), false));
                            }
                            if icon_button(ui, "ℹ", 17.0, if self.show_info { pal.accent } else { pal.dim }, pal, t!("Kontakt-/Gruppeninfo", "Contact/group info")).clicked() {
                                toggle_info = true;
                            }
                            if icon_button(ui, "🔍", 17.0, pal.dim, pal, t!("Im Chat suchen", "Search in chat")).clicked() {
                                self.chat_search = if self.chat_search.is_some() { None } else { Some(String::new()) };
                            }
                        });
                    });
                });
            ui.painter().hline(ui.max_rect().x_range(), ui.min_rect().bottom(), egui::Stroke::new(1.0_f32, pal.border));

            if let Some(q) = self.chat_search.as_mut() {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.add(egui::TextEdit::singleline(q).hint_text(t!("Nachrichten durchsuchen", "Search messages")).desired_width(320.0).margin(egui::vec2(10.0, 6.0)));
                });
            }
            let needle = self.chat_search.as_deref().unwrap_or("").to_lowercase();

            // Infozeile (Fehler, Hinweise)
            if let Some(msg) = self.info.clone() {
                egui::Frame::new()
                    .fill(pal.bubble_them)
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(12, 6))
                    .outer_margin(egui::Margin::symmetric(16, 6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(msg).size(13.0));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("×").clicked() {
                                    self.info = None;
                                }
                            });
                        });
                    });
            }

            let Some(chat) = self.chats.get(&key) else { return };
            let area = ui.available_rect_before_wrap();
            paint_pattern(ui, area, pal);
            // Nur die letzten Nachrichten zeichnen: schnell auch bei langen Chats.
            let limit = *self.show_limit.get(&key).unwrap_or(&400);
            let hidden = chat.messages.len().saturating_sub(limit);
            let msgs: Vec<&ChatMessage> = if needle.is_empty() {
                chat.messages[hidden..].iter().collect()
            } else {
                // Trefferliste nur neu berechnen, wenn sich Suchtext oder Chat ändern.
                let fresh = matches!(&chat_hits, Some((k, n, len, _)) if *k == key && *n == needle && *len == chat.messages.len());
                if !fresh {
                    let idx: Vec<usize> = chat.messages.iter().enumerate().filter(|(_, m)| m.text.to_lowercase().contains(&needle)).map(|(i, _)| i).collect();
                    chat_hits = Some((key.clone(), needle.clone(), chat.messages.len(), idx));
                }
                chat_hits.as_ref().map(|(_, _, _, idx)| idx.iter().filter_map(|i| chat.messages.get(*i)).collect()).unwrap_or_default()
            };
            // Nicht `vertical_scroll_offset(f32::MAX)`: der Inhalt landet dann für ein Bild weit
            // außerhalb, seine Höhe wird zu 0 und der Chat springt an den Anfang oder bleibt leer.
            let to_bottom = self.scroll_bottom;
            let area = egui::ScrollArea::vertical().auto_shrink([false; 2]).stick_to_bottom(true);
            let out = area
                .show(ui, |ui| {
                    ui.add_space(10.0);
                    if needle.is_empty() {
                        ui.vertical_centered(|ui| {
                            let label = if hidden > 0 { tf!("Ältere Nachrichten anzeigen ({hidden})", "Show older messages ({hidden})") } else { t!("Ältere Nachrichten vom Handy laden", "Load older messages from phone").to_owned() };
                            if ui.add(egui::Button::new(RichText::new(label).size(12.5).color(pal.dim)).fill(pal.bubble_them).corner_radius(12.0)).clicked() {
                                older_req = true;
                            }
                        });
                        ui.add_space(4.0);
                    }
                    let full_w = ui.available_width();
                    let mut last_day = i32::MIN;
                    let mut prev: Option<(bool, &str)> = None;
                    for m in msgs {
                        let dk = day_key(m.ts);
                        if dk != last_day {
                            last_day = dk;
                            prev = None;
                            ui.add_space(8.0);
                            {
                                let g = ui.painter().layout_no_wrap(day_label(m.ts), egui::FontId::proportional(12.5), pal.dim);
                                let (row, _) = ui.allocate_exact_size(egui::vec2(full_w, g.size().y + 12.0), egui::Sense::hover());
                                let r = egui::Rect::from_center_size(row.center(), g.size() + egui::vec2(24.0, 12.0));
                                ui.painter().rect_filled(r, 8.0, pal.bubble_them);
                                ui.painter().galley(r.center() - g.size() / 2.0, g, pal.dim);
                            }
                            ui.add_space(6.0);
                        }
                        let first = prev != Some((m.from_me, m.sender_name.as_str()));
                        prev = Some((m.from_me, m.sender_name.as_str()));
                        ui.add_space(if first { 6.0 } else { 1.0 });
                        // Außerhalb des sichtbaren Bereichs nur die gemessene Höhe freihalten.
                        let sig = self.height_sig(m, first, full_w);
                        if let Some(&(s, h)) = heights.get(&m.id) {
                            let (top, clip) = (ui.cursor().top(), ui.clip_rect());
                            let off = top + h < clip.top() - 400.0 || top > clip.bottom() + 400.0;
                            if s == sig && off && self.jump_to.as_deref() != Some(m.id.as_str()) {
                                ui.allocate_space(egui::vec2(full_w, h));
                                continue;
                            }
                        }
                        let (mut bg, align) = if m.from_me {
                            (pal.bubble_me, egui::Align::Max)
                        } else {
                            (pal.bubble_them, egui::Align::Min)
                        };
                        let sticker = m.media.as_ref().map(|x| x.kind == "sticker").unwrap_or(false);
                        if m.x.deleted {
                            bg = bg.gamma_multiply(0.75);
                        }
                        if sticker {
                            bg = Color32::TRANSPARENT;
                        }
                        let margin_side = (full_w * 0.06).clamp(16.0, 63.0) as i8;
                        let row = ui.with_layout(egui::Layout::top_down(align), |ui| {
                            // Fläche fürs Rechtsklick-Menü VOR dem Inhalt anmelden, damit Knöpfe in der Blase Vorrang haben
                            let bid = egui::Id::new(("bubble", &m.id));
                            let bubble_hit = ui.ctx().data(|d| d.get_temp::<egui::Rect>(bid)).map(|r| ui.interact(r, bid, egui::Sense::click()));
                            let resp = egui::Frame::new()
                                .fill(bg)
                                .corner_radius(8.0)
                                .inner_margin(egui::Margin { left: 9, right: 7, top: 6, bottom: 5 })
                                .outer_margin(egui::Margin::symmetric(margin_side, 0))
                                .show(ui, |ui| {
                                    // Breite der Blase aus dem Text berechnen, sonst nimmt das Umbruch-Layout die volle Breite.
                                    let max_w = (full_w * 0.65).max(160.0);
                                    let tw = rich_width(ui, &m.text, self.settings.font_size + 0.5, max_w);
                                    let sw = if is_group && !m.from_me && first {
                                        ui.fonts(|f| f.layout_no_wrap(m.sender_name.clone(), egui::FontId::new(13.0, theme::bold()), pal.text).size().x)
                                    } else {
                                        0.0
                                    };
                                    let meta_w = if m.from_me { 58.0 } else { 40.0 };
                                    let media_w = match &m.media {
                                        Some(md) if md.is_picture() => media_size(md, self.tex(&m.id), max_w).x,
                                        Some(_) => 260.0_f32.min(max_w),
                                        None => 0.0,
                                    };
                                    let extra_w = match &m.x.kind {
                                        Some(backend::Extra::Link { .. }) => 260.0,
                                        Some(_) => 260.0,
                                        None if m.x.view_once => 300.0,
                                        None => 0.0,
                                    };
                                    let w = (tw + 8.0 + meta_w).max(sw).max(media_w).max(extra_w).min(max_w);
                                    ui.set_max_width(w);
                                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                    if is_group && !m.from_me && first && !m.sender_name.is_empty() {
                                        ui.label(
                                            RichText::new(&m.sender_name)
                                                .size(13.0)
                                                .family(theme::bold())
                                                .color(theme::sender_color(&m.sender_name, dark)),
                                        );
                                    }
                                    if let Some(q) = &m.quote {
                                        // Zitat: farbiger Balken links, Name und Text gekürzt
                                        let qname = if q.sender.is_empty() || self.me.contains(&user_of(&q.sender)) {
                                            t!("Du", "You").to_owned()
                                        } else {
                                            let u = user_of(&q.sender);
                                            self.names.get(&u).map(|n| n.0.clone()).unwrap_or_else(|| if q.name.is_empty() { format!("+{u}") } else { q.name.clone() })
                                        };
                                        let col = theme::sender_color(&qname, dark);
                                        let fr = egui::Frame::new()
                                            .fill(if dark { Color32::from_black_alpha(60) } else { Color32::from_black_alpha(12) })
                                            .corner_radius(6.0)
                                            .inner_margin(egui::Margin { left: 10, right: 8, top: 5, bottom: 5 })
                                            .show(ui, |ui| {
                                                ui.set_min_width((w - 16.0).max(120.0));
                                                ui.label(RichText::new(&qname).size(12.5).family(theme::bold()).color(col));
                                                let t: String = q.text.replace('\n', " ").chars().take(120).collect();
                                                ui.add(egui::Label::new(RichText::new(t).size(13.0).color(pal.dim)).truncate());
                                            });
                                        if fr.response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                            jump_req = Some(q.id.clone());
                                        }
                                        let r = fr.response.rect;
                                        ui.painter().rect_filled(
                                            egui::Rect::from_min_max(r.left_top(), egui::pos2(r.left() + 4.0, r.bottom())),
                                            egui::CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 },
                                            col,
                                        );
                                        ui.add_space(3.0);
                                    }
                                    if m.x.view_once {
                                        ui.label(RichText::new(t!("⏱ Einmal-Ansicht – nur auf dem Handy sichtbar", "⏱ View once – only visible on the phone")).size(14.0).italics().color(pal.dim));
                                    } else if let Some(md) = &m.media {
                                        self.media_block(ui, m, md, max_w, pal, &mut media_req, &mut open_req, &mut audio_req);
                                    }
                                    if let Some(k) = &m.x.kind {
                                        self.extra_block(ui, m, k, w, pal, &mut msg_act);
                                    }
                                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing.x = 8.0;
                                        if !m.text.is_empty() {
                                            let shown = self.with_mention_names(&m.text, &m.x.mentions);
                                            rich_label(ui, &shown, self.settings.font_size + 0.5, &mut ecache);
                                            ui.add_space(8.0);
                                        }
                                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Max), |ui| {
                                            ui.spacing_mut().item_spacing.x = 3.0;
                                            if self.starred.contains(&m.id) {
                                                ui.label(RichText::new("★").size(11.0).color(pal.dim));
                                            }
                                            if m.x.expiration > 0 {
                                                ui.label(RichText::new("⏱").size(10.0).color(pal.dim)).on_hover_text(t!("Selbstlöschende Nachricht", "Disappearing message"));
                                            }
                                            if m.x.deleted {
                                                ui.label(RichText::new(t!("🚫 gelöscht", "🚫 deleted")).size(11.0).color(pal.danger))
                                                    .on_hover_text(t!("Der Absender hat diese Nachricht für alle gelöscht. WhatsSnap zeigt sie weiter an.", "The sender deleted this message for everyone. WhatsSnap keeps showing it."));
                                            }
                                            if m.x.edited {
                                                let l = ui.label(RichText::new(if m.x.prev.is_empty() { t!("Bearbeitet", "Edited") } else { t!("Bearbeitet ⓘ", "Edited ⓘ") }).size(11.0).italics().color(pal.dim));
                                                if !m.x.prev.is_empty() {
                                                    let hist: Vec<String> = m.x.prev.iter().enumerate().map(|(i, t)| tf!("Fassung {}: {t}", "Version {}: {t}", i + 1)).collect();
                                                    l.on_hover_text(tf!("Frühere Fassungen:\n{}", "Earlier versions:\n{}", hist.join("\n")));
                                                }
                                            }
                                            ui.label(RichText::new(fmt_time(m.ts)).size(11.0).color(pal.dim));
                                            if m.from_me {
                                                draw_ticks(ui, m.status, pal);
                                            }
                                        });
                                    }));
                                    });
                                });
                            if self.jump_to.as_deref() == Some(m.id.as_str()) {
                                resp.response.scroll_to_me(Some(egui::Align::Center));
                                jumped = true;
                            }
                            if let Some((hid, t)) = &self.highlight {
                                if *hid == m.id && t.elapsed().as_millis() < 1600 {
                                    let a = 1.0 - t.elapsed().as_millis() as f32 / 1600.0;
                                    ui.painter().rect_stroke(resp.response.rect.shrink2(egui::vec2(margin_side as f32, 0.0)), 8.0, egui::Stroke::new(2.0_f32, pal.accent.gamma_multiply(a)), egui::StrokeKind::Outside);
                                    ui.ctx().request_repaint();
                                }
                            }
                            // Reaktionen unter der Blase
                            if let Some(list) = self.reactions.get(&m.id).filter(|l| !l.is_empty()) {
                                let mut uniq: Vec<&str> = Vec::new();
                                for (_, e) in list {
                                    if !uniq.contains(&e.as_str()) {
                                        uniq.push(e);
                                    }
                                }
                                let r = resp.response.rect;
                                let esz = 16.0;
                                let count = if list.len() > 1 { list.len().to_string() } else { String::new() };
                                let cw = if count.is_empty() { 0.0 } else { 6.0 + 7.0 * count.len() as f32 };
                                let pw = uniq.len().min(3) as f32 * (esz + 2.0) + cw + 12.0;
                                let x = if m.from_me { r.right() - margin_side as f32 - pw - 6.0 } else { r.left() + margin_side as f32 + 6.0 };
                                let pill = egui::Rect::from_min_size(egui::pos2(x, r.bottom() - 4.0), egui::vec2(pw, 24.0));
                                ui.allocate_space(egui::vec2(0.0, 18.0));
                                let p = ui.painter();
                                p.rect_filled(pill, 12.0, pal.bubble_them);
                                p.rect_stroke(pill, 12.0, egui::Stroke::new(1.0_f32, pal.chat), egui::StrokeKind::Outside);
                                let mut ex = pill.left() + 6.0;
                                for e in uniq.iter().take(3) {
                                    let k: String = e.chars().filter(|c| *c != '\u{FE0F}').collect();
                                    if let Some(id) = ecache.get(ui.ctx(), &k) {
                                        let er = egui::Rect::from_min_size(egui::pos2(ex, pill.center().y - esz / 2.0), egui::vec2(esz, esz));
                                        p.image(id, er, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                                    }
                                    ex += esz + 2.0;
                                }
                                if !count.is_empty() {
                                    p.text(egui::pos2(ex + 2.0, pill.center().y), egui::Align2::LEFT_CENTER, count, egui::FontId::proportional(12.0), pal.dim);
                                }
                                let tip: Vec<String> = list
                                    .iter()
                                    .map(|(f, e)| {
                                        let n = if f.is_empty() { t!("Du", "You").to_owned() } else { self.names.get(f).map(|n| n.0.clone()).unwrap_or_else(|| format!("+{f}")) };
                                        format!("{e} {n}")
                                    })
                                    .collect();
                                ui.interact(pill, egui::Id::new(("react", &m.id)), egui::Sense::hover()).on_hover_text(tip.join("\n"));
                            }
                            // Rechtsklick-Menü der Nachricht
                            let mine = self.reactions.get(&m.id).and_then(|l| l.iter().find(|(f, _)| f.is_empty()).map(|x| x.1.clone()));
                            ui.ctx().data_mut(|d| d.insert_temp(bid, resp.response.rect));
                            let hit = bubble_hit.unwrap_or_else(|| resp.response.clone());
                            hit.context_menu(|ui| {
                                ui.horizontal(|ui| {
                                    for e in ["👍", "❤️", "😂", "😮", "😢", "🙏"] {
                                        let k: String = e.chars().filter(|c| *c != '\u{FE0F}').collect();
                                        let on = mine.as_deref() == Some(e);
                                        let clicked = match ecache.get(ui.ctx(), &k) {
                                            Some(id) => ui.add(egui::Button::image(egui::Image::new((id, egui::vec2(24.0, 24.0)))).frame(on)).clicked(),
                                            None => ui.button(e).clicked(),
                                        };
                                        if clicked {
                                            msg_act = Some(MsgAct::React(m.id.clone(), if on { String::new() } else { e.to_owned() }));
                                            ui.close_menu();
                                        }
                                    }
                                });
                                ui.separator();
                                if ui.button(t!("Antworten", "Reply")).clicked() {
                                    msg_act = Some(MsgAct::Reply(m.id.clone()));
                                    ui.close_menu();
                                }
                                if ui.button(t!("Weiterleiten", "Forward")).clicked() {
                                    msg_act = Some(MsgAct::Forward(m.id.clone()));
                                    ui.close_menu();
                                }
                                let is_star = self.starred.contains(&m.id);
                                if ui.button(if is_star { t!("Markierung entfernen", "Unstar") } else { t!("Mit Stern markieren", "Star") }).clicked() {
                                    msg_act = Some(MsgAct::Star(m.id.clone(), !is_star));
                                    ui.close_menu();
                                }
                                if m.from_me && m.media.is_none() && m.x.kind.is_none() && !m.id.starts_with("local-") && now_ts() - m.ts < 15 * 60 && ui.button(t!("Bearbeiten", "Edit")).clicked() {
                                    msg_act = Some(MsgAct::Edit(m.id.clone()));
                                    ui.close_menu();
                                }
                                if m.from_me && m.status == 9 && (self.outbox.contains_key(&m.id) || self.outbox_files.contains_key(&m.id)) {
                                    if ui.button(t!("Erneut senden", "Send again")).clicked() {
                                        msg_act = Some(MsgAct::Resend(m.id.clone()));
                                        ui.close_menu();
                                    }
                                    if ui.button(t!("Nicht senden (verwerfen)", "Don't send (discard)")).clicked() {
                                        msg_act = Some(MsgAct::Discard(m.id.clone()));
                                        ui.close_menu();
                                    }
                                }
                                if !m.text.is_empty() && ui.button(t!("Text kopieren", "Copy text")).clicked() {
                                    ui.ctx().copy_text(m.text.clone());
                                    ui.close_menu();
                                }
                                if m.media.is_some() && ui.button(t!("Öffnen", "Open")).clicked() {
                                    if let Some(md) = &m.media {
                                        open_req.push((m.id.clone(), md.clone()));
                                    }
                                    ui.close_menu();
                                }
                                if m.from_me && !m.id.starts_with("local-") && now_ts() - m.ts < 2 * 86_400 && ui.button(t!("Für alle löschen", "Delete for everyone")).clicked() {
                                    msg_act = Some(MsgAct::Revoke(m.id.clone()));
                                    ui.close_menu();
                                }
                            });
                            // kleiner Zipfel an der ersten Blase einer Folge
                            if first && !sticker {
                                let r = resp.response.rect;
                                let p = ui.painter();
                                let (a, b, c) = if m.from_me {
                                    let x = r.right() - margin_side as f32;
                                    (egui::pos2(x - 6.0, r.top()), egui::pos2(x + 8.0, r.top()), egui::pos2(x - 6.0, r.top() + 12.0))
                                } else {
                                    let x = r.left() + margin_side as f32;
                                    (egui::pos2(x + 6.0, r.top()), egui::pos2(x - 8.0, r.top()), egui::pos2(x + 6.0, r.top() + 12.0))
                                };
                                p.add(egui::Shape::convex_polygon(vec![a, b, c], bg, egui::Stroke::NONE));
                            }
                        });
                        let h = row.response.rect.height();
                        if heights.get(&m.id) != Some(&(sig, h)) {
                            heights.insert(m.id.clone(), (sig, h));
                        }
                    }
                    ui.add_space(10.0);
                    if to_bottom {
                        ui.scroll_to_cursor_animation(Some(egui::Align::BOTTOM), egui::style::ScrollAnimation::none());
                    }
                });
            let dist = out.content_size.y - (out.state.offset.y + out.inner_rect.height());
            scroll_out = Some((dist, out.inner_rect));
        });
        self.emoji = ecache;
        self.heights = heights;
        self.chat_search_cache = chat_hits;
        self.scroll_bottom = false;
        if let Some(c) = chat_cmd {
            let _ = self.cmd.send(c);
        }
        if toggle_info {
            self.show_info = !self.show_info;
        }
        if export_chat {
            self.export_current();
        }
        if refetch {
            if let Some(c) = self.selected.as_ref().and_then(|k| self.chats.get(k)) {
                // Jede 50. Nachricht von neu nach alt als Anker: das Handy liefert jeweils die 50 davor.
                let real: Vec<&ChatMessage> = c.messages.iter().rev().filter(|m| !m.id.is_empty() && !m.id.starts_with("local-") && !m.id.starts_with("demo")).collect();
                let anchors: Vec<(String, bool, i64)> = real.iter().step_by(50).take(6).map(|m| (m.id.clone(), m.from_me, m.ts)).collect();
                if !anchors.is_empty() {
                    let _ = self.cmd.send(Cmd::Refetch { chat: c.jid.clone(), anchors });
                }
            }
        }
        if let Some((id, media, seek)) = audio_req {
            self.audio_action(id, media, seek);
        }
        if older_req {
            if let Some(key) = self.selected.clone() {
                if let Some(c) = self.chats.get(&key) {
                    let limit = *self.show_limit.get(&key).unwrap_or(&400);
                    if c.messages.len() > limit {
                        self.show_limit.insert(key, limit + 300);
                    } else if let Some(o) = c.messages.iter().find(|m| !m.id.starts_with("local-") && !m.id.starts_with("demo")) {
                        let _ = self.cmd.send(Cmd::LoadOlder { chat: c.jid.clone(), id: o.id.clone(), from_me: o.from_me, ts: o.ts });
                        self.show_limit.insert(key, limit + 300);
                    }
                }
            }
        }
        if jumped {
            if let Some(id) = self.jump_to.take() {
                self.highlight = Some((id, std::time::Instant::now()));
            }
        }
        if let Some(id) = jump_req {
            if self.msg_index.contains_key(&id) {
                self.jump_to = Some(id);
            } else {
                self.info = Some(t!("Die zitierte Nachricht ist nicht im geladenen Verlauf.", "The quoted message isn't in the loaded history.").into());
            }
        }
        // Knopf "Nach unten" mit Zähler neuer Nachrichten
        if let Some((dist, rect)) = scroll_out {
            self.at_bottom = dist < 80.0;
            if self.at_bottom {
                self.new_below = 0;
            } else if !self.show_settings {
                let c = rect.right_bottom() + egui::vec2(-36.0, -30.0);
                let r = egui::Rect::from_center_size(c, egui::vec2(40.0, 40.0));
                let pal2 = *pal;
                let n = self.new_below;
                let clicked = egui::Area::new(egui::Id::new("to_bottom"))
                    .fixed_pos(r.min)
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        let (rr, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
                        let p = ui.painter();
                        p.circle_filled(rr.center(), 20.0, pal2.bubble_them);
                        p.circle_stroke(rr.center(), 20.0, egui::Stroke::new(1.0_f32, pal2.border));
                        let cc = rr.center();
                        let st = egui::Stroke::new(2.0_f32, pal2.dim);
                        p.line_segment([cc + egui::vec2(-6.0, -3.0), cc + egui::vec2(0.0, 3.0)], st);
                        p.line_segment([cc + egui::vec2(0.0, 3.0), cc + egui::vec2(6.0, -3.0)], st);
                        if n > 0 {
                            let bc = rr.right_top() + egui::vec2(-4.0, 4.0);
                            p.circle_filled(bc, 10.0, pal2.accent);
                            p.text(bc, egui::Align2::CENTER_CENTER, n.min(99).to_string(), egui::FontId::new(11.0, theme::bold()), pal2.on_accent);
                        }
                        resp.on_hover_text(t!("Nach unten", "Scroll down")).clicked()
                    })
                    .inner;
                if clicked {
                    self.scroll_bottom = true;
                    self.new_below = 0;
                }
            }
        }
        for (id, media) in media_req {
            if let Some(real) = id.strip_suffix("#ok") {
                self.img_requested.insert(id.clone());
                if self.img_requested.insert(real.to_owned()) {
                    let _ = self.cmd.send(Cmd::LoadMedia { id: real.to_owned(), media });
                }
                continue;
            }
            if self.img_requested.insert(id.clone()) {
                let _ = self.cmd.send(Cmd::LoadMedia { id, media });
            }
        }
        for (id, media) in open_req {
            // Bilder und Videos im eingebauten Betrachter, alles andere mit dem Standardprogramm.
            match self.selected.clone() {
                Some(chat) if viewer::viewable(&media) => self.open_viewer(ctx, chat, id, media),
                _ => {
                    let _ = self.cmd.send(Cmd::OpenMedia { id, media });
                }
            }
        }
        if let Some(a) = msg_act {
            self.run_msg_act(a);
        }
        if let Some(a) = act {
            self.run_act(a);
        }
    }
}
