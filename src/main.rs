// Bewusst nicht umgebaut: Tupel-Typen, große Enum-Varianten und lange Argumentlisten sind hier Absicht.
#![allow(clippy::type_complexity, clippy::large_enum_variant, clippy::too_many_arguments)]
#[macro_use]
mod i18n;
mod audio;
mod backend;
mod call;
mod emoji;
mod store;
mod theme;
mod tray;
mod safety;
mod dialogs;
mod video;
mod viewer;
mod util;
mod app_state;
mod app_pump;
mod settings_ui;
mod lists;
mod input;
mod chat_view;
mod messages;
mod panels;
mod overlays;

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver};

use backend::{ChatMessage, Cmd, HistChat, UiEvent};
use eframe::egui::{self, Color32, RichText};
use store::{Rec, Store};
use theme::Palette;
use util::*;
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
    /// Ausführliches Log: eine Zeile pro Nachricht (Nummern bleiben auch dann unkenntlich).
    #[serde(default)]
    debug_log: bool,
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
            debug_log: false,
            local_pins: Vec::new(),
            lang: String::new(),
        }
    }
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
/// Noch nicht bestätigte Textnachricht (überlebt einen Neustart, siehe `Rec::Out`).
#[derive(Clone)]
struct Outgoing {
    chat: String,
    is_group: bool,
    text: String,
    ts: i64,
    quote: Option<backend::Quote>,
    mentions: Vec<String>,
    exp: u32,
}

enum MsgAct {
    /// Fehlgeschlagene Nachricht erneut senden / verwerfen
    Resend(String),
    Discard(String),
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
    /// Wann ein Bild zuletzt gezeichnet wurde; lange nicht gezeigte fliegen aus dem Speicher.
    img_seen: std::cell::RefCell<HashMap<String, std::time::Instant>>,
    img_sweep: std::time::Instant,
    img_stale: HashSet<String>,
    /// Fehlgeschlagene Bild-Downloads mit Zeitpunkt: frühestens nach einer Minute neu versuchen.
    img_failed: HashMap<String, std::time::Instant>,
    /// Geöffneter Bild-/Video-Betrachter.
    viewer: Option<viewer::Viewer>,
    /// Zustellstatus für Nachrichten, deren ID noch nicht bekannt ist (siehe `set_status`).
    early_status: HashMap<String, u8>,
    /// Dateien im Ausgang, die der Server noch nicht bestätigt hat: vorläufige ID → (Chat, Gruppe, Zeit, Text, Medium).
    outbox_files: HashMap<String, (String, bool, i64, String, backend::MediaRef)>,
    /// Zähler, der bei jeder neuen Nachricht steigt (macht Such-Zwischenspeicher ungültig).
    msg_gen: u64,
    /// Suche über alle Chats: (Suchtext, Stand, Treffer). Wird nur neu berechnet, wenn sich
    /// Suchtext oder Verlauf ändern, und erst nach einer kurzen Tipp-Pause.
    search_cache: Option<(String, u64, Vec<(i64, String, String, String)>)>,
    search_typed: Option<(String, std::time::Instant)>,
    /// Suche im offenen Chat: (Chat, Suchtext, Nachrichtenzahl, Positionen der Treffer).
    chat_search_cache: Option<(String, String, usize, Vec<usize>)>,
    /// Größe der Medienordner für die Speicher-Seite: (gemessen um, Bytes).
    storage_size: Option<(std::time::Instant, u64)>,
    /// Status-Gruppen: (Nachrichtenzahl, Minute, Ergebnis).
    status_cache: std::cell::RefCell<Option<(usize, i64, Vec<(String, Vec<ChatMessage>)>)>>,
    /// Bild aus der Zwischenablage wird im Hintergrund als PNG gespeichert.
    paste_rx: Option<Receiver<Result<(std::path::PathBuf, [usize; 2], Vec<u8>), String>>>,
    /// Reihenfolge, in der Sprachnachrichten dekodiert wurden (älteste zuerst), fürs Speicherbudget.
    decoded_order: std::collections::VecDeque<String>,
    /// Rückfrage vor dem Öffnen einer riskanten Datei.
    open_warn: Option<dialogs::OpenWarn>,
    /// Nach dem Abmelden: fragen, ob auch Verlauf und Medien gelöscht werden sollen.
    logout_prompt: bool,
    /// Zuletzt gemessene Höhe je Nachricht (Prüfwert, Höhe): Nachrichten außerhalb des
    /// sichtbaren Bereichs werden damit nur freigehalten statt neu aufgebaut.
    heights: HashMap<String, (u64, f32)>,
    img_requested: HashSet<String>,
    /// Reaktionen je Nachrichten-ID: (Absender, Emoji); Absender leer = ich.
    reactions: HashMap<String, Vec<(String, String)>>,
    /// Antwort in Vorbereitung: (Chat, Zitat, zitierte Nachricht ist von mir).
    reply_to: Option<(String, backend::Quote, bool)>,
    me: Vec<String>,
    drafts: HashMap<String, String>,
    /// LID → Telefonnummer derselben Person. Einzelchats laufen immer unter der Nummer,
    /// sonst zerfällt ein Gespräch in zwei Chats (alter Verlauf hier, neue Nachrichten dort).
    alias: HashMap<String, String>,
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
    outbox: HashMap<String, Outgoing>,
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let was_focused = self.focused;
        self.focused = ctx.input(|i| i.focused);
        self.pump(ctx);
        self.sweep_images();
        self.poll_paste(ctx);
        // Der Betrachter liegt über allem und bekommt die Tasten zuerst.
        if self.viewer.is_some() {
            self.viewer_ui(ctx);
        }
        self.safety_dialogs(ctx);
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
                if let Some(e) = p.error() {
                    self.info = Some(tf!("Abspielen nicht möglich: {e}", "Playback not possible: {e}"));
                }
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

