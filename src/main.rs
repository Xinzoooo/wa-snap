#[macro_use]
mod i18n;
mod audio;
mod backend;
mod call;
mod emoji;
mod store;
mod theme;
mod tray;

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
    /// Fenster schließen lässt WhatsSnap im Tray weiterlaufen.
    background: bool,
    /// Ende einer befristeten Stummschaltung (Unix-Sekunden) je Nummer/LID.
    muted_until: HashMap<String, i64>,
    /// Eigener Giphy-API-Schlüssel für die GIF-Suche.
    giphy_key: String,
    /// Benachrichtigungen auch für Gruppen
    notify_groups: bool,
    /// Ton bei Benachrichtigungen
    sound: bool,
    /// Enter sendet (sonst Strg+Enter)
    enter_sends: bool,
    /// Fotos und Sticker automatisch laden
    autoload: bool,
    /// Online anzeigen, solange das Fenster aktiv ist
    show_online: bool,
    /// „schreibt …“ an andere senden
    send_typing: bool,
    /// Zuletzt benutzte Emojis
    recent_emojis: Vec<String>,
    /// Hautton 0 = Standard, 1–5 = hell bis dunkel
    skin_tone: u8,
    /// Von anderen gelöschte Nachrichten weiter anzeigen
    keep_deleted: bool,
    /// Nur lokal angeheftete Chats (über die 3 von WhatsApp hinaus)
    local_pins: Vec<String>,
    /// Sprache: "" = Systemsprache, "de" oder "en"
    lang: String,
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
            background: true,
            muted_until: HashMap::new(),
            giphy_key: String::new(),
            notify_groups: true,
            sound: true,
            enter_sends: true,
            autoload: true,
            show_online: true,
            send_typing: true,
            recent_emojis: Vec::new(),
            skin_tone: 0,
            keep_deleted: false,
            local_pins: Vec::new(),
            lang: String::new(),
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
            .replace("@EXEC@", &format!("\"{}\" --hidden", exe.display()));
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
        t!("Gestern", "Yesterday").to_owned()
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
        0 => t!("Heute", "Today").to_owned(),
        1 => t!("Gestern", "Yesterday").to_owned(),
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

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Unread,
    Groups,
}

fn mute_opts() -> [(&'static str, Option<i64>); 3] {
    [(t!("8 Stunden", "8 hours"), Some(8 * 3600)), (t!("1 Woche", "1 week"), Some(7 * 86_400)), (t!("Immer", "Always"), None)]
}

/// Aktion auf eine einzelne Nachricht.
enum MsgAct {
    React(String, String),
    Reply(String),
    Revoke(String),
    Forward(String),
    Star(String, bool),
    Edit(String),
    /// Umfrage: neue Auswahl
    Vote(String, Vec<String>),
    /// Zu Nachricht springen (Suche, Sterne)
    Jump(String),
    /// Chat mit Nummer öffnen (Kontaktkarte)
    OpenNumber(String),
}

/// Aktion aus Kontextmenü oder Chat-Kopf.
enum Act {
    /// Stummschalten umschalten; Sekunden = Dauer (None = dauerhaft).
    Mute(String, Option<i64>),
    Pin(String),
    Archive(String),
    Read(String),
}

struct App {
    rx: Receiver<UiEvent>,
    cmd: tokio::sync::mpsc::UnboundedSender<Cmd>,
    store: Store,
    conn: Conn,
    status: String,
    qr: Option<String>,
    chats: HashMap<String, Chat>,
    /// Nachrichten-ID → Chat-Schlüssel (für Lesebestätigungen).
    msg_index: HashMap<String, String>,
    names: HashMap<String, (String, bool)>,
    avatars: HashMap<String, egui::TextureHandle>,
    avatar_requested: HashSet<String>,
    selected: Option<String>,
    search: String,
    input: String,
    info: Option<String>,
    settings: Settings,
    show_settings: bool,
    show_archive: bool,
    filter: Filter,
    chat_search: Option<String>,
    autostart: bool,
    focused: bool,
    notify: backend::SharedNotify,
    names_dirty: bool,
    temp_counter: u64,
    emoji: emoji::Cache,
    /// Bilder/Sticker je Nachrichten-ID (Textur, volle Größe?).
    images: HashMap<String, (egui::TextureHandle, bool)>,
    img_requested: HashSet<String>,
    /// Reaktionen je Nachrichten-ID: (Absender, Emoji); Absender leer = ich.
    reactions: HashMap<String, Vec<(String, String)>>,
    /// Antwort in Vorbereitung: (Chat, Zitat, zitierte Nachricht ist von mir).
    reply_to: Option<(String, backend::Quote, bool)>,
    me: Vec<String>,
    drafts: HashMap<String, String>,
    /// Tippt gerade: Chat-Nummer → (Absender, nimmt auf, Zeitpunkt)
    typing: HashMap<String, (String, bool, std::time::Instant)>,
    /// Online-Status: Nummer → (online, zuletzt online)
    presence: HashMap<String, (bool, Option<i64>)>,
    /// Zuletzt gesendeter eigener Tippstatus (Chat, Zeitpunkt)
    my_typing: Option<(String, std::time::Instant)>,
    /// Reihenfolge der sichtbaren Chats (für Alt+↑/↓)
    row_keys: Vec<String>,
    at_bottom: bool,
    new_below: u32,
    scroll_bottom: bool,
    jump_to: Option<String>,
    highlight: Option<(String, std::time::Instant)>,
    /// Weiterleiten-Dialog: (Nachrichten-ID, Suchtext)
    forward: Option<(String, String)>,
    /// Bild aus der Zwischenablage, wartet auf Bestätigung
    pending_paste: Option<(std::path::PathBuf, egui::TextureHandle)>,
    starred: HashSet<String>,
    /// Umfrage-ID → Abstimmender (leer = ich) → gewählte Optionen
    votes: HashMap<String, HashMap<String, Vec<String>>>,
    /// Gruppe → Mitglieder (Nummer/LID, Telefonnummer)
    members: HashMap<String, Vec<(String, String)>>,
    /// Nachricht wird bearbeitet: (Chat, Nachrichten-ID)
    editing: Option<(String, String)>,
    show_starred: bool,
    /// Wie viele Nachrichten je Chat angezeigt werden
    show_limit: HashMap<String, usize>,
    /// Gefundene Nachrichten bei der Suche einblenden
    show_msg_results: bool,
    /// 0 = Emoji, 1 = GIF, 2 = Sticker
    picker_tab: u8,
    player: Option<audio::Playback>,
    /// Pausierte Position je Sprachnachricht
    paused_at: HashMap<String, f32>,
    /// Dekodierte Sprachnachrichten: PCM, Kanäle, Wellenform
    decoded: HashMap<String, (std::sync::Arc<Vec<i16>>, usize, Vec<u8>)>,
    /// Abspielen, sobald die Datei da ist: (ID, Startposition)
    want_play: Option<(String, f32)>,
    recording: Option<audio::Recording>,
    /// Animierte Sticker: Einzelbilder, Dauer je Bild (ms)
    anims: HashMap<String, (Vec<egui::TextureHandle>, Vec<u32>)>,
    gif_query: String,
    gif_results: Vec<(backend::GifItem, Option<egui::TextureHandle>)>,
    gif_loading: bool,
    /// Einstellungsseite: 0 Profil, 1 Konto, 2 Datenschutz, 3 Chats, 4 Benachrichtigungen, 5 Speicher, 6 WhatsSnap, 7 Hilfe
    settings_page: u8,
    privacy: HashMap<String, String>,
    privacy_loaded: bool,
    blocklist: Vec<String>,
    confirm: Option<u8>,
    emoji_search: String,
    emoji_jump: Option<usize>,
    show_info: bool,
    group_info: HashMap<String, backend::GroupDetails>,
    group_requested: HashSet<String>,
    add_numbers: String,
    rename_buf: String,
    /// Neue Gruppe: (Name, ausgewählte Chats, zusätzliche Nummern)
    new_group: Option<(String, HashSet<String>, String)>,
    /// Klingelnder Anruf: (ID, Nummer, Name, Video)
    call_in: Option<(String, String, String, bool)>,
    /// Laufender Anruf: (Nummer, Zustand, Beginn, stumm)
    call: Option<(String, String, std::time::Instant, bool)>,
    show_status: bool,
    /// Ausgewählte Person im Status-Bereich ("" = ich) und Position
    status_sel: Option<(String, usize)>,
    status_seen: HashSet<String>,
    status_text: String,
    status_color: usize,
    channels: Vec<backend::Channel>,
    channels_loaded: bool,
    /// Ausgewählter Kanal und seine Beiträge
    channel_sel: Option<String>,
    channel_posts: HashMap<String, Vec<backend::ChannelPost>>,
    channel_search: String,
    channel_results: Vec<backend::Channel>,
    channel_link: String,
    /// Kanal anlegen: (Name, Beschreibung)
    channel_new: Option<(String, String)>,
    channel_post_text: String,
    channel_refresh: Option<std::time::Instant>,
    /// 0 = Status, 1 = Kanäle finden
    news_tab: u8,
}

fn user_of(key: &str) -> String {
    key.split('@').next().unwrap_or("").split(':').next().unwrap_or("").to_owned()
}

fn draw_avatar(ui: &mut egui::Ui, tex: Option<&egui::TextureHandle>, name: &str, size: f32, dark: bool) {
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
    let (bg, fg) = if dark {
        (Color32::from_rgb(0x2e, 0x2f, 0x2f), Color32::from_rgb(0xa5, 0xa5, 0xa5))
    } else {
        (Color32::from_rgb(0xdf, 0xe5, 0xe7), Color32::from_rgb(0x8a, 0x96, 0x9c))
    };
    ui.painter().circle_filled(rect.center(), size / 2.0, bg);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        egui::FontId::new(size * 0.42, theme::bold()),
        fg,
    );
}

