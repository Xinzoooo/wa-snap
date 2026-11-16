use super::*;

impl App {
    pub(crate) fn settings_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, pal: &Palette) {
        let pages: [(&str, &str, &str); 8] = [
            ("👤", t!("Profil", "Profile"), t!("Name, Info, Profilbild", "Name, about, profile photo")),
            ("🔑", t!("Konto", "Account"), t!("Nummer, Abmelden", "Number, log out")),
            ("🔒", t!("Datenschutz", "Privacy"), t!("Zuletzt online, Lesebestätigungen, Blockiert", "Last seen, read receipts, blocked")),
            ("💬", t!("Chats", "Chats"), t!("Design, Schrift, Enter zum Senden", "Theme, font, Enter to send")),
            ("🔔", t!("Benachrichtigungen", "Notifications"), t!("Nachrichten, Gruppen, Ton", "Messages, groups, sound")),
            ("💾", t!("Speicher und Daten", "Storage and data"), t!("Automatischer Download, Speicherplatz", "Auto-download, storage usage")),
            ("⚡", t!("WhatsSnap", "WhatsSnap"), t!("Tray, Autostart, Online-Status, GIFs", "Tray, autostart, online status, GIFs")),
            ("❓", t!("Hilfe", "Help"), t!("Fehler melden, Version, Lizenzen", "Report bugs, version, licenses")),
        ];
        let mut changed = false;
        let mut theme_changed = false;
        ui.horizontal_top(|ui| {
            // linke Liste
            ui.vertical(|ui| {
                ui.set_width(300.0);
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.label(RichText::new(t!("Einstellungen", "Settings")).size(22.0).family(theme::bold()));
                });
                ui.add_space(12.0);
                for (i, (icon, title, sub)) in pages.iter().enumerate() {
                    let on = self.settings_page == i as u8;
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(292.0, 58.0), egui::Sense::click());
                    let r = rect.shrink2(egui::vec2(8.0, 2.0));
                    if on {
                        ui.painter().rect_filled(r, 10.0, pal.selected);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 10.0, pal.hover);
                    }
                    let p = ui.painter();
                    p.text(r.left_center() + egui::vec2(24.0, 0.0), egui::Align2::CENTER_CENTER, *icon, egui::FontId::proportional(18.0), pal.dim);
                    p.text(r.left_center() + egui::vec2(52.0, -9.0), egui::Align2::LEFT_CENTER, *title, egui::FontId::proportional(15.5), pal.text);
                    p.text(r.left_center() + egui::vec2(52.0, 10.0), egui::Align2::LEFT_CENTER, *sub, egui::FontId::proportional(12.0), pal.dim);
                    if resp.clicked() {
                        self.settings_page = i as u8;
                        self.confirm = None;
                    }
                }
            });
            let sep = ui.available_rect_before_wrap();
            ui.painter().vline(sep.left(), sep.y_range(), egui::Stroke::new(1.0_f32, pal.border));
            // rechte Seite
            ui.vertical(|ui| {
                egui::ScrollArea::vertical().id_salt("settings_detail").auto_shrink([false; 2]).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(28.0);
                        ui.vertical(|ui| {
                            ui.set_max_width(520.0);
                            ui.add_space(22.0);
                            let (_, title, _) = pages[self.settings_page as usize];
                            ui.label(RichText::new(title).size(20.0).family(theme::bold()));
                            ui.add_space(10.0);
                            match self.settings_page {
                                0 => self.page_profile(ui, pal, &mut changed),
                                1 => self.page_account(ui, pal),
                                2 => self.page_privacy(ui, pal),
                                3 => self.page_chats(ui, pal, &mut changed, &mut theme_changed),
                                4 => self.page_notify(ui, pal, &mut changed),
                                5 => self.page_storage(ui, pal, &mut changed),
                                6 => self.page_wasnap(ui, pal, &mut changed),
                                _ => self.page_help(ui, pal),
                            }
                            ui.add_space(30.0);
                        });
                    });
                });
            });
        });
        if theme_changed {
            theme::apply(ctx, self.settings.dark, self.settings.font_size);
        }
        if changed || theme_changed {
            self.save_settings();
        }
    }

    pub(crate) fn page_profile(&mut self, ui: &mut egui::Ui, pal: &Palette, changed: &mut bool) {
        let me = self.me.first().cloned().unwrap_or_default();
        ui.horizontal(|ui| {
            draw_avatar(ui, self.avatars.get(&me), &self.settings.profile_name, 96.0, self.settings.dark);
            ui.add_space(12.0);
            if ui.button(t!("Profilbild ändern …", "Change profile photo …")).clicked() {
                let _ = self.cmd.send(Cmd::SetProfilePicture(None));
            }
        });
        ui.add_space(10.0);
        ui.label(RichText::new(t!("Name", "Name")).small().color(pal.dim));
        ui.add(egui::TextEdit::singleline(&mut self.settings.profile_name).desired_width(360.0).margin(egui::vec2(10.0, 7.0)));
        ui.label(RichText::new(t!("Diesen Namen sehen deine Kontakte, wenn sie dich nicht gespeichert haben.", "Your contacts see this name if they haven't saved you.")).small().color(pal.dim));
        ui.add_space(8.0);
        ui.label(RichText::new(t!("Info", "About")).small().color(pal.dim));
        ui.add(egui::TextEdit::singleline(&mut self.settings.profile_about).desired_width(360.0).margin(egui::vec2(10.0, 7.0)));
        ui.add_space(8.0);
        if ui.button(t!("Bei WhatsApp speichern", "Save to WhatsApp")).clicked() {
            if !self.settings.profile_name.trim().is_empty() {
                let _ = self.cmd.send(Cmd::SetPushName(self.settings.profile_name.trim().to_owned()));
            }
            if !self.settings.profile_about.trim().is_empty() {
                let _ = self.cmd.send(Cmd::SetAbout(self.settings.profile_about.trim().to_owned()));
            }
            *changed = true;
        }
    }

    pub(crate) fn page_account(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let num = self.me.iter().find(|u| u.len() <= 15).cloned().unwrap_or_default();
        ui.label(RichText::new(t!("Telefonnummer", "Phone number")).small().color(pal.dim));
        ui.label(RichText::new(if num.is_empty() { "–".to_owned() } else { format!("+{num}") }).size(15.0));
        ui.add_space(8.0);
        ui.label(RichText::new(t!("Verbindung", "Connection")).small().color(pal.dim));
        ui.label(RichText::new(match self.conn {
            Conn::Online => t!("Verbunden", "Connected"),
            Conn::LoggedOut => t!("Abgemeldet", "Logged out"),
            Conn::NeedsQr => t!("Nicht verknüpft", "Not linked"),
            Conn::Starting => t!("Verbinde …", "Connecting …"),
        }));
        ui.add_space(16.0);
        ui.label(RichText::new(t!("Dieses Gerät erscheint am Handy unter WhatsApp → Verknüpfte Geräte. Nummer ändern, Konto löschen und Sicherheitsbenachrichtigungen gibt es nur in der Handy-App.", "This device appears on your phone under WhatsApp → Linked devices. Changing your number, deleting your account and security notifications are only available in the phone app.")).small().color(pal.dim));
        ui.add_space(12.0);
        if self.confirm == Some(1) {
            ui.label(RichText::new(t!("Wirklich abmelden? WhatsSnap wird vom Handy getrennt, danach ist ein neuer QR-Scan nötig.", "Really log out? WhatsSnap will be unlinked from your phone; you'll need to scan a new QR code.")).color(pal.danger));
            ui.horizontal(|ui| {
                if ui.button(t!("Ja, abmelden", "Yes, log out")).clicked() {
                    let _ = self.cmd.send(Cmd::Logout);
                    self.confirm = None;
                }
                if ui.button(t!("Abbrechen", "Cancel")).clicked() {
                    self.confirm = None;
                }
            });
        } else if ui.button(RichText::new(t!("Abmelden", "Log out")).color(pal.danger)).clicked() {
            self.confirm = Some(1);
        }
        ui.add_space(8.0);
        if self.confirm == Some(2) {
            ui.label(RichText::new(t!("Gespeicherten Verlauf, Medien, Profilbilder, Log und Zwischendateien auf diesem PC löschen? Die Chats am Handy bleiben.", "Delete saved history, media, profile photos, log and temporary files on this PC? Your chats on the phone stay.")).color(pal.danger));
            ui.horizontal(|ui| {
                if ui.button(t!("Ja, löschen", "Yes, delete")).clicked() {
                    self.wipe_local_data();
                    self.confirm = None;
                }
                if ui.button(t!("Abbrechen", "Cancel")).clicked() {
                    self.confirm = None;
                }
            });
        } else if ui.button(t!("Lokale Daten löschen …", "Delete local data …")).clicked() {
            self.confirm = Some(2);
        }
    }

    pub(crate) fn page_privacy(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        if !self.privacy_loaded && self.conn == Conn::Online {
            self.privacy_loaded = true;
            let _ = self.cmd.send(Cmd::FetchPrivacy);
            let _ = self.cmd.send(Cmd::FetchBlocklist);
        }
        if self.conn != Conn::Online {
            ui.label(RichText::new(t!("Nur verfügbar, wenn WhatsSnap verbunden ist.", "Only available while WhatsSnap is connected.")).color(pal.dim));
            return;
        }
        let opts_std: &[(&str, &str)] = &[("all", t!("Alle", "Everyone")), ("contacts", t!("Meine Kontakte", "My contacts")), ("none", t!("Niemand", "Nobody"))];
        let rows: [(&str, &str, &[(&str, &str)]); 6] = [
            ("last", t!("Zuletzt online", "Last seen"), opts_std),
            ("online", t!("Wer sieht, wenn ich online bin", "Who can see when I'm online"), &[("all", t!("Alle", "Everyone")), ("matchlastseen", t!("Wie „Zuletzt online“", "Same as last seen"))]),
            ("profile", t!("Profilbild", "Profile photo"), opts_std),
            ("status", t!("Info", "About"), opts_std),
            ("groupadd", t!("Wer mich zu Gruppen hinzufügen darf", "Who can add me to groups"), &[("all", t!("Alle", "Everyone")), ("contacts", t!("Meine Kontakte", "My contacts"))]),
            ("calladd", t!("Anrufe von Unbekannten", "Calls from unknown numbers"), &[("all", t!("Zulassen", "Allow")), ("known", t!("Stummschalten", "Silence"))]),
        ];
        let mut set: Option<(String, String)> = None;
        for (cat, label, opts) in rows {
            let cur = self.privacy.get(cat).cloned().unwrap_or_default();
            ui.add_space(6.0);
            ui.label(RichText::new(label).size(14.5));
            ui.horizontal_wrapped(|ui| {
                for (val, l) in opts {
                    if ui.radio(cur == *val, *l).clicked() && cur != *val {
                        set = Some((cat.to_owned(), (*val).to_owned()));
                    }
                }
            });
        }
        ui.add_space(8.0);
        let rr = self.privacy.get("readreceipts").map(|v| v != "none").unwrap_or(true);
        let mut rr2 = rr;
        if toggle(ui, &mut rr2, t!("Lesebestätigungen (blaue Haken)", "Read receipts (blue ticks)"), pal).changed() {
            set = Some(("readreceipts".into(), if rr2 { "all".into() } else { "none".into() }));
        }
        ui.label(RichText::new(t!("Wenn aus, siehst du auch die Lesebestätigungen anderer nicht. In Gruppen werden sie immer gesendet.", "If off, you won't see other people's read receipts either. Read receipts are always sent in groups.")).small().color(pal.dim));
        if let Some((c, v)) = set {
            self.privacy.insert(c.clone(), v.clone());
            let _ = self.cmd.send(Cmd::SetPrivacy { cat: c, val: v });
        }
        ui.add_space(14.0);
        ui.label(RichText::new(t!("Standarddauer für selbstlöschende Nachrichten (neue Chats)", "Default timer for disappearing messages (new chats)")).size(14.5));
        ui.horizontal_wrapped(|ui| {
            for (secs, l) in [(0u32, t!("Aus", "Off")), (86_400, t!("24 Stunden", "24 hours")), (604_800, t!("7 Tage", "7 days")), (7_776_000, t!("90 Tage", "90 days"))] {
                if ui.button(l).clicked() {
                    let _ = self.cmd.send(Cmd::SetDefaultDisappearing(secs));
                    self.info = Some(tf!("Standard für selbstlöschende Nachrichten: {l}", "Default for disappearing messages: {l}"));
                }
            }
        });
        ui.add_space(14.0);
        ui.label(RichText::new(tf!("Blockierte Kontakte ({})", "Blocked contacts ({})", self.blocklist.len())).size(14.5));
        let mut unblock = None;
        for j in &self.blocklist {
            let u = user_of(j);
            let name = self.names.get(&u).map(|n| n.0.clone()).unwrap_or_else(|| format!("+{u}"));
            ui.horizontal(|ui| {
                ui.label(name);
                if ui.small_button(t!("Freigeben", "Unblock")).clicked() {
                    unblock = Some(j.clone());
                }
            });
        }
        if let Some(j) = unblock.and_then(|j| j.parse::<Jid>().ok()) {
            let _ = self.cmd.send(Cmd::Block { jid: j, on: false });
        }
        ui.label(RichText::new(t!("Blockieren geht über das ⋮-Menü im Chat.", "Block someone via the ⋮ menu in their chat.")).small().color(pal.dim));
    }

    pub(crate) fn page_chats(&mut self, ui: &mut egui::Ui, pal: &Palette, changed: &mut bool, theme_changed: &mut bool) {
        ui.label(RichText::new("Sprache / Language").size(14.5));
        ui.horizontal(|ui| {
            for (v, l) in [("", t!("Systemsprache", "System language")), ("de", "Deutsch"), ("en", "English")] {
                if ui.radio(self.settings.lang == v, l).clicked() && self.settings.lang != v {
                    self.settings.lang = v.to_owned();
                    apply_lang(v);
                    *changed = true;
                }
            }
        });
        ui.add_space(8.0);
        ui.label(RichText::new(t!("Design", "Theme")).size(14.5));
        ui.horizontal(|ui| {
            *theme_changed |= ui.radio_value(&mut self.settings.dark, true, t!("Dunkel", "Dark")).changed();
            *theme_changed |= ui.radio_value(&mut self.settings.dark, false, t!("Hell", "Light")).changed();
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(t!("Schriftgröße", "Font size"));
            *theme_changed |= ui.add(egui::Slider::new(&mut self.settings.font_size, 12.0..=20.0)).changed();
        });
        ui.add_space(8.0);
        *changed |= toggle(ui, &mut self.settings.enter_sends, t!("Enter sendet die Nachricht (sonst Strg+Enter)", "Enter sends the message (otherwise Ctrl+Enter)"), pal).changed();
        ui.label(RichText::new(t!("Zeilenumbruch: Shift+Enter", "New line: Shift+Enter")).small().color(pal.dim));
    }

    pub(crate) fn page_notify(&mut self, ui: &mut egui::Ui, pal: &Palette, changed: &mut bool) {
        *changed |= toggle(ui, &mut self.settings.notifications, t!("Benachrichtigungen für Nachrichten", "Notifications for messages"), pal).changed();
        *changed |= toggle(ui, &mut self.settings.notify_groups, t!("Benachrichtigungen für Gruppen", "Notifications for groups"), pal).changed();
        *changed |= toggle(ui, &mut self.settings.preview, t!("Nachrichtentext anzeigen", "Show message text"), pal).changed();
        *changed |= toggle(ui, &mut self.settings.sound, t!("Ton abspielen", "Play sound"), pal).changed();
        ui.add_space(8.0);
        if ui.button(t!("Test-Benachrichtigung senden", "Send test notification")).clicked() {
            let _ = self.cmd.send(Cmd::TestNotify);
            if self.settings.sound {
                backend::play_notify_sound();
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new(t!("Einzelne Chats stummschalten: Rechtsklick auf den Chat. Das wird mit WhatsApp abgeglichen.", "Mute single chats: right-click the chat. This is synced with WhatsApp.")).small().color(pal.dim));
    }

    pub(crate) fn page_storage(&mut self, ui: &mut egui::Ui, pal: &Palette, changed: &mut bool) {
        *changed |= toggle(ui, &mut self.settings.autoload, t!("Fotos und Sticker automatisch herunterladen", "Download photos and stickers automatically"), pal).changed();
        ui.label(RichText::new(t!("Videos, Dokumente und Sprachnachrichten werden immer erst beim Anklicken geladen.", "Videos, documents and voice messages are only downloaded when you click them.")).small().color(pal.dim));
        ui.add_space(12.0);
        // Ordnergröße nicht in jedem Bild neu ausmessen: höchstens alle paar Sekunden.
        let size = match self.storage_size {
            Some((at, s)) if at.elapsed().as_secs() < 5 => s,
            _ => {
                let s = dir_size(&data_dir().join("media")) + dir_size(&data_dir().join("avatars"));
                self.storage_size = Some((std::time::Instant::now(), s));
                s
            }
        };
        ui.label(RichText::new(tf!("Belegt durch Medien: {}", "Used by media: {}", fmt_size(size))).size(14.5));
        if ui.button(t!("Heruntergeladene Medien löschen", "Delete downloaded media")).clicked() {
            if let Err(e) = std::fs::remove_dir_all(data_dir().join("media")) {
                backend::log(format!("Medien löschen: {e}"));
            }
            self.storage_size = None;
            self.images.clear();
            self.img_requested.clear();
            self.decoded.clear();
            self.decoded_order.clear();
            self.info = Some(t!("Medien gelöscht. Sie werden bei Bedarf neu geladen.", "Media deleted. It will be downloaded again when needed.").into());
        }
    }

    pub(crate) fn page_wasnap(&mut self, ui: &mut egui::Ui, pal: &Palette, changed: &mut bool) {
        *changed |= toggle(ui, &mut self.settings.background, t!("Beim Schließen im Hintergrund weiterlaufen (Symbol in der Leiste)", "Keep running in the background when closed (icon in the panel)"), pal).changed();
        if toggle(ui, &mut self.autostart, t!("Beim Anmelden am PC automatisch starten (unsichtbar in der Leiste)", "Start automatically when logging in (hidden in the panel)"), pal).changed() {
            if let Err(e) = set_autostart(self.autostart) {
                self.info = Some(tf!("Autostart: {e}", "Autostart: {e}"));
                self.autostart = !self.autostart;
            }
        }
        ui.add_space(8.0);
        *changed |= toggle(ui, &mut self.settings.show_online, t!("Online anzeigen, solange das Fenster aktiv ist", "Show me online while the window is active"), pal).changed();
        ui.label(RichText::new(t!("Aus: andere sehen dich über WhatsSnap nie online. Dein Handy bekommt dann weiter alle Benachrichtigungen.", "Off: others never see you online via WhatsSnap. Your phone keeps receiving all notifications.")).small().color(pal.dim));
        *changed |= toggle(ui, &mut self.settings.send_typing, t!("Anderen „schreibt …“ anzeigen", "Show others when I'm typing"), pal).changed();
        ui.add_space(8.0);
        ui.label(RichText::new(t!("Zusatzfunktionen", "Extra features")).size(14.5));
        *changed |= toggle(ui, &mut self.settings.keep_deleted, t!("Gelöschte Nachrichten anderer weiter anzeigen", "Keep showing messages others deleted"), pal).changed();
        if toggle(ui, &mut self.settings.debug_log, t!("Ausführliches Log (eine Zeile pro Nachricht, zur Fehlersuche)", "Verbose log (one line per message, for troubleshooting)"), pal).changed() {
            backend::DEBUG_LOG.store(self.settings.debug_log, std::sync::atomic::Ordering::Relaxed);
            *changed = true;
        }
        ui.label(RichText::new(t!("Bleiben mit „🚫 gelöscht“ markiert sichtbar. Wirkt auch für ältere, schon gespeicherte Löschungen (nach Neustart). Fotos nur, wenn sie vorher geladen waren.", "They stay visible, marked “🚫 deleted”. Also applies to older deletions already saved (after a restart). Photos only if they were downloaded before.")).small().color(pal.dim));
        ui.label(RichText::new(t!("Frühere Fassungen bearbeiteter Nachrichten: Maus über „Bearbeitet ⓘ“ halten. Mehr als 3 Chats anheften geht immer; ab dem 4. nur in WhatsSnap.", "Earlier versions of edited messages: hover over “Edited ⓘ”. You can pin more than 3 chats; from the 4th on they're pinned in WhatsSnap only.")).small().color(pal.dim));
        ui.add_space(12.0);
        ui.label(RichText::new(t!("GIF-Suche: Giphy-API-Schlüssel (kostenlos auf developers.giphy.com)", "GIF search: Giphy API key (free at developers.giphy.com)")).size(14.0));
        *changed |= ui.add(egui::TextEdit::singleline(&mut self.settings.giphy_key).desired_width(360.0).margin(egui::vec2(10.0, 7.0))).changed();
        ui.add_space(12.0);
        ui.label(RichText::new(t!("Tastenkürzel", "Keyboard shortcuts")).size(14.5));
        for (k, d) in [
            (t!("Strg+F", "Ctrl+F"), t!("Chats und Nachrichten durchsuchen", "Search chats and messages")),
            (t!("Strg+N", "Ctrl+N"), t!("Neuer Chat (Nummer eingeben)", "New chat (enter number)")),
            ("Alt+↑ / Alt+↓", t!("Vorheriger / nächster Chat", "Previous / next chat")),
            ("Esc", t!("Antwort, Suche oder Chat schließen", "Close reply, search or chat")),
            (t!("Strg+V", "Ctrl+V"), t!("Bild aus der Zwischenablage senden", "Send image from clipboard")),
            ("Shift+Enter", t!("Neue Zeile", "New line")),
        ] {
            ui.horizontal(|ui| {
                ui.label(RichText::new(k).code());
                ui.label(RichText::new(d).color(pal.dim));
            });
        }
    }

    pub(crate) fn page_help(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        ui.label(format!("WhatsSnap {}", env!("CARGO_PKG_VERSION")));
        ui.label(RichText::new(t!("Inoffizieller WhatsApp-Client für Linux. Nicht von WhatsApp oder Meta.", "Unofficial WhatsApp client for Linux. Not affiliated with WhatsApp or Meta.")).small().color(pal.dim));
        ui.add_space(8.0);
        ui.hyperlink_to(t!("Projekt auf GitHub", "Project on GitHub"), "https://github.com/Xinzoooo/wa-snap");
        ui.hyperlink_to(t!("Fehler melden", "Report a bug"), "https://github.com/Xinzoooo/wa-snap/issues");
        ui.add_space(8.0);
        if ui.button(t!("Log-Ordner öffnen", "Open log folder")).clicked() {
            safety::spawn_opener(data_dir().as_os_str());
        }
        ui.add_space(12.0);
        ui.label(RichText::new(t!("Lizenzen: WhatsSnap MIT · Schrift Inter (OFL 1.1) · Emojis Fluent Emoji (MIT, Microsoft) und Twemoji (CC-BY 4.0) · libopus, libwebp (BSD)", "Licenses: WhatsSnap MIT · Inter font (OFL 1.1) · Emojis Fluent Emoji (MIT, Microsoft) and Twemoji (CC-BY 4.0) · libopus, libwebp (BSD)")).small().color(pal.dim));
    }
}
