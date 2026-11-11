mod backend;
mod store;
mod theme;

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver};

use backend::{ChatMessage, Cmd, HistChat, UiEvent};
use eframe::egui::{self, Color32, RichText};
use store::{Rec, Store};
use theme::Palette;
use whatsapp_rust::Jid;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(default)]
struct Settings {
    notifications: bool,
    preview: bool,
    font_size: f32,
    dark: bool,
    muted: Vec<String>,
    pinned: Vec<String>,
    archived: Vec<String>,
    /// Eigener Name und Info-Text (wie bei WhatsApp), nur zum Bearbeiten hier.
    profile_name: String,
    profile_about: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            notifications: true,
            preview: true,
            font_size: 14.0,
            dark: true,
            muted: Vec::new(),
            pinned: Vec::new(),
            archived: Vec::new(),
            profile_name: String::new(),
            profile_about: String::new(),
        }
    }
}

fn data_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
        })
        .or_else(|| std::env::var_os("APPDATA").map(std::path::PathBuf::from))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = base.join("wa-snap");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn settings_path() -> std::path::PathBuf {
    data_dir().join("settings.json")
}

fn load_settings() -> Settings {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn autostart_file() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("autostart/wa-snap.desktop"))
}

fn set_autostart(on: bool) -> std::io::Result<()> {
    let Some(file) = autostart_file() else { return Ok(()) };
    if on {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let exe = std::env::current_exe()?;
        let entry = include_str!("../assets/wa-snap.desktop")
            .replace("@EXEC@", &format!("\"{}\"", exe.display()));
        std::fs::write(file, entry)
    } else if file.exists() {
        std::fs::remove_file(file)
    } else {
        Ok(())
    }
}

fn set_flag(list: &mut Vec<String>, key: String, on: bool) {
    let pos = list.iter().position(|k| k == &key);
    match (on, pos) {
        (true, None) => list.push(key),
        (false, Some(i)) => {
            list.remove(i);
        }
        _ => {}
    }
}

fn notify(title: String, body: String) {
    // Eigener Thread: die Oberfläche wartet nie auf den Benachrichtigungsdienst.
    std::thread::spawn(move || {
        let r = notify_rust::Notification::new()
            .appname("wa-snap")
            .summary(&title)
            .body(&body)
            .icon("wa-snap")
            .show();
        match r {
            Ok(_) => backend::log("Benachrichtigung gesendet"),
            Err(e) => {
                backend::log(format!("Benachrichtigung (D-Bus) fehlgeschlagen: {e}; versuche notify-send"));
                let r2 = std::process::Command::new("notify-send")
                    .args(["-a", "wa-snap", "-i", "wa-snap", &title, &body])
                    .status();
                backend::log(format!("notify-send: {r2:?}"));
            }
        }
    });
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn fmt_time(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// Kurze Zeit für die Chatliste: heute Uhrzeit, gestern "Gestern", sonst Datum.
fn fmt_list_time(ts: i64) -> String {
    use chrono::TimeZone;
    if ts <= 0 {
        return String::new();
    }
    let Some(d) = chrono::Local.timestamp_opt(ts, 0).single() else { return String::new() };
    let today = chrono::Local::now().date_naive();
    let day = d.date_naive();
    if day == today {
        d.format("%H:%M").to_string()
    } else if today.signed_duration_since(day).num_days() == 1 {
        "Gestern".to_owned()
    } else {
        d.format("%d.%m.%y").to_string()
    }
}

fn day_label(ts: i64) -> String {
    use chrono::TimeZone;
    let Some(d) = chrono::Local.timestamp_opt(ts, 0).single() else { return String::new() };
    let today = chrono::Local::now().date_naive();
    let day = d.date_naive();
    match today.signed_duration_since(day).num_days() {
        0 => "Heute".to_owned(),
        1 => "Gestern".to_owned(),
        _ => d.format("%d.%m.%Y").to_string(),
    }
}

fn day_key(ts: i64) -> i32 {
    use chrono::{Datelike, TimeZone};
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.date_naive().num_days_from_ce())
        .unwrap_or(0)
}

struct Chat {
    jid: Jid,
    name: String,
    is_group: bool,
    messages: Vec<ChatMessage>,
    unread: u32,
    last_ts: i64,
}

#[derive(PartialEq)]
enum Conn {
    Starting,
    NeedsQr,
    Online,
    LoggedOut,
}

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Live,
    History,
    Disk,
}