/// Haken wie bei WhatsApp: einer = gesendet, zwei = zugestellt, zwei blau = gelesen.
fn draw_ticks(ui: &mut egui::Ui, status: u8, pal: &Palette) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(17.0, 11.0), egui::Sense::hover());
    let p = ui.painter();
    match status {
        0 => {
            // Uhr: wird gesendet
            p.circle_stroke(rect.center(), 4.5, egui::Stroke::new(1.2_f32, pal.dim));
            p.line_segment([rect.center(), rect.center() + egui::vec2(0.0, -3.0)], egui::Stroke::new(1.2_f32, pal.dim));
            p.line_segment([rect.center(), rect.center() + egui::vec2(2.0, 0.0)], egui::Stroke::new(1.2_f32, pal.dim));
        }
        9 => {
            p.text(rect.center(), egui::Align2::CENTER_CENTER, "!", egui::FontId::new(12.0, theme::bold()), pal.danger);
        }
        _ => {
            let col = if status >= 3 { pal.tick_read } else { pal.dim };
            let st = egui::Stroke::new(1.4_f32, col);
            let tick = |dx: f32| {
                let a = rect.left_center() + egui::vec2(dx + 1.0, 0.5);
                let b = a + egui::vec2(3.0, 3.0);
                let c = b + egui::vec2(6.5, -7.5);
                p.line_segment([a, b], st);
                p.line_segment([b, c], st);
            };
            if status >= 2 {
                tick(0.0);
                tick(5.0);
            } else {
                tick(3.0);
            }
        }
    }
}