        // QR-Code und „Abgemeldet“ immer zeigen, auch wenn schon Chats im Verlauf liegen.
        if (self.conn != Conn::Online && self.chats.is_empty()) || matches!(self.conn, Conn::NeedsQr | Conn::LoggedOut) {
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
                                RichText::new(t!("Die Verknüpfung mit dem Handy wurde aufgehoben. Dein Verlauf auf diesem PC bleibt erhalten.", "The link with your phone was removed. Your history on this PC is kept."))
                                    .color(pal.dim),
                            );
                            ui.add_space(18.0);
                            if ui.button(RichText::new(t!("Neu verknüpfen", "Link again")).size(16.0)).clicked() {
                                let _ = dialogs::mark_logged_out(&data_dir());
                                backend::log("Neu verknüpfen: Neustart");
                                RESTART.store(true, std::sync::atomic::Ordering::SeqCst);
                                QUIT.store(true, std::sync::atomic::Ordering::SeqCst);
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
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
/// Nach dem Beenden das Programm neu starten (für „Neu verknüpfen“).
static RESTART: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
    last_frame: std::time::Instant,
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
            // Ohne Backend (kann nur bei einem Programmfehler fehlen) startet das Fenster leer,
            // statt abzustürzen; die Meldung steht im Log.
            None => match core.take() {
                Some(c) => App::new(cc, c),
                None => {
                    backend::log("Start: Verbindung zum Backend fehlt");
                    return Shell { app: None, last_frame: std::time::Instant::now() };
                }
            },
        };
        Shell { app: Some(app), last_frame: std::time::Instant::now() }
    }
}