struct App {
    rx: Receiver<UiEvent>,
    cmd: tokio::sync::mpsc::UnboundedSender<Cmd>,
    store: Store,
    conn: Conn,
    status: String,
    qr: Option<String>,
    chats: HashMap<String, Chat>,
    names: HashMap<String, (String, bool)>,
    avatars: HashMap<String, egui::TextureHandle>,
    avatar_requested: HashSet<String>,
    selected: Option<String>,
    search: String,
    input: String,
    error: Option<String>,
    settings: Settings,
    show_settings: bool,
    show_archive: bool,
    autostart: bool,
    focused: bool,
}

fn draw_avatar(ui: &mut egui::Ui, tex: Option<&egui::TextureHandle>, name: &str, size: f32) {
    if let Some(t) = tex {
        ui.add(egui::Image::new((t.id(), egui::vec2(size, size))).corner_radius(size / 2.0));
        return;
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let initial = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into());
    let hue = name.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) % 6;
    let colors = [
        Color32::from_rgb(0x2f, 0x9e, 0x8a),
        Color32::from_rgb(0x5b, 0x7f, 0xd1),
        Color32::from_rgb(0x9b, 0x6b, 0xc9),
        Color32::from_rgb(0xd0, 0x80, 0x45),
        Color32::from_rgb(0x3f, 0x9b, 0xb8),
        Color32::from_rgb(0xc2, 0x5b, 0x6f),
    ];
    ui.painter().circle_filled(rect.center(), size / 2.0, colors[hue as usize]);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        egui::FontId::proportional(size * 0.44),
        Color32::WHITE,
    );
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = load_settings();
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, settings.dark, settings.font_size);

        let (tx, rx) = channel();
        let ctx = cc.egui_ctx.clone();
        let dir = data_dir();
        let cmd = backend::spawn(dir.clone(), tx, move || ctx.request_repaint());
        let (store, recs) = Store::open(&dir.join("history.jsonl"));

        let mut app = Self {
            rx,
            cmd,
            store,
            conn: Conn::Starting,
            status: "Starte …".into(),
            qr: None,
            chats: HashMap::new(),
            names: HashMap::new(),
            avatars: HashMap::new(),
            avatar_requested: HashSet::new(),
            selected: None,
            search: String::new(),
            input: String::new(),
            error: None,
            settings,
            show_settings: false,
            show_archive: false,
            autostart: autostart_file().map(|f| f.exists()).unwrap_or(false),
            focused: true,
        };
        for r in recs {
            app.apply_rec(r);
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("WA_SNAP_DEMO").is_some() {
            app.load_demo();
        }
        app
    }

    /// Nur Debug-Build: Beispieldaten für Screenshots ohne Konto (WA_SNAP_DEMO=1).
    #[cfg(debug_assertions)]
    fn load_demo(&mut self) {
        let now = now_ts();
        let people = [
            ("4915100000001", "Anna Berger", false),
            ("4915100000002", "Max Keller", false),
            ("120363000000001", "Familie", true),
            ("4915100000004", "Lena", false),
        ];
        let texts = [
            "Hey, bist du später noch online?",
            "Klar, ich melde mich gegen 20 Uhr.
Dann können wir alles besprechen.",
            "Perfekt, bis dann! 👍",
        ];
        for (i, (num, name, group)) in people.iter().enumerate() {
            let server = if *group { whatsapp_rust::Server::Group } else { whatsapp_rust::Server::Pn };
            let jid = Jid::new(*num, server);
            let c = self.chat_entry(&jid, *group);
            c.name = (*name).to_owned();
            c.unread = if i == 0 { 2 } else { 0 };
            for (j, t) in texts.iter().enumerate() {
                let m = ChatMessage {
                    chat: jid.clone(),
                    is_group: *group,
                    from_me: j % 2 == 1,
                    sender_name: (*name).to_owned(),
                    text: (*t).to_owned(),
                    ts: now - 86_400 * (i as i64 / 2) - 600 * (3 - j as i64),
                    id: format!("demo-{i}-{j}"),
                };
                self.add_message(m, Source::Disk);
            }
        }
        self.selected = Some(Jid::new("4915100000001", whatsapp_rust::Server::Pn).to_string());
        self.conn = Conn::Online;
    }

    fn apply_rec(&mut self, rec: Rec) {
        match rec {
            Rec::Msg { chat, is_group, from_me, sender, text, ts, id } => {
                let Ok(jid) = chat.parse::<Jid>() else { return };
                self.add_message(
                    ChatMessage { chat: jid, is_group, from_me, sender_name: sender, text, ts, id },
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
        }
    }

    fn chat_entry(&mut self, jid: &Jid, is_group: bool) -> &mut Chat {
        self.chats.entry(jid.to_string()).or_insert_with(|| Chat {
            jid: jid.clone(),
            name: jid.user.to_string(),
            is_group,
            messages: Vec::new(),
            unread: 0,
            last_ts: 0,
        })
    }

    fn display_name(&self, chat: &Chat) -> String {
        if let Some((n, _)) = self.names.get(chat.jid.user.as_str()) {
            return n.clone();
        }
        if chat.is_group && chat.name == chat.jid.user.as_str() {
            return format!("Gruppe {}", chat.jid.user);
        }
        chat.name.clone()
    }

    fn save_settings(&self) {
        if let Ok(t) = serde_json::to_string_pretty(&self.settings) {
            let _ = std::fs::write(settings_path(), t);
        }
    }

    fn is_muted(&self, key: &str) -> bool {
        self.settings.muted.iter().any(|m| m == key)
    }

    /// Nachricht einsortieren. Gibt false zurück, wenn es ein Duplikat war.
    fn add_message(&mut self, m: ChatMessage, src: Source) -> bool {
        let key = m.chat.to_string();
        let selected = self.selected.as_deref() == Some(key.as_str());
        let chat = self.chat_entry(&m.chat.clone(), m.is_group);
        // Gesprächsname: bei Einzelchats der Name des Gegenübers.
        if !m.is_group
            && !m.from_me
            && !m.sender_name.is_empty()
            && chat.name == chat.jid.user.as_str()
        {
            chat.name = m.sender_name.clone();
        }
        if !m.id.is_empty() && chat.messages.iter().any(|x| x.id == m.id) {
            return false;
        }
        if src == Source::Live && !m.from_me && !selected {
            chat.unread += 1;
        }
        chat.last_ts = chat.last_ts.max(m.ts);
        let rec = Rec::Msg {
            chat: key.clone(),
            is_group: m.is_group,
            from_me: m.from_me,
            sender: m.sender_name.clone(),
            text: m.text.clone(),
            ts: m.ts,
            id: m.id.clone(),
        };
        let notify_data = (m.from_me, m.ts, m.sender_name.clone(), m.text.clone(), m.is_group);
        chat.messages.push(m);
        let n = chat.messages.len();
        if n > 1 && chat.messages[n - 2].ts > chat.messages[n - 1].ts {
            chat.messages.sort_by_key(|x| x.ts);
        }
        if src != Source::Disk {
            self.store.append(&rec);
        }
        let (from_me, ts, sender, text, is_group) = notify_data;
        if src == Source::Live && !from_me {
            backend::log(format!(
                "Benachrichtigung? einstellung={} stumm={} alt={}s gewaehlt={} fokus={}",
                self.settings.notifications,
                self.is_muted(&key),
                now_ts() - ts,
                selected,
                self.focused
            ));
        }
        if src == Source::Live
            && !from_me
            && now_ts() - ts < 120
            && self.settings.notifications
            && !self.is_muted(&key)
            && (!selected || !self.focused)
        {
            let title = self.chats.get(&key).map(|c| self.display_name(c)).unwrap_or_default();
            let body = if !self.settings.preview {
                "Neue Nachricht".to_owned()
            } else if is_group && !sender.is_empty() {
                format!("{sender}: {text}")
            } else {
                text
            };
            notify(title, body);
        }
        true
    }

    fn ingest_history(&mut self, chats: Vec<HistChat>) {
        for h in chats {
            let name = h.name.clone();
            let c = self.chat_entry(&h.jid, h.is_group);
            if !name.is_empty() {
                c.name = name.clone();
            }
            c.unread = c.unread.max(h.unread);
            c.last_ts = c.last_ts.max(h.last_ts);
            let last_ts = c.last_ts;
            self.store.append(&Rec::Chat {
                jid: h.jid.to_string(),
                name,
                is_group: h.is_group,
                last_ts,
            });
            for m in h.msgs {
                self.add_message(m, Source::History);
            }
        }
    }

    fn pump(&mut self, ctx: &egui::Context) {
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
                }
                UiEvent::LoggedOut => self.conn = Conn::LoggedOut,
                UiEvent::Message(m) => {
                    self.add_message(m, Source::Live);
                }
                UiEvent::History(c) => self.ingest_history(c),
                UiEvent::Contact { user, name, from_book } => {
                    // Adressbuch-Namen überschreiben Push-Namen, nicht umgekehrt.
                    let keep = matches!(self.names.get(&user), Some((_, true))) && !from_book;
                    let same = self.names.get(&user).map(|(n, _)| n == &name).unwrap_or(false);
                    if !keep && !same {
                        self.store.append(&Rec::Name {
                            user: user.clone(),
                            name: name.clone(),
                            book: from_book,
                        });
                        self.names.insert(user, (name, from_book));
                    }
                }
                UiEvent::Avatar { user, size, rgba } => {
                    let img = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                    let tex =
                        ctx.load_texture(format!("av-{user}"), img, egui::TextureOptions::LINEAR);
                    self.avatars.insert(user, tex);
                }
                UiEvent::SendFailed(e) => self.error = Some(format!("Senden fehlgeschlagen: {e}")),
                UiEvent::Info(m) => self.error = Some(m),
                UiEvent::Groups(list) => {
                    for (jid, name) in list {
                        if name.is_empty() {
                            continue;
                        }
                        let c = self.chat_entry(&jid, true);
                        c.name = name.clone();
                        let last_ts = c.last_ts;
                        self.store.append(&Rec::Chat {
                            jid: jid.to_string(),
                            name,
                            is_group: true,
                            last_ts,
                        });
                    }
                }
                UiEvent::Muted { jid, on } => {
                    set_flag(&mut self.settings.muted, jid, on);
                    self.save_settings();
                }
                UiEvent::Pinned { jid, on } => {
                    set_flag(&mut self.settings.pinned, jid, on);
                    self.save_settings();
                }
                UiEvent::Archived { jid, on } => {
                    set_flag(&mut self.settings.archived, jid, on);
                    self.save_settings();
                }
            }
        }
    }

    fn send_current(&mut self) {
        let text = self.input.trim().to_owned();
        let Some(key) = self.selected.clone() else { return };
        if text.is_empty() {
            return;
        }
        if let Some(chat) = self.chats.get(&key) {
            let jid = chat.jid.clone();
            let m = ChatMessage {
                chat: jid.clone(),
                is_group: chat.is_group,
                from_me: true,
                sender_name: String::new(),
                text: text.clone(),
                ts: now_ts(),
                id: String::new(),
            };
            self.add_message(m, Source::Live);
            let _ = self.cmd.send(Cmd::Send { chat: jid, text });
        }
        self.input.clear();
    }

    fn qr_view(&self, ui: &mut egui::Ui, data: &str) {
        let Ok(code) = qrcode::QrCode::new(data.as_bytes()) else {
            ui.label("QR-Code konnte nicht erzeugt werden.");
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
                    let min = rect.min
                        + egui::vec2((x as f32 + quiet) * cell, (y as f32 + quiet) * cell);
                    p.rect_filled(
                        egui::Rect::from_min_size(min, egui::vec2(cell, cell)),
                        0.0,
                        Color32::BLACK,
                    );
                }
            }
        }
    }

    fn settings_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, pal: &Palette) {
        let mut changed = false;
        let mut theme_changed = false;
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("Einstellungen").size(22.0).strong());
                ui.add_space(12.0);
                ui.label(RichText::new("WhatsApp-Profil").strong().color(pal.dim));
                ui.label(RichText::new("Name").small().color(pal.dim));
                ui.add(egui::TextEdit::singleline(&mut self.settings.profile_name).desired_width(300.0));
                ui.label(RichText::new("Info").small().color(pal.dim));
                ui.add(egui::TextEdit::singleline(&mut self.settings.profile_about).desired_width(300.0));
                if ui.button("Profil bei WhatsApp speichern").clicked() {
                    if !self.settings.profile_name.trim().is_empty() {
                        let _ = self.cmd.send(Cmd::SetPushName(self.settings.profile_name.trim().to_owned()));
                    }
                    if !self.settings.profile_about.trim().is_empty() {
                        let _ = self.cmd.send(Cmd::SetAbout(self.settings.profile_about.trim().to_owned()));
                    }
                    changed = true;
                }
                ui.label(
                    RichText::new("Stumm, Anheften und Archiv werden im Chat-Kopf mit WhatsApp abgeglichen.")
                        .small()
                        .color(pal.dim),
                );
                ui.add_space(14.0);
                ui.label(RichText::new("Benachrichtigungen").strong().color(pal.dim));
                changed |= ui
                    .checkbox(&mut self.settings.notifications, "Bei neuen Nachrichten benachrichtigen")
                    .changed();
                changed |= ui
                    .checkbox(&mut self.settings.preview, "Nachrichtentext anzeigen")
                    .changed();
                ui.label(
                    RichText::new(format!("{} Chats stummgeschaltet", self.settings.muted.len()))
                        .small()
                        .color(pal.dim),
                );
                ui.add_space(14.0);
                ui.label(RichText::new("Darstellung").strong().color(pal.dim));
                ui.horizontal(|ui| {
                    theme_changed |=
                        ui.radio_value(&mut self.settings.dark, true, "Dunkel").changed();
                    theme_changed |=
                        ui.radio_value(&mut self.settings.dark, false, "Hell").changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Schriftgröße");
                    theme_changed |= ui
                        .add(egui::Slider::new(&mut self.settings.font_size, 11.0..=20.0))
                        .changed();
                });
                ui.add_space(14.0);
                ui.label(RichText::new("System").strong().color(pal.dim));
                if ui
                    .checkbox(&mut self.autostart, "Beim Anmelden automatisch starten")
                    .changed()
                {
                    if let Err(e) = set_autostart(self.autostart) {
                        self.error = Some(format!("Autostart: {e}"));
                        self.autostart = !self.autostart;
                    }
                }
            });
        });
        if theme_changed {
            theme::apply(ctx, self.settings.dark, self.settings.font_size);
        }
        if changed || theme_changed {
            self.save_settings();
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.focused = ctx.input(|i| i.focused);
        self.pump(ctx);
        let pal = Palette::of(self.settings.dark);

        if self.conn != Conn::Online && self.chats.is_empty() {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(56.0);
                    match self.conn {
                        Conn::NeedsQr => {
                            ui.label(RichText::new("Mit WhatsApp verknüpfen").size(24.0).strong());
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(
                                    "Handy: WhatsApp → Verknüpfte Geräte → Gerät hinzufügen",
                                )
                                .color(pal.dim),
                            );
                            ui.add_space(18.0);
                            if let Some(q) = self.qr.clone() {
                                self.qr_view(ui, &q);
                            }
                        }
                        Conn::LoggedOut => {
                            ui.label(RichText::new("Abgemeldet").size(24.0).strong());
                            ui.label(
                                RichText::new("Die Verknüpfung wurde auf dem Handy entfernt. Bitte neu starten und erneut scannen.")
                                    .color(pal.dim),
                            );
                        }
                        _ => {
                            ui.label(RichText::new("wa-snap").size(24.0).strong());
                            ui.label(RichText::new(&self.status).color(pal.dim));
                            ui.add_space(8.0);
                            ui.spinner();
                        }
                    }
                });
            });
            return;
        }

        // ---------- linke Spalte: Chatliste ----------
        let mut want_avatars: Vec<Jid> = Vec::new();
        egui::SidePanel::left("chats")
            .resizable(true)
            .default_width(340.0)
            .min_width(260.0)
            .frame(egui::Frame::new().fill(pal.side).stroke(egui::Stroke::new(1.0_f32, pal.border)))
            .show(ctx, |ui| {
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(RichText::new("Chats").size(22.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        let (txt, col) = match self.conn {
                            Conn::Online => ("online", pal.accent),
                            Conn::LoggedOut => ("abgemeldet", Color32::from_rgb(0xe0, 0x5a, 0x5a)),
                            _ => ("verbinde …", Color32::from_rgb(0xd9, 0xa4, 0x2b)),
                        };
                        let label = if self.show_settings { "Chats" } else { "Einstellungen" };
                        if ui.add(egui::Button::new(RichText::new(label).small())).clicked() {
                            self.show_settings = !self.show_settings;
                        }
                        ui.label(RichText::new(format!("● {txt}")).small().color(col));
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Suchen oder Nummer eingeben")
                            .margin(egui::vec2(10.0, 7.0))
                            .desired_width(ui.available_width() - 14.0),
                    );
                });
                ui.add_space(6.0);

                let needle = self.search.to_lowercase();
                let digits: String = self.search.chars().filter(|c| c.is_ascii_digit()).collect();
                let looks_like_number =
                    self.search.chars().all(|c| c.is_ascii_digit() || c == '+' || c == ' ');
                if digits.len() >= 7 && looks_like_number {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        if ui.button(format!("Chat mit +{digits} starten")).clicked() {
                            let jid = Jid::pn(digits.clone());
                            let key = jid.to_string();
                            let c = self.chat_entry(&jid, false);
                            c.name = format!("+{digits}");
                            self.selected = Some(key);
                            self.show_settings = false;
                            self.search.clear();
                        }
                    });
                }

                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    let n_arch = self.settings.archived.len();
                    let lbl = if self.show_archive {
                        "← Zurück zu Chats".to_owned()
                    } else {
                        format!("Archiviert ({n_arch})")
                    };
                    if n_arch > 0 || self.show_archive {
                        if ui.add(egui::Button::new(RichText::new(lbl).small())).clicked() {
                            self.show_archive = !self.show_archive;
                        }
                    }
                });
                let (pins, arch, show_archive) =
                    (&self.settings.pinned, &self.settings.archived, self.show_archive);
                let mut rows: Vec<(String, String, i64)> = self
                    .chats
                    .iter()
                    .filter(|(k, _)| arch.contains(*k) == show_archive)
                    .map(|(k, c)| (k.clone(), self.display_name(c), c.last_ts))
                    .filter(|(_, n, _)| needle.is_empty() || n.to_lowercase().contains(&needle))
                    .collect();
                rows.sort_by_key(|r| (!pins.contains(&r.0), std::cmp::Reverse(r.2)));

                let mut clicked = None;
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show_rows(
                    ui,
                    68.0,
                    rows.len(),
                    |ui, range| {
                        for (key, name, ts) in &rows[range] {
                            let c = &self.chats[key];
                            let sel = self.selected.as_deref() == Some(key.as_str());
                            let user = c.jid.user.to_string();
                            if !self.avatars.contains_key(&user)
                                && !self.avatar_requested.contains(&user)
                            {
                                want_avatars.push(c.jid.clone());
                            }
                            let (rect, resp) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 68.0),
                                egui::Sense::click(),
                            );
                            let inner_rect = rect.shrink2(egui::vec2(8.0, 3.0));
                            let bg = if sel {
                                pal.selected
                            } else if resp.hovered() {
                                pal.hover
                            } else {
                                Color32::TRANSPARENT
                            };
                            ui.painter().rect_filled(inner_rect, 12.0, bg);
                            let mut inner = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(inner_rect.shrink2(egui::vec2(10.0, 6.0)))
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                            );
                            draw_avatar(&mut inner, self.avatars.get(&user), name, 46.0);
                            inner.add_space(10.0);
                            let right_w = 52.0;
                            let text_w = (inner.available_width() - right_w).max(20.0);
                            inner.allocate_ui(egui::vec2(text_w, 46.0), |ui| {
                                ui.set_min_width(text_w);
                                ui.set_max_width(text_w);
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    let shown = if self.settings.pinned.contains(key) {
                                        format!("{name}  ·  angeheftet")
                                    } else {
                                        name.clone()
                                    };
                                    ui.add(
                                        egui::Label::new(RichText::new(shown).size(15.0).strong())
                                            .truncate(),
                                    );
                                    let last = c
                                        .messages
                                        .last()
                                        .map(|m| m.text.replace('\n', " "))
                                        .unwrap_or_default();
                                    ui.add(
                                        egui::Label::new(RichText::new(last).small().color(pal.dim))
                                            .truncate(),
                                    );
                                });
                            });
                            inner.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 4.0;
                                let col = if c.unread > 0 { pal.accent } else { pal.dim };
                                ui.label(RichText::new(fmt_list_time(*ts)).size(11.0).color(col));
                                if c.unread > 0 {
                                    ui.label(
                                        RichText::new(format!(" {} ", c.unread))
                                            .size(11.0)
                                            .strong()
                                            .color(pal.on_accent)
                                            .background_color(pal.accent),
                                    );
                                }
                            });
                            if resp.clicked() {
                                clicked = Some(key.clone());
                            }
                        }
                    },
                );
                if let Some(k) = clicked {
                    if let Some(c) = self.chats.get_mut(&k) {
                        c.unread = 0;
                    }
                    self.selected = Some(k);
                    self.show_settings = false;
                }
            });
        for jid in want_avatars.into_iter().take(12) {
            self.avatar_requested.insert(jid.user.to_string());
            let _ = self.cmd.send(Cmd::Avatar { jid });
        }

        // ---------- unten: Eingabe ----------
        let input_id = egui::Id::new("wa_input");
        let mut do_send = false;
        egui::TopBottomPanel::bottom("input")
            .frame(
                egui::Frame::new()
                    .fill(pal.bar)
                    .stroke(egui::Stroke::new(1.0_f32, pal.border))
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(ctx, |ui| {
                if let Some(e) = self.error.clone() {
                    ui.colored_label(Color32::from_rgb(0xe0, 0x5a, 0x5a), e);
                }
                let enabled =
                    self.selected.is_some() && self.conn == Conn::Online && !self.show_settings;
                if enabled
                    && ctx.memory(|m| m.has_focus(input_id))
                    && !ctx.input(|i| i.modifiers.shift)
                    && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                {
                    do_send = true;
                }
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.menu_button("Emoji", |ui| {
                            ui.set_max_width(300.0);
                            egui::Grid::new("emoji_grid").spacing([2.0, 2.0]).show(ui, |ui| {
                                for (i, e) in theme::EMOJIS.iter().enumerate() {
                                    if ui.add(egui::Button::new(*e).frame(false)).clicked() {
                                        self.input.push_str(e);
                                    }
                                    if (i + 1) % 8 == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                        });
                    });
                    let width = (ui.available_width() - 76.0 - 2.0 * ui.spacing().item_spacing.x - 4.0)
                        .max(120.0);
                    ui.add_enabled_ui(enabled, |ui| {
                        egui::ScrollArea::vertical()
                            .max_width(width)
                            .max_height(120.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut self.input)
                                        .id(input_id)
                                        .hint_text("Nachricht schreiben (Shift+Enter: neue Zeile)")
                                        .desired_rows(1)
                                        .margin(egui::vec2(12.0, 8.0))
                                        .desired_width(width - 4.0),
                                );
                            });
                    });
                    let btn = ui.add_enabled(
                        enabled,
                        egui::Button::new(RichText::new("Senden").strong().color(pal.on_accent))
                            .fill(pal.accent)
                            .min_size(egui::vec2(76.0, 34.0)),
                    );
                    if btn.clicked() {
                        do_send = true;
                    }
                });
            });
        if do_send {
            self.send_current();
            ctx.memory_mut(|m| m.request_focus(input_id));
        }

        // ---------- Mitte: Chat oder Einstellungen ----------
        let mut toggle_mute = false;
        let mut toggle_pin = false;
        let mut toggle_arch = false;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(pal.chat)).show(ctx, |ui| {
            if self.show_settings {
                self.settings_view(ui, ctx, &pal);
                return;
            }
            let Some(key) = self.selected.clone() else {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Wähle links einen Chat").color(pal.dim).size(18.0));
                });
                return;
            };
            let Some(chat) = self.chats.get(&key) else { return };
            let title = self.display_name(chat);
            let user = chat.jid.user.to_string();
            let muted = self.settings.muted.iter().any(|m| m == &key);
            let pinned = self.settings.pinned.iter().any(|m| m == &key);
            let archived = self.settings.archived.iter().any(|m| m == &key);
            egui::Frame::new()
                .fill(pal.bar)
                .stroke(egui::Stroke::new(1.0_f32, pal.border))
                .inner_margin(egui::Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        draw_avatar(ui, self.avatars.get(&user), &title, 40.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(RichText::new(&title).size(16.0).strong());
                            ui.label(
                                RichText::new(if chat.is_group { "Gruppe" } else { "Kontakt" })
                                    .small()
                                    .color(pal.dim),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let label = if muted { "Stumm" } else { "Laut" };
                            if ui
                                .add(egui::Button::new(RichText::new(label).small()))
                                .on_hover_text("Stummschalten (wird mit WhatsApp abgeglichen)")
                                .clicked()
                            {
                                toggle_mute = true;
                            }
                            let pl = if pinned { "Angeheftet" } else { "Anheften" };
                            if ui.add(egui::Button::new(RichText::new(pl).small())).clicked() {
                                toggle_pin = true;
                            }
                            let al = if archived { "Aus Archiv" } else { "Archivieren" };
                            if ui.add(egui::Button::new(RichText::new(al).small())).clicked() {
                                toggle_arch = true;
                            }
                        });
                    });
                });
            // Nur die letzten Nachrichten zeichnen: schnell auch bei langen Chats.
            let start = chat.messages.len().saturating_sub(300);
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add_space(10.0);
                    let mut last_day = i32::MIN;
                    for m in &chat.messages[start..] {
                        let dk = day_key(m.ts);
                        if dk != last_day {
                            last_day = dk;
                            ui.vertical_centered(|ui| {
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(format!(" {} ", day_label(m.ts)))
                                        .size(11.5)
                                        .color(pal.dim)
                                        .background_color(pal.hover),
                                );
                                ui.add_space(4.0);
                            });
                        }
                        let (bg, align) = if m.from_me {
                            (pal.bubble_me, egui::Align::Max)
                        } else {
                            (pal.bubble_them, egui::Align::Min)
                        };
                        ui.with_layout(egui::Layout::top_down(align), |ui| {
                            egui::Frame::new()
                                .fill(bg)
                                .stroke(egui::Stroke::new(
                                    if m.from_me { 0.0_f32 } else { 1.0_f32 },
                                    pal.border,
                                ))
                                .corner_radius(14.0)
                                .inner_margin(egui::Margin::symmetric(12, 8))
                                .outer_margin(egui::Margin::symmetric(16, 2))
                                .show(ui, |ui| {
                                    ui.set_max_width((ui.available_width() * 0.72).max(140.0));
                                    if chat.is_group && !m.from_me && !m.sender_name.is_empty() {
                                        ui.label(
                                            RichText::new(&m.sender_name)
                                                .small()
                                                .strong()
                                                .color(pal.accent),
                                        );
                                    }
                                    ui.label(&m.text);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Min),
                                        |ui| {
                                            ui.label(
                                                RichText::new(fmt_time(m.ts))
                                                    .size(10.5)
                                                    .color(pal.dim),
                                            );
                                        },
                                    );
                                });
                        });
                    }
                    ui.add_space(8.0);
                });
        });
        let Some(key) = self.selected.clone() else { return };
        let Some(jid) = self.chats.get(&key).map(|c| c.jid.clone()) else { return };
        if toggle_mute {
            let on = !self.is_muted(&key);
            set_flag(&mut self.settings.muted, key.clone(), on);
            let _ = self.cmd.send(Cmd::SetMute { jid: jid.clone(), on });
        }
        if toggle_pin {
            let on = !self.settings.pinned.contains(&key);
            set_flag(&mut self.settings.pinned, key.clone(), on);
            let _ = self.cmd.send(Cmd::SetPin { jid: jid.clone(), on });
        }
        if toggle_arch {
            let on = !self.settings.archived.contains(&key);
            set_flag(&mut self.settings.archived, key.clone(), on);
            let _ = self.cmd.send(Cmd::SetArchive { jid, on });
        }
        if toggle_mute || toggle_pin || toggle_arch {
            self.save_settings();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.cmd.send(Cmd::Quit);
    }
}