/// WhatsApp-Formatierung: *fett*, _kursiv_, ~durchgestrichen~, `Code`/```Code```.
/// Ergebnis: Textstücke mit Stil-Bits (1 fett, 2 kursiv, 4 durchgestrichen, 8 Code).
fn fmt_spans(t: &str, base: u8) -> Vec<(String, u8)> {
    let chars: Vec<char> = t.chars().collect();
    let mut out: Vec<(String, u8)> = Vec::new();
    let mut buf = String::new();
    let boundary = |c: Option<&char>| c.map(|c| !c.is_alphanumeric()).unwrap_or(true);
    let mut i = 0;
    let mut base = base;
    while i < chars.len() {
        let c = chars[i];
        // Erwähnungen sind mit U+E000 … U+E001 eingerahmt (siehe with_mention_names)
        if c == '\u{E000}' || c == '\u{E001}' {
            if !buf.is_empty() {
                out.push((std::mem::take(&mut buf), base));
            }
            if c == '\u{E000}' {
                base |= 16;
            } else {
                base &= !16;
            }
            i += 1;
            continue;
        }
        if base & 8 == 0 && matches!(c, '*' | '_' | '~' | '`') {
            // ```Block```
            let triple = c == '`' && chars.get(i + 1) == Some(&'`') && chars.get(i + 2) == Some(&'`');
            let mlen = if triple { 3 } else { 1 };
            let open_ok = (i == 0 || boundary(chars.get(i - 1))) && chars.get(i + mlen).map(|n| !n.is_whitespace()).unwrap_or(false);
            if open_ok {
                let mut j = i + mlen;
                let mut close = None;
                while j < chars.len() {
                    if chars[j] == '\n' && !triple {
                        break;
                    }
                    let is_close = if triple {
                        chars[j] == '`' && chars.get(j + 1) == Some(&'`') && chars.get(j + 2) == Some(&'`')
                    } else {
                        chars[j] == c
                    };
                    if is_close && j > i + mlen && !chars[j - 1].is_whitespace() && boundary(chars.get(j + mlen)) {
                        close = Some(j);
                        break;
                    }
                    j += 1;
                }
                if let Some(j) = close {
                    if !buf.is_empty() {
                        out.push((std::mem::take(&mut buf), base));
                    }
                    let inner: String = chars[i + mlen..j].iter().collect();
                    let bit = match c {
                        '*' => 1,
                        '_' => 2,
                        '~' => 4,
                        _ => 8,
                    };
                    if bit == 8 {
                        out.push((inner, base | 8));
                    } else {
                        out.extend(fmt_spans(&inner, base | bit));
                    }
                    i = j + mlen;
                    continue;
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    if !buf.is_empty() {
        out.push((buf, base));
    }
    out
}

fn has_format(t: &str) -> bool {
    t.contains(['*', '_', '~', '`', '\u{E000}'])
}

fn styled(t: String, style: u8, size: f32) -> RichText {
    let mut r = RichText::new(t).size(size);
    if style & 8 != 0 {
        r = r.code();
    }
    if style & 1 != 0 {
        r = r.family(theme::bold());
    }
    if style & 2 != 0 {
        r = r.italics();
    }
    if style & 4 != 0 {
        r = r.strikethrough();
    }
    if style & 16 != 0 {
        r = r.color(Color32::from_rgb(0x21, 0xc0, 0x63)).family(theme::bold());
    }
    r
}

/// Text mit bunten Emojis in einer Umbruch-Zeile ausgeben. Nur Emojis (bis 3) werden groß gezeigt.
fn rich_label(ui: &mut egui::Ui, text: &str, size: f32, cache: &mut emoji::Cache) {
    if !emoji::has_emoji(text) && !has_format(text) {
        ui.add(egui::Label::new(RichText::new(text).size(size)).selectable(true));
        return;
    }
    let segs = emoji::segments(text);
    let esz = emoji_size(&segs, size);
    let old = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = 0.0;
    for s in segs {
        match s {
            emoji::Seg::Text(t) => {
                for (part, style) in fmt_spans(t, 0) {
                    ui.add(egui::Label::new(styled(part, style, size)).selectable(true));
                }
            }
            emoji::Seg::Emoji(k) => match cache.get(ui.ctx(), &k) {
                Some(id) => {
                    ui.add(egui::Image::new((id, egui::vec2(esz, esz))));
                }
                None => {
                    ui.label(RichText::new(k).size(size));
                }
            },
        }
    }
    ui.spacing_mut().item_spacing.x = old;
}

fn emoji_size(segs: &[emoji::Seg<'_>], size: f32) -> f32 {
    let only = segs.iter().all(|s| match s {
        emoji::Seg::Emoji(_) => true,
        emoji::Seg::Text(t) => t.trim().is_empty(),
    });
    let n = segs.iter().filter(|s| matches!(s, emoji::Seg::Emoji(_))).count();
    if only && n <= 3 { size * 2.6 } else { size * 1.3 }
}

/// Ungefähre Breite von Text mit Emojis (für die Blasenbreite).
fn rich_width(ui: &egui::Ui, text: &str, size: f32, max_w: f32) -> f32 {
    let font = egui::FontId::proportional(size);
    if !emoji::has_emoji(text) {
        return ui.fonts(|f| f.layout(text.to_owned(), font, Color32::WHITE, max_w).size().x);
    }
    let segs = emoji::segments(text);
    let esz = emoji_size(&segs, size);
    let mut line = 0.0_f32;
    let mut widest = 0.0_f32;
    for s in &segs {
        match s {
            emoji::Seg::Emoji(_) => line += esz,
            emoji::Seg::Text(t) => {
                let mut parts = t.split('\n').peekable();
                while let Some(part) = parts.next() {
                    line += ui.fonts(|f| f.layout_no_wrap(part.to_owned(), font.clone(), Color32::WHITE).size().x);
                    if parts.peek().is_some() {
                        widest = widest.max(line);
                        line = 0.0;
                    }
                }
            }
        }
    }
    widest.max(line).min(max_w)
}

/// Eine Zeile Text mit bunten Emojis direkt zeichnen (Chatliste).
fn paint_rich_line(
    painter: &egui::Painter,
    ctx: &egui::Context,
    cache: &mut emoji::Cache,
    left_center: egui::Pos2,
    text: &str,
    font: egui::FontId,
    color: Color32,
) {
    let mut x = left_center.x;
    let max_x = painter.clip_rect().right();
    let esz = font.size * 1.2;
    for s in emoji::segments(text) {
        if x > max_x {
            break;
        }
        match s {
            emoji::Seg::Text(t) => {
                let g = painter.layout_no_wrap(t.to_owned(), font.clone(), color);
                let w = g.size().x;
                painter.galley(egui::pos2(x, left_center.y - g.size().y / 2.0), g, color);
                x += w;
            }
            emoji::Seg::Emoji(k) => {
                if let Some(id) = cache.get(ctx, &k) {
                    let r = egui::Rect::from_min_size(egui::pos2(x, left_center.y - esz / 2.0), egui::vec2(esz, esz));
                    painter.image(id, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                }
                x += esz + 1.0;
            }
        }
    }
}

/// Schalter wie bei WhatsApp (Text links, grüner Schalter rechts).
fn toggle(ui: &mut egui::Ui, on: &mut bool, label: &str, pal: &Palette) -> egui::Response {
    let w = ui.available_width().min(520.0);
    let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(w, 40.0), egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let p = ui.painter();
    let galley = p.layout(label.to_owned(), egui::FontId::proportional(14.5), pal.text, w - 70.0);
    p.galley(egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, pal.text);
    let t = ui.ctx().animate_bool(resp.id, *on);
    let sw = egui::Rect::from_center_size(egui::pos2(rect.right() - 24.0, rect.center().y), egui::vec2(38.0, 20.0));
    let bg = if *on { pal.accent } else { pal.border.gamma_multiply(2.0) };
    p.rect_filled(sw, 10.0, bg);
    let knob = egui::pos2(egui::lerp(sw.left() + 10.0..=sw.right() - 10.0, t), sw.center().y);
    p.circle_filled(knob, 7.5, if *on { pal.on_accent } else { pal.text });
    resp
}

/// Runder Filter-Knopf ("Alle", "Ungelesen", …).
fn chip(ui: &mut egui::Ui, label: &str, on: bool, pal: &Palette) -> bool {
    let font = egui::FontId::new(13.5, if on { theme::bold() } else { egui::FontFamily::Proportional });
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, if on { pal.chip_on_text } else { pal.dim });
    let size = egui::vec2(galley.size().x + 24.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let p = ui.painter();
    if on {
        p.rect_filled(rect, 15.0, pal.chip_on);
    } else {
        p.rect_filled(rect, 15.0, if resp.hovered() { pal.hover } else { Color32::TRANSPARENT });
        p.rect_stroke(rect, 15.0, egui::Stroke::new(1.0_f32, pal.border), egui::StrokeKind::Inside);
    }
    p.galley(rect.center() - galley.size() / 2.0, galley, pal.text);
    resp.clicked()
}

/// Kleiner runder Symbolknopf ohne Rahmen.
fn icon_button(ui: &mut egui::Ui, icon: &str, size: f32, color: Color32, pal: &Palette, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size + 14.0, size + 14.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().circle_filled(rect.center(), rect.width() / 2.0, pal.hover);
    }
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(size), color);
    resp.on_hover_text(tip)
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, core: Core) -> Self {
        let mut settings = load_settings();
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
            img_requested: HashSet::new(),
            reactions: HashMap::new(),
            reply_to: None,
            me: Vec::new(),
            drafts: HashMap::new(),
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

    fn apply_rec(&mut self, rec: Rec) {
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

    fn save_settings(&self) {
        if let Ok(t) = serde_json::to_string_pretty(&self.settings) {
            let _ = std::fs::write(settings_path(), t);
        }
    }

    fn has(list: &[String], key: &str) -> bool {
        let u = user_of(key);
        list.iter().any(|m| *m == u)
    }

    fn set_status(&mut self, id: &str, st: u8, persist: bool) {
        let Some(key) = self.msg_index.get(id).cloned() else { return };
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

    fn persist_msg(&mut self, key: &str, m: &ChatMessage) {
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
    fn add_message(&mut self, m: ChatMessage, src: Source) -> bool {
        let key = m.chat.to_string();
        let selected = self.selected.as_deref() == Some(key.as_str());
        let focused = self.focused;
        let chat = self.chat_entry(&m.chat.clone(), m.is_group);
        // Gesprächsname: bei Einzelchats der Name des Gegenübers.
        if !m.is_group && !m.from_me && !m.sender_name.is_empty() && chat.name == chat.jid.user.as_str() {
            chat.name = m.sender_name.clone();
        }
        if !m.id.is_empty() && chat.messages.iter().rev().take(400).any(|x| x.id == m.id) {
            return false;
        }
        if src == Source::Live && !m.from_me && !(selected && focused) {
            chat.unread += 1;
        }
        chat.last_ts = chat.last_ts.max(m.ts);
        chat.messages.push(m.clone());
        let n = chat.messages.len();
        if n > 1 && chat.messages[n - 2].ts > chat.messages[n - 1].ts {
            chat.messages.sort_by_key(|x| x.ts);
        }
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
            self.store.append(&Rec::Chat { jid: h.jid.to_string(), name, is_group: h.is_group, last_ts });
            for m in h.msgs {
                self.add_message(m, Source::History);
            }
        }
    }

    /// Chat als gelesen markieren: lokal und bei WhatsApp (Lesebestätigungen).
    fn mark_read(&mut self, key: &str) {
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

    /// Sprachnachricht ab `from` Sekunden abspielen (vorher laufende stoppen).
    fn start_play(&mut self, id: &str, from: f32) {
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
    fn audio_action(&mut self, id: String, media: backend::MediaRef, seek: Option<f32>) {
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
    fn select_chat(&mut self, key: String) {
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

    fn run_msg_act(&mut self, a: MsgAct) {
        let id = match &a {
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
            MsgAct::Jump(_) | MsgAct::OpenNumber(_) => {}
        }
    }

    fn set_reaction(&mut self, target: String, from: String, emoji: String, persist: bool) {
        if persist {
            self.store.append(&Rec::React { target: target.clone(), from: from.clone(), emoji: emoji.clone() });
        }
        let list = self.reactions.entry(target).or_default();
        list.retain(|(f, _)| *f != from);
        if !emoji.is_empty() {
            list.push((from, emoji));
        }
    }

    fn find_msg_mut(&mut self, id: &str) -> Option<&mut ChatMessage> {
        let key = self.msg_index.get(id)?.clone();
        self.chats.get_mut(&key)?.messages.iter_mut().rev().find(|m| m.id == id)
    }

    fn revoke_local(&mut self, id: &str, persist: bool) {
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

    fn edit_local(&mut self, id: &str, text: String, persist: bool) {
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
    fn known_stickers(&self) -> Vec<(String, backend::MediaRef)> {
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

    fn last_msg(&self, key: &str) -> Option<backend::LastMsg> {
        let m = self.chats.get(key)?.messages.iter().rev().find(|m| !m.id.starts_with("local-"))?;
        Some(backend::LastMsg { id: m.id.clone(), from_me: m.from_me, sender: m.sender.clone(), ts: m.ts })
    }

    fn apply_flag(&mut self, which: u8, users: Vec<String>, on: bool) {
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

    fn run_act(&mut self, act: Act) {
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
                    let _ = self.cmd.send(Cmd::FetchBlocklist);
                    if !self.settings.show_online {
                        let _ = self.cmd.send(Cmd::SetOnline(false));
                    }
                }
                UiEvent::LoggedOut => self.conn = Conn::LoggedOut,
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
                UiEvent::Sent { chat, temp, id } => {
                    let mut done = None;
                    if let Some(c) = self.chats.get_mut(&chat) {
                        if let Some(m) = c.messages.iter_mut().rev().find(|m| m.id == temp) {
                            m.id = id.clone();
                            m.status = m.status.max(1);
                            done = Some(m.clone());
                        }
                    }
                    if let Some(m) = done {
                        if let Some(t) = self.images.remove(&temp) {
                            self.images.insert(id.clone(), t);
                        }
                        self.msg_index.insert(id, chat.clone());
                        self.persist_msg(&chat, &m);
                    }
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
                UiEvent::FileReady { id, path } => {
                    if self.want_play.as_ref().map(|w| w.0 == id).unwrap_or(false) {
                        let (_, from) = self.want_play.take().unwrap_or_default();
                        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| audio::decode_ogg_opus(&b)) {
                            Ok((pcm, ch)) => {
                                let wf = audio::waveform(&pcm, ch);
                                self.decoded.insert(id.clone(), (std::sync::Arc::new(pcm), ch, wf));
                                self.start_play(&id, from);
                            }
                            Err(e) => {
                                // Nicht dekodierbar (z. B. MP3/M4A): mit dem Standardprogramm öffnen
                                backend::log(format!("Audio {id}: {e}"));
                                let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
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
                    if on {
                        self.typing.insert(chat, (from, recording, std::time::Instant::now()));
                    } else {
                        self.typing.remove(&chat);
                    }
                }
                UiEvent::Presence { user, online, last_seen } => {
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
    fn sync_notify_state(&mut self) {
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

    fn send_current(&mut self) {
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
            let _ = self.cmd.send(Cmd::Send { chat: jid, text, temp, quote, mentions, expiration });
        }
        self.input.clear();
    }

    fn qr_view(&self, ui: &mut egui::Ui, data: &str) {
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

/// Dezentes Hintergrundmuster im Chat (wie die Kritzeleien bei WhatsApp, nur ruhiger).
fn paint_pattern(ui: &egui::Ui, rect: egui::Rect, pal: &Palette) {
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, pal.chat);
    let step = 46.0;
    let mut y = rect.top() - (rect.top() % step);
    let mut row = 0;
    while y < rect.bottom() + step {
        let off = if row % 2 == 0 { 0.0 } else { step / 2.0 };
        let mut x = rect.left() - (rect.left() % step) + off;
        while x < rect.right() + step {
            let c = egui::pos2(x, y);
            match (row + (x / step) as i32) % 3 {
                0 => {
                    p.circle_stroke(c, 5.0, egui::Stroke::new(1.3_f32, pal.pattern));
                }
                1 => {
                    p.line_segment([c + egui::vec2(-5.0, 0.0), c + egui::vec2(5.0, 0.0)], egui::Stroke::new(1.3_f32, pal.pattern));
                    p.line_segment([c + egui::vec2(0.0, -5.0), c + egui::vec2(0.0, 5.0)], egui::Stroke::new(1.3_f32, pal.pattern));
                }
                _ => {
                    p.rect_stroke(
                        egui::Rect::from_center_size(c, egui::vec2(8.0, 8.0)),
                        2.0,
                        egui::Stroke::new(1.3_f32, pal.pattern),
                        egui::StrokeKind::Middle,
                    );
                }
            }
            x += step;
        }
        y += step;
        row += 1;
    }
}

impl App {
    fn settings_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, pal: &Palette) {
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

    fn page_profile(&mut self, ui: &mut egui::Ui, pal: &Palette, changed: &mut bool) {
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

    fn page_account(&mut self, ui: &mut egui::Ui, pal: &Palette) {
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
            ui.label(RichText::new(t!("Gespeicherten Verlauf, Medien und Profilbilder auf diesem PC löschen? Die Chats am Handy bleiben.", "Delete saved history, media and profile photos on this PC? Your chats on the phone stay.")).color(pal.danger));
            ui.horizontal(|ui| {
                if ui.button(t!("Ja, löschen", "Yes, delete")).clicked() {
                    let d = data_dir();
                    let _ = std::fs::write(d.join("history.jsonl"), b"");
                    let _ = std::fs::remove_dir_all(d.join("media"));
                    let _ = std::fs::remove_dir_all(d.join("avatars"));
                    self.chats.clear();
                    self.msg_index.clear();
                    self.selected = None;
                    self.images.clear();
                    self.img_requested.clear();
                    self.info = Some(t!("Lokale Daten gelöscht. Neue Nachrichten kommen weiter an.", "Local data deleted. New messages keep arriving.").into());
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

    fn page_privacy(&mut self, ui: &mut egui::Ui, pal: &Palette) {
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

    fn page_chats(&mut self, ui: &mut egui::Ui, pal: &Palette, changed: &mut bool, theme_changed: &mut bool) {
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

    fn page_notify(&mut self, ui: &mut egui::Ui, pal: &Palette, changed: &mut bool) {
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

    fn page_storage(&mut self, ui: &mut egui::Ui, pal: &Palette, changed: &mut bool) {
        *changed |= toggle(ui, &mut self.settings.autoload, t!("Fotos und Sticker automatisch herunterladen", "Download photos and stickers automatically"), pal).changed();
        ui.label(RichText::new(t!("Videos, Dokumente und Sprachnachrichten werden immer erst beim Anklicken geladen.", "Videos, documents and voice messages are only downloaded when you click them.")).small().color(pal.dim));
        ui.add_space(12.0);
        let size = dir_size(&data_dir().join("media")) + dir_size(&data_dir().join("avatars"));
        ui.label(RichText::new(tf!("Belegt durch Medien: {}", "Used by media: {}", fmt_size(size))).size(14.5));
        if ui.button(t!("Heruntergeladene Medien löschen", "Delete downloaded media")).clicked() {
            let _ = std::fs::remove_dir_all(data_dir().join("media"));
            self.images.clear();
            self.img_requested.clear();
            self.decoded.clear();
            self.info = Some(t!("Medien gelöscht. Sie werden bei Bedarf neu geladen.", "Media deleted. It will be downloaded again when needed.").into());
        }
    }

    fn page_wasnap(&mut self, ui: &mut egui::Ui, pal: &Palette, changed: &mut bool) {
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

    fn page_help(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        ui.label(format!("WhatsSnap {}", env!("CARGO_PKG_VERSION")));
        ui.label(RichText::new(t!("Inoffizieller WhatsApp-Client für Linux. Nicht von WhatsApp oder Meta.", "Unofficial WhatsApp client for Linux. Not affiliated with WhatsApp or Meta.")).small().color(pal.dim));
        ui.add_space(8.0);
        ui.hyperlink_to(t!("Projekt auf GitHub", "Project on GitHub"), "https://github.com/Xinzoooo/wa-snap");
        ui.hyperlink_to(t!("Fehler melden", "Report a bug"), "https://github.com/Xinzoooo/wa-snap/issues");
        ui.add_space(8.0);
        if ui.button(t!("Log-Ordner öffnen", "Open log folder")).clicked() {
            let _ = std::process::Command::new("xdg-open").arg(data_dir()).spawn();
        }
        ui.add_space(12.0);
        ui.label(RichText::new(t!("Lizenzen: WhatsSnap MIT · Schrift Inter (OFL 1.1) · Emojis Fluent Emoji (MIT, Microsoft) und Twemoji (CC-BY 4.0) · libopus, libwebp (BSD)", "Licenses: WhatsSnap MIT · Inter font (OFL 1.1) · Emojis Fluent Emoji (MIT, Microsoft) and Twemoji (CC-BY 4.0) · libopus, libwebp (BSD)")).small().color(pal.dim));
    }

    /// Emoji-Auswahl wie bei WhatsApp: Suche, Zuletzt, Kategorien, Hauttöne.
    fn emoji_grid(&mut self, ui: &mut egui::Ui, pal: &Palette) {
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
    fn rail(&mut self, ctx: &egui::Context, pal: &Palette) {
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

    fn chat_list(&mut self, ctx: &egui::Context, pal: &Palette) {
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
                    let this = &*self;
                    let mut hits: Vec<(i64, String, String, String)> = this
                        .chats
                        .values()
                        .flat_map(|c| {
                            c.messages
                                .iter()
                                .filter(|m| !m.text.is_empty() && m.text.to_lowercase().contains(&needle))
                                .map(move |m| (m.ts, m.id.clone(), this.display_name(c), m.text.replace('\n', " ")))
                        })
                        .collect();
                    hits.sort_by_key(|h| std::cmp::Reverse(h.0));
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
                let archived_n = self.chats.keys().filter(|k| Self::has(&self.settings.archived, k)).count();
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

    fn input_bar(&mut self, ctx: &egui::Context, pal: &Palette) {
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
                                            self.pending_paste = None;
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
                                                    match self.images.get(&id) {
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

    fn chat_view(&mut self, ctx: &egui::Context, pal: &Palette) {
        let mut act: Option<Act> = None;
        let mut ecache = std::mem::take(&mut self.emoji);
        let mut media_req: Vec<(String, backend::MediaRef)> = Vec::new();
        let mut open_req: Vec<(String, backend::MediaRef)> = Vec::new();
        let mut msg_act: Option<MsgAct> = None;
        let mut jump_req: Option<String> = None;
        let mut older_req = false;
        let mut audio_req: Option<(String, backend::MediaRef, Option<f32>)> = None;
        let mut chat_cmd: Option<Cmd> = None;
        let mut toggle_info = false;
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
                chat.messages.iter().filter(|m| m.text.to_lowercase().contains(&needle)).collect()
            };
            let mut area = egui::ScrollArea::vertical().auto_shrink([false; 2]).stick_to_bottom(true);
            if self.scroll_bottom {
                area = area.vertical_scroll_offset(f32::MAX);
            }
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
                        ui.with_layout(egui::Layout::top_down(align), |ui| {
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
                                        Some(md) if md.is_picture() => media_size(md, self.images.get(&m.id), max_w).x,
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
                    }
                    ui.add_space(10.0);
                });
            let dist = out.content_size.y - (out.state.offset.y + out.inner_rect.height());
            scroll_out = Some((dist, out.inner_rect));
        });
        self.emoji = ecache;
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
            let _ = self.cmd.send(Cmd::OpenMedia { id, media });
        }
        if let Some(a) = msg_act {
            self.run_msg_act(a);
        }
        if let Some(a) = act {
            self.run_act(a);
        }
    }

    /// "@nummer" durch "@Name" ersetzen und für die Hervorhebung markieren.
    fn with_mention_names(&self, text: &str, mentions: &[String]) -> String {
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
    fn extra_block(&self, ui: &mut egui::Ui, m: &ChatMessage, k: &backend::Extra, w: f32, pal: &Palette, msg_act: &mut Option<MsgAct>) {
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
                    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
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
                    let _ = std::process::Command::new("xdg-open").arg(u).spawn();
                }
                ui.add_space(3.0);
            }
        }
    }

    /// Übersicht der mit Stern markierten Nachrichten.
    fn starred_view(&mut self, ui: &mut egui::Ui, pal: &Palette) {
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
    fn audio_block(&self, ui: &mut egui::Ui, m: &ChatMessage, md: &backend::MediaRef, max_w: f32, pal: &Palette, audio_req: &mut Option<(String, backend::MediaRef, Option<f32>)>) {
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
    fn media_block(
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
        let tex = self.images.get(&m.id);
        let auto = self.settings.autoload || m.from_me || self.img_requested.contains(&format!("{}#ok", m.id));
        if auto && !self.img_requested.contains(&m.id) && tex.map(|t| !t.1).unwrap_or(true) {
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
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, t!("Lädt …", "Loading …"), egui::FontId::proportional(13.0), pal.dim);
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

fn fmt_last_seen(ts: i64) -> String {
    let label = day_label(ts);
    let t = fmt_time(ts);
    match label.as_str() {
        "Heute" => tf!("heute um {t}", "today at {t}"),
        "Gestern" => tf!("gestern um {t}", "yesterday at {t}"),
        d => tf!("am {d} um {t}", "on {d} at {t}"),
    }
}

/// 1234 → "1.234", 1234567 → "1,2 Mio."
fn fmt_count(n: u64) -> String {
    if n >= 1_000_000 {
        tf!("{} Mio.", "{}M", format!("{:.1}", n as f64 / 1e6).replace('.', ","))
    } else if n >= 1000 {
        format!("{}.{:03}", n / 1000, n % 1000)
    } else {
        n.to_string()
    }
}

fn apply_lang(lang: &str) {
    i18n::set_en(match lang {
        "en" => true,
        "de" => false,
        _ => !i18n::system_is_german(),
    });
}

fn dir_size(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| rd.flatten().map(|e| e.metadata().map(|m| if m.is_dir() { dir_size(&e.path()) } else { m.len() }).unwrap_or(0)).sum())
        .unwrap_or(0)
}

fn fmt_size(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / 1048576.0)
    } else {
        format!("{} KB", (n / 1024).max(1))
    }
}

/// Anzeigegröße eines Bildes/Stickers in der Blase.
fn media_size(md: &backend::MediaRef, tex: Option<&(egui::TextureHandle, bool)>, max_w: f32) -> egui::Vec2 {
    let (w, h) = match tex {
        Some((t, _)) => (t.size()[0] as f32, t.size()[1] as f32),
        None if md.w > 0 && md.h > 0 => (md.w as f32, md.h as f32),
        None => (300.0, 300.0),
    };
    let limit = if md.kind == "sticker" { 150.0 } else { 320.0_f32.min(max_w - 16.0) };
    let scale = (limit / w).min(380.0 / h);
    egui::vec2(w * scale, h * scale)
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let was_focused = self.focused;
        self.focused = ctx.input(|i| i.focused);
        self.pump(ctx);
        // Online-Status wie bei WhatsApp Web: online, solange das Fenster aktiv ist.
        if self.focused != was_focused && self.conn == Conn::Online {
            let _ = self.cmd.send(Cmd::SetOnline(self.focused && self.settings.show_online));
        }
        self.shortcuts(ctx);
        // Fenster wieder im Vordergrund: offenen Chat als gelesen markieren.
        if self.focused && !was_focused {
            if let Some(k) = self.selected.clone() {
                if self.chats.get(&k).map(|c| c.unread > 0).unwrap_or(false) {
                    self.mark_read(&k);
                }
            }
        }
        self.sync_notify_state();
        if self.player.as_mut().map(|p| p.finished()).unwrap_or(false) {
            if let Some(p) = self.player.take() {
                self.paused_at.remove(&p.id);
                p.stop();
            }
        }
        let unread: u32 = self
            .chats
            .iter()
            .filter(|(k, c)| c.unread > 0 && c.jid.server != whatsapp_rust::Server::Broadcast && !Self::has(&self.settings.archived, k) && !Self::has(&self.settings.muted, k))
            .map(|(_, c)| c.unread)
            .sum();
        tray::BADGE.store(unread, std::sync::atomic::Ordering::Relaxed);
        let pal = Palette::of(self.settings.dark);

        if self.conn != Conn::Online && self.chats.is_empty() {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(pal.side)).show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(56.0);
                    match self.conn {
                        Conn::NeedsQr => {
                            ui.label(RichText::new(t!("Mit WhatsApp verknüpfen", "Link with WhatsApp")).size(26.0).family(theme::bold()));
                            ui.add_space(6.0);
                            ui.label(RichText::new(t!("1. WhatsApp auf dem Handy öffnen", "1. Open WhatsApp on your phone")).color(pal.dim));
                            ui.label(RichText::new(t!("2. Einstellungen → Verknüpfte Geräte → Gerät hinzufügen", "2. Settings → Linked devices → Link a device")).color(pal.dim));
                            ui.label(RichText::new(t!("3. Diesen Code scannen", "3. Scan this code")).color(pal.dim));
                            ui.add_space(18.0);
                            if let Some(q) = self.qr.clone() {
                                self.qr_view(ui, &q);
                            }
                        }
                        Conn::LoggedOut => {
                            ui.label(RichText::new(t!("Abgemeldet", "Logged out")).size(24.0).family(theme::bold()));
                            ui.label(
                                RichText::new(t!("Die Verknüpfung wurde auf dem Handy entfernt. Bitte neu starten und erneut scannen.", "The link was removed on the phone. Please restart and scan again."))
                                    .color(pal.dim),
                            );
                        }
                        _ => {
                            ui.label(RichText::new(t!("WhatsSnap", "WhatsSnap")).size(26.0).family(theme::bold()));
                            ui.label(RichText::new(&self.status).color(pal.dim));
                            ui.add_space(8.0);
                            ui.spinner();
                        }
                    }
                });
            });
            return;
        }

        self.rail(ctx, &pal);
        if self.show_status {
            self.status_view(ctx, &pal);
            self.channel_create_dialog(ctx);
            self.forward_dialog(ctx, &pal);
            self.call_overlay(ctx, &pal);
            return;
        }
        if !self.show_settings {
            self.chat_list(ctx, &pal);
        }
        if self.show_info && self.selected.is_some() && !self.show_settings && !self.show_starred {
            self.info_panel(ctx, &pal);
        }
        if self.selected.is_some() && !self.show_settings && !self.show_starred {
            self.input_bar(ctx, &pal);
        }
        self.chat_view(ctx, &pal);
        self.forward_dialog(ctx, &pal);
        self.new_group_dialog(ctx, &pal);
        self.call_overlay(ctx, &pal);
    }
}

impl App {
    /// Bekannter Name zu einer Nummer/LID (Adressbuch, Push-Name oder Einzelchat).
    fn name_for_user(&self, u: &str) -> Option<String> {
        if u.is_empty() {
            return None;
        }
        self.names
            .get(u)
            .map(|n| n.0.clone())
            .or_else(|| self.chats.values().find(|c| !c.is_group && c.jid.user.as_str() == u).map(|c| self.display_name(c)).filter(|n| !n.starts_with('+')))
    }

    /// Seitenleiste mit Kontakt- oder Gruppeninfo, Medien und Verwaltung.
    fn info_panel(&mut self, ctx: &egui::Context, pal: &Palette) {
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
                            match self.images.get(id) {
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
    fn export_current(&mut self) {
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
    fn status_groups(&self) -> Vec<(String, Vec<ChatMessage>)> {
        let Some(c) = self.chats.values().find(|c| c.jid.server == whatsapp_rust::Server::Broadcast && c.jid.user.as_str() == "status") else {
            return Vec::new();
        };
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
    fn status_view(&mut self, ctx: &egui::Context, pal: &Palette) {
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
                    match self.images.get(&m.id) {
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

    fn channel_create_dialog(&mut self, ctx: &egui::Context) {
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
    fn status_recipients(&self) -> Vec<Jid> {
        self.chats.values().filter(|c| !c.is_group && c.jid.server == whatsapp_rust::Server::Pn).map(|c| c.jid.clone()).collect()
    }

    /// Anruf-Anzeige oben in der Mitte: klingelnd oder laufend.
    fn call_overlay(&mut self, ctx: &egui::Context, pal: &Palette) {
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

    fn new_group_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
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
                people.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
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
    fn shortcuts(&mut self, ctx: &egui::Context) {
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
                self.pending_paste = None;
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

    fn forward_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
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
    fn try_paste_image(&mut self, ctx: &egui::Context) {
        let img = match arboard::Clipboard::new().and_then(|mut c| c.get_image()) {
            Ok(i) => i,
            Err(e) => {
                backend::log(format!("Zwischenablage: {e}"));
                return;
            }
        };
        let Some(rgba) = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned()) else { return };
        let dir = data_dir().join("tmp");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("Zwischenablage-{}.png", now_ts()));
        if let Err(e) = rgba.save(&path) {
            self.info = Some(tf!("Bild aus der Zwischenablage: {e}", "Image from clipboard: {e}"));
            return;
        }
        let thumb = image::DynamicImage::ImageRgba8(rgba).thumbnail(240, 240).to_rgba8();
        let ci = egui::ColorImage::from_rgba_unmultiplied([thumb.width() as usize, thumb.height() as usize], thumb.as_raw());
        let tex = ctx.load_texture("paste", ci, egui::TextureOptions::LINEAR);
        self.pending_paste = Some((path, tex));
    }
}

/// Verbindung zum Backend; lebt länger als ein einzelnes Fenster.
struct Core {
    rx: Receiver<UiEvent>,
    cmd: tokio::sync::mpsc::UnboundedSender<Cmd>,
    notify: backend::SharedNotify,
}

thread_local! {
    /// Zustand der Oberfläche, solange das Fenster geschlossen ist (App läuft im Tray weiter).
    static STASH: std::cell::RefCell<Option<App>> = const { std::cell::RefCell::new(None) };
}

/// Aktuelles egui-Fenster, damit Backend und Tray es neu zeichnen oder nach vorne holen können.
static CTX: std::sync::Mutex<Option<egui::Context>> = std::sync::Mutex::new(None);
static WINDOW_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static QUIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn with_ctx(f: impl FnOnce(&egui::Context)) {
    if let Ok(g) = CTX.lock() {
        if let Some(c) = g.as_ref() {
            f(c);
        }
    }
}

/// Hülle um die App: beim Schließen des Fensters wird der Zustand aufgehoben statt verworfen.
struct Shell {
    app: Option<App>,
}

impl Shell {
    fn new(cc: &eframe::CreationContext<'_>, core: &mut Option<Core>) -> Self {
        if let Ok(mut g) = CTX.lock() {
            *g = Some(cc.egui_ctx.clone());
        }
        WINDOW_OPEN.store(true, std::sync::atomic::Ordering::SeqCst);
        let app = match STASH.with(|s| s.borrow_mut().take()) {
            Some(mut app) => {
                // Texturen gehörten zum alten Fenster und müssen neu geladen werden.
                theme::install_fonts(&cc.egui_ctx);
                theme::apply(&cc.egui_ctx, app.settings.dark, app.settings.font_size);
                app.avatars.clear();
                app.avatar_requested.clear();
                app.images.clear();
                app.img_requested.clear();
                app.emoji = emoji::Cache::default();
                app
            }
            None => App::new(cc, core.take().expect("Backend")),
        };
        Shell { app: Some(app) }
    }
}

impl eframe::App for Shell {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some(app) = self.app.as_mut() {
            app.update(ctx, frame);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        WINDOW_OPEN.store(false, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut g) = CTX.lock() {
            *g = None;
        }
        if let Some(mut app) = self.app.take() {
            if app.conn == Conn::Online {
                let _ = app.cmd.send(Cmd::SetOnline(false));
            }
            app.focused = false;
            app.sync_notify_state();
            app.save_settings();
            STASH.with(|s| *s.borrow_mut() = Some(app));
        }
    }
}

/// Nur eine laufende Instanz: ein zweiter Start bittet die erste, ihr Fenster zu öffnen.
fn single_instance(tx: std::sync::mpsc::Sender<tray::TrayMsg>) -> bool {
    use std::io::Write;
    use std::os::unix::net::{UnixListener, UnixStream};
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(data_dir);
    let sock = dir.join("wa-snap.sock");
    if let Ok(mut s) = UnixStream::connect(&sock) {
        let _ = s.write_all(b"open");
        return false;
    }
    let _ = std::fs::remove_file(&sock);
    if let Ok(listener) = UnixListener::bind(&sock) {
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                drop(conn);
                let _ = tx.send(tray::TrayMsg::Open);
            }
        });
    }
    true
}

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
    println!("{}", t!("Fertig. WhatsSnap steht jetzt im Anwendungsmenü und kann an die Taskleiste angeheftet werden.", "Done. WhatsSnap is now in the application menu and can be pinned to the taskbar.")
    );
    Ok(())
}

fn main() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--install-desktop") {
        if let Err(e) = install_desktop() {
            eprintln!("{}", tf!("Installation fehlgeschlagen: {e}", "Installation failed: {e}"));
            std::process::exit(1);
        }
        return Ok(());
    }
    // Tray-Klicks, zweiter Programmstart und Menü landen alle hier.
    let (ev_tx, ev_rx) = channel::<tray::TrayMsg>();
    if !single_instance(ev_tx.clone()) {
        println!("{}", t!("WhatsSnap läuft schon, das Fenster wird geöffnet.", "WhatsSnap is already running, opening its window."));
        return Ok(());
    }

    let (tx, rx) = channel();
    let notify: backend::SharedNotify = Default::default();
    let cmd = backend::spawn(data_dir(), tx, notify.clone(), || with_ctx(|c| c.request_repaint()));
    let has_tray = tray::start(ev_tx.clone());
    let mut core = Some(Core { rx, cmd: cmd.clone(), notify });

    // Ereignisse verteilen: bei offenem Fenster direkt dorthin, sonst an die Hauptschleife.
    let (main_tx, main_rx) = channel::<tray::TrayMsg>();
    std::thread::spawn(move || {
        for msg in ev_rx {
            let open = WINDOW_OPEN.load(std::sync::atomic::Ordering::SeqCst);
            match msg {
                tray::TrayMsg::Open if open => with_ctx(|c| {
                    c.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    c.send_viewport_cmd(egui::ViewportCommand::Focus);
                }),
                tray::TrayMsg::Quit if open => {
                    QUIT.store(true, std::sync::atomic::Ordering::SeqCst);
                    with_ctx(|c| c.send_viewport_cmd(egui::ViewportCommand::Close));
                }
                m => {
                    let _ = main_tx.send(m);
                }
            }
        }
    });

    let mut show = !(has_tray && std::env::args().any(|a| a == "--hidden"));
    loop {
        if show {
            let opts = eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default()
                    .with_inner_size([1280.0, 800.0])
                    .with_min_inner_size([640.0, 420.0])
                    .with_app_id("wa-snap")
                    .with_title(t!("WhatsSnap", "WhatsSnap")),
                run_and_return: true,
                ..Default::default()
            };
            let core_ref = &mut core;
            eframe::run_native(t!("WhatsSnap", "WhatsSnap"), opts, Box::new(|cc| Ok(Box::new(Shell::new(cc, core_ref)))))?;
        }
        show = true;
        let background = STASH.with(|s| s.borrow().as_ref().map(|a| a.settings.background).unwrap_or(true));
        if QUIT.load(std::sync::atomic::Ordering::SeqCst) || !has_tray || !background {
            break;
        }
        match main_rx.recv() {
            Ok(tray::TrayMsg::Open) => continue,
            _ => break,
        }
    }
    let _ = cmd.send(Cmd::Quit);
    std::thread::sleep(std::time::Duration::from_millis(400));
    Ok(())
}

#[cfg(test)]
mod format_tests {
    use super::fmt_spans;

    #[test]
    fn bold_italic_strike_code() {
        let v = fmt_spans("a *fett* _kursiv_ ~weg~ `x*y` b", 0);
        let got: Vec<(&str, u8)> = v.iter().map(|(t, s)| (t.as_str(), *s)).collect();
        assert_eq!(
            got,
            vec![("a ", 0), ("fett", 1), (" ", 0), ("kursiv", 2), (" ", 0), ("weg", 4), (" ", 0), ("x*y", 8), (" b", 0)]
        );
    }

    #[test]
    fn no_format_inside_words_or_spaces() {
        let v = fmt_spans("2*3*4 und * nicht *", 0);
        assert_eq!(v, vec![("2*3*4 und * nicht *".to_owned(), 0)]);
    }

    #[test]
    fn nested() {
        let v = fmt_spans("*_beides_*", 0);
        assert_eq!(v, vec![("beides".to_owned(), 3)]);
    }
}