impl eframe::App for Shell {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Ohne vsync (Wayland, siehe main) würde eine laufende Animation ungebremst zeichnen
        // und einen CPU-Kern auslasten. Deshalb höchstens rund 144 Bilder pro Sekunde.
        const MIN_FRAME: std::time::Duration = std::time::Duration::from_micros(6900);
        if let Some(wait) = MIN_FRAME.checked_sub(self.last_frame.elapsed()) {
            std::thread::sleep(wait);
        }
        self.last_frame = std::time::Instant::now();
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
    // Erst die Sperre, dann der Socket: bei zwei gleichzeitigen Starts gewinnt genau einer.
    // Ohne Sperre könnten beide den Socket als verwaist ansehen, löschen und doppelt laufen.
    match safety::lock_instance(&dir) {
        Some(lock) => {
            // Die Sperre gilt, solange die Datei offen ist: bis zum Programmende behalten.
            std::mem::forget(lock);
        }
        None => {
            // Die andere Instanz bindet ihren Socket vielleicht gerade erst: kurz nachfassen.
            for _ in 0..20 {
                if let Ok(mut s) = UnixStream::connect(&sock) {
                    let _ = s.write_all(b"open");
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            return false;
        }
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

    // Vor allem anderen: neue Dateien nur für den eigenen Nutzer, abgemeldete Sitzung entfernen.
    safety::restrict_new_files();
    if let Err(e) = std::fs::create_dir_all(data_dir()) {
        eprintln!("{}", tf!("Datenordner {} lässt sich nicht anlegen: {e}", "Cannot create data folder {}: {e}", data_dir().display()));
        std::process::exit(1);
    }
    dialogs::finish_logout(&data_dir());
    harden_data_dir();
    // Abstürze ins Log schreiben: beim Start aus dem Menü sieht sonst niemand die Meldung.
    let log_file = data_dir().join("wa-snap.log");
    let old_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_file) {
            let thread = std::thread::current();
            let line = format!("{} ABSTURZ (Thread {}): {info}\n", chrono::Local::now().format("%F %T"), thread.name().unwrap_or("?"));
            let _ = f.write_all(line.as_bytes());
        }
        old_hook(info);
    }));
    let (tx, rx) = channel();
    let notify: backend::SharedNotify = Default::default();
    let (cmd, backend_thread) = backend::spawn(data_dir(), tx, notify.clone(), || with_ctx(|c| c.request_repaint()));
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
                // Unter Wayland wartet eglSwapBuffers mit vsync auf einen Frame-Callback, den der
                // Compositor bei verdecktem oder minimiertem Fenster nie schickt: die App hinge dann
                // beim nächsten Neuzeichnen fest („Reagiert nicht“). winit taktet dort ohnehin selbst.
                vsync: std::env::var_os("WAYLAND_DISPLAY").is_none(),
                ..Default::default()
            };
            let core_ref = &mut core;
            if let Err(e) = eframe::run_native(t!("WhatsSnap", "WhatsSnap"), opts, Box::new(|cc| Ok(Box::new(Shell::new(cc, core_ref))))) {
                backend::log(format!("Fenster-Fehler: {e}"));
                return Err(e);
            }
        }
        show = true;
        let background = STASH.with(|s| s.borrow().as_ref().map(|a| a.settings.background).unwrap_or(true));
        let quit = QUIT.load(std::sync::atomic::Ordering::SeqCst);
        if quit || !has_tray || !background {
            backend::log(format!("Beenden: Fenster zu (beenden={quit} tray={has_tray} im_hintergrund={background})"));
            break;
        }
        backend::log("Fenster geschlossen, läuft im Tray weiter");
        match main_rx.recv() {
            Ok(tray::TrayMsg::Open) => continue,
            _ => break,
        }
    }
    // Backend sauber herunterfahren lassen (Verbindung schließen, Datenbank schreiben),
    // aber höchstens ein paar Sekunden warten.
    let _ = cmd.send(Cmd::Quit);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while !backend_thread.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    if !backend_thread.is_finished() {
        backend::log("Beenden: Backend hat nicht rechtzeitig aufgehört");
    }
    if RESTART.load(std::sync::atomic::Ordering::SeqCst) {
        // Prozess ersetzen: Sperrdatei und Socket sind CLOEXEC und werden dabei freigegeben,
        // die neue Instanz löscht die alte Sitzung (Merker) und zeigt den QR-Code.
        use std::os::unix::process::CommandExt;
        if let Ok(exe) = std::env::current_exe() {
            let e = std::process::Command::new(exe).exec();
            backend::log(format!("Neustart fehlgeschlagen: {e}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod order_tests {
    use super::*;

    fn msg(id: &str, ts: i64) -> ChatMessage {
        ChatMessage {
            chat: Jid::new("49151", whatsapp_rust::Server::Pn),
            is_group: false,
            from_me: false,
            sender_name: String::new(),
            sender: String::new(),
            text: String::new(),
            ts,
            id: id.into(),
            status: 0,
            media: None,
            quote: None,
            x: Default::default(),
        }
    }

    #[test]
    fn inserts_keep_time_order_and_arrival_order_for_equal_times() {
        let mut v: Vec<ChatMessage> = Vec::new();
        for (id, ts) in [("a", 10), ("b", 30), ("c", 20), ("d", 30), ("e", 5), ("f", 20)] {
            let pos = insert_pos(&v, ts);
            v.insert(pos, msg(id, ts));
        }
        let ids: Vec<&str> = v.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["e", "a", "c", "f", "b", "d"]);
    }
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
