use super::*;

impl App {
    pub(crate) fn pump(&mut self, ctx: &egui::Context) {
        if let Some(e) = self.store.take_error() {
            backend::log(format!("Verlauf speichern: {e}"));
            self.info = Some(tf!("Verlauf konnte nicht gespeichert werden: {e}", "Could not save history: {e}"));
        }
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                UiEvent::Status(s) => self.status = s,
                UiEvent::Qr(code) => {
                    self.qr = Some(code);
                    self.conn = Conn::NeedsQr;
                }
                UiEvent::Connected => {
                    self.conn = Conn::Online;
                    self.qr = None;
                    let _ = self.cmd.send(Cmd::FetchBlocklist);
                    // Für alle Einzelchats nachschlagen, ob LID und Nummer zur selben Person gehören.
                    let singles: Vec<Jid> = self.chats.values().filter(|c| !c.is_group && (c.jid.is_lid() || c.jid.is_pn())).map(|c| c.jid.clone()).collect();
                    if !singles.is_empty() {
                        let _ = self.cmd.send(Cmd::ResolveAliases(singles));
                    }
                    if !self.settings.show_online {
                        let _ = self.cmd.send(Cmd::SetOnline(false));
                    }
                }
                UiEvent::LoggedOut => {
                    // Egal ob hier oder am Handy abgemeldet: die alte Sitzung ist ungültig und
                    // muss beim nächsten Start weg, sonst kommt nie ein neuer QR-Code.
                    if let Err(e) = dialogs::mark_logged_out(&data_dir()) {
                        backend::log(format!("Abmelde-Merker nicht geschrieben: {e}"));
                    }
                    self.conn = Conn::LoggedOut;
                }
                UiEvent::LogoutDone => {
                    if let Err(e) = dialogs::mark_logged_out(&data_dir()) {
                        backend::log(format!("Abmelde-Merker nicht geschrieben: {e}"));
                    }
                    self.logout_prompt = true;
                }
                UiEvent::OpenBlocked { path, reason } => self.open_warn = Some(dialogs::OpenWarn { path, reason }),
                UiEvent::Alias { lid, pn } => self.set_alias(lid, pn, true),
                UiEvent::Disconnected => {
                    if self.conn == Conn::Online {
                        self.conn = Conn::Starting;
                    }
                }
                UiEvent::Message(m) => {
                    // Bei „Gelöschte behalten“ Fotos sofort laden, damit sie nach einer Löschung noch da sind.
                    if self.settings.keep_deleted && !m.from_me {
                        if let Some(md) = m.media.as_ref().filter(|md| md.is_picture()) {
                            if self.img_requested.insert(m.id.clone()) {
                                let _ = self.cmd.send(Cmd::LoadMedia { id: m.id.clone(), media: md.clone() });
                            }
                        }
                    }
                    self.add_message(m, Source::Live);
                }
                UiEvent::History(c) => {
                    self.ingest_history(c);
                    self.names_dirty = true;
                }
                UiEvent::Contact { user, name, from_book } => {
                    // Adressbuch-Namen überschreiben Push-Namen, nicht umgekehrt.
                    let keep = matches!(self.names.get(&user), Some((_, true))) && !from_book;
                    let same = self.names.get(&user).map(|(n, _)| n == &name).unwrap_or(false);
                    if !keep && !same {
                        self.store.append(&Rec::Name { user: user.clone(), name: name.clone(), book: from_book });
                        self.names.insert(user, (name, from_book));
                        self.names_dirty = true;
                    }
                }
                UiEvent::Avatar { user, size, rgba } => {
                    let img = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                    let tex = ctx.load_texture(format!("av-{user}"), img, egui::TextureOptions::LINEAR);
                    self.avatars.insert(user, tex);
                }
                UiEvent::AvatarRetry { user } => {
                    self.avatar_requested.remove(&user);
                }
                UiEvent::Sent { chat, temp, id } => {
                    let mut done = None;
                    // Das eigene Echo kann schneller sein als diese Bestätigung: dann steht die
                    // Nachricht schon mit echter ID da und der vorläufige Eintrag muss weg.
                    let echoed = self.msg_index.get(&id).map(|k| *k == chat).unwrap_or(false);
                    if let Some(c) = self.chats.get_mut(&chat) {
                        if echoed {
                            c.messages.retain(|m| m.id != temp);
                        } else if let Some(m) = c.messages.iter_mut().rev().find(|m| m.id == temp) {
                            m.id = id.clone();
                            m.status = m.status.max(1);
                            done = Some(m.clone());
                        }
                    }
                    self.msg_index.remove(&temp);
                    if echoed {
                        self.images.remove(&temp);
                    }
                    if let Some(m) = done {
                        if let Some(t) = self.images.remove(&temp) {
                            self.images.insert(id.clone(), t);
                        }
                        self.msg_index.insert(id.clone(), chat.clone());
                        self.persist_msg(&chat, &m);
                    }
                    if let Some(st) = self.early_status.remove(&id) {
                        self.set_status(&id, st, true);
                    }
                    if self.outbox.remove(&temp).is_some() | self.outbox_files.remove(&temp).is_some() {
                        self.store.append(&Rec::OutDone { temp });
                    }
                }
                UiEvent::OutMedia { temp, chat, is_group, caption, media } => {
                    let ts = now_ts();
                    self.store.append(&Rec::OutFile { temp: temp.clone(), chat: chat.clone(), is_group, ts, caption: caption.clone(), media: media.clone() });
                    self.outbox_files.insert(temp, (chat, is_group, ts, caption, media));
                }
                UiEvent::MediaFailed { id } => {
                    self.img_requested.remove(&id);
                    self.img_failed.insert(id, std::time::Instant::now());
                }
                UiEvent::SendFailed { chat, temp, err } => {
                    if let Some(c) = self.chats.get_mut(&chat) {
                        if let Some(m) = c.messages.iter_mut().rev().find(|m| m.id == temp) {
                            m.status = 9;
                        }
                    }
                    self.info = Some(tf!("Senden fehlgeschlagen: {err}", "Sending failed: {err}"));
                }
                UiEvent::Receipt { ids, status } => {
                    for id in ids {
                        self.set_status(&id, status, true);
                    }
                }
                UiEvent::Info(m) => {
                    if let Some(link) = m.strip_prefix("Einladungslink (in die Zwischenablage kopiert): ") {
                        ctx.copy_text(link.to_owned());
                    }
                    self.info = if m.is_empty() { None } else { Some(m) };
                }
                UiEvent::Me(users) => self.me = users,
                UiEvent::Starred { id, on } => {
                    self.store.append(&Rec::Star { id: id.clone(), on });
                    if on {
                        self.starred.insert(id);
                    } else {
                        self.starred.remove(&id);
                    }
                }
                UiEvent::PollVote { poll, voter, options } => {
                    self.store.append(&Rec::Vote { poll: poll.clone(), voter: voter.clone(), options: options.clone() });
                    self.votes.entry(poll).or_default().insert(voter, options);
                }
                UiEvent::Privacy(list) => {
                    self.privacy = list.into_iter().collect();
                }
                UiEvent::Blocklist(list) => self.blocklist = list,
                UiEvent::Channels(list) => {
                    self.channels = list;
                    self.channels_loaded = true;
                }
                UiEvent::ChannelSearch(list) => {
                    if list.is_empty() {
                        self.info = Some(t!("Keine Kanäle gefunden.", "No channels found.").into());
                    }
                    self.channel_results = list;
                }
                UiEvent::ChannelPosts { jid, posts } => {
                    self.channel_posts.insert(jid, posts);
                }
                UiEvent::CallIncoming { id, from, name, video } => {
                    self.call_in = Some((id, from, name, video));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
                }
                UiEvent::CallState { peer, text } => {
                    self.call_in = None;
                    let muted = self.call.as_ref().map(|c| c.3).unwrap_or(false);
                    let started = self.call.as_ref().map(|c| c.2).unwrap_or_else(std::time::Instant::now);
                    let started = if text == t!("Verbunden", "Connected") && self.call.as_ref().map(|c| c.1 != t!("Verbunden", "Connected")).unwrap_or(true) { std::time::Instant::now() } else { started };
                    self.call = Some((peer, text, started, muted));
                }
                UiEvent::CallEnded(reason) => {
                    self.call_in = None;
                    if let Some((peer, text, start, _)) = self.call.take() {
                        let secs = if text == t!("Verbunden", "Connected") { start.elapsed().as_secs() } else { 0 };
                        let name = self.name_for_user(&peer).unwrap_or_else(|| format!("+{peer}"));
                        self.info = Some(if secs > 0 { tf!("{reason} – {name}, {}:{:02}", "{reason} – {name}, {}:{:02}", secs / 60, secs % 60) } else { tf!("{reason} – {name}", "{reason} – {name}") });
                    } else {
                        self.info = Some(reason);
                    }
                }
                UiEvent::GroupInfo(d) => {
                    let members: Vec<(String, String)> = d.members.iter().map(|(j, pn, _, _)| (user_of(j), pn.clone())).collect();
                    self.members.insert(d.group.clone(), members);
                    self.group_info.insert(d.group.clone(), d);
                }
                UiEvent::ViewReady { id, path, image, video } => self.viewer_ready(ctx, id, path, image, video),
                UiEvent::ViewFailed { id, err } => self.viewer_failed(&id, err),
                UiEvent::FileReady { id, path, voice } => {
                    if self.want_play.as_ref().map(|w| w.0 == id).unwrap_or(false) {
                        let (_, from) = self.want_play.take().unwrap_or_default();
                        match voice.unwrap_or_else(|| Err("kein Opus".into())) {
                            Ok(v) => {
                                self.decoded.insert(id.clone(), v);
                                self.decoded_order.retain(|x| *x != id);
                                self.decoded_order.push_back(id.clone());
                                self.trim_decoded();
                                self.start_play(&id, from);
                            }
                            Err(e) => {
                                // Nicht dekodierbar (z. B. MP3/M4A): mit dem Standardprogramm öffnen
                                backend::log(format!("Audio {id}: {e}"));
                                match safety::open_verdict(&path) {
                                    Ok(()) => safety::spawn_opener(path.as_os_str()),
                                    Err(reason) => self.open_warn = Some(dialogs::OpenWarn { path, reason }),
                                }
                            }
                        }
                    }
                }
                UiEvent::Anim { id, frames } => {
                    let mut texs = Vec::new();
                    let mut delays = Vec::new();
                    for (i, (size, rgba, ms)) in frames.into_iter().enumerate() {
                        let img = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                        texs.push(ctx.load_texture(format!("anim-{id}-{i}"), img, egui::TextureOptions::LINEAR));
                        delays.push(ms);
                    }
                    self.anims.insert(id, (texs, delays));
                }
                UiEvent::GifResults(list) => {
                    self.gif_loading = false;
                    self.gif_results = list
                        .into_iter()
                        .map(|g| {
                            let tex = g.preview.as_ref().map(|(size, rgba)| {
                                ctx.load_texture(format!("gif-{}", g.id), egui::ColorImage::from_rgba_unmultiplied(*size, rgba), egui::TextureOptions::LINEAR)
                            });
                            (g, tex)
                        })
                        .collect();
                    if self.gif_results.is_empty() {
                        self.info = Some(t!("Keine GIFs gefunden.", "No GIFs found.").into());
                    }
                }
                UiEvent::Members { group, members } => {
                    self.members.insert(group, members);
                }
                UiEvent::Typing { chat, from, on, recording } => {
                    let chat = self.alias.get(&chat).cloned().unwrap_or(chat);
                    if on {
                        self.typing.insert(chat, (from, recording, std::time::Instant::now()));
                    } else {
                        self.typing.remove(&chat);
                    }
                }
                UiEvent::Presence { user, online, last_seen } => {
                    let user = self.alias.get(&user).cloned().unwrap_or(user);
                    let e = self.presence.entry(user).or_insert((false, None));
                    e.0 = online;
                    if last_seen.is_some() {
                        e.1 = last_seen;
                    } else if !online {
                        e.1 = Some(now_ts());
                    }
                }
                UiEvent::Reaction { target, from, emoji } => self.set_reaction(target, from, emoji, true),
                UiEvent::Revoked { id } => self.revoke_local(&id, true),
                UiEvent::Edited { id, text } => self.edit_local(&id, text, true),
                UiEvent::Image { id, size, rgba, full } => {
                    if self.images.get(&id).map(|t| t.1 && !full).unwrap_or(false) {
                        continue;
                    }
                    let img = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                    let tex = ctx.load_texture(format!("img-{id}"), img, egui::TextureOptions::LINEAR);
                    self.images.insert(id, (tex, full));
                }
                UiEvent::Groups(list) => {
                    for (jid, name) in list {
                        if name.is_empty() {
                            continue;
                        }
                        let c = self.chat_entry(&jid, true);
                        if c.name == name {
                            continue;
                        }
                        c.name = name.clone();
                        let last_ts = c.last_ts;
                        self.store.append(&Rec::Chat { jid: jid.to_string(), name, is_group: true, last_ts });
                    }
                    self.names_dirty = true;
                }
                UiEvent::Muted { users, on, until } => {
                    for u in &users {
                        match until {
                            Some(t) if on => {
                                self.settings.muted_until.insert(u.clone(), t);
                            }
                            _ => {
                                self.settings.muted_until.remove(u);
                            }
                        }
                    }
                    self.apply_flag(0, users, on)
                }
                UiEvent::Pinned { users, on } => {
                    // lokale Zusatz-Pins nicht vom Handy-Abgleich entfernen lassen
                    let users: Vec<String> = users.into_iter().filter(|u| on || !self.settings.local_pins.contains(u)).collect();
                    self.apply_flag(1, users, on)
                }
                UiEvent::Archived { users, on } => self.apply_flag(2, users, on),
                UiEvent::ChatRead { users, read } => {
                    for c in self.chats.values_mut() {
                        if users.iter().any(|u| *u == c.jid.user.as_str()) {
                            c.unread = if read { 0 } else { c.unread.max(1) };
                        }
                    }
                }
            }
        }
    }

    /// Zustand für Benachrichtigungen im Backend aktuell halten.
    pub(crate) fn sync_notify_state(&mut self) {
        // Abgelaufene befristete Stummschaltungen aufheben.
        let now = now_ts();
        let expired: Vec<String> = self.settings.muted_until.iter().filter(|(_, t)| **t <= now).map(|(u, _)| u.clone()).collect();
        if !expired.is_empty() {
            for u in &expired {
                self.settings.muted_until.remove(u);
                set_flag(&mut self.settings.muted, u.clone(), false);
            }
            self.save_settings();
        }
        let Ok(mut st) = self.notify.lock() else { return };
        st.enabled = self.settings.notifications;
        st.groups = self.settings.notify_groups;
        st.sound = self.settings.sound;
        st.preview = self.settings.preview;
        st.focused = self.focused;
        st.selected_user = self.selected.as_deref().map(user_of);
        if st.muted.len() != self.settings.muted.len() || !self.settings.muted.iter().all(|m| st.muted.contains(m)) {
            st.muted = self.settings.muted.iter().cloned().collect();
        }
        if self.names_dirty {
            self.names_dirty = false;
            st.names = self
                .chats
                .values()
                .map(|c| (c.jid.user.to_string(), self.display_name(c)))
                .filter(|(u, n)| !n.ends_with(u.as_str()) && n != t!("Gruppe", "Group"))
                .collect();
        }
    }

    pub(crate) fn send_current(&mut self) {
        let text = self.input.trim().to_owned();
        let Some(key) = self.selected.clone() else { return };
        if text.is_empty() {
            return;
        }
        if let Some((k, id)) = self.editing.take() {
            if k == key {
                if let Some(jid) = self.chats.get(&key).map(|c| c.jid.clone()) {
                    self.edit_local(&id, text.clone(), true);
                    let _ = self.cmd.send(Cmd::Edit { chat: jid, id, text });
                }
                self.input.clear();
                return;
            }
        }
        let quote = match self.reply_to.take() {
            Some((k, q, _)) if k == key => Some(q),
            _ => None,
        };
        // @Erwähnungen: alle "@nummer" im Text, die zu Gruppenmitgliedern gehören
        let chat_user = self.chats.get(&key).map(|c| c.jid.user.to_string()).unwrap_or_default();
        let mentions: Vec<String> = self
            .members
            .get(&chat_user)
            .map(|ms| {
                ms.iter()
                    .filter_map(|(u, pn)| {
                        if !pn.is_empty() && text.contains(&format!("@{pn}")) {
                            Some(format!("{pn}@s.whatsapp.net"))
                        } else if text.contains(&format!("@{u}")) {
                            Some(format!("{u}@lid"))
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Selbstlöschende Nachrichten: Zeitspanne der letzten Nachricht im Chat übernehmen
        let expiration = self.chats.get(&key).and_then(|c| c.messages.iter().rev().find(|m| !m.id.starts_with("local-"))).map(|m| m.x.expiration).unwrap_or(0);
        if let Some(chat) = self.chats.get(&key) {
            let jid = chat.jid.clone();
            let chat_is_group = chat.is_group;
            self.temp_counter += 1;
            let temp = format!("local-{}-{}", now_ts(), self.temp_counter);
            let m = ChatMessage {
                chat: jid.clone(),
                is_group: chat.is_group,
                from_me: true,
                sender_name: String::new(),
                sender: String::new(),
                text: text.clone(),
                ts: now_ts(),
                id: temp.clone(),
                status: 0,
                media: None,
                quote: quote.clone(),
                x: backend::MsgExtra { expiration, mentions: mentions.iter().map(|m| user_of(m)).collect(), ..Default::default() },
            };
            self.add_message(m, Source::Live);
            let out = Outgoing { chat: key.clone(), is_group: chat_is_group, text: text.clone(), ts: now_ts(), quote: quote.clone(), mentions: mentions.clone(), exp: expiration };
            self.store.append(&Rec::Out {
                temp: temp.clone(),
                chat: out.chat.clone(),
                is_group: out.is_group,
                text: out.text.clone(),
                ts: out.ts,
                quote: out.quote.clone(),
                mentions: out.mentions.clone(),
                exp: out.exp,
            });
            self.outbox.insert(temp.clone(), out);
            let _ = self.cmd.send(Cmd::Send { chat: jid, text, temp, quote, mentions, expiration });
        }
        self.input.clear();
    }

    pub(crate) fn qr_view(&self, ui: &mut egui::Ui, data: &str) {
        let Ok(code) = qrcode::QrCode::new(data.as_bytes()) else {
            ui.label(t!("QR-Code konnte nicht erzeugt werden.", "Could not create QR code."));
            return;
        };
        let n = code.width();
        let cell = 7.0;
        let quiet = 3.0;
        let side = (n as f32 + 2.0 * quiet) * cell;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 12.0, Color32::WHITE);
        let colors = code.to_colors();
        for y in 0..n {
            for x in 0..n {
                if colors[y * n + x] == qrcode::Color::Dark {
                    let min = rect.min + egui::vec2((x as f32 + quiet) * cell, (y as f32 + quiet) * cell);
                    p.rect_filled(egui::Rect::from_min_size(min, egui::vec2(cell, cell)), 0.0, Color32::BLACK);
                }
            }
        }
    }
}
