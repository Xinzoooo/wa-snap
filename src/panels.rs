use super::*;

impl App {
    /// Bekannter Name zu einer Nummer/LID (Adressbuch, Push-Name oder Einzelchat).
    pub(crate) fn name_for_user(&self, u: &str) -> Option<String> {
        if u.is_empty() {
            return None;
        }
        self.names
            .get(u)
            .map(|n| n.0.clone())
            .or_else(|| self.chats.values().find(|c| !c.is_group && c.jid.user.as_str() == u).map(|c| self.display_name(c)).filter(|n| !n.starts_with('+')))
    }

    /// Seitenleiste mit Kontakt- oder Gruppeninfo, Medien und Verwaltung.
    pub(crate) fn info_panel(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(key) = self.selected.clone() else { return };
        let Some(chat) = self.chats.get(&key) else { return };
        let (jid, is_group, title) = (chat.jid.clone(), chat.is_group, self.display_name(chat));
        let user = jid.user.to_string();
        if is_group && self.conn == Conn::Online && self.group_requested.insert(user.clone()) {
            let _ = self.cmd.send(Cmd::GroupInfo(jid.clone()));
        }
        // Medien des Chats (neueste zuerst)
        let pics: Vec<(String, backend::MediaRef)> = chat
            .messages
            .iter()
            .rev()
            .filter_map(|m| m.media.as_ref().filter(|md| md.kind == "image").map(|md| (m.id.clone(), md.clone())))
            .take(30)
            .collect();
        let docs = chat.messages.iter().filter(|m| m.media.as_ref().map(|md| md.kind == "document").unwrap_or(false)).count();
        let links = chat.messages.iter().filter(|m| matches!(m.x.kind, Some(backend::Extra::Link { .. })) || m.text.contains("http")).count();
        let mut cmds: Vec<Cmd> = Vec::new();
        let mut jump: Option<String> = None;
        let mut close = false;
        let mut want: Vec<(String, backend::MediaRef)> = Vec::new();
        egui::SidePanel::right("info")
            .resizable(false)
            .exact_width(340.0)
            .frame(egui::Frame::new().fill(pal.side).stroke(egui::Stroke::new(1.0_f32, pal.border)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        if icon_button(ui, "×", 18.0, pal.dim, pal, t!("Schließen", "Close")).clicked() {
                            close = true;
                        }
                        ui.label(RichText::new(if is_group { t!("Gruppeninfo", "Group info") } else { t!("Kontaktinfo", "Contact info") }).size(16.0).family(theme::bold()));
                    });
                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| {
                        draw_avatar(ui, self.avatars.get(&user), &title, 140.0, self.settings.dark);
                        ui.add_space(8.0);
                        ui.label(RichText::new(&title).size(20.0).family(theme::bold()));
                        let gi = self.group_info.get(&user);
                        let sub = if is_group {
                            tf!("Gruppe · {} Mitglieder", "Group · {} members", gi.map(|g| g.members.len()).unwrap_or(0))
                        } else if jid.server == whatsapp_rust::Server::Pn {
                            format!("+{user}")
                        } else {
                            String::new()
                        };
                        ui.label(RichText::new(sub).color(pal.dim));
                    });
                    ui.add_space(10.0);
                    let section = |ui: &mut egui::Ui, t: &str| {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            ui.label(RichText::new(t).size(13.0).color(pal.dim));
                        });
                    };
                    if let Some(g) = self.group_info.get(&user).cloned() {
                        if !g.description.is_empty() {
                            section(ui, t!("Beschreibung", "Description"));
                            ui.horizontal(|ui| {
                                ui.add_space(16.0);
                                ui.label(&g.description);
                            });
                        }
                    }
                    section(ui, &tf!("Medien, Links und Doks  ·  {} Fotos · {} Links · {} Doks", "Media, links and docs  ·  {} photos · {} links · {} docs", pics.len(), links, docs));
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(12.0);
                        for (id, md) in pics.iter().take(9) {
                            match self.tex(id) {
                                Some((t, _)) => {
                                    if ui.add(egui::Button::image(egui::Image::new((t.id(), egui::vec2(96.0, 96.0))).corner_radius(6.0)).frame(false)).clicked() {
                                        jump = Some(id.clone());
                                    }
                                }
                                None => {
                                    ui.add_sized([96.0, 96.0], egui::Spinner::new());
                                    want.push((id.clone(), md.clone()));
                                }
                            }
                        }
                    });
                    if is_group {
                        let gi = self.group_info.get(&user).cloned();
                        let admin = gi.as_ref().map(|g| g.i_am_admin).unwrap_or(false);
                        if admin {
                            section(ui, t!("Gruppenname", "Group name"));
                            ui.horizontal(|ui| {
                                ui.add_space(16.0);
                                if self.rename_buf.is_empty() {
                                    self.rename_buf = title.clone();
                                }
                                ui.add(egui::TextEdit::singleline(&mut self.rename_buf).desired_width(200.0));
                                if ui.button(t!("Ändern", "Change")).clicked() && !self.rename_buf.trim().is_empty() {
                                    cmds.push(Cmd::GroupRename { jid: jid.clone(), name: self.rename_buf.trim().to_owned() });
                                }
                            });
                        }
                        section(ui, &tf!("{} Mitglieder", "{} members", gi.as_ref().map(|g| g.members.len()).unwrap_or(0)));
                        if admin {
                            ui.horizontal(|ui| {
                                ui.add_space(16.0);
                                ui.add(egui::TextEdit::singleline(&mut self.add_numbers).hint_text(t!("Nummern, z. B. +49151…", "Numbers, e.g. +49151…")).desired_width(180.0));
                                if ui.button(t!("Hinzufügen", "Add")).clicked() {
                                    let nums: Vec<String> = self
                                        .add_numbers
                                        .split([',', ';', ' '])
                                        .map(|n| n.chars().filter(|c| c.is_ascii_digit()).collect::<String>())
                                        .filter(|n| n.len() >= 7)
                                        .collect();
                                    if !nums.is_empty() {
                                        cmds.push(Cmd::GroupAdd { jid: jid.clone(), numbers: nums });
                                        self.add_numbers.clear();
                                        self.group_requested.remove(&user);
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.add_space(16.0);
                                if ui.button(t!("🔗 Einladungslink kopieren", "🔗 Copy invite link")).clicked() {
                                    cmds.push(Cmd::GroupInvite(jid.clone()));
                                }
                            });
                        }
                        if let Some(g) = &gi {
                            let mut list = g.members.clone();
                            list.sort_by_key(|(_, _, a, s)| (!s, !a));
                            for (mj, pn, is_admin, owner) in list {
                                let mu = user_of(&mj);
                                let me = self.me.contains(&mu) || self.me.contains(&pn);
                                let name = if me {
                                    t!("Du", "You").to_owned()
                                } else {
                                    self.name_for_user(&pn).or_else(|| self.name_for_user(&mu)).unwrap_or_else(|| if pn.is_empty() { mu.clone() } else { format!("+{pn}") })
                                };
                                let resp = ui
                                    .horizontal(|ui| {
                                        ui.add_space(16.0);
                                        draw_avatar(ui, self.avatars.get(&pn).or_else(|| self.avatars.get(&mu)), &name, 34.0, self.settings.dark);
                                        ui.label(RichText::new(&name).size(14.5));
                                        if owner || is_admin {
                                            ui.label(RichText::new(if owner { t!("Ersteller", "Creator") } else { t!("Admin", "Admin") }).size(11.0).color(pal.accent));
                                        }
                                    })
                                    .response
                                    .interact(egui::Sense::click());
                                if !me {
                                    resp.context_menu(|ui| {
                                        if !pn.is_empty() && ui.button(t!("Nachricht senden", "Send message")).clicked() {
                                            jump = Some(format!("chat:{pn}"));
                                            ui.close_menu();
                                        }
                                        if admin {
                                            if ui.button(if is_admin { t!("Admin-Rechte entziehen", "Dismiss as admin") } else { t!("Zum Admin machen", "Make group admin") }).clicked() {
                                                cmds.push(Cmd::GroupAdmin { jid: jid.clone(), member: mj.clone(), on: !is_admin });
                                                ui.close_menu();
                                            }
                                            if ui.button(RichText::new(t!("Aus der Gruppe entfernen", "Remove from group")).color(pal.danger)).clicked() {
                                                cmds.push(Cmd::GroupRemove { jid: jid.clone(), member: mj.clone() });
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                }
                            }
                        } else {
                            ui.horizontal(|ui| {
                                ui.add_space(16.0);
                                ui.spinner();
                            });
                        }
                        ui.add_space(8.0);
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            if ui.button(RichText::new(t!("Gruppe verlassen", "Exit group")).color(pal.danger)).clicked() {
                                cmds.push(Cmd::GroupLeave(jid.clone()));
                            }
                        });
                    } else {
                        // Gemeinsame Gruppen
                        let common: Vec<String> = self
                            .members
                            .iter()
                            .filter(|(_, ms)| ms.iter().any(|(u, pn)| *u == user || *pn == user))
                            .filter_map(|(g, _)| self.chats.values().find(|c| c.is_group && c.jid.user.as_str() == g).map(|c| self.display_name(c)))
                            .collect();
                        section(ui, &tf!("{} gemeinsame Gruppen", "{} groups in common", common.len()));
                        for g in common {
                            ui.horizontal(|ui| {
                                ui.add_space(16.0);
                                ui.label(g);
                            });
                        }
                        ui.add_space(8.0);
                        ui.separator();
                        let blocked = self.blocklist.iter().any(|b| user_of(b) == user);
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            if ui.button(RichText::new(if blocked { t!("Blockierung aufheben", "Unblock").to_owned() } else { tf!("{title} blockieren", "Block {title}") }).color(pal.danger)).clicked() {
                                cmds.push(Cmd::Block { jid: jid.clone(), on: !blocked });
                            }
                        });
                    }
                    ui.add_space(20.0);
                });
            });
        for c in cmds {
            let _ = self.cmd.send(c);
        }
        for (id, md) in want {
            if self.img_requested.insert(id.clone()) {
                let _ = self.cmd.send(Cmd::LoadMedia { id, media: md });
            }
        }
        if close {
            self.show_info = false;
        }
        if let Some(j) = jump {
            if let Some(n) = j.strip_prefix("chat:") {
                self.run_msg_act(MsgAct::OpenNumber(n.to_owned()));
            } else {
                self.run_msg_act(MsgAct::Jump(j));
            }
        }
    }

    /// Offenen Chat als Textdatei in ~/Downloads (oder ~) speichern, wie der WhatsApp-Export.
    pub(crate) fn export_current(&mut self) {
        let Some(key) = self.selected.clone() else { return };
        let Some(c) = self.chats.get(&key) else { return };
        let title = self.display_name(c);
        let mut out = String::new();
        for m in &c.messages {
            let dt = chrono::DateTime::from_timestamp(m.ts, 0).map(|d| d.with_timezone(&chrono::Local).format("%d.%m.%y, %H:%M").to_string()).unwrap_or_default();
            let who = if m.from_me { t!("Ich", "Me").to_owned() } else if c.is_group { m.sender_name.clone() } else { title.clone() };
            let body = match (&m.media, &m.x.kind) {
                (Some(md), _) if m.text.is_empty() => md.label(),
                (Some(md), _) => format!("{} {}", md.label(), m.text),
                (None, Some(k)) if m.text.is_empty() => k.label(),
                _ => m.text.clone(),
            };
            out.push_str(&format!("{dt} - {who}: {body}\n"));
        }
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
        let dir = ["Downloads", "Download"].iter().map(|d| home.join(d)).find(|p| p.is_dir()).unwrap_or(home);
        let safe: String = title.chars().map(|ch| if ch.is_alphanumeric() || ch == ' ' || ch == '-' { ch } else { '_' }).collect();
        let path = dir.join(tf!("WhatsSnap-Chat mit {}.txt", "WhatsSnap chat with {}.txt", safe.trim()));
        match std::fs::write(&path, out) {
            Ok(()) => self.info = Some(tf!("Exportiert: {}", "Exported: {}", path.display())),
            Err(e) => self.info = Some(tf!("Export fehlgeschlagen: {e}", "Export failed: {e}")),
        }
    }

    /// Status der letzten 24 Stunden, gruppiert nach Person ("" = ich), neueste Person zuerst.
    pub(crate) fn status_groups(&self) -> Vec<(String, Vec<ChatMessage>)> {
        let Some(c) = self.chats.get("status@broadcast") else {
            return Vec::new();
        };
        // Wird in jedem Bild gebraucht (Punkt in der Seitenleiste): nur neu gruppieren, wenn ein
        // Status dazukam oder eine Minute um ist (dann fallen alte heraus).
        let stamp = (c.messages.len(), now_ts() / 60);
        if let Some((n, minute, v)) = self.status_cache.borrow().as_ref() {
            if (*n, *minute) == stamp {
                return v.clone();
            }
        }
        let v = self.status_groups_now(c);
        *self.status_cache.borrow_mut() = Some((stamp.0, stamp.1, v.clone()));
        v
    }

    pub(crate) fn status_groups_now(&self, c: &Chat) -> Vec<(String, Vec<ChatMessage>)> {
        let since = now_ts() - 86_400;
        let mut map: HashMap<String, Vec<ChatMessage>> = HashMap::new();
        for m in c.messages.iter().filter(|m| m.ts >= since) {
            let who = if m.from_me { String::new() } else { user_of(&m.sender) };
            map.entry(who).or_default().push(m.clone());
        }
        let mut v: Vec<(String, Vec<ChatMessage>)> = map.into_iter().collect();
        v.sort_by_key(|(_, items)| std::cmp::Reverse(items.last().map(|m| m.ts).unwrap_or(0)));
        v
    }

    /// Status-Bereich: links Personen, rechts Anzeige; eigenen Status posten.
    pub(crate) fn status_view(&mut self, ctx: &egui::Context, pal: &Palette) {
        const COLORS: [(u32, Color32); 6] = [
            (0xFF00A884, Color32::from_rgb(0x00, 0xa8, 0x84)),
            (0xFF5696FF, Color32::from_rgb(0x56, 0x96, 0xff)),
            (0xFFFF7A6B, Color32::from_rgb(0xff, 0x7a, 0x6b)),
            (0xFF8294CA, Color32::from_rgb(0x82, 0x94, 0xca)),
            (0xFFC69FCC, Color32::from_rgb(0xc6, 0x9f, 0xcc)),
            (0xFF243640, Color32::from_rgb(0x24, 0x36, 0x40)),
        ];
        let groups = self.status_groups();
        let mut select: Option<(String, usize)> = None;
        let mut post: Option<Cmd> = None;
        egui::SidePanel::left("status_list")
            .resizable(false)
            .exact_width(360.0)
            .frame(egui::Frame::new().fill(pal.side).stroke(egui::Stroke::new(1.0_f32, pal.border)))
            .show(ctx, |ui| {
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.label(RichText::new(t!("Aktuelles", "Updates")).size(22.0).family(theme::bold()));
                });
                ui.add_space(10.0);
                // eigener Status
                egui::Frame::new().inner_margin(egui::Margin::symmetric(20, 6)).show(ui, |ui| {
                    ui.label(RichText::new(t!("Mein Status", "My status")).size(13.0).color(pal.dim));
                    ui.add(egui::TextEdit::multiline(&mut self.status_text).hint_text(t!("Text-Status schreiben …", "Write a text status …")).desired_rows(2).desired_width(f32::INFINITY));
                    ui.horizontal(|ui| {
                        for (i, (_, c)) in COLORS.iter().enumerate() {
                            let (r, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                            ui.painter().circle_filled(r.center(), 10.0, *c);
                            if self.status_color == i {
                                ui.painter().circle_stroke(r.center(), 11.0, egui::Stroke::new(2.0_f32, pal.text));
                            }
                            if resp.clicked() {
                                self.status_color = i;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        let recipients = self.status_recipients();
                        let ok = self.conn == Conn::Online;
                        if ui.add_enabled(ok && !self.status_text.trim().is_empty(), egui::Button::new(t!("Text posten", "Post text"))).clicked() {
                            post = Some(Cmd::PostTextStatus { text: self.status_text.trim().to_owned(), argb: COLORS[self.status_color].0, recipients: recipients.clone() });
                            self.status_text.clear();
                        }
                        if ui.add_enabled(ok, egui::Button::new(t!("📷 Foto posten …", "📷 Post photo …"))).clicked() {
                            post = Some(Cmd::PostPhotoStatus { caption: std::mem::take(&mut self.status_text).trim().to_owned(), recipients });
                        }
                    });
                    ui.label(RichText::new(tf!("Geht an {} Kontakte mit Einzelchat.", "Goes to {} contacts you have a chat with.", self.status_recipients().len())).size(11.0).color(pal.dim));
                });
                ui.separator();
                if groups.is_empty() {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new(t!("Keine Status-Updates in den letzten 24 Stunden.", "No status updates in the last 24 hours.")).color(pal.dim)));
                }
                if !self.channels_loaded && self.conn == Conn::Online {
                    self.channels_loaded = true;
                    let _ = self.cmd.send(Cmd::FetchChannels);
                }
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                    // Kanäle (dieselben Abos wie am Handy)
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.label(RichText::new(t!("Kanäle", "Channels")).size(13.0).color(pal.dim));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(12.0);
                            if ui.small_button(t!("+ Erstellen", "+ Create")).clicked() {
                                self.channel_new = Some((String::new(), String::new()));
                            }
                            if ui.small_button(if self.news_tab == 1 { t!("Fertig", "Done") } else { t!("🔍 Finden", "🔍 Find") }).clicked() {
                                self.news_tab = if self.news_tab == 1 { 0 } else { 1 };
                            }
                        });
                    });
                    if self.news_tab == 1 {
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(20, 4)).show(ui, |ui| {
                            let r = ui.add(egui::TextEdit::singleline(&mut self.channel_search).hint_text(t!("Kanäle suchen (Enter)", "Search channels (Enter)")).desired_width(f32::INFINITY));
                            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.channel_search.trim().is_empty() {
                                let _ = self.cmd.send(Cmd::ChannelSearch(self.channel_search.trim().to_owned()));
                            }
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut self.channel_link).hint_text(t!("oder Kanal-Link einfügen", "or paste a channel link")).desired_width(220.0));
                                if ui.button(t!("Abonnieren", "Follow")).clicked() && self.channel_link.contains("channel") {
                                    let _ = self.cmd.send(Cmd::ChannelByLink(std::mem::take(&mut self.channel_link)));
                                }
                            });
                            for c in self.channel_results.clone() {
                                let followed = self.channels.iter().any(|x| x.jid == c.jid);
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(format!("📢 {}", c.name)).size(14.0));
                                        ui.label(RichText::new(tf!("{} Abonnenten", "{} followers", fmt_count(c.subscribers))).size(11.5).color(pal.dim));
                                    });
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if !followed && ui.small_button(t!("Abonnieren", "Follow")).clicked() {
                                            if let Ok(j) = c.jid.parse::<Jid>() {
                                                let _ = self.cmd.send(Cmd::ChannelFollow { jid: j, on: true });
                                            }
                                        }
                                        if followed {
                                            ui.label(RichText::new(t!("✓ abonniert", "✓ following")).size(11.5).color(pal.accent));
                                        }
                                    });
                                });
                            }
                        });
                    }
                    if self.channels.is_empty() && self.news_tab == 0 {
                        ui.horizontal(|ui| {
                            ui.add_space(20.0);
                            ui.label(RichText::new(if self.channels_loaded { t!("Keine Kanäle abonniert.", "Not following any channels.") } else { t!("Kanäle werden geladen …", "Loading channels …") }).size(12.5).color(pal.dim));
                        });
                    }
                    for c in self.channels.clone() {
                        let sel = self.channel_sel.as_deref() == Some(c.jid.as_str());
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.0), egui::Sense::click());
                        let inner = rect.shrink2(egui::vec2(8.0, 2.0));
                        if sel || resp.hovered() {
                            ui.painter().rect_filled(inner, 10.0, if sel { pal.selected } else { pal.hover });
                        }
                        ui.painter().text(inner.left_center() + egui::vec2(16.0, -8.0), egui::Align2::LEFT_CENTER, format!("📢 {}", c.name), egui::FontId::proportional(15.0), pal.text);
                        let sub = if c.admin { tf!("Dein Kanal · {} Abonnenten", "Your channel · {} followers", fmt_count(c.subscribers)) } else { tf!("{} Abonnenten", "{} followers", fmt_count(c.subscribers)) };
                        ui.painter().text(inner.left_center() + egui::vec2(16.0, 11.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(12.0), pal.dim);
                        if resp.clicked() {
                            self.channel_sel = Some(c.jid.clone());
                            self.status_sel = None;
                            self.channel_refresh = None;
                        }
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.label(RichText::new(t!("Status", "Status")).size(13.0).color(pal.dim));
                    });
                    for (who, items) in &groups {
                        let name = if who.is_empty() { t!("Mein Status", "My status").to_owned() } else { self.name_for_user(who).unwrap_or_else(|| format!("+{who}")) };
                        let unseen = items.iter().any(|m| !self.status_seen.contains(&m.id));
                        let sel = self.status_sel.as_ref().map(|s| &s.0 == who).unwrap_or(false);
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 66.0), egui::Sense::click());
                        let inner = rect.shrink2(egui::vec2(8.0, 2.0));
                        if sel || resp.hovered() {
                            ui.painter().rect_filled(inner, 10.0, if sel { pal.selected } else { pal.hover });
                        }
                        let c = inner.left_center() + egui::vec2(34.0, 0.0);
                        ui.painter().circle_stroke(c, 25.0, egui::Stroke::new(2.5_f32, if unseen && !who.is_empty() { pal.accent } else { pal.dim.gamma_multiply(0.5) }));
                        let mut av = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_center_size(c, egui::vec2(44.0, 44.0))));
                        draw_avatar(&mut av, self.avatars.get(who), &name, 44.0, self.settings.dark);
                        let p = ui.painter();
                        p.text(inner.left_center() + egui::vec2(72.0, -9.0), egui::Align2::LEFT_CENTER, &name, egui::FontId::proportional(15.5), pal.text);
                        let last = items.last().map(|m| fmt_list_time(m.ts)).unwrap_or_default();
                        p.text(inner.left_center() + egui::vec2(72.0, 11.0), egui::Align2::LEFT_CENTER, format!("{last} · {}", items.len()), egui::FontId::proportional(12.5), pal.dim);
                        if resp.clicked() {
                            let first_unseen = items.iter().position(|m| !self.status_seen.contains(&m.id)).unwrap_or(0);
                            select = Some((who.clone(), first_unseen));
                            self.channel_sel = None;
                        }
                    }
                });
            });
        if let Some(c) = post {
            let _ = self.cmd.send(c);
        }
        if let Some(s) = select {
            self.status_sel = Some(s);
        }
        let mut go: i32 = 0;
        let mut open_chat: Option<String> = None;
        let mut want: Option<(String, backend::MediaRef)> = None;
        let mut mark: Option<(String, String)> = None;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::from_rgb(0x10, 0x10, 0x10))).show(ctx, |ui| {
            if let Some(ch) = self.channel_sel.clone() {
                // alle 60 Sekunden neu laden, solange der Kanal offen ist
                if self.channel_refresh.map(|t| t.elapsed().as_secs() >= 60).unwrap_or(true) && self.conn == Conn::Online {
                    self.channel_refresh = Some(std::time::Instant::now());
                    if let Ok(j) = ch.parse::<Jid>() {
                        let _ = self.cmd.send(Cmd::FetchChannelPosts(j));
                    }
                }
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(5));
                let info = self.channels.iter().find(|c| c.jid == ch).cloned().unwrap_or_default();
                let jid = ch.parse::<Jid>().ok();
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(60.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(format!("📢 {}", info.name)).size(18.0).color(Color32::WHITE).family(theme::bold()));
                        ui.label(RichText::new(tf!("{} Abonnenten", "{} followers", fmt_count(info.subscribers))).color(Color32::from_gray(160)));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(60.0);
                        if !info.admin && ui.button(t!("Deabonnieren", "Unfollow")).clicked() {
                            if let Some(j) = jid.clone() {
                                let _ = self.cmd.send(Cmd::ChannelFollow { jid: j, on: false });
                                self.channel_sel = None;
                            }
                        }
                    });
                });
                if !info.description.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(60.0);
                        ui.label(RichText::new(&info.description).color(Color32::from_gray(180)).size(13.0));
                    });
                }
                ui.add_space(8.0);
                let post_h = if info.admin { 70.0 } else { 0.0 };
                let mut react: Option<(u64, String)> = None;
                match self.channel_posts.get(&ch) {
                    None => {
                        ui.vertical_centered(|ui| ui.spinner());
                    }
                    Some(posts) => {
                        egui::ScrollArea::vertical().auto_shrink([false; 2]).max_height(ui.available_height() - post_h).stick_to_bottom(true).show(ui, |ui| {
                            for p in posts {
                                egui::Frame::new()
                                    .fill(Color32::from_rgb(0x24, 0x26, 0x26))
                                    .corner_radius(8.0)
                                    .inner_margin(egui::Margin::symmetric(12, 8))
                                    .outer_margin(egui::Margin::symmetric(60, 4))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width().min(640.0));
                                        ui.style_mut().visuals.override_text_color = Some(Color32::WHITE);
                                        if !p.media.is_empty() {
                                            ui.label(RichText::new(&p.media).color(Color32::from_gray(170)));
                                        }
                                        if !p.text.is_empty() {
                                            ui.horizontal_wrapped(|ui| rich_label(ui, &p.text, 14.5, &mut self.emoji));
                                        }
                                        ui.horizontal(|ui| {
                                            for (e, n) in &p.reactions {
                                                if let Some(id) = self.emoji.get(ui.ctx(), &emoji::key(e)) {
                                                    ui.add(egui::Image::new((id, egui::vec2(16.0, 16.0))));
                                                }
                                                ui.label(RichText::new(fmt_count(*n)).size(11.5).color(Color32::from_gray(170)));
                                            }
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                ui.label(RichText::new(format!("{} {}", day_label(p.ts), fmt_time(p.ts))).size(11.0).color(Color32::from_gray(150)));
                                                if !p.mine {
                                                    ui.menu_button(RichText::new("☺+").size(12.0), |ui| {
                                                        ui.horizontal(|ui| {
                                                            for e in ["👍", "❤️", "😂", "😮", "😢", "🙏"] {
                                                                let clicked = match self.emoji.get(ui.ctx(), &emoji::key(e)) {
                                                                    Some(id) => ui.add(egui::Button::image(egui::Image::new((id, egui::vec2(24.0, 24.0)))).frame(false)).clicked(),
                                                                    None => ui.button(e).clicked(),
                                                                };
                                                                if clicked {
                                                                    react = Some((p.server_id, e.to_owned()));
                                                                    ui.close_menu();
                                                                }
                                                            }
                                                        });
                                                    });
                                                }
                                            });
                                        });
                                    });
                            }
                            if posts.is_empty() {
                                ui.vertical_centered(|ui| ui.label(RichText::new(t!("Noch keine Beiträge.", "No posts yet.")).color(Color32::from_gray(160))));
                            }
                        });
                    }
                }
                if let (Some((sid, e)), Some(j)) = (react, jid.clone()) {
                    let _ = self.cmd.send(Cmd::ChannelReact { jid: j, server_id: sid, emoji: e });
                    self.channel_refresh = Some(std::time::Instant::now() - std::time::Duration::from_secs(57));
                }
                if info.admin {
                    ui.horizontal(|ui| {
                        ui.add_space(60.0);
                        ui.add(egui::TextEdit::multiline(&mut self.channel_post_text).hint_text(t!("Beitrag für deinen Kanal …", "Post to your channel …")).desired_rows(2).desired_width(ui.available_width() - 200.0));
                        if ui.add_enabled(!self.channel_post_text.trim().is_empty(), egui::Button::new(t!("Veröffentlichen", "Publish"))).clicked() {
                            if let Some(j) = jid.clone() {
                                let _ = self.cmd.send(Cmd::ChannelPost { jid: j, text: std::mem::take(&mut self.channel_post_text).trim().to_owned() });
                            }
                        }
                    });
                }
                return;
            }
            let Some((who, idx)) = self.status_sel.clone() else {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.4);
                    ui.label(RichText::new(t!("Klicke auf einen Kontakt, um seinen Status anzusehen.", "Click a contact to view their status.")).color(Color32::from_gray(170)));
                });
                return;
            };
            let Some((_, items)) = groups.iter().find(|(w, _)| *w == who) else { return };
            let idx = idx.min(items.len().saturating_sub(1));
            let m = &items[idx];
            if !self.status_seen.contains(&m.id) && !who.is_empty() {
                mark = Some((m.id.clone(), m.sender.clone()));
            }
            let name = if who.is_empty() { t!("Mein Status", "My status").to_owned() } else { self.name_for_user(&who).unwrap_or_else(|| format!("+{who}")) };
            // Fortschrittsbalken
            let full = ui.available_rect_before_wrap();
            let bar_y = full.top() + 12.0;
            let n = items.len().max(1) as f32;
            let w = (full.width() * 0.6) / n;
            let x0 = full.center().x - full.width() * 0.3;
            for i in 0..items.len() {
                let r = egui::Rect::from_min_size(egui::pos2(x0 + i as f32 * w + 2.0, bar_y), egui::vec2(w - 4.0, 3.0));
                ui.painter().rect_filled(r, 2.0, if i <= idx { Color32::WHITE } else { Color32::from_gray(90) });
            }
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(format!("{name} · {} {}", day_label(m.ts), fmt_time(m.ts))).color(Color32::WHITE).size(14.0));
            });
            ui.add_space(10.0);
            let area = ui.available_rect_before_wrap().shrink2(egui::vec2(80.0, 60.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Center)));
            if let Some(md) = &m.media {
                if md.is_picture() {
                    match self.tex(&m.id) {
                        Some((t, _)) => {
                            let sz = t.size_vec2();
                            let k = (area.width() / sz.x).min((area.height() - 60.0) / sz.y).min(3.0);
                            child.add(egui::Image::new((t.id(), sz * k)).corner_radius(8.0));
                        }
                        None => {
                            child.spinner();
                            if !self.img_requested.contains(&m.id) {
                                want = Some((m.id.clone(), md.clone()));
                            }
                        }
                    }
                } else {
                    child.label(RichText::new(md.label()).size(20.0).color(Color32::WHITE));
                    if child.button(t!("Öffnen", "Open")).clicked() {
                        let _ = self.cmd.send(Cmd::OpenMedia { id: m.id.clone(), media: md.clone() });
                    }
                }
                if !m.text.is_empty() {
                    child.add_space(8.0);
                    child.label(RichText::new(&m.text).size(16.0).color(Color32::WHITE));
                }
            } else {
                let bg = if m.x.bg != 0 {
                    let c = m.x.bg;
                    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
                } else {
                    Color32::from_rgb(0x24, 0x36, 0x40)
                };
                child.painter().rect_filled(area, 12.0, bg);
                child.add_space(area.height() * 0.35);
                child.style_mut().visuals.override_text_color = Some(Color32::WHITE);
                child.horizontal_wrapped(|ui| rich_label(ui, &m.text, 28.0, &mut self.emoji));
            }
            // Bedienung unten
            let bottom = egui::Rect::from_min_size(egui::pos2(full.left(), full.bottom() - 50.0), egui::vec2(full.width(), 40.0));
            let mut b = ui.new_child(egui::UiBuilder::new().max_rect(bottom).layout(egui::Layout::left_to_right(egui::Align::Center)));
            b.add_space(full.width() / 2.0 - 170.0);
            if b.add_enabled(idx > 0, egui::Button::new(t!("◀ Zurück", "◀ Back"))).clicked() {
                go = -1;
            }
            if !who.is_empty() && b.button(t!("Antworten im Chat", "Reply in chat")).clicked() {
                open_chat = Some(who.clone());
            }
            if b.button(t!("Weiter ▶", "Next ▶")).clicked() {
                go = 1;
            }
            let keys = ui.input(|i| (i.key_pressed(egui::Key::ArrowRight), i.key_pressed(egui::Key::ArrowLeft)));
            if keys.0 {
                go = 1;
            }
            if keys.1 && idx > 0 {
                go = -1;
            }
        });
        if let Some((id, sender)) = mark {
            self.status_seen.insert(id.clone());
            if self.conn == Conn::Online {
                let _ = self.cmd.send(Cmd::MarkRead { jid: Jid::new("status", whatsapp_rust::Server::Broadcast), msgs: vec![(id, sender)], last: None });
            }
        }
        if let Some((id, md)) = want {
            self.img_requested.insert(id.clone());
            let _ = self.cmd.send(Cmd::LoadMedia { id, media: md });
        }
        if go != 0 {
            if let Some((who, idx)) = self.status_sel.clone() {
                let len = groups.iter().find(|(w, _)| *w == who).map(|g| g.1.len()).unwrap_or(0);
                let ni = idx as i32 + go;
                if ni >= 0 && (ni as usize) < len {
                    self.status_sel = Some((who, ni as usize));
                } else if ni as usize >= len {
                    // nächste Person mit ungesehenem Status
                    let next = groups.iter().find(|(w, items)| *w != who && !w.is_empty() && items.iter().any(|m| !self.status_seen.contains(&m.id)));
                    self.status_sel = next.map(|(w, _)| (w.clone(), 0));
                }
            }
        }
        if let Some(u) = open_chat {
            self.show_status = false;
            self.run_msg_act(MsgAct::OpenNumber(u));
        }
    }
}
