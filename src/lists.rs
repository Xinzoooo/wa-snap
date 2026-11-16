use super::*;

impl App {
    /// Emoji-Auswahl wie bei WhatsApp: Suche, Zuletzt, Kategorien, Hauttöne.
    pub(crate) fn emoji_grid(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        const PER_ROW: usize = 9;
        const ROW_H: f32 = 40.0;
        ui.add(egui::TextEdit::singleline(&mut self.emoji_search).hint_text(t!("🔍 Emoji suchen (z. B. lachen, Herz, Daumen)", "🔍 Search emoji (e.g. laugh, heart, thumbs)")).desired_width(f32::INFINITY));
        let cats = emoji::picker();
        // Zeilen bauen: Überschrift oder bis zu 9 Emojis (mit Hautton-Varianten)
        let mut rows: Vec<(Option<String>, Vec<(String, Vec<String>)>)> = Vec::new();
        let mut cat_rows: Vec<usize> = Vec::new();
        let q = self.emoji_search.trim().to_lowercase();
        let push_items = |rows: &mut Vec<(Option<String>, Vec<(String, Vec<String>)>)>, items: Vec<(String, Vec<String>)>| {
            for chunk in items.chunks(PER_ROW) {
                rows.push((None, chunk.to_vec()));
            }
        };
        if !q.is_empty() {
            let hits: Vec<(String, Vec<String>)> = cats
                .iter()
                .flat_map(|(_, it)| it.iter())
                .filter(|i| i.name.to_lowercase().contains(&q) || i.tags.contains(&q))
                .take(200)
                .map(|i| (i.e.clone(), i.skins.clone()))
                .collect();
            rows.push((Some(tf!("Suchergebnisse ({})", "Search results ({})", hits.len())), Vec::new()));
            push_items(&mut rows, hits);
        } else {
            if !self.settings.recent_emojis.is_empty() {
                cat_rows.push(rows.len());
                rows.push((Some(t!("Letzte", "Recent").into()), Vec::new()));
                push_items(&mut rows, self.settings.recent_emojis.iter().map(|e| (e.clone(), Vec::new())).collect());
            } else {
                cat_rows.push(0);
            }
            for (name, items) in cats {
                cat_rows.push(rows.len());
                rows.push((Some(i18n::tr(name).to_owned()), Vec::new()));
                push_items(&mut rows, items.iter().map(|i| (i.e.clone(), i.skins.clone())).collect());
            }
        }
        let mut area = egui::ScrollArea::vertical().id_salt("emoji_grid").max_height(300.0).auto_shrink([false, true]);
        if let Some(r) = self.emoji_jump.take() {
            area = area.vertical_scroll_offset(r as f32 * (ROW_H + ui.spacing().item_spacing.y));
        }
        let tone = self.settings.skin_tone as usize;
        let mut chosen: Option<String> = None;
        let mut new_tone: Option<u8> = None;
        area.show_rows(ui, ROW_H, rows.len(), |ui, range| {
            for (head, items) in &rows[range] {
                if let Some(h) = head {
                    ui.add_sized([ui.available_width(), ROW_H], egui::Label::new(RichText::new(h).size(13.0).color(pal.dim)));
                    continue;
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (e, skins) in items {
                        let shown = if tone > 0 && skins.len() >= tone { skins[tone - 1].clone() } else { e.clone() };
                        let resp = match self.emoji.get(ui.ctx(), &emoji::key(&shown)) {
                            Some(id) => ui.add_sized([ROW_H, ROW_H], egui::Button::image(egui::Image::new((id, egui::vec2(30.0, 30.0)))).frame(false)),
                            None => ui.add_sized([ROW_H, ROW_H], egui::Button::new(RichText::new(&shown).size(22.0)).frame(false)),
                        };
                        if !skins.is_empty() {
                            let r = resp.rect;
                            ui.painter().add(egui::Shape::convex_polygon(
                                vec![r.right_bottom() + egui::vec2(-3.0, -3.0), r.right_bottom() + egui::vec2(-9.0, -3.0), r.right_bottom() + egui::vec2(-3.0, -9.0)],
                                pal.dim,
                                egui::Stroke::NONE,
                            ));
                        }
                        if resp.clicked() {
                            chosen = Some(shown.clone());
                        }
                        if !skins.is_empty() {
                            resp.context_menu(|ui| {
                                ui.horizontal(|ui| {
                                    for (i, v) in std::iter::once(e).chain(skins.iter()).enumerate() {
                                        let c = match self.emoji.get(ui.ctx(), &emoji::key(v)) {
                                            Some(id) => ui.add(egui::Button::image(egui::Image::new((id, egui::vec2(28.0, 28.0)))).frame(i == tone)).clicked(),
                                            None => ui.button(v).clicked(),
                                        };
                                        if c {
                                            chosen = Some(v.clone());
                                            new_tone = Some(i as u8);
                                            ui.close_menu();
                                        }
                                    }
                                });
                            });
                        }
                    }
                });
            }
        });
        // Kategorie-Leiste unten
        if q.is_empty() {
            ui.separator();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let icons = ["🕒", "😀", "🐻", "🍔", "⚽", "🚗", "💡", "🔣", "🏁"];
                for (i, ic) in icons.iter().enumerate() {
                    if i >= cat_rows.len() {
                        break;
                    }
                    let resp = match (i, self.emoji.get(ui.ctx(), &emoji::key(ic))) {
                        (0, _) | (_, None) => ui.add(egui::Button::new(RichText::new(*ic).size(17.0).color(pal.dim)).frame(false)),
                        (_, Some(id)) => ui.add(egui::Button::image(egui::Image::new((id, egui::vec2(22.0, 22.0)))).frame(false)),
                    };
                    if resp.clicked() {
                        self.emoji_jump = Some(cat_rows[i]);
                    }
                }
            });
        }
        if let Some(t) = new_tone {
            self.settings.skin_tone = t;
        }
        if let Some(e) = chosen {
            self.input.push_str(&e);
            self.settings.recent_emojis.retain(|x| *x != e);
            self.settings.recent_emojis.insert(0, e);
            self.settings.recent_emojis.truncate(36);
            self.save_settings();
        }
    }

    /// Schmale Leiste ganz links (wie bei WhatsApp Web).
    pub(crate) fn rail(&mut self, ctx: &egui::Context, pal: &Palette) {
        egui::SidePanel::left("rail")
            .exact_width(64.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(pal.rail).stroke(egui::Stroke::new(1.0_f32, pal.border)))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(12.0);
                    let chats_on = !self.show_settings;
                    let unread: u32 = self
                        .chats
                        .iter()
                        .filter(|(k, _)| !Self::has(&self.settings.archived, k))
                        .map(|(_, c)| c.unread.min(1))
                        .sum();
                    let r = icon_button(ui, "💬", 20.0, if chats_on { pal.text } else { pal.dim }, pal, t!("Chats", "Chats"));
                    if chats_on {
                        ui.painter().circle_filled(r.rect.center(), r.rect.width() / 2.0, pal.selected);
                        ui.painter().text(r.rect.center(), egui::Align2::CENTER_CENTER, "💬", egui::FontId::proportional(20.0), pal.text);
                    }
                    if unread > 0 {
                        let c = r.rect.right_top() + egui::vec2(-6.0, 8.0);
                        ui.painter().circle_filled(c, 8.5, pal.accent);
                        ui.painter().text(c, egui::Align2::CENTER_CENTER, unread.min(99).to_string(), egui::FontId::new(10.0, theme::bold()), pal.on_accent);
                    }
                    if r.clicked() {
                        self.show_settings = false;
                        self.show_starred = false;
                        self.show_status = false;
                    }
                    ui.add_space(4.0);
                    let new_status = self.status_groups().iter().any(|(u, items)| !u.is_empty() && items.iter().any(|m| !self.status_seen.contains(&m.id)));
                    let col = if self.show_status { pal.text } else { pal.dim };
                    let r = icon_button(ui, "◎", 20.0, col, pal, t!("Aktuelles: Status und Kanäle", "Updates: status and channels"));
                    if self.show_status {
                        ui.painter().circle_filled(r.rect.center(), r.rect.width() / 2.0, pal.selected);
                        ui.painter().text(r.rect.center(), egui::Align2::CENTER_CENTER, "◎", egui::FontId::proportional(20.0), pal.text);
                    }
                    if new_status {
                        ui.painter().circle_filled(r.rect.right_top() + egui::vec2(-8.0, 8.0), 4.5, pal.accent);
                    }
                    if r.clicked() {
                        self.show_status = !self.show_status;
                        self.show_starred = false;
                        self.show_settings = false;
                    }
                    ui.add_space(4.0);
                    let col = if self.show_starred { pal.text } else { pal.dim };
                    let r = icon_button(ui, "☆", 20.0, col, pal, t!("Markierte Nachrichten", "Starred messages"));
                    if self.show_starred {
                        ui.painter().circle_filled(r.rect.center(), r.rect.width() / 2.0, pal.selected);
                        ui.painter().text(r.rect.center(), egui::Align2::CENTER_CENTER, "★", egui::FontId::proportional(20.0), pal.text);
                    }
                    if r.clicked() {
                        self.show_starred = !self.show_starred;
                        self.show_settings = false;
                        self.show_status = false;
                    }
                });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    ui.add_space(12.0);
                    let col = if self.show_settings { pal.text } else { pal.dim };
                    let r = icon_button(ui, "⚙", 20.0, col, pal, t!("Einstellungen", "Settings"));
                    if self.show_settings {
                        ui.painter().circle_filled(r.rect.center(), r.rect.width() / 2.0, pal.selected);
                        ui.painter().text(r.rect.center(), egui::Align2::CENTER_CENTER, "⚙", egui::FontId::proportional(20.0), pal.text);
                    }
                    if r.clicked() {
                        self.show_settings = !self.show_settings;
                        self.show_status = false;
                    }
                    let icon = if self.settings.dark { "☀" } else { "🌙" };
                    if icon_button(ui, icon, 18.0, pal.dim, pal, t!("Hell/Dunkel", "Light/dark")).clicked() {
                        self.settings.dark = !self.settings.dark;
                        theme::apply(ctx, self.settings.dark, self.settings.font_size);
                        self.save_settings();
                    }
                });
            });
    }

    pub(crate) fn chat_list(&mut self, ctx: &egui::Context, pal: &Palette) {
        let mut ecache = std::mem::take(&mut self.emoji);
        let mut want_avatars: Vec<Jid> = Vec::new();
        let mut act: Option<Act> = None;
        egui::SidePanel::left("chats")
            .resizable(true)
            .default_width(400.0)
            .min_width(300.0)
            .max_width(560.0)
            .frame(egui::Frame::new().fill(pal.side).stroke(egui::Stroke::new(1.0_f32, pal.border)))
            .show(ctx, |ui| {
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    if self.show_archive {
                        if icon_button(ui, "⬅", 18.0, pal.text, pal, t!("Zurück", "Back")).clicked() {
                            self.show_archive = false;
                        }
                        ui.label(RichText::new(t!("Archiviert", "Archived")).size(19.0).family(theme::bold()));
                    } else {
                        ui.label(RichText::new(t!("Chats", "Chats")).size(22.0).family(theme::bold()));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(14.0);
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
                        let fill = if resp.hovered() { pal.accent.gamma_multiply(0.85) } else { pal.accent };
                        ui.painter().rect_filled(rect.shrink(2.0), 12.0, fill);
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "+", egui::FontId::new(24.0, theme::bold()), pal.on_accent);
                        let new_id = egui::Id::new("new_menu");
                        if resp.on_hover_text(t!("Neuer Chat oder neue Gruppe", "New chat or new group")).clicked() {
                            ui.memory_mut(|m| m.toggle_popup(new_id));
                        }
                        let anchor = ui.interact(rect, egui::Id::new("new_btn_anchor"), egui::Sense::hover());
                        egui::popup::popup_below_widget(ui, new_id, &anchor, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                            ui.set_min_width(200.0);
                            if ui.button(t!("💬  Neuer Chat (Nummer)", "💬  New chat (number)")).clicked() {
                                ui.memory_mut(|m| m.request_focus(egui::Id::new("list_search")));
                                self.info = Some(t!("Telefonnummer mit Ländervorwahl in die Suche tippen, z. B. +49 151 …", "Type a phone number with country code into the search, e.g. +49 151 …").into());
                            }
                            if ui.button(t!("👥  Neue Gruppe", "👥  New group")).clicked() {
                                self.new_group = Some((String::new(), HashSet::new(), String::new()));
                            }
                        });
                        let st = match self.conn {
                            Conn::Online => None,
                            Conn::LoggedOut => Some(("abgemeldet", pal.danger)),
                            _ => Some((t!("verbinde …", "connecting …"), Color32::from_rgb(0xd9, 0xa4, 0x2b))),
                        };
                        if let Some((t, c)) = st {
                            ui.label(RichText::new(t).small().color(c));
                        }
                    });
                });
                ui.add_space(10.0);
                // Suche
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    egui::Frame::new()
                        .fill(pal.input)
                        .corner_radius(20.0)
                        .inner_margin(egui::Margin::symmetric(14, 2))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("🔍").color(pal.dim));
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.search)
                                        .id(egui::Id::new("list_search"))
                                        .frame(false)
                                        .hint_text(t!("Suchen oder neuen Chat beginnen", "Search or start a new chat"))
                                        .margin(egui::vec2(4.0, 8.0))
                                        .desired_width(ui.available_width() - 30.0),
                                );
                            });
                        });
                });
                ui.add_space(10.0);
                if !self.show_archive {
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.spacing_mut().item_spacing.x = 8.0;
                        for (f, l) in [(Filter::All, t!("Alle", "All")), (Filter::Unread, t!("Ungelesen", "Unread")), (Filter::Groups, t!("Gruppen", "Groups"))] {
                            if chip(ui, l, self.filter == f, pal) {
                                self.filter = f;
                            }
                        }
                    });
                    ui.add_space(6.0);
                }

                let needle = self.search.to_lowercase();
                // Suche über alle Chats: passende Nachrichten
                if needle.chars().count() >= 2 {
                    let hits = self.global_hits(ui.ctx(), &needle);
                    if !hits.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(20.0);
                            let t = tf!("{} Nachrichten {}", "{} messages {}", hits.len(), if self.show_msg_results { "▾" } else { "▸" });
                            if ui.add(egui::Label::new(RichText::new(t).size(12.5).color(pal.accent)).sense(egui::Sense::click())).clicked() {
                                self.show_msg_results = !self.show_msg_results;
                            }
                        });
                        if self.show_msg_results {
                            let mut jump = None;
                            egui::ScrollArea::vertical().id_salt("msg_hits").max_height(ui.available_height() * 0.45).show(ui, |ui| {
                                for (ts, id, chat, text) in hits.into_iter().take(60) {
                                    let resp = ui
                                        .horizontal(|ui| {
                                            ui.add_space(20.0);
                                            ui.vertical(|ui| {
                                                ui.spacing_mut().item_spacing.y = 1.0;
                                                ui.horizontal(|ui| {
                                                    ui.label(RichText::new(&chat).size(13.5).family(theme::bold()));
                                                    ui.label(RichText::new(fmt_list_time(ts)).size(11.5).color(pal.dim));
                                                });
                                                let t: String = text.chars().take(90).collect();
                                                ui.add(egui::Label::new(RichText::new(t).size(13.0).color(pal.dim)).truncate());
                                            });
                                        })
                                        .response
                                        .interact(egui::Sense::click());
                                    if resp.hovered() {
                                        ui.painter().rect_filled(resp.rect, 6.0, pal.hover.gamma_multiply(0.6));
                                    }
                                    if resp.clicked() {
                                        jump = Some(id);
                                    }
                                    ui.add_space(4.0);
                                }
                            });
                            if let Some(id) = jump {
                                self.run_msg_act(MsgAct::Jump(id));
                            }
                            ui.separator();
                        }
                    }
                }
                let digits: String = self.search.chars().filter(|c| c.is_ascii_digit()).collect();
                let looks_like_number = self.search.chars().all(|c| c.is_ascii_digit() || c == '+' || c == ' ');
                if digits.len() >= 7 && looks_like_number {
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        if ui.button(tf!("Chat mit +{digits} starten", "Chat with +{digits}")).clicked() {
                            let jid = Jid::pn(digits.clone());
                            let key = jid.to_string();
                            let c = self.chat_entry(&jid, false);
                            c.name = format!("+{digits}");
                            self.select_chat(key);
                            self.search.clear();
                        }
                    });
                }

                // Archiviert-Zeile oben (wie bei WhatsApp)
                // Nur zählen, was die Archiv-Ansicht auch zeigt (keine leeren Einträge aus dem Adressbuch).
                let archived_n = self
                    .chats
                    .iter()
                    .filter(|(_, c)| c.jid.server != whatsapp_rust::Server::Broadcast && c.jid.server != whatsapp_rust::Server::Newsletter)
                    .filter(|(_, c)| !c.messages.is_empty() || c.last_ts > 0)
                    .filter(|(k, _)| Self::has(&self.settings.archived, k))
                    .count();
                if !self.show_archive && archived_n > 0 && needle.is_empty() {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 50.0), egui::Sense::click());
                    let inner = rect.shrink2(egui::vec2(8.0, 2.0));
                    if resp.hovered() {
                        ui.painter().rect_filled(inner, 10.0, pal.hover);
                    }
                    let p = ui.painter();
                    p.text(inner.left_center() + egui::vec2(30.0, 0.0), egui::Align2::CENTER_CENTER, "🗄", egui::FontId::proportional(17.0), pal.dim);
                    p.text(inner.left_center() + egui::vec2(70.0, 0.0), egui::Align2::LEFT_CENTER, t!("Archiviert", "Archived"), egui::FontId::proportional(15.5), pal.text);
                    p.text(inner.right_center() - egui::vec2(14.0, 0.0), egui::Align2::RIGHT_CENTER, archived_n.to_string(), egui::FontId::proportional(12.5), pal.dim);
                    if resp.clicked() {
                        self.show_archive = true;
                    }
                }

                let (pins, arch, show_archive, filter) =
                    (&self.settings.pinned, &self.settings.archived, self.show_archive, self.filter);
                let mut rows: Vec<(String, String, i64, bool)> = self
                    .chats
                    .iter()
                    .filter(|(_, c)| c.jid.server != whatsapp_rust::Server::Broadcast && c.jid.server != whatsapp_rust::Server::Newsletter)
                    .filter(|(k, _)| !needle.is_empty() || Self::has(arch, k) == show_archive)
                    .filter(|(_, c)| match filter {
                        _ if show_archive => true,
                        Filter::All => true,
                        Filter::Unread => c.unread > 0,
                        Filter::Groups => c.is_group,
                    })
                    .filter(|(_, c)| !c.messages.is_empty() || c.last_ts > 0 || !needle.is_empty())
                    .map(|(k, c)| (k.clone(), self.display_name(c), c.last_ts, Self::has(pins, k)))
                    .filter(|(_, n, _, _)| needle.is_empty() || n.to_lowercase().contains(&needle))
                    .collect();
                rows.sort_by_key(|r| (!r.3, std::cmp::Reverse(r.2)));
                self.row_keys = rows.iter().map(|r| r.0.clone()).collect();
                if rows.is_empty() {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        let t = match (self.filter, self.show_archive) {
                            (_, true) => t!("Keine archivierten Chats", "No archived chats"),
                            (Filter::Unread, _) => t!("Keine ungelesenen Chats", "No unread chats"),
                            (Filter::Groups, _) => t!("Keine Gruppen", "No groups"),
                            _ => t!("Noch keine Chats", "No chats yet"),
                        };
                        ui.label(RichText::new(t).color(pal.dim));
                    });
                }

                let mut clicked = None;
                let row_h = 72.0;
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show_rows(ui, row_h, rows.len(), |ui, range| {
                    for (key, name, ts, pinned) in &rows[range] {
                        let c = &self.chats[key];
                        let sel = self.selected.as_deref() == Some(key.as_str());
                        let muted = Self::has(&self.settings.muted, key);
                        let user = c.jid.user.to_string();
                        if !self.avatars.contains_key(&user) && !self.avatar_requested.contains(&user) {
                            want_avatars.push(c.jid.clone());
                        }
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), egui::Sense::click());
                        let inner_rect = rect.shrink2(egui::vec2(8.0, 2.0));
                        let bg = if sel {
                            pal.selected
                        } else if resp.hovered() {
                            pal.hover
                        } else {
                            Color32::TRANSPARENT
                        };
                        ui.painter().rect_filled(inner_rect, 10.0, bg);
                        let mut inner = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(inner_rect.shrink2(egui::vec2(12.0, 0.0)))
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        draw_avatar(&mut inner, self.avatars.get(&user), name, 49.0, self.settings.dark);
                        inner.add_space(14.0);
                        let w = inner.available_width();
                        let (txt_rect, _) = inner.allocate_exact_size(egui::vec2(w, 50.0), egui::Sense::hover());
                        let p = ui.painter().with_clip_rect(txt_rect);
                        // Zeile 1: Name links, Zeit rechts
                        let time = fmt_list_time(*ts);
                        let time_col = if c.unread > 0 && !muted { pal.accent } else { pal.dim };
                        let tg = p.layout_no_wrap(time, egui::FontId::proportional(12.0), time_col);
                        let name_w = txt_rect.width() - tg.size().x - 10.0;
                        let ng = p.layout(
                            name.clone(),
                            egui::FontId::proportional(16.0),
                            pal.text,
                            f32::INFINITY,
                        );
                        let name_clip = egui::Rect::from_min_size(txt_rect.min, egui::vec2(name_w.max(10.0), 24.0));
                        p.with_clip_rect(name_clip).galley(txt_rect.min + egui::vec2(0.0, 3.0), ng, pal.text);
                        p.galley(egui::pos2(txt_rect.right() - tg.size().x, txt_rect.top() + 6.0), tg, time_col);
                        // Zeile 2: Vorschau links, Symbole rechts
                        let mut icons_w = 0.0;
                        let mut x = txt_rect.right();
                        let y2 = txt_rect.top() + 36.0;
                        if c.unread > 0 {
                            let t = c.unread.min(999).to_string();
                            let bw = (t.len() as f32 * 7.0 + 12.0).max(20.0);
                            let r = egui::Rect::from_center_size(egui::pos2(x - bw / 2.0, y2), egui::vec2(bw, 20.0));
                            p.rect_filled(r, 10.0, if muted { pal.dim } else { pal.accent });
                            p.text(r.center(), egui::Align2::CENTER_CENTER, t, egui::FontId::new(11.5, theme::bold()), pal.on_accent);
                            x -= bw + 6.0;
                            icons_w += bw + 6.0;
                        }
                        if muted {
                            p.text(egui::pos2(x - 9.0, y2), egui::Align2::CENTER_CENTER, "🔇", egui::FontId::proportional(14.0), pal.dim);
                            x -= 22.0;
                            icons_w += 22.0;
                        }
                        if *pinned {
                            p.text(egui::pos2(x - 9.0, y2), egui::Align2::CENTER_CENTER, "📌", egui::FontId::proportional(14.0), pal.dim);
                            icons_w += 22.0;
                        }
                        let mut px = txt_rect.left();
                        let typing = self.typing.get(user.as_str()).filter(|t| t.2.elapsed().as_secs() < 12);
                        let draft = if sel { None } else { self.drafts.get(key).filter(|d| !d.trim().is_empty()) };
                        if let Some((from, rec, _)) = typing {
                            let what = if *rec { t!("nimmt Sprachnachricht auf …", "recording audio …") } else { t!("schreibt …", "typing …") };
                            let t = if c.is_group {
                                format!("{} {what}", self.names.get(from).map(|n| n.0.clone()).unwrap_or_else(|| format!("+{from}")))
                            } else {
                                what.to_owned()
                            };
                            p.text(egui::pos2(px, y2), egui::Align2::LEFT_CENTER, t, egui::FontId::proportional(14.0), pal.accent);
                        } else if let Some(d) = draft {
                            let g = p.layout_no_wrap(t!("Entwurf: ", "Draft: ").into(), egui::FontId::proportional(14.0), pal.accent);
                            let w = g.size().x;
                            p.galley(egui::pos2(px, y2 - g.size().y / 2.0), g, pal.accent);
                            let pclip = egui::Rect::from_min_max(egui::pos2(px, y2 - 10.0), egui::pos2(txt_rect.right() - icons_w - 6.0, y2 + 10.0));
                            paint_rich_line(&p.with_clip_rect(pclip), ui.ctx(), &mut ecache, egui::pos2(px + w, y2), &d.replace('\n', " "), egui::FontId::proportional(14.0), pal.dim);
                        } else if let Some(m) = c.messages.last() {
                            if m.from_me {
                                let mut tick_ui = ui.new_child(
                                    egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::pos2(px, y2 - 6.0), egui::vec2(18.0, 12.0))),
                                );
                                draw_ticks(&mut tick_ui, m.status, pal);
                                px += 20.0;
                            }
                            let prefix = if m.from_me {
                                ""
                            } else if c.is_group && !m.sender_name.is_empty() {
                                "~"
                            } else {
                                ""
                            };
                            let body = match (&m.media, &m.x.kind) {
                                _ if m.x.view_once => t!("⏱ Einmal-Ansicht", "⏱ View once").to_owned(),
                                (Some(md), _) if m.text.is_empty() => md.label(),
                                (Some(md), _) => format!("{} {}", md.label(), m.text.replace('\n', " ")),
                                (None, Some(k)) if m.text.is_empty() => k.label(),
                                _ => self.with_mention_names(&m.text, &m.x.mentions).replace(['\u{E000}', '\u{E001}'], "").replace('\n', " "),
                            };
                            let preview = if prefix == "~" { format!("{}: {}", m.sender_name, body) } else { body };
                            let pclip = egui::Rect::from_min_max(
                                egui::pos2(px, y2 - 10.0),
                                egui::pos2(txt_rect.right() - icons_w - 6.0, y2 + 10.0),
                            );
                            paint_rich_line(&p.with_clip_rect(pclip), ui.ctx(), &mut ecache, egui::pos2(px, y2), &preview, egui::FontId::proportional(14.0), pal.dim);
                        }
                        if resp.clicked() {
                            clicked = Some(key.clone());
                        }
                        let k = key.clone();
                        let (is_muted, is_arch) = (muted, Self::has(&self.settings.archived, key));
                        let unread = c.unread;
                        resp.context_menu(|ui| {
                            if ui.button(if is_arch { t!("Aus Archiv holen", "Unarchive") } else { t!("Chat archivieren", "Archive chat") }).clicked() {
                                act = Some(Act::Archive(k.clone()));
                                ui.close_menu();
                            }
                            if is_muted {
                                if ui.button(t!("Stummschaltung aufheben", "Unmute")).clicked() {
                                    act = Some(Act::Mute(k.clone(), None));
                                    ui.close_menu();
                                }
                            } else {
                                ui.menu_button(t!("Stummschalten", "Silence"), |ui| {
                                    for (l, d) in mute_opts().iter() {
                                        if ui.button(*l).clicked() {
                                            act = Some(Act::Mute(k.clone(), *d));
                                            ui.close_menu();
                                        }
                                    }
                                });
                            }
                            if ui.button(if *pinned { t!("Chat loslösen", "Unpin chat") } else { t!("Chat anheften", "Pin chat") }).clicked() {
                                act = Some(Act::Pin(k.clone()));
                                ui.close_menu();
                            }
                            if unread > 0 && ui.button(t!("Als gelesen markieren", "Mark as read")).clicked() {
                                act = Some(Act::Read(k.clone()));
                                ui.close_menu();
                            }
                        });
                    }
                });
                if let Some(k) = clicked {
                    self.select_chat(k);
                }
            });
        self.emoji = ecache;
        for jid in want_avatars.into_iter().take(12) {
            self.avatar_requested.insert(jid.user.to_string());
            let _ = self.cmd.send(Cmd::Avatar { jid });
        }
        if let Some(a) = act {
            self.run_act(a);
        }
    }
}