/// `wa-snap --install-desktop`: legt Startmenü-Eintrag und Icon an, damit das Programm
/// im Anwendungsmenü erscheint und an die Taskleiste angeheftet werden kann (Linux).
fn install_desktop() -> std::io::Result<()> {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| std::io::Error::other("HOME ist nicht gesetzt"))?;
    let share = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let exe = std::env::current_exe()?;
    let apps = share.join("applications");
    let icons = share.join("icons/hicolor/scalable/apps");
    std::fs::create_dir_all(&apps)?;
    std::fs::create_dir_all(&icons)?;
    std::fs::write(icons.join("wa-snap.svg"), include_str!("../assets/wa-snap.svg"))?;
    let desktop = include_str!("../assets/wa-snap.desktop")
        .replace("@EXEC@", &format!("\"{}\"", exe.display()));
    std::fs::write(apps.join("wa-snap.desktop"), desktop)?;
    let _ = std::process::Command::new("update-desktop-database").arg(&apps).status();
    let _ = std::process::Command::new("gtk-update-icon-cache")
        .args(["-f", "-t"])
        .arg(share.join("icons/hicolor"))
        .status();
    println!(
        "Fertig. wa-snap steht jetzt im Anwendungsmenü und kann an die Taskleiste angeheftet werden."
    );
    Ok(())
}

fn main() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--install-desktop") {
        if let Err(e) = install_desktop() {
            eprintln!("Installation fehlgeschlagen: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1040.0, 700.0])
            .with_min_inner_size([640.0, 420.0])
            .with_app_id("wa-snap")
            .with_title("wa-snap"),
        ..Default::default()
    };
    eframe::run_native("wa-snap", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
