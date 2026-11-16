use super::*;

impl App {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, core: Core) -> Self {
        let mut settings = load_settings();
        backend::DEBUG_LOG.store(settings.debug_log, std::sync::atomic::Ordering::Relaxed);
        // Ältere Versionen speicherten volle JIDs; jetzt zählt nur die Nummer/LID.
        for list in [&mut settings.muted, &mut settings.pinned, &mut settings.archived] {
            let mut v: Vec<String> = list.iter().map(|k| user_of(k)).collect();
            v.sort();
            v.dedup();
            *list = v;
        }
        apply_lang(&settings.lang);
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, settings.dark, settings.font_size);

        let Core { rx, cmd, notify } = core;
        let dir = data_dir();
        let (store, recs) = Store::open(&dir.join("history.jsonl"));

        let mut app = Self {
            rx,
            cmd,
            store,
            conn: Conn::Starting,
            status: t!("Starte …", "Starting …").into(),
            qr: None,
            chats: HashMap::new(),
            msg_index: HashMap::new(),
            names: HashMap::new(),
            avatars: HashMap::new(),
            avatar_requested: HashSet::new(),
            selected: None,
            search: String::new(),
            input: String::new(),
            info: None,
            settings,
            show_settings: false,
            show_archive: false,
            filter: Filter::All,
            chat_search: None,
            autostart: autostart_file().map(|f| f.exists()).unwrap_or(false),
            focused: true,
            notify,
            names_dirty: true,
            temp_counter: 0,
            emoji: emoji::Cache::default(),
            images: HashMap::new(),
            img_seen: Default::default(),
            img_sweep: std::time::Instant::now(),
            img_stale: HashSet::new(),
            img_failed: HashMap::new(),
            viewer: None,
            early_status: HashMap::new(),
            outbox_files: HashMap::new(),
            msg_gen: 0,
            search_cache: None,
            search_typed: None,
            chat_search_cache: None,
            storage_size: None,
            status_cache: Default::default(),
            paste_rx: None,
            decoded_order: Default::default(),
            open_warn: None,
            logout_prompt: false,
            heights: HashMap::new(),
            img_requested: HashSet::new(),
            reactions: HashMap::new(),
            reply_to: None,
            me: Vec::new(),
            drafts: HashMap::new(),
            alias: HashMap::new(),
            typing: HashMap::new(),
            presence: HashMap::new(),
            my_typing: None,
            row_keys: Vec::new(),
            at_bottom: true,
            new_below: 0,
            scroll_bottom: false,
            jump_to: None,
            highlight: None,
            forward: None,
            pending_paste: None,
            outbox: HashMap::new(),
            starred: HashSet::new(),
            votes: HashMap::new(),
            members: HashMap::new(),
            editing: None,
            show_starred: false,
            show_limit: HashMap::new(),
            show_msg_results: true,
            picker_tab: 0,
            player: None,
            paused_at: HashMap::new(),
            decoded: HashMap::new(),
            want_play: None,
            recording: None,
            anims: HashMap::new(),
            gif_query: String::new(),
            gif_results: Vec::new(),
            gif_loading: false,
            settings_page: 0,
            privacy: HashMap::new(),
            privacy_loaded: false,
            blocklist: Vec::new(),
            confirm: None,
            emoji_search: String::new(),
            emoji_jump: None,
            show_info: false,
            group_info: HashMap::new(),
            group_requested: HashSet::new(),
            add_numbers: String::new(),
            rename_buf: String::new(),
            new_group: None,
            call_in: None,
            call: None,
            show_status: false,
            status_sel: None,
            status_seen: HashSet::new(),
            status_text: String::new(),
            status_color: 0,
            channels: Vec::new(),
            channels_loaded: false,
            channel_sel: None,
            channel_posts: HashMap::new(),
            channel_search: String::new(),
            channel_results: Vec::new(),
            channel_link: String::new(),
            channel_new: None,
            channel_post_text: String::new(),
            channel_refresh: None,
            news_tab: 0,
        };
        for r in recs {
            app.apply_rec(r);
        }
        // Beim letzten Mal nicht bestätigte Nachrichten wieder anzeigen – als „nicht gesendet“.
        let mut pending: Vec<(String, Outgoing)> = app.outbox.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        pending.sort_by_key(|(_, o)| o.ts);
        for (temp, o) in pending {
            let Ok(jid) = o.chat.parse::<Jid>() else { continue };
            app.add_message(
                ChatMessage {
                    chat: jid,
                    is_group: o.is_group,
                    from_me: true,
                    sender_name: String::new(),
                    sender: String::new(),
                    text: o.text,
                    ts: o.ts,
                    id: temp,
                    status: 9,
                    media: None,
                    quote: o.quote,
                    x: backend::MsgExtra { expiration: o.exp, ..Default::default() },
                },
                Source::Disk,
            );
        }
        // Dasselbe für Dateien: die Kopie liegt im Medienordner und lässt sich erneut senden.
        let mut files: Vec<(String, (String, bool, i64, String, backend::MediaRef))> = app.outbox_files.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        files.sort_by_key(|(_, f)| f.2);
        for (temp, (chat, is_group, ts, caption, media)) in files {
            let Ok(jid) = chat.parse::<Jid>() else { continue };
            app.add_message(
                ChatMessage { chat: jid, is_group, from_me: true, sender_name: String::new(), sender: String::new(), text: caption, ts, id: temp, status: 9, media: Some(media), quote: None, x: Default::default() },
                Source::Disk,
            );
        }
        // Gespeicherte Umfragen dem Backend melden, damit neue Stimmen lesbar sind.
        let polls: Vec<(String, Vec<u8>, String, Vec<String>)> = app
            .chats
            .values()
            .flat_map(|c| c.messages.iter())
            .filter_map(|m| match &m.x.kind {
                Some(backend::Extra::Poll { secret, options, .. }) if !secret.is_empty() => {
                    Some((m.id.clone(), secret.clone(), m.sender.clone(), options.clone()))
                }
                _ => None,
            })
            .collect();
        if !polls.is_empty() {
            let _ = app.cmd.send(Cmd::RegisterPolls(polls));
        }
        let media: Vec<(String, Jid, bool, String)> = app
            .chats
            .values()
            .flat_map(|c| c.messages.iter().filter(|m| m.media.is_some() && !m.id.is_empty()).map(|m| (m.id.clone(), c.jid.clone(), m.from_me, m.sender.clone())))
            .collect();
        if !media.is_empty() {
            let _ = app.cmd.send(Cmd::RegisterMedia(media));
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("WA_SNAP_DEMO").is_some() {
            app.load_demo();
        }
        app
    }

    /// Nur Debug-Build: Beispieldaten für Screenshots ohne Konto (WA_SNAP_DEMO=1).
    #[cfg(debug_assertions)]
    pub(crate) fn load_demo(&mut self) {
        let now = now_ts();
        let people = [
            ("4915100000001", "Anna Berger", false),
            ("120363000000001", "Wichtig", true),
            ("4915100000002", "Max Keller", false),
            ("120363000000002", "Flohmarkt im Rheingau", true),
            ("4915100000004", "Lena", false),
        ];
        let texts = [
            ("Hey, bist du später noch online?", false),
            ("Klar, ich melde mich gegen 20 Uhr.\nDann können wir alles besprechen.", true),
            ("Perfekt, *bis dann*! 👍 _Bring_ ~Kuchen~ `Pizza` mit", false),
            ("Super 😊 bis später ❤️", true),
            ("😂👍", false),
        ];
        for (i, (num, name, group)) in people.iter().enumerate() {
            let server = if *group { whatsapp_rust::Server::Group } else { whatsapp_rust::Server::Pn };
            let jid = Jid::new(*num, server);
            let c = self.chat_entry(&jid, *group);
            c.name = (*name).to_owned();
            c.unread = if i == 3 { 1 } else { 0 };
            for (j, (t, me)) in texts.iter().enumerate() {
                let m = ChatMessage {
                    chat: jid.clone(),
                    is_group: *group,
                    from_me: *me,
                    sender_name: if *group { ["Tom", "Sara"][j % 2].to_owned() } else { (*name).to_owned() },
                    sender: String::new(),
                    text: (*t).to_owned(),
                    ts: now - 86_400 * (i as i64 / 2) - 600 * (5 - j as i64),
                    id: format!("demo-{i}-{j}"),
                    status: if j == 3 { 2 } else { 3 },
                    media: None,
                    quote: None,
                    x: Default::default(),
                };
                self.add_message(m, Source::Disk);
            }
        }
        // Demo-Bild (Farbverlauf) als empfangenes Foto
        let media_dir = data_dir().join("media");
        let _ = std::fs::create_dir_all(&media_dir);
        let img = image::RgbImage::from_fn(480, 320, |x, y| image::Rgb([(x / 2) as u8, (y * 3 / 4) as u8, 160]));
        let _ = img.save(media_dir.join("demo-img.png"));
        let anna = Jid::new("4915100000001", whatsapp_rust::Server::Pn);
        self.add_message(
            ChatMessage {
                chat: anna.clone(),
                is_group: false,
                from_me: false,
                sender_name: "Anna Berger".into(),
                sender: String::new(),
                text: "Schau mal, Sonnenuntergang 🌅".into(),
                ts: now - 30,
                id: "demo-img".into(),
                status: 0,
                media: Some(backend::MediaRef { kind: "image".into(), mime: "image/png".into(), w: 480, h: 320, ..Default::default() }),
                quote: None,
                x: Default::default(),
            },
            Source::Disk,
        );
        let tone: Vec<i16> = (0..audio::RATE as usize * 3)
            .map(|i| ((i as f32 * 330.0 * std::f32::consts::TAU / audio::RATE as f32).sin() * 6000.0 * (1.0 + (i as f32 / 9000.0).sin()) / 2.0) as i16)
            .collect();
        if let Ok(ogg) = audio::encode_ogg_opus(&tone) {
            let _ = std::fs::write(media_dir.join("demo-voice.ogg"), ogg);
        }
        self.add_message(
            ChatMessage {
                chat: anna.clone(),
                is_group: false,
                from_me: false,
                sender_name: "Anna Berger".into(),
                sender: String::new(),
                text: String::new(),
                ts: now - 20,
                id: "demo-voice".into(),
                status: 0,
                media: Some(backend::MediaRef { kind: "ptt".into(), mime: "audio/ogg; codecs=opus".into(), seconds: 3, ..Default::default() }),
                quote: None,
                x: Default::default(),
            },
            Source::Disk,
        );
        self.add_message(
            ChatMessage {
                chat: anna,
                is_group: false,
                from_me: true,
                sender_name: String::new(),
                sender: String::new(),
                text: String::new(),
                ts: now - 10,
                id: "demo-doc".into(),
                status: 3,
                media: Some(backend::MediaRef { kind: "document".into(), name: "Bewerbung.pdf".into(), mime: "application/pdf".into(), len: 245_000, ..Default::default() }),
                quote: Some(backend::Quote { id: "demo-img".into(), sender: String::new(), name: "Anna Berger".into(), text: "Schau mal, Sonnenuntergang 🌅".into() }),
                x: backend::MsgExtra { edited: true, ..Default::default() },
            },
            Source::Disk,
        );
        let wichtig = Jid::new("120363000000001", whatsapp_rust::Server::Group);
        let extras = [
            ("demo-poll", "Tom", String::new(), Some(backend::Extra::Poll { name: "Wann grillen wir?".into(), options: vec!["Samstag".into(), "Sonntag".into(), "Nächste Woche".into()], secret: vec![1; 32], multi: false }), vec![]),
            ("demo-loc", "Sara", String::new(), Some(backend::Extra::Location { lat: 50.0, lon: 8.06, name: "Rheinufer Eltville".into(), address: "Rheinallee, Eltville".into(), live: false }), vec![]),
            ("demo-link", "Tom", "Schaut mal: https://www.openstreetmap.org".to_owned(), Some(backend::Extra::Link { url: "https://www.openstreetmap.org".into(), title: "OpenStreetMap".into(), description: "Die freie Weltkarte, gemacht von Menschen wie dir.".into() }), vec![]),
            ("demo-ment", "Sara", "@4915100000001 bringst du *Kohle* mit?".to_owned(), None, vec!["4915100000001".to_owned()]),
        ];
        for (i, (id, who, text, kind, mentions)) in extras.into_iter().enumerate() {
            self.add_message(
                ChatMessage {
                    chat: wichtig.clone(),
                    is_group: true,
                    from_me: false,
                    sender_name: who.into(),
                    sender: String::new(),
                    text,
                    ts: now - 200 + i as i64 * 20,
                    id: id.into(),
                    status: 0,
                    media: None,
                    quote: None,
                    x: backend::MsgExtra { kind, mentions, ..Default::default() },
                },
                Source::Disk,
            );
        }
        self.group_info.insert(
            "120363000000001".into(),
            backend::GroupDetails {
                group: "120363000000001".into(),
                subject: "Wichtig".into(),
                description: "Alles fürs Grillen am Rhein 🔥".into(),
                members: vec![
                    ("4915100000001@s.whatsapp.net".into(), "4915100000001".into(), true, true),
                    ("4915100000002@s.whatsapp.net".into(), "4915100000002".into(), false, false),
                    ("4915100000009@s.whatsapp.net".into(), "4915100000009".into(), false, false),
                ],
                created: 0,
                i_am_admin: true,
            },
        );
        self.group_requested.insert("120363000000001".into());
        if std::env::var_os("WA_SNAP_DEMO_INFO").is_some() {
            self.show_info = true;
        }
        let v = self.votes.entry("demo-poll".into()).or_default();
        v.insert("1".into(), vec!["Samstag".into()]);
        v.insert("2".into(), vec!["Samstag".into()]);
        v.insert(String::new(), vec!["Sonntag".into()]);
        self.starred.insert("demo-link".into());
        self.set_reaction("demo-img".into(), String::new(), "😍".into(), false);
        let st = Jid::new("status", whatsapp_rust::Server::Broadcast);
        for (i, (who, text, bg)) in [("4915100000001", "Sonnenuntergang am Rhein 🌅", 0xFFFF7A6Bu32), ("4915100000001", "Wer kommt mit zum Grillen? 🔥", 0xFF00A884), ("4915100000002", "Neuer Job ab Montag 💼", 0xFF5696FF)].into_iter().enumerate() {
            self.add_message(
                ChatMessage {
                    chat: st.clone(),
                    is_group: false,
                    from_me: false,
                    sender_name: String::new(),
                    sender: format!("{who}@s.whatsapp.net"),
                    text: text.into(),
                    ts: now - 3600 * (3 - i as i64),
                    id: format!("demo-st-{i}"),
                    status: 0,
                    media: None,
                    quote: None,
                    x: backend::MsgExtra { bg, ..Default::default() },
                },
                Source::Disk,
            );
        }
        if std::env::var_os("WA_SNAP_DEMO_STATUS").is_some() {
            self.show_status = true;
            self.status_sel = Some(("4915100000001".into(), 1));
        }
        if std::env::var_os("WA_SNAP_DEMO_CALL").is_some() {
            self.call_in = Some(("demo".into(), "4915100000001".into(), "Anna".into(), false));
        }
        if std::env::var_os("WA_SNAP_DEMO_MOD").is_some() {
            self.settings.keep_deleted = true;
            self.revoke_local("demo-0-2", false);
            self.edit_local("demo-0-0", "Hey, bist du *heute* noch online?".into(), false);
        }
        self.set_reaction("demo-1-2".into(), "4915100000001".into(), "❤️".into(), false);
        self.settings.pinned = vec!["4915100000001".into(), "120363000000001".into()];
        self.settings.muted = vec!["120363000000002".into()];
        self.settings.archived = vec!["4915100000004".into()];
        self.selected = Some(Jid::new("4915100000001", whatsapp_rust::Server::Pn).to_string());
        if std::env::var_os("WA_SNAP_DEMO_GROUP").is_some() {
            self.selected = Some(wichtig.to_string());
        }
        self.conn = Conn::Online;
    }

    pub(crate) fn apply_rec(&mut self, rec: Rec) {
        match rec {
            Rec::Msg { chat, is_group, from_me, sender, text, ts, id, sjid, st, media, quote, x } => {
                let Ok(jid) = chat.parse::<Jid>() else { return };
                self.add_message(
                    ChatMessage {
                        chat: jid,
                        is_group,
                        from_me,
                        sender_name: sender,
                        sender: sjid,
                        text,
                        ts,
                        id,
                        status: if from_me && st == 0 { 1 } else { st },
                        media,
                        quote,
                        x,
                    },
                    Source::Disk,
                );
            }
            Rec::Chat { jid, name, is_group, last_ts } => {
                let Ok(j) = jid.parse::<Jid>() else { return };
                let c = self.chat_entry(&j, is_group);
                if !name.is_empty() {
                    c.name = name;
                }
                c.last_ts = c.last_ts.max(last_ts);
            }
            Rec::Name { user, name, book } => {
                self.names.insert(user, (name, book));
            }
            Rec::Out { temp, chat, is_group, text, ts, quote, mentions, exp } => {
                self.outbox.insert(temp, Outgoing { chat, is_group, text, ts, quote, mentions, exp });
            }
            Rec::OutFile { temp, chat, is_group, ts, caption, media } => {
                self.outbox_files.insert(temp, (chat, is_group, ts, caption, media));
            }
            Rec::OutDone { temp } => {
                self.outbox_files.remove(&temp);
                self.outbox.remove(&temp);
            }
            Rec::Status { id, st } => self.set_status(&id, st, false),
            Rec::React { target, from, emoji } => self.set_reaction(target, from, emoji, false),
            Rec::Del { id } => self.revoke_local(&id, false),
            Rec::Edit { id, text } => self.edit_local(&id, text, false),
            Rec::Star { id, on } => {
                if on {
                    self.starred.insert(id);
                } else {
                    self.starred.remove(&id);
                }
            }
            Rec::Vote { poll, voter, options } => {
                self.votes.entry(poll).or_default().insert(voter, options);
            }
            Rec::Alias { lid, pn } => self.set_alias(lid, pn, false),
        }
    }

    /// Einzelchat unter der Telefonnummer führen, wenn zur LID eine bekannt ist.
    pub(crate) fn canon(&self, jid: &Jid) -> Jid {
        if jid.is_lid() {
            if let Some(pn) = self.alias.get(jid.user.as_str()) {
                return Jid::new(pn, whatsapp_rust::Server::Pn);
            }
        }
        jid.clone()
    }

    /// Zuordnung LID ↔ Nummer merken und einen schon vorhandenen LID-Chat in den Nummern-Chat einfügen.
    pub(crate) fn set_alias(&mut self, lid: String, pn: String, persist: bool) {
        if lid.is_empty() || pn.is_empty() || lid == pn || self.alias.get(&lid) == Some(&pn) {
            return;
        }
        if persist {
            self.store.append(&Rec::Alias { lid: lid.clone(), pn: pn.clone() });
        }
        self.alias.insert(lid.clone(), pn.clone());
        // Name, Stumm/Angeheftet/Archiviert gelten für die Person, nicht für die Kennung.
        if !self.names.contains_key(&pn) {
            if let Some(n) = self.names.get(&lid).cloned() {
                self.names.insert(pn.clone(), n);
            }
        }
        for list in [&mut self.settings.muted, &mut self.settings.pinned, &mut self.settings.archived, &mut self.settings.local_pins] {
            if list.contains(&lid) && !list.contains(&pn) {
                list.push(pn.clone());
            }
        }
        let old_key = Jid::new(&lid, whatsapp_rust::Server::Lid).to_string();
        let Some(old) = self.chats.remove(&old_key) else { return };
        let pn_jid = Jid::new(&pn, whatsapp_rust::Server::Pn);
        let new_key = pn_jid.to_string();
        let default_name = old.name == lid;
        let c = self.chat_entry(&pn_jid, false);
        if c.name == pn && !default_name {
            c.name = old.name.clone();
        }
        c.unread += old.unread;
        c.last_ts = c.last_ts.max(old.last_ts);
        let have: HashSet<String> = c.messages.iter().filter(|m| !m.id.is_empty()).map(|m| m.id.clone()).collect();
        let mut moved = Vec::new();
        for mut m in old.messages {
            if !m.id.is_empty() && have.contains(&m.id) {
                continue;
            }
            m.chat = pn_jid.clone();
            moved.push(m.id.clone());
            c.messages.push(m);
        }
        c.messages.sort_by_key(|m| m.ts);
        for id in moved {
            if !id.is_empty() {
                self.msg_index.insert(id, new_key.clone());
            }
        }
        if self.selected.as_deref() == Some(old_key.as_str()) {
            self.selected = Some(new_key.clone());
        }
        if let Some(d) = self.drafts.remove(&old_key) {
            self.drafts.entry(new_key.clone()).or_insert(d);
        }
        for o in self.outbox.values_mut() {
            if o.chat == old_key {
                o.chat = new_key.clone();
            }
        }
        self.names_dirty = true;
    }

    pub(crate) fn chat_entry(&mut self, jid: &Jid, is_group: bool) -> &mut Chat {
        let jid = &self.canon(jid);
        self.chats.entry(jid.to_string()).or_insert_with(|| Chat {
            jid: jid.clone(),
            name: jid.user.to_string(),
            is_group,
            messages: Vec::new(),
            unread: 0,
            last_ts: 0,
        })
    }

    pub(crate) fn display_name(&self, chat: &Chat) -> String {
        if !chat.is_group {
            if let Some((n, _)) = self.names.get(chat.jid.user.as_str()) {
                return n.clone();
            }
        }
        if chat.name == chat.jid.user.as_str() {
            return if chat.is_group {
                t!("Gruppe", "Group").to_owned()
            } else {
                format!("+{}", chat.jid.user)
            };
        }
        chat.name.clone()
    }

    pub(crate) fn save_settings(&self) {
        if let Ok(t) = serde_json::to_string_pretty(&self.settings) {
            if let Err(e) = write_atomic(&settings_path(), t.as_bytes()) {
                backend::log(format!("Einstellungen speichern: {e}"));
            }
        }
    }

    pub(crate) fn has(list: &[String], key: &str) -> bool {
        let u = user_of(key);
        list.contains(&u)
    }

    pub(crate) fn set_status(&mut self, id: &str, st: u8, persist: bool) {
        let Some(key) = self.msg_index.get(id).cloned() else {
            // Zustellbestätigung kam, bevor der Server die ID der eigenen Nachricht gemeldet hat:
            // merken und anwenden, sobald „gesendet“ eintrifft.
            if persist && self.early_status.len() < 500 {
                let e = self.early_status.entry(id.to_owned()).or_insert(0);
                *e = (*e).max(st);
            }
            return;
        };
        if let Some(c) = self.chats.get_mut(&key) {
            if let Some(m) = c.messages.iter_mut().rev().find(|m| m.id == id) {
                if st > m.status || st == 9 {
                    m.status = st;
                    if persist {
                        self.store.append(&Rec::Status { id: id.to_owned(), st });
                    }
                }
            }
        }
    }

    pub(crate) fn persist_msg(&mut self, key: &str, m: &ChatMessage) {
        self.store.append(&Rec::Msg {
            chat: key.to_owned(),
            is_group: m.is_group,
            from_me: m.from_me,
            sender: m.sender_name.clone(),
            text: m.text.clone(),
            ts: m.ts,
            id: m.id.clone(),
            sjid: m.sender.clone(),
            st: m.status,
            media: m.media.clone(),
            quote: m.quote.clone(),
            x: m.x.clone(),
        });
    }

    /// Nachricht einsortieren. Gibt false zurück, wenn es ein Duplikat war.
    pub(crate) fn add_message(&mut self, mut m: ChatMessage, src: Source) -> bool {
        m.chat = self.canon(&m.chat);
        let key = m.chat.to_string();
        let selected = self.selected.as_deref() == Some(key.as_str());
        let focused = self.focused;
        // Duplikat? Der Index kennt jede Nachricht des ganzen Verlaufs, nicht nur die letzten.
        let known = !m.id.is_empty() && self.msg_index.get(&m.id).map(|k| *k == key).unwrap_or(false);
        let chat = self.chat_entry(&m.chat.clone(), m.is_group);
        // Gesprächsname: bei Einzelchats der Name des Gegenübers.
        if !m.is_group && !m.from_me && !m.sender_name.is_empty() && chat.name == chat.jid.user.as_str() {
            chat.name = m.sender_name.clone();
        }
        if known {
            return false;
        }
        if src == Source::Live && !m.from_me && !(selected && focused) {
            chat.unread += 1;
        }
        chat.last_ts = chat.last_ts.max(m.ts);
        // An der richtigen Stelle einfügen (fast immer am Ende) statt die ganze Liste neu zu sortieren.
        let pos = insert_pos(&chat.messages, m.ts);
        chat.messages.insert(pos, m.clone());
        self.msg_gen += 1;
        if !m.id.is_empty() {
            self.msg_index.insert(m.id.clone(), key.clone());
        }
        // Eigene Nachrichten erst speichern, wenn der Server die echte ID geliefert hat.
        if src != Source::Disk && !m.id.starts_with("local-") {
            self.persist_msg(&key, &m);
        }
        if src == Source::Live && !m.from_me {
            self.typing.remove(m.chat.user.as_str());
        }
        if src == Source::Live && selected && !self.at_bottom && !m.from_me {
            self.new_below += 1;
        }
        if src == Source::Live && m.from_me && selected {
            self.scroll_bottom = true;
        }
        if src == Source::Live && !m.from_me && selected && focused {
            self.mark_read(&key);
        }
        true
    }

    pub(crate) fn ingest_history(&mut self, chats: Vec<HistChat>) {
        for h in chats {
            let name = h.name.clone();
            let c = self.chat_entry(&h.jid, h.is_group);
            if !name.is_empty() {
                c.name = name.clone();
            }
            c.unread = c.unread.max(h.unread);
            c.last_ts = c.last_ts.max(h.last_ts);
            let last_ts = c.last_ts;
            self.store.append(&Rec::Chat { jid: h.jid.to_string(), name, is_group: h.is_group, last_ts });
            for m in h.msgs {
                self.add_message(m, Source::History);
            }
        }
    }

    /// Chat als gelesen markieren: lokal und bei WhatsApp (Lesebestätigungen).
    pub(crate) fn mark_read(&mut self, key: &str) {
        let Some(c) = self.chats.get_mut(key) else { return };
        let had_unread = c.unread > 0;
        c.unread = 0;
        let msgs: Vec<(String, String)> = c
            .messages
            .iter()
            .rev()
            .take_while(|m| !m.from_me)
            .take(30)
            .filter(|m| !m.id.is_empty())
            .map(|m| (m.id.clone(), m.sender.clone()))
            .collect();
        if !had_unread && msgs.is_empty() {
            return;
        }
        let last = c.messages.last().map(|m| backend::LastMsg {
            id: m.id.clone(),
            from_me: m.from_me,
            sender: m.sender.clone(),
            ts: m.ts,
        });
        if self.conn == Conn::Online && (had_unread || !msgs.is_empty()) {
            let _ = self.cmd.send(Cmd::MarkRead { jid: c.jid.clone(), msgs, last });
        }
    }

    /// Dekodierte Sprachnachrichten kosten rund 6 MB pro Minute. Älteste freigeben, sobald das
    /// Budget überschritten ist; die gerade laufende bleibt. Die Datei liegt auf der Platte und
    /// wird beim nächsten Abspielen neu dekodiert.
    pub(crate) fn trim_decoded(&mut self) {
        const BUDGET: usize = 48 * 1024 * 1024;
        let playing = self.player.as_ref().map(|p| p.id.clone());
        let mut total: usize = self.decoded.values().map(|(pcm, _, _)| pcm.len() * 2).sum();
        let mut keep = std::collections::VecDeque::new();
        while let Some(id) = self.decoded_order.pop_front() {
            if total <= BUDGET || Some(&id) == playing.as_ref() || self.decoded_order.is_empty() {
                keep.push_back(id);
                continue;
            }
            if let Some((pcm, _, _)) = self.decoded.remove(&id) {
                total -= pcm.len() * 2;
            }
        }
        self.decoded_order = keep;
    }

    /// Treffer der Suche über alle Chats (neueste zuerst, höchstens 300). Gerechnet wird nicht
    /// bei jedem Tastendruck und nicht in jedem Bild: erst nach einer kurzen Tipp-Pause, danach
    /// kommt das Ergebnis aus dem Zwischenspeicher, bis sich Suchtext oder Verlauf ändern.
    pub(crate) fn global_hits(&mut self, ctx: &egui::Context, needle: &str) -> Vec<(i64, String, String, String)> {
        const PAUSE_MS: u128 = 180;
        const MAX_HITS: usize = 300;
        if let Some((n, gen, hits)) = &self.search_cache {
            if n == needle && *gen == self.msg_gen {
                return hits.clone();
            }
        }
        match &self.search_typed {
            Some((n, at)) if n == needle => {
                let waited = at.elapsed().as_millis();
                if waited < PAUSE_MS {
                    ctx.request_repaint_after(std::time::Duration::from_millis((PAUSE_MS - waited) as u64 + 5));
                    // Bis dahin die alten Treffer zeigen, falls sie zum Anfang des Suchtexts passen.
                    return self.search_cache.as_ref().filter(|(old, _, _)| needle.starts_with(old.as_str())).map(|(_, _, h)| h.clone()).unwrap_or_default();
                }
            }
            _ => {
                self.search_typed = Some((needle.to_owned(), std::time::Instant::now()));
                ctx.request_repaint_after(std::time::Duration::from_millis(PAUSE_MS as u64 + 5));
                return self.search_cache.as_ref().filter(|(old, _, _)| needle.starts_with(old.as_str())).map(|(_, _, h)| h.clone()).unwrap_or_default();
            }
        }
        let mut hits: Vec<(i64, String, String, String)> = self
            .chats
            .values()
            .flat_map(|c| {
                c.messages
                    .iter()
                    .filter(|m| !m.text.is_empty() && m.text.to_lowercase().contains(needle))
                    .map(move |m| (m.ts, m.id.clone(), c, m.text.as_str()))
            })
            .map(|(ts, id, c, text)| (ts, id, self.display_name(c), text.replace('\n', " ")))
            .collect();
        hits.sort_by_key(|h| std::cmp::Reverse(h.0));
        hits.truncate(MAX_HITS);
        self.search_cache = Some((needle.to_owned(), self.msg_gen, hits.clone()));
        hits
    }

    /// Sprachnachricht ab `from` Sekunden abspielen (vorher laufende stoppen).
    pub(crate) fn start_play(&mut self, id: &str, from: f32) {
        if let Some(p) = self.player.take() {
            let pos = p.position();
            self.paused_at.insert(p.id.clone(), pos);
            p.stop();
        }
        let Some((pcm, ch, _)) = self.decoded.get(id).cloned() else { return };
        match audio::play(id, &pcm, ch, from, &data_dir().join("tmp")) {
            Ok(p) => self.player = Some(p),
            Err(e) => self.info = Some(tf!("Abspielen nicht möglich: {e}", "Playback not possible: {e}")),
        }
    }

    /// Klick auf Play/Pause oder in die Wellenform (`seek` = Anteil 0..1).
    pub(crate) fn audio_action(&mut self, id: String, media: backend::MediaRef, seek: Option<f32>) {
        let playing_this = self.player.as_ref().map(|p| p.id == id).unwrap_or(false);
        if playing_this && seek.is_none() {
            if let Some(p) = self.player.take() {
                self.paused_at.insert(id, p.position());
                p.stop();
            }
            return;
        }
        let dur = self.decoded.get(&id).map(|(pcm, ch, _)| pcm.len() as f32 / *ch as f32 / audio::RATE as f32).unwrap_or(media.seconds as f32);
        let mut from = match seek {
            Some(f) => f * dur,
            None => *self.paused_at.get(&id).unwrap_or(&0.0),
        };
        if from >= dur - 0.2 {
            from = 0.0;
        }
        if self.decoded.contains_key(&id) {
            self.start_play(&id, from);
        } else {
            self.want_play = Some((id.clone(), from));
            let _ = self.cmd.send(Cmd::FetchFile { id, media });
        }
    }

    /// Chat öffnen: Entwurf des alten Chats merken, den des neuen laden.
    pub(crate) fn select_chat(&mut self, key: String) {
        if self.selected.as_deref() == Some(key.as_str()) {
            return;
        }
        if let Some(old) = self.selected.take() {
            let d = std::mem::take(&mut self.input);
            if d.trim().is_empty() {
                self.drafts.remove(&old);
            } else {
                self.drafts.insert(old.clone(), d);
            }
            if let Some((k, _)) = &self.my_typing {
                if *k == old {
                    if let Some(c) = self.chats.get(&old) {
                        let _ = self.cmd.send(Cmd::Typing { chat: c.jid.clone(), on: false });
                    }
                    self.my_typing = None;
                }
            }
        }
        self.input = self.drafts.remove(&key).unwrap_or_default();
        self.show_settings = false;
        self.chat_search = None;
        self.at_bottom = true;
        self.new_below = 0;
        self.scroll_bottom = true;
        if let Some(c) = self.chats.get(&key) {
            if !c.is_group && self.conn == Conn::Online {
                let _ = self.cmd.send(Cmd::Subscribe(c.jid.clone()));
            }
        }
        self.selected = Some(key.clone());
        self.mark_read(&key);
    }

    /// Fehlgeschlagene Textnachricht mit derselben vorläufigen ID noch einmal senden.
    pub(crate) fn resend(&mut self, temp: &str) {
        let Some(o) = self.outbox.get(temp).cloned() else { return };
        if self.conn != Conn::Online {
            self.info = Some(t!("Nicht verbunden – bitte später erneut versuchen.", "Not connected – please try again later.").into());
            return;
        }
        let Ok(jid) = o.chat.parse::<Jid>() else { return };
        if let Some(m) = self.chats.get_mut(&o.chat).and_then(|c| c.messages.iter_mut().find(|m| m.id == temp)) {
            m.status = 0;
        }
        let _ = self.cmd.send(Cmd::Send { chat: jid, text: o.text, temp: temp.to_owned(), quote: o.quote, mentions: o.mentions, expiration: o.exp });
    }

    /// Nicht gesendete Nachricht endgültig verwerfen.
    /// Nicht gesendete Datei erneut senden: läuft als neue Sendung, der alte Eintrag verschwindet.
    pub(crate) fn resend_file(&mut self, temp: &str) {
        if self.conn != Conn::Online {
            self.info = Some(t!("Nicht verbunden – bitte später erneut versuchen.", "Not connected – please try again later.").into());
            return;
        }
        let Some((chat, _, _, caption, media)) = self.outbox_files.remove(temp) else { return };
        let Ok(jid) = chat.parse::<Jid>() else { return };
        if let Some(c) = self.chats.get_mut(&chat) {
            c.messages.retain(|m| m.id != temp);
        }
        self.msg_index.remove(temp);
        self.store.append(&Rec::OutDone { temp: temp.to_owned() });
        let _ = self.cmd.send(Cmd::ResendFile { chat: jid, temp: temp.to_owned(), media, caption });
    }

    pub(crate) fn discard_outgoing(&mut self, temp: &str) {
        if let Some((chat, _, _, _, media)) = self.outbox_files.remove(temp) {
            if let Some(c) = self.chats.get_mut(&chat) {
                c.messages.retain(|m| m.id != temp);
            }
            self.msg_index.remove(temp);
            self.store.append(&Rec::OutDone { temp: temp.to_owned() });
            let name: String = temp.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
            let _ = std::fs::remove_file(data_dir().join("media").join(format!("{name}.{}", media.ext())));
            return;
        }
        let Some(o) = self.outbox.remove(temp) else { return };
        if let Some(c) = self.chats.get_mut(&o.chat) {
            c.messages.retain(|m| m.id != temp);
        }
        self.msg_index.remove(temp);
        self.store.append(&Rec::OutDone { temp: temp.to_owned() });
    }

    pub(crate) fn run_msg_act(&mut self, a: MsgAct) {
        match &a {
            MsgAct::Resend(temp) if self.outbox_files.contains_key(temp) => return self.resend_file(temp),
            MsgAct::Resend(temp) => return self.resend(temp),
            MsgAct::Discard(temp) => return self.discard_outgoing(temp),
            _ => {}
        }
        let id = match &a {
            MsgAct::Resend(_) | MsgAct::Discard(_) => return,
            MsgAct::React(id, _) | MsgAct::Reply(id) | MsgAct::Revoke(id) | MsgAct::Forward(id) | MsgAct::Star(id, _) | MsgAct::Edit(id) | MsgAct::Vote(id, _) | MsgAct::Jump(id) => id.clone(),
            MsgAct::OpenNumber(n) => {
                let jid = Jid::pn(n.clone());
                let key = jid.to_string();
                let c = self.chat_entry(&jid, false);
                if c.name == *n {
                    c.name = format!("+{n}");
                }
                self.select_chat(key);
                return;
            }
        };
        if let MsgAct::Jump(_) = &a {
            if let Some(k) = self.msg_index.get(&id).cloned() {
                self.show_starred = false;
                self.select_chat(k.clone());
                self.chat_search = None;
                // genug Nachrichten anzeigen, damit die gesuchte dabei ist
                if let Some(c) = self.chats.get(&k) {
                    if let Some(pos) = c.messages.iter().position(|m| m.id == id) {
                        let need = c.messages.len() - pos + 20;
                        let lim = self.show_limit.entry(k).or_insert(400);
                        *lim = (*lim).max(need);
                    }
                }
                self.jump_to = Some(id);
            }
            return;
        }
        let Some(key) = self.msg_index.get(&id).cloned() else { return };
        let Some(chat) = self.chats.get(&key) else { return };
        let jid = chat.jid.clone();
        let Some(m) = chat.messages.iter().rev().find(|m| m.id == id).cloned() else { return };
        let quote = backend::Quote {
            id: m.id.clone(),
            sender: if m.from_me { String::new() } else if m.sender.is_empty() { jid.to_string() } else { m.sender.clone() },
            name: if m.from_me { t!("Du", "You").into() } else { m.sender_name.clone() },
            text: match &m.media {
                Some(md) if m.text.is_empty() => md.label(),
                _ => m.text.clone(),
            },
        };
        match a {
            MsgAct::React(_, emoji) => {
                self.set_reaction(id, String::new(), emoji.clone(), true);
                let _ = self.cmd.send(Cmd::React { chat: jid, target: quote, from_me: m.from_me, emoji });
            }
            MsgAct::Reply(_) => {
                self.reply_to = Some((key, quote, m.from_me));
            }
            MsgAct::Revoke(_) => {
                let _ = self.cmd.send(Cmd::Revoke { chat: jid, id });
            }
            MsgAct::Forward(_) => {
                self.forward = Some((id, String::new()));
            }
            MsgAct::Star(_, on) => {
                self.store.append(&Rec::Star { id: id.clone(), on });
                if on {
                    self.starred.insert(id.clone());
                } else {
                    self.starred.remove(&id);
                }
                let _ = self.cmd.send(Cmd::Star { chat: jid, id, from_me: m.from_me, sender: m.sender.clone(), on });
            }
            MsgAct::Edit(_) => {
                self.editing = Some((key, id));
                self.reply_to = None;
                self.input = m.text.clone();
            }
            MsgAct::Vote(_, options) => {
                if let Some(backend::Extra::Poll { secret, .. }) = &m.x.kind {
                    self.store.append(&Rec::Vote { poll: id.clone(), voter: String::new(), options: options.clone() });
                    self.votes.entry(id.clone()).or_default().insert(String::new(), options.clone());
                    let creator = if m.from_me { String::new() } else { m.sender.clone() };
                    let _ = self.cmd.send(Cmd::Vote { chat: jid, poll: id, creator, secret: secret.clone(), options });
                }
            }
            MsgAct::Jump(_) | MsgAct::OpenNumber(_) | MsgAct::Resend(_) | MsgAct::Discard(_) => {}
        }
    }

    pub(crate) fn set_reaction(&mut self, target: String, from: String, emoji: String, persist: bool) {
        if persist {
            self.store.append(&Rec::React { target: target.clone(), from: from.clone(), emoji: emoji.clone() });
        }
        let list = self.reactions.entry(target).or_default();
        list.retain(|(f, _)| *f != from);
        if !emoji.is_empty() {
            list.push((from, emoji));
        }
    }

    pub(crate) fn find_msg_mut(&mut self, id: &str) -> Option<&mut ChatMessage> {
        let key = self.msg_index.get(id)?.clone();
        self.chats.get_mut(&key)?.messages.iter_mut().rev().find(|m| m.id == id)
    }

    pub(crate) fn revoke_local(&mut self, id: &str, persist: bool) {
        let keep = self.settings.keep_deleted;
        if persist {
            self.store.append(&Rec::Del { id: id.to_owned() });
        }
        if let Some(m) = self.find_msg_mut(id) {
            // Eigene Löschungen immer ausführen; fremde auf Wunsch nur markieren.
            if keep && !m.from_me {
                m.x.deleted = true;
                return;
            }
            m.text = t!("🚫 Diese Nachricht wurde gelöscht.", "🚫 This message was deleted.").into();
            m.media = None;
            m.quote = None;
            m.x = backend::MsgExtra::default();
        }
        self.reactions.remove(id);
    }

    pub(crate) fn edit_local(&mut self, id: &str, text: String, persist: bool) {
        if let Some(m) = self.find_msg_mut(id) {
            if m.text != text {
                let old = std::mem::replace(&mut m.text, text.clone());
                m.x.prev.push(old);
            }
            m.x.edited = true;
            if persist {
                self.store.append(&Rec::Edit { id: id.to_owned(), text });
            }
        }
    }

    /// Zuletzt empfangene/gesendete Sticker (ohne Doppelte), neueste zuerst.
    pub(crate) fn known_stickers(&self) -> Vec<(String, backend::MediaRef)> {
        let mut all: Vec<(i64, String, backend::MediaRef)> = self
            .chats
            .values()
            .flat_map(|c| c.messages.iter())
            .filter_map(|m| m.media.as_ref().filter(|md| md.kind == "sticker" && !md.direct_path.is_empty()).map(|md| (m.ts, m.id.clone(), md.clone())))
            .collect();
        all.sort_by_key(|x| std::cmp::Reverse(x.0));
        let mut seen = HashSet::new();
        all.into_iter()
            .filter(|(_, _, md)| seen.insert(md.sha.clone()))
            .take(48)
            .map(|(_, id, md)| (id, md))
            .collect()
    }

    pub(crate) fn last_msg(&self, key: &str) -> Option<backend::LastMsg> {
        let m = self.chats.get(key)?.messages.iter().rev().find(|m| !m.id.starts_with("local-"))?;
        Some(backend::LastMsg { id: m.id.clone(), from_me: m.from_me, sender: m.sender.clone(), ts: m.ts })
    }

    pub(crate) fn apply_flag(&mut self, which: u8, users: Vec<String>, on: bool) {
        let list = match which {
            0 => &mut self.settings.muted,
            1 => &mut self.settings.pinned,
            _ => &mut self.settings.archived,
        };
        for u in users {
            set_flag(list, u, on);
        }
        self.save_settings();
    }

    pub(crate) fn run_act(&mut self, act: Act) {
        let key = match &act {
            Act::Mute(k, _) | Act::Pin(k) | Act::Archive(k) | Act::Read(k) => k.clone(),
        };
        let Some(jid) = self.chats.get(&key).map(|c| c.jid.clone()) else { return };
        let user = jid.user.to_string();
        match act {
            Act::Mute(_, secs) => {
                let on = !Self::has(&self.settings.muted, &key);
                set_flag(&mut self.settings.muted, user.clone(), on);
                let until = if on { secs.map(|d| now_ts() + d) } else { None };
                match until {
                    Some(t) => {
                        self.settings.muted_until.insert(user, t);
                    }
                    None => {
                        self.settings.muted_until.remove(&user);
                    }
                }
                let _ = self.cmd.send(Cmd::SetMute { jid, on, until_ms: until.map(|t| t * 1000) });
            }
            Act::Pin(_) => {
                let on = !Self::has(&self.settings.pinned, &key);
                // WhatsApp erlaubt 3 angeheftete Chats; weitere heftet WhatsSnap nur lokal an.
                let local_only = on && self.settings.pinned.len() >= 3;
                set_flag(&mut self.settings.pinned, user.clone(), on);
                if local_only {
                    if !self.settings.local_pins.contains(&user) {
                        self.settings.local_pins.push(user);
                    }
                    self.info = Some(t!("Nur in WhatsSnap angeheftet (am Handy sind höchstens 3 möglich).", "Pinned in WhatsSnap only (the phone allows at most 3).").into());
                } else {
                    let was_local = self.settings.local_pins.contains(&user);
                    self.settings.local_pins.retain(|u| *u != user);
                    if !was_local {
                        let _ = self.cmd.send(Cmd::SetPin { jid, on });
                    }
                }
            }
            Act::Archive(_) => {
                let on = !Self::has(&self.settings.archived, &key);
                set_flag(&mut self.settings.archived, user, on);
                let last = self.last_msg(&key);
                let _ = self.cmd.send(Cmd::SetArchive { jid, on, last });
                if on && self.selected.as_deref() == Some(key.as_str()) {
                    self.selected = None;
                }
            }
            Act::Read(_) => self.mark_read(&key),
        }
        self.save_settings();
    }
}
