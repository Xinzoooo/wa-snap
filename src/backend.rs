//! Hintergrund-Thread: hält die WhatsApp-Verbindung (tokio) und spricht
//! per Channels mit der Oberfläche. Die UI blockiert nie auf Netzwerk.
//!
//! Benachrichtigungen werden hier ausgelöst, nicht in der Oberfläche: Ist das
//! Fenster minimiert oder verdeckt, zeichnet die Oberfläche unter Wayland nicht
//! und würde neue Nachrichten erst beim nächsten Öffnen bemerken.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use whatsapp_rust::prelude::*;
use whatsapp_rust::wacore::proto_helpers::{MessageBuilderExt, MessageExt};
use whatsapp_rust::wacore_binary::JidExt;
use whatsapp_rust::Jid;

/// Was das Backend der Oberfläche meldet.
pub enum UiEvent {
    Status(String),
    Qr(String),
    Connected,
    LoggedOut,
    Message(ChatMessage),
    /// Anzeigename für eine Rufnummer/LID (Adressbuch hat Vorrang vor Push-Namen).
    Contact { user: String, name: String, from_book: bool },
    /// Chats und Nachrichten aus dem Verlauf-Sync nach der Verknüpfung.
    History(Vec<HistChat>),
    /// Profilbild als RGBA-Pixel (96x96 oder kleiner), Schlüssel ist die Rufnummer/LID.
    Avatar { user: String, size: [usize; 2], rgba: Vec<u8> },
    SendFailed { chat: String, temp: String, err: String },
    /// Eigene Nachricht wurde vom Server angenommen: lokale ID durch echte ersetzen.
    Sent { chat: String, temp: String, id: String },
    /// Zustellung/Lesen eigener Nachrichten: 2 = zugestellt, 3 = gelesen.
    Receipt { ids: Vec<String>, status: u8 },
    /// Stumm/Angeheftet/Archiviert/Gelesen wie bei WhatsApp. `users` enthält
    /// Rufnummer und LID, damit der Chat in beiden Schreibweisen gefunden wird.
    Muted { users: Vec<String>, on: bool },
    Pinned { users: Vec<String>, on: bool },
    Archived { users: Vec<String>, on: bool },
    ChatRead { users: Vec<String>, read: bool },
    /// Rückmeldung für die Statuszeile.
    Info(String),
    /// Alle Gruppen mit Namen (direkt vom Server abgefragt).
    Groups(Vec<(Jid, String)>),
    /// Eigene Nummer und LID (um eigene zitierte Nachrichten zu erkennen).
    Me(Vec<String>),
    /// Reaktion auf eine Nachricht; leeres Emoji = Reaktion entfernt.
    Reaction { target: String, from: String, emoji: String },
    /// Nachricht wurde für alle gelöscht.
    Revoked { id: String },
    /// Nachricht wurde bearbeitet.
    Edited { id: String, text: String },
    /// Bild/Sticker einer Nachricht dekodiert (Vorschau oder volle Größe).
    Image { id: String, size: [usize; 2], rgba: Vec<u8>, full: bool },
}

/// Was die Oberfläche dem Backend aufträgt.
pub enum Cmd {
    Send { chat: Jid, text: String, temp: String, quote: Option<Quote> },
    /// Mit Emoji reagieren (leer = Reaktion entfernen).
    React { chat: Jid, target: Quote, from_me: bool, emoji: String },
    /// Eigene Nachricht für alle löschen.
    Revoke { chat: Jid, id: String },
    Avatar { jid: Jid },
    SetMute { jid: Jid, on: bool },
    SetPin { jid: Jid, on: bool },
    SetArchive { jid: Jid, on: bool, last: Option<LastMsg> },
    /// Chat gelesen: Lesebestätigungen senden und Ungelesen-Markierung abgleichen.
    MarkRead { jid: Jid, msgs: Vec<(String, String)>, last: Option<LastMsg> },
    SetPushName(String),
    SetAbout(String),
    TestNotify,
    /// Bild/Sticker laden (erst Vorschau, dann herunterladen und dekodieren).
    LoadMedia { id: String, media: MediaRef },
    /// Anhang herunterladen und mit dem Standardprogramm öffnen.
    OpenMedia { id: String, media: MediaRef },
    /// Datei senden (Bild als Foto, sonst als Dokument). Ohne Pfad: Dateiauswahl öffnen.
    SendFile { chat: Jid, path: Option<std::path::PathBuf>, caption: String },
    /// Bekannten Sticker erneut senden (ohne neuen Upload).
    SendSticker { chat: Jid, media: MediaRef },
    Quit,
}

/// Zitierte Nachricht (Antwort).
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Quote {
    pub id: String,
    /// JID des Absenders der zitierten Nachricht (leer = ich selbst)
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub text: String,
}

/// Verweis auf ein Medium beim WhatsApp-CDN; reicht zum späteren Herunterladen.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MediaRef {
    /// image, sticker, video, gif, audio, ptt, document
    pub kind: String,
    #[serde(default)]
    pub mime: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub direct_path: String,
    #[serde(default)]
    pub key: Vec<u8>,
    #[serde(default)]
    pub sha: Vec<u8>,
    #[serde(default)]
    pub enc_sha: Vec<u8>,
    #[serde(default)]
    pub len: u64,
    #[serde(default)]
    pub w: u32,
    #[serde(default)]
    pub h: u32,
    #[serde(default)]
    pub seconds: u32,
    #[serde(default)]
    pub animated: bool,
}

impl MediaRef {
    pub fn is_picture(&self) -> bool {
        self.kind == "image" || self.kind == "sticker"
    }

    pub fn ext(&self) -> &str {
        match (self.kind.as_str(), self.mime.split(';').next().unwrap_or("")) {
            (_, "image/jpeg") => "jpg",
            (_, "image/png") => "png",
            (_, "image/webp") | ("sticker", _) => "webp",
            (_, "image/gif") => "gif",
            (_, "video/mp4") | ("video" | "gif", _) => "mp4",
            (_, "audio/ogg") | (_, "audio/ogg; codecs=opus") | ("ptt", _) => "ogg",
            (_, "audio/mpeg") => "mp3",
            (_, "audio/mp4") => "m4a",
            (_, "application/pdf") => "pdf",
            ("image", _) => "jpg",
            _ => self.name.rsplit('.').next().filter(|e| e.len() <= 5 && *e != self.name).unwrap_or("bin"),
        }
    }

    /// Kurzer Text für Chatliste und Benachrichtigung.
    pub fn label(&self) -> String {
        match self.kind.as_str() {
            "image" => "📷 Foto".into(),
            "sticker" => "Sticker".into(),
            "video" => "🎥 Video".into(),
            "gif" => "GIF".into(),
            "ptt" => format!("🎤 Sprachnachricht ({}:{:02})", self.seconds / 60, self.seconds % 60),
            "audio" => "🎵 Audio".into(),
            "document" => format!("📄 {}", if self.name.is_empty() { "Dokument" } else { &self.name }),
            other => other.to_owned(),
        }
    }

    fn params(&self) -> whatsapp_rust::download::DownloadParams {
        use whatsapp_rust::wacore::download::MediaType as M;
        let mt = match self.kind.as_str() {
            "image" => M::Image,
            "sticker" => M::Sticker,
            "video" | "gif" => M::Video,
            "audio" | "ptt" => M::Audio,
            _ => M::Document,
        };
        whatsapp_rust::download::DownloadParams::encrypted(
            self.direct_path.clone(),
            &self.key,
            &self.sha,
            &self.enc_sha,
            self.len,
            mt,
        )
    }
}

/// Medium aus einer Nachricht lesen, dazu das eingebettete Vorschaubild.
pub fn media_of(msg: &wa::Message) -> Option<(MediaRef, Option<Vec<u8>>, Option<String>)> {
    let b = msg.get_base_message();
    macro_rules! base {
        ($m:expr, $kind:expr) => {
            MediaRef {
                kind: $kind.into(),
                mime: $m.mimetype.clone().unwrap_or_default(),
                url: $m.url.clone().unwrap_or_default(),
                direct_path: $m.direct_path.clone().unwrap_or_default(),
                key: $m.media_key.clone().unwrap_or_default(),
                sha: $m.file_sha256.clone().unwrap_or_default(),
                enc_sha: $m.file_enc_sha256.clone().unwrap_or_default(),
                len: $m.file_length.unwrap_or(0),
                ..Default::default()
            }
        };
    }
    if let Some(m) = b.image_message.as_option() {
        let mut r = base!(m, "image");
        r.w = m.width.unwrap_or(0);
        r.h = m.height.unwrap_or(0);
        return Some((r, m.jpeg_thumbnail.clone(), m.caption.clone()));
    }
    if let Some(m) = b.sticker_message.as_option() {
        let mut r = base!(m, "sticker");
        r.w = m.width.unwrap_or(0);
        r.h = m.height.unwrap_or(0);
        r.animated = m.is_animated.unwrap_or(false);
        return Some((r, m.png_thumbnail.clone(), None));
    }
    if let Some(m) = b.video_message.as_option() {
        let gif = m.gif_playback.unwrap_or(false);
        let mut r = base!(m, if gif { "gif" } else { "video" });
        r.w = m.width.unwrap_or(0);
        r.h = m.height.unwrap_or(0);
        r.seconds = m.seconds.unwrap_or(0);
        return Some((r, m.jpeg_thumbnail.clone(), m.caption.clone()));
    }
    if let Some(m) = b.audio_message.as_option() {
        let mut r = base!(m, if m.ptt.unwrap_or(false) { "ptt" } else { "audio" });
        r.seconds = m.seconds.unwrap_or(0);
        return Some((r, None, None));
    }
    if let Some(m) = b.document_message.as_option() {
        let mut r = base!(m, "document");
        r.name = m.file_name.clone().or_else(|| m.title.clone()).unwrap_or_default();
        return Some((r, m.jpeg_thumbnail.clone(), m.caption.clone()));
    }
    None
}

/// Zitat (Antwort) aus einer Nachricht lesen.
pub fn quote_of(msg: &wa::Message) -> Option<Quote> {
    let b = msg.get_base_message();
    let ci = b
        .extended_text_message
        .as_option()
        .and_then(|m| m.context_info.as_option())
        .or_else(|| b.image_message.as_option().and_then(|m| m.context_info.as_option()))
        .or_else(|| b.video_message.as_option().and_then(|m| m.context_info.as_option()))
        .or_else(|| b.sticker_message.as_option().and_then(|m| m.context_info.as_option()))
        .or_else(|| b.document_message.as_option().and_then(|m| m.context_info.as_option()))
        .or_else(|| b.audio_message.as_option().and_then(|m| m.context_info.as_option()))?;
    let id = ci.stanza_id.clone()?;
    let q = ci.quoted_message.as_option()?;
    let text = q
        .text_content()
        .map(|t| t.to_owned())
        .or_else(|| media_of(q).map(|(m, _, cap)| match cap {
            Some(c) if !c.is_empty() => format!("{} {c}", m.label()),
            _ => m.label(),
        }))
        .unwrap_or_default();
    Some(Quote { id, sender: ci.participant.clone().unwrap_or_default(), name: String::new(), text })
}

fn mime_of(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("pdf") => "application/pdf",
        Some("mp4") => "video/mp4",
        Some("mp3") => "audio/mpeg",
        Some("ogg" | "opus") => "audio/ogg",
        Some("txt") => "text/plain",
        Some("zip") => "application/zip",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

/// Bilddatei dekodieren und auf höchstens `max` Pixel verkleinern.
fn decode_rgba(bytes: &[u8], max: u32) -> Option<([usize; 2], Vec<u8>)> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
    let rgba = img.to_rgba8();
    Some(([rgba.width() as usize, rgba.height() as usize], rgba.into_raw()))
}

/// Datei des Mediums im Datenordner; lädt sie herunter, falls sie noch fehlt.
async fn ensure_file(client: &Client, dir: &std::path::Path, id: &str, m: &MediaRef) -> Result<std::path::PathBuf, String> {
    let path = dir.join(format!("{}.{}", safe_name(id), m.ext()));
    if path.exists() {
        return Ok(path);
    }
    if m.direct_path.is_empty() || m.key.is_empty() {
        return Err("Medium ist nicht mehr verfügbar".into());
    }
    let bytes = client.download_from_params(&m.params()).await.map_err(|e| e.to_string())?;
    let _ = std::fs::create_dir_all(dir);
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

fn safe_name(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect()
}

/// Kleines JPEG als eingebettete Vorschau für gesendete Fotos.
fn jpeg_thumb(img: &image::DynamicImage) -> Vec<u8> {
    let t = img.thumbnail(72, 72).to_rgb8();
    let mut out = Vec::new();
    let _ = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 60).encode_image(&t);
    out
}


/// Letzte Nachricht eines Chats; WhatsApp braucht sie, um Archiv/Gelesen zuzuordnen.
pub struct LastMsg {
    pub id: String,
    pub from_me: bool,
    pub sender: String,
    pub ts: i64,
}

#[derive(Clone)]
pub struct ChatMessage {
    pub chat: Jid,
    pub is_group: bool,
    pub from_me: bool,
    pub sender_name: String,
    /// JID des Absenders (für Lesebestätigungen in Gruppen), kann leer sein.
    pub sender: String,
    pub text: String,
    pub ts: i64,
    pub id: String,
    /// 0 = wird gesendet, 1 = gesendet, 2 = zugestellt, 3 = gelesen, 9 = Fehler.
    pub status: u8,
    pub media: Option<MediaRef>,
    pub quote: Option<Quote>,
}

/// Was das Backend für Benachrichtigungen wissen muss. Die Oberfläche hält es aktuell.
#[derive(Default)]
pub struct NotifyState {
    pub enabled: bool,
    pub preview: bool,
    pub muted: HashSet<String>,
    pub focused: bool,
    pub selected_user: Option<String>,
    pub names: HashMap<String, String>,
}

pub type SharedNotify = Arc<Mutex<NotifyState>>;

pub struct HistChat {
    pub jid: Jid,
    pub name: String,
    pub is_group: bool,
    pub last_ts: i64,
    pub unread: u32,
    pub msgs: Vec<ChatMessage>,
}

static LOG_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
static MEDIA_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Eingebettetes Vorschaubild ablegen (`media/<id>.thumb`).
fn save_thumb(id: &str, bytes: &[u8]) {
    if let Some(d) = MEDIA_DIR.get() {
        let p = d.join(format!("{}.thumb", safe_name(id)));
        if !p.exists() {
            let _ = std::fs::create_dir_all(d);
            let _ = std::fs::write(p, bytes);
        }
    }
}

/// Diagnose-Log (`wa-snap.log` im Datenordner), damit sich Probleme nachvollziehen lassen.
pub fn log(msg: impl AsRef<str>) {
    use std::io::Write;
    let Some(p) = LOG_PATH.get() else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{} {}", chrono::Local::now().format("%F %T"), msg.as_ref());
    }
}

/// Desktop-Benachrichtigung zeigen. Erst direkt über D-Bus, sonst über `notify-send`.
/// Blockiert kurz, deshalb immer in einem eigenen Thread aufrufen.
pub fn show_notification(title: &str, body: &str) -> Result<(), String> {
    let r = notify_rust::Notification::new()
        .appname("wa-snap")
        .summary(title)
        .body(body)
        .icon("wa-snap")
        .show();
    match r {
        Ok(_) => {
            log("Benachrichtigung über D-Bus gezeigt");
            Ok(())
        }
        Err(e) => {
            log(format!("Benachrichtigung über D-Bus fehlgeschlagen: {e}"));
            match std::process::Command::new("notify-send")
                .args(["-a", "wa-snap", "-i", "wa-snap", title, body])
                .status()
            {
                Ok(s) if s.success() => {
                    log("Benachrichtigung über notify-send gezeigt");
                    Ok(())
                }
                Ok(s) => Err(format!("D-Bus: {e}; notify-send: Exit {s}")),
                Err(e2) => Err(format!("D-Bus: {e}; notify-send: {e2}")),
            }
        }
    }
}

fn maybe_notify(state: &SharedNotify, m: &ChatMessage) {
    if m.from_me {
        return;
    }
    let age = chrono::Utc::now().timestamp() - m.ts;
    let chat_user = m.chat.user.to_string();
    let (title, body) = {
        let Ok(st) = state.lock() else { return };
        let viewing = st.focused && st.selected_user.as_deref() == Some(chat_user.as_str());
        log(format!(
            "Benachrichtigung? an={} stumm={} alt={age}s sichtbar={viewing}",
            st.enabled,
            st.muted.contains(&chat_user)
        ));
        if !st.enabled || st.muted.contains(&chat_user) || viewing || age > 300 {
            return;
        }
        let chat_name = st.names.get(&chat_user).cloned().unwrap_or_else(|| {
            if m.is_group {
                "Gruppe".to_owned()
            } else if !m.sender_name.is_empty() {
                m.sender_name.clone()
            } else {
                format!("+{chat_user}")
            }
        });
        let text = match &m.media {
            Some(md) if m.text.is_empty() => md.label(),
            Some(md) => format!("{} {}", md.label(), m.text),
            None => m.text.clone(),
        };
        let body = if !st.preview {
            "Neue Nachricht".to_owned()
        } else if m.is_group && !m.sender_name.is_empty() {
            format!("{}: {}", m.sender_name, text)
        } else {
            text
        };
        (chat_name, body)
    };
    std::thread::spawn(move || {
        let _ = show_notification(&title, &body);
    });
}

/// Klartext einer Nachricht; bei Medien ein kurzer Platzhalter.
fn describe(message: &wa::Message, media_type: &str) -> String {
    if let Some(t) = message.text_content() {
        return t.to_owned();
    }
    let label = match media_type {
        "image" => "📷 Foto",
        "video" | "gif" => "🎥 Video",
        "ptt" | "audio" => "🎤 Sprachnachricht",
        "document" => "📄 Dokument",
        "sticker" => "Sticker",
        "vcard" => "👤 Kontakt",
        "location" | "livelocation" => "📍 Standort",
        "" => "Nachricht",
        other => other,
    };
    if let Some(c) = message.get_caption() {
        return format!("{label}: {c}");
    }
    label.to_owned()
}

/// Interne Nachrichten (Schlüsselabgleich, Verlauf-Sync, Reaktionen …) haben weder
/// Text noch Medien und werden nicht angezeigt.
fn is_visible(message: &wa::Message, media_type: &str) -> bool {
    message.text_content().is_some() || message.get_caption().is_some() || !media_type.is_empty()
}

/// Verlauf-Sync (Chats, Nachrichten, Namen) in Anzeigedaten umwandeln.
fn convert_history(h: &wa::HistorySync, send: &impl Fn(UiEvent)) {
    log(format!(
        "HistorySync: {} Chats, {} Push-Namen",
        h.conversations.len(),
        h.pushnames.len()
    ));
    for p in &h.pushnames {
        if let (Some(id), Some(name)) = (&p.id, &p.pushname) {
            let user = user_of(id);
            if !user.is_empty() && !name.is_empty() {
                send(UiEvent::Contact { user, name: name.clone(), from_book: false });
            }
        }
    }
    let mut chats = Vec::new();
    for c in &h.conversations {
        let Ok(jid) = c.id.parse::<Jid>() else { continue };
        if jid.server == whatsapp_rust::Server::Broadcast
            || jid.server == whatsapp_rust::Server::Newsletter
        {
            continue;
        }
        let is_group = jid.server == whatsapp_rust::Server::Group;
        let mut msgs = Vec::new();
        // Nur die letzten Nachrichten übernehmen, das hält den Start schnell.
        let skip = c.messages.len().saturating_sub(80);
        for hm in &c.messages[skip..] {
            let wm = &hm.message;
            let Some(inner) = wm.message.as_option() else { continue };
            let key = &wm.key;
            let media = media_of(inner);
            let text = match (inner.text_content(), &media) {
                (Some(t), _) => t.to_owned(),
                (None, Some((_, _, cap))) => cap.clone().unwrap_or_default(),
                (None, None) => continue,
            };
            let id = key.id.clone().unwrap_or_default();
            if let Some((_, Some(th), _)) = &media {
                save_thumb(&id, th);
            }
            let from_me = key.from_me.unwrap_or(false);
            let sender = wm
                .participant
                .clone()
                .or_else(|| key.participant.clone())
                .unwrap_or_else(|| if is_group { String::new() } else { c.id.clone() });
            msgs.push(ChatMessage {
                chat: jid.clone(),
                is_group,
                from_me,
                sender_name: wm
                    .push_name
                    .clone()
                    .or_else(|| wm.participant.as_deref().map(user_of))
                    .unwrap_or_default(),
                sender,
                text,
                ts: wm.message_timestamp.unwrap_or(0) as i64,
                id,
                status: if from_me { 2 } else { 0 },
                media: media.map(|m| m.0),
                quote: quote_of(inner),
            });
        }
        let last_ts = c
            .conversation_timestamp
            .or(c.last_msg_timestamp)
            .map(|t| t as i64)
            .or_else(|| msgs.last().map(|m| m.ts))
            .unwrap_or(0);
        chats.push(HistChat {
            jid,
            name: c.name.clone().unwrap_or_default(),
            is_group,
            last_ts,
            unread: c.unread_count.unwrap_or(0),
            msgs,
        });
    }
    if !chats.is_empty() {
        send(UiEvent::History(chats));
    }
}

/// Profilbild holen (Cache auf Platte, 7 Tage; leere Datei = hat keins, 1 Tag) und verkleinern.
async fn load_avatar(client: Arc<Client>, dir: std::path::PathBuf, jid: Jid) -> Option<UiEvent> {
    let user = jid.user.to_string();
    let path = dir.join(format!("{user}.img"));
    let age = |p: &std::path::Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|e| e.as_secs())
    };
    let cached = match age(&path) {
        Some(a) => std::fs::read(&path).ok().map(|b| (a, b)),
        None => None,
    };
    let bytes: Vec<u8> = match cached {
        Some((a, b)) if (!b.is_empty() && a < 7 * 86_400) || (b.is_empty() && a < 86_400) => b,
        _ => {
            let fetched = match client.contacts().get_profile_picture(&jid, true).await {
                Ok(Some(pp)) => tokio::task::spawn_blocking(move || {
                    let mut resp = ureq::get(&pp.url).call().ok()?;
                    resp.body_mut().read_to_vec().ok()
                })
                .await
                .ok()
                .flatten(),
                _ => None,
            };
            let data = fetched.unwrap_or_default();
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&path, &data);
            data
        }
    };
    if bytes.is_empty() {
        return None;
    }
    tokio::task::spawn_blocking(move || {
        let img = image::load_from_memory(&bytes).ok()?.thumbnail(96, 96).to_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        Some(UiEvent::Avatar { user, size, rgba: img.into_raw() })
    })
    .await
    .ok()
    .flatten()
}

static CONTACTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn user_of(jid: &str) -> String {
    jid.split('@').next().unwrap_or("").split(':').next().unwrap_or("").to_owned()
}

/// Rufnummer und LID eines Chats (WhatsApp nutzt je nach Stelle die eine oder andere).
async fn aliases(client: &Client, jid: &Jid) -> Vec<String> {
    let mut v = vec![jid.user.to_string()];
    if let Ok(Some(e)) = client.get_lid_pn_entry(jid).await {
        for u in [e.lid.to_string(), e.phone_number.to_string()] {
            if !v.contains(&u) {
                v.push(u);
            }
        }
    }
    v
}

fn range_of(chat: &Jid, last: Option<LastMsg>) -> Option<whatsapp_rust::features::SyncActionMessageRange> {
    let l = last?;
    let participant = if l.sender.is_empty() || !chat.is_group() {
        None
    } else {
        l.sender.parse::<Jid>().ok()
    };
    let key = whatsapp_rust::features::message_key(l.id, chat, l.from_me, participant.as_ref());
    Some(whatsapp_rust::features::message_range(l.ts, None, vec![(key, l.ts)]))
}

/// Datei hochladen und als Foto (Bilder) oder Dokument senden.
async fn send_file(
    client: &Client,
    send: &(impl Fn(UiEvent) + Clone),
    dir: &std::path::Path,
    chat: &Jid,
    path: &std::path::Path,
    caption: String,
) -> Result<(), String> {
    use whatsapp_rust::wacore::download::MediaType as M;
    let data = tokio::fs::read(path).await.map_err(|e| format!("{}: {e}", path.display()))?;
    if data.len() > 100 * 1024 * 1024 {
        return Err("Datei ist größer als 100 MB".into());
    }
    let mime = mime_of(path);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // GIFs schickt WhatsApp als Video; ohne Umwandlung gehen sie als Dokument.
    let picture = matches!(mime, "image/jpeg" | "image/png" | "image/webp");
    let decoded = if picture {
        let d = data.clone();
        tokio::task::spawn_blocking(move || image::load_from_memory(&d).ok()).await.ok().flatten()
    } else {
        None
    };
    let mut mref = MediaRef {
        kind: if decoded.is_some() { "image".into() } else { "document".into() },
        mime: mime.into(),
        name: name.clone(),
        len: data.len() as u64,
        ..Default::default()
    };
    if let Some(img) = &decoded {
        mref.w = img.width();
        mref.h = img.height();
    }
    let temp = format!("local-f-{}", chrono::Utc::now().timestamp_millis());
    let _ = std::fs::create_dir_all(dir);
    let local = dir.join(format!("{}.{}", safe_name(&temp), mref.ext()));
    let _ = std::fs::write(&local, &data);
    send(UiEvent::Message(ChatMessage {
        chat: chat.clone(),
        is_group: chat.is_group(),
        from_me: true,
        sender_name: String::new(),
        sender: String::new(),
        text: caption.clone(),
        ts: chrono::Utc::now().timestamp(),
        id: temp.clone(),
        status: 0,
        media: Some(mref.clone()),
        quote: None,
    }));
    let key = chat.to_string();
    let fail = |e: String| {
        send(UiEvent::SendFailed { chat: key.clone(), temp: temp.clone(), err: e.clone() });
        e
    };
    let mt = if decoded.is_some() { M::Image } else { M::Document };
    let up = client.upload(data, mt, Default::default()).await.map_err(|e| fail(e.to_string()))?;
    let cap = if caption.is_empty() { None } else { Some(caption) };
    let msg = if let Some(img) = &decoded {
        wa::Message {
            image_message: whatsapp_rust::waproto::buffa::MessageField::some(wa::message::ImageMessage {
                url: Some(up.url.clone()),
                direct_path: Some(up.direct_path.clone()),
                media_key: Some(up.media_key.to_vec()),
                file_sha256: Some(up.file_sha256.to_vec()),
                file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                file_length: Some(up.file_length),
                media_key_timestamp: Some(up.media_key_timestamp),
                mimetype: Some(mime.into()),
                width: Some(img.width()),
                height: Some(img.height()),
                jpeg_thumbnail: Some(jpeg_thumb(img)),
                caption: cap,
                ..Default::default()
            }),
            ..Default::default()
        }
    } else {
        wa::Message {
            document_message: whatsapp_rust::waproto::buffa::MessageField::some(wa::message::DocumentMessage {
                url: Some(up.url.clone()),
                direct_path: Some(up.direct_path.clone()),
                media_key: Some(up.media_key.to_vec()),
                file_sha256: Some(up.file_sha256.to_vec()),
                file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                file_length: Some(up.file_length),
                media_key_timestamp: Some(up.media_key_timestamp),
                mimetype: Some(mime.into()),
                file_name: Some(name.clone()),
                title: Some(name),
                caption: cap,
                ..Default::default()
            }),
            ..Default::default()
        }
    };
    let r = client.send_message(chat, msg).await.map_err(|e| fail(e.to_string()))?;
    // Lokale Kopie unter der echten ID ablegen, damit sie nicht erneut geladen wird.
    let _ = std::fs::rename(&local, dir.join(format!("{}.{}", safe_name(&r.message_id), mref.ext())));
    send(UiEvent::Sent { chat: key, temp, id: r.message_id });
    Ok(())
}

pub fn spawn(
    dir: std::path::PathBuf,
    ui_tx: Sender<UiEvent>,
    notify_state: SharedNotify,
    repaint: impl Fn() + Send + Sync + Clone + 'static,
) -> tokio::sync::mpsc::UnboundedSender<Cmd> {
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<Cmd>();

    std::thread::Builder::new()
        .name("wa-backend".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                let send = {
                    let tx = ui_tx.clone();
                    let rp = repaint.clone();
                    move |e: UiEvent| {
                        let _ = tx.send(e);
                        rp();
                    }
                };

                let _ = std::fs::create_dir_all(&dir);
                let lp = dir.join("wa-snap.log");
                if std::fs::metadata(&lp).map(|m| m.len() > 512_000).unwrap_or(false) {
                    let _ = std::fs::remove_file(&lp);
                }
                let _ = LOG_PATH.set(lp);
                log(format!("--- Start v{} ---", env!("CARGO_PKG_VERSION")));
                let avatar_dir = dir.join("avatars");
                let media_dir = dir.join("media");
                let _ = MEDIA_DIR.set(media_dir.clone());
                let store = match SqliteStore::new(&dir.join("whatsapp.db").to_string_lossy()).await {
                    Ok(s) => s,
                    Err(e) => {
                        send(UiEvent::Status(format!("Datenbank-Fehler: {e}")));
                        return;
                    }
                };

                let (s_qr, s_conn, s_out, s_msg, s_ev) =
                    (send.clone(), send.clone(), send.clone(), send.clone(), send.clone());
                let n_msg = notify_state.clone();

                // Beim Verknüpfen den vollständigen Verlauf (365 Tage) und das Adressbuch anfordern.
                let props = whatsapp_rust::wacore::store::DevicePropsOverride::new()
                    .with_platform_type(wa::device_props::PlatformType::UWP)
                    .with_require_full_sync(true)
                    .with_history_sync_config(wa::device_props::HistorySyncConfig {
                        full_sync_days_limit: Some(365),
                        on_demand_ready: Some(true),
                        complete_on_demand_ready: Some(true),
                        ..whatsapp_rust::wacore::store::device::default_history_sync_config()
                    });

                let bot = Bot::builder()
                    .with_backend(store)
                    .with_device_props(props)
                    .on_qr_code(move |code, _timeout| {
                        let s = s_qr.clone();
                        async move { s(UiEvent::Qr(code)) }
                    })
                    .on_connected(move |client| {
                        let s = s_conn.clone();
                        async move {
                            log("Verbunden");
                            s(UiEvent::Connected);
                            let me: Vec<String> = [client.pn(), client.lid()].into_iter().flatten().map(|j| j.user.to_string()).collect();
                            s(UiEvent::Me(me));
                            match client.groups().get_participating().await {
                                Ok(map) => {
                                    log(format!("Gruppen vom Server: {}", map.len()));
                                    s(UiEvent::Groups(
                                        map.into_iter().map(|(j, m)| (j, m.subject)).collect(),
                                    ));
                                }
                                Err(e) => log(format!("Gruppenliste fehlgeschlagen: {e}")),
                            }
                        }
                    })
                    .on_logged_out(move |_info| {
                        let s = s_out.clone();
                        async move {
                            log("Abgemeldet");
                            s(UiEvent::LoggedOut)
                        }
                    })
                    .on_message(move |ctx| {
                        let s = s_msg.clone();
                        let st = n_msg.clone();
                        async move {
                            let info = &ctx.info;
                            log(format!(
                                "Nachricht {} von_mir={} typ='{}'",
                                info.source.chat, info.source.is_from_me, info.media_type
                            ));
                            let base = ctx.message.get_base_message();
                            if let Some(r) = base.reaction_message.as_option() {
                                if let Some(target) = r.key.as_option().and_then(|k| k.id.clone()) {
                                    let from = if info.source.is_from_me { String::new() } else { info.source.sender.user.to_string() };
                                    s(UiEvent::Reaction { target, from, emoji: r.text.clone().unwrap_or_default() });
                                }
                                return;
                            }
                            if let Some(pm) = base.protocol_message.as_option() {
                                use wa::message::protocol_message::Type as T;
                                let target = pm.key.as_option().and_then(|k| k.id.clone()).unwrap_or_default();
                                match pm.r#type {
                                    Some(T::REVOKE) if !target.is_empty() => s(UiEvent::Revoked { id: target }),
                                    Some(T::MESSAGE_EDIT) if !target.is_empty() => {
                                        if let Some(t) = pm.edited_message.as_option().and_then(|m| m.text_content().map(|t| t.to_owned())) {
                                            s(UiEvent::Edited { id: target, text: t });
                                        }
                                    }
                                    _ => {}
                                }
                                return;
                            }
                            if !is_visible(&ctx.message, &info.media_type) {
                                return;
                            }
                            let media = media_of(&ctx.message);
                            if let Some((_, Some(th), _)) = &media {
                                save_thumb(&info.id, th);
                            }
                            let text = match (&media, ctx.message.text_content()) {
                                (_, Some(t)) => t.to_owned(),
                                (Some((_, _, cap)), None) => cap.clone().unwrap_or_default(),
                                (None, None) => describe(&ctx.message, &info.media_type),
                            };
                            let msg = ChatMessage {
                                chat: info.source.chat.clone(),
                                is_group: info.source.is_group,
                                from_me: info.source.is_from_me,
                                sender_name: if info.push_name.is_empty() {
                                    info.source.sender.user.to_string()
                                } else {
                                    info.push_name.clone()
                                },
                                sender: info.source.sender.to_string(),
                                text,
                                ts: info.timestamp.timestamp(),
                                id: info.id.to_string(),
                                status: if info.source.is_from_me { 1 } else { 0 },
                                media: media.map(|m| m.0),
                                quote: quote_of(&ctx.message),
                            };
                            maybe_notify(&st, &msg);
                            s(UiEvent::Message(msg));
                        }
                    })
                    .on_event_for(
                        &[
                            EventKind::ContactUpdate,
                            EventKind::PushNameUpdate,
                            EventKind::HistorySync,
                            EventKind::MuteUpdate,
                            EventKind::PinUpdate,
                            EventKind::ArchiveUpdate,
                            EventKind::MarkChatAsReadUpdate,
                            EventKind::Receipt,
                        ],
                        move |event, client| {
                            let s = s_ev.clone();
                            async move {
                                match &*event {
                                    Event::MuteUpdate(m) => {
                                        let end = m.action.mute_end_timestamp.unwrap_or(0);
                                        let now_ms = chrono::Utc::now().timestamp_millis();
                                        let on = m.action.muted.unwrap_or(false)
                                            && (end <= 0 || end > now_ms);
                                        let users = aliases(&client, &m.jid).await;
                                        log(format!("Stumm-Abgleich {} -> {on} {users:?}", m.jid));
                                        s(UiEvent::Muted { users, on });
                                    }
                                    Event::PinUpdate(p) => {
                                        let on = p.action.pinned.unwrap_or(false);
                                        let users = aliases(&client, &p.jid).await;
                                        log(format!("Anheften-Abgleich {} -> {on}", p.jid));
                                        s(UiEvent::Pinned { users, on });
                                    }
                                    Event::ArchiveUpdate(a) => {
                                        let on = a.action.archived.unwrap_or(false);
                                        let users = aliases(&client, &a.jid).await;
                                        log(format!("Archiv-Abgleich {} -> {on}", a.jid));
                                        s(UiEvent::Archived { users, on });
                                    }
                                    Event::MarkChatAsReadUpdate(r) => {
                                        let read = r.action.read.unwrap_or(true);
                                        let users = aliases(&client, &r.jid).await;
                                        s(UiEvent::ChatRead { users, read });
                                    }
                                    Event::Receipt(r) => {
                                        use whatsapp_rust::wacore::types::presence::ReceiptType;
                                        let status = match r.r#type {
                                            ReceiptType::Delivered => 2,
                                            ReceiptType::Read | ReceiptType::Played => 3,
                                            ReceiptType::ReadSelf | ReceiptType::PlayedSelf => {
                                                let users = aliases(&client, &r.source.chat).await;
                                                s(UiEvent::ChatRead { users, read: true });
                                                return;
                                            }
                                            _ => return,
                                        };
                                        let ids = r.message_ids.iter().map(|i| i.to_string()).collect();
                                        s(UiEvent::Receipt { ids, status });
                                    }
                                    Event::ContactUpdate(c) => {
                                        let n = CONTACTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                                        if n == 1 || n % 50 == 0 {
                                            log(format!("Adressbuch-Einträge empfangen: {n}"));
                                        }
                                        let a = &c.action;
                                        let name = a
                                            .full_name
                                            .clone()
                                            .or_else(|| a.first_name.clone())
                                            .unwrap_or_default();
                                        if name.is_empty() {
                                            return;
                                        }
                                        let mut users = vec![c.jid.user.to_string()];
                                        if let Some(l) = &a.lid_jid {
                                            users.push(user_of(l));
                                        }
                                        if let Some(p) = &a.pn_jid {
                                            users.push(user_of(p));
                                        }
                                        for user in users.into_iter().filter(|u| !u.is_empty()) {
                                            s(UiEvent::Contact { user, name: name.clone(), from_book: true });
                                        }
                                    }
                                    Event::HistorySync(h) => {
                                        if let Some(hs) = h.get() {
                                            convert_history(hs, &s);
                                        }
                                    }
                                    Event::PushNameUpdate(p) if !p.new_push_name.is_empty() => {
                                        s(UiEvent::Contact {
                                            user: p.jid.user.to_string(),
                                            name: p.new_push_name.clone(),
                                            from_book: false,
                                        });
                                    }
                                    _ => {}
                                }
                            }
                        },
                    )
                    .build()
                    .await;

                let bot = match bot {
                    Ok(b) => b,
                    Err(e) => {
                        send(UiEvent::Status(format!("Start fehlgeschlagen: {e}")));
                        return;
                    }
                };

                send(UiEvent::Status("Verbinde …".into()));
                let client: Arc<Client> = bot.client();
                let mut handle = bot.spawn();

                // Ergebnis einer Einstellungsänderung melden.
                let report = |send: &dyn Fn(UiEvent), what: &str, r: Result<(), String>| match r {
                    Ok(()) => log(format!("{what}: ok")),
                    Err(e) => {
                        log(format!("{what} fehlgeschlagen: {e}"));
                        send(UiEvent::Info(format!("{what} fehlgeschlagen: {e}")));
                    }
                };

                loop {
                    tokio::select! {
                        _ = &mut handle => break,
                        cmd = cmd_rx.recv() => {
                            let Some(cmd) = cmd else {
                                handle.shutdown().await;
                                break;
                            };
                            let (client, send) = (client.clone(), send.clone());
                            match cmd {
                                Cmd::Send { chat, text, temp, quote } => {
                                    tokio::spawn(async move {
                                        let key = chat.to_string();
                                        let msg = match quote {
                                            Some(q) => {
                                                let sender = if q.sender.is_empty() { chat.clone() } else { q.sender.parse::<Jid>().unwrap_or_else(|_| chat.clone()) };
                                                let ctx = whatsapp_rust::wacore::proto_helpers::build_quote_context_with_info(
                                                    q.id.clone(), &sender, &chat, &chat, &wa::Message::text(q.text.clone()),
                                                );
                                                wa::Message::text_with_context(text, ctx)
                                            }
                                            None => wa::Message::text(text),
                                        };
                                        match client.send_message(&chat, msg).await {
                                            Ok(r) => send(UiEvent::Sent { chat: key, temp, id: r.message_id }),
                                            Err(e) => {
                                                log(format!("Senden fehlgeschlagen: {e}"));
                                                send(UiEvent::SendFailed { chat: key, temp, err: e.to_string() });
                                            }
                                        }
                                    });
                                }
                                Cmd::React { chat, target, from_me, emoji } => {
                                    tokio::spawn(async move {
                                        let participant = if chat.is_group() && !from_me && !target.sender.is_empty() { Some(target.sender.clone()) } else { None };
                                        let msg = wa::Message {
                                            reaction_message: whatsapp_rust::waproto::buffa::MessageField::some(wa::message::ReactionMessage {
                                                key: whatsapp_rust::waproto::buffa::MessageField::some(wa::MessageKey {
                                                    remote_jid: Some(chat.to_string()),
                                                    from_me: Some(from_me),
                                                    id: Some(target.id.clone()),
                                                    participant,
                                                }),
                                                text: Some(emoji),
                                                sender_timestamp_ms: Some(chrono::Utc::now().timestamp_millis()),
                                                ..Default::default()
                                            }),
                                            ..Default::default()
                                        };
                                        if let Err(e) = client.send_message(&chat, msg).await {
                                            log(format!("Reaktion fehlgeschlagen: {e}"));
                                            send(UiEvent::Info(format!("Reaktion fehlgeschlagen: {e}")));
                                        }
                                    });
                                }
                                Cmd::Revoke { chat, id } => {
                                    tokio::spawn(async move {
                                        match client.revoke_message(chat, id.clone(), whatsapp_rust::RevokeType::Sender).await {
                                            Ok(()) => send(UiEvent::Revoked { id }),
                                            Err(e) => send(UiEvent::Info(format!("Löschen fehlgeschlagen: {e}"))),
                                        }
                                    });
                                }
                                Cmd::Avatar { jid } => {
                                    let dir = avatar_dir.clone();
                                    tokio::spawn(async move {
                                        if let Some(ev) = load_avatar(client, dir, jid).await {
                                            send(ev);
                                        }
                                    });
                                }
                                Cmd::SetMute { jid, on } => {
                                    tokio::spawn(async move {
                                        let ca = client.chat_actions();
                                        let r = if on { ca.mute_chat(&jid).await } else { ca.unmute_chat(&jid).await };
                                        report(&send, &format!("Stummschalten {jid}"), r.map_err(|e| e.to_string()));
                                    });
                                }
                                Cmd::SetPin { jid, on } => {
                                    tokio::spawn(async move {
                                        let ca = client.chat_actions();
                                        let r = if on { ca.pin_chat(&jid).await } else { ca.unpin_chat(&jid).await };
                                        report(&send, &format!("Anheften {jid}"), r.map_err(|e| e.to_string()));
                                    });
                                }
                                Cmd::SetArchive { jid, on, last } => {
                                    tokio::spawn(async move {
                                        let ca = client.chat_actions();
                                        let range = range_of(&jid, last);
                                        let r = if on {
                                            ca.archive_chat(&jid, range).await
                                        } else {
                                            ca.unarchive_chat(&jid, range).await
                                        };
                                        report(&send, &format!("Archivieren {jid}"), r.map_err(|e| e.to_string()));
                                    });
                                }
                                Cmd::MarkRead { jid, msgs, last } => {
                                    tokio::spawn(async move {
                                        // Lesebestätigungen, gruppiert nach Absender (in Gruppen nötig).
                                        let mut by_sender: HashMap<String, Vec<String>> = HashMap::new();
                                        for (id, sender) in msgs {
                                            by_sender.entry(sender).or_default().push(id);
                                        }
                                        for (sender, ids) in by_sender {
                                            let sj = if jid.is_group() { sender.parse::<Jid>().ok() } else { None };
                                            let refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
                                            if let Err(e) = client.mark_as_read(&jid, sj.as_ref(), &refs).await {
                                                log(format!("Lesebestätigung fehlgeschlagen: {e}"));
                                            }
                                        }
                                        let range = range_of(&jid, last);
                                        let r = client.chat_actions().mark_chat_as_read(&jid, true, range).await;
                                        if let Err(e) = r {
                                            log(format!("Als gelesen markieren fehlgeschlagen: {e}"));
                                        }
                                    });
                                }
                                Cmd::SetPushName(n) => {
                                    tokio::spawn(async move {
                                        let r = client.profile().set_push_name(&n).await;
                                        let ok = r.is_ok();
                                        report(&send, "Profilname", r.map_err(|e| e.to_string()));
                                        if ok {
                                            send(UiEvent::Info("Profilname bei WhatsApp gespeichert".into()));
                                        }
                                    });
                                }
                                Cmd::SetAbout(t) => {
                                    tokio::spawn(async move {
                                        let r = client.profile().set_status_text(&t).await;
                                        report(&send, "Info-Text", r.map_err(|e| e.to_string()));
                                    });
                                }
                                Cmd::TestNotify => {
                                    std::thread::spawn(move || {
                                        let m = match show_notification("wa-snap", "Test-Benachrichtigung") {
                                            Ok(()) => "Test-Benachrichtigung verschickt. Wenn keine erschien, blockiert das System sie (Systemeinstellungen → Benachrichtigungen).".to_owned(),
                                            Err(e) => format!("Benachrichtigung fehlgeschlagen: {e}"),
                                        };
                                        send(UiEvent::Info(m));
                                    });
                                }
                                Cmd::LoadMedia { id, media } => {
                                    let dir = media_dir.clone();
                                    tokio::spawn(async move {
                                        let thumb = dir.join(format!("{}.thumb", safe_name(&id)));
                                        let full = dir.join(format!("{}.{}", safe_name(&id), media.ext()));
                                        if !full.exists() {
                                            if let Ok(b) = std::fs::read(&thumb) {
                                                let id2 = id.clone();
                                                if let Ok(Some((size, rgba))) = tokio::task::spawn_blocking(move || decode_rgba(&b, 400)).await {
                                                    send(UiEvent::Image { id: id2, size, rgba, full: false });
                                                }
                                            }
                                        }
                                        if !media.is_picture() {
                                            return;
                                        }
                                        match ensure_file(&client, &dir, &id, &media).await {
                                            Ok(path) => {
                                                let r = tokio::task::spawn_blocking(move || std::fs::read(path).ok().and_then(|b| decode_rgba(&b, 640))).await;
                                                if let Ok(Some((size, rgba))) = r {
                                                    send(UiEvent::Image { id, size, rgba, full: true });
                                                }
                                            }
                                            Err(e) => log(format!("Medium {id} laden fehlgeschlagen: {e}")),
                                        }
                                    });
                                }
                                Cmd::OpenMedia { id, media } => {
                                    let dir = media_dir.clone();
                                    tokio::spawn(async move {
                                        send(UiEvent::Info(format!("{} wird geladen …", media.label())));
                                        match ensure_file(&client, &dir, &id, &media).await {
                                            Ok(path) => {
                                                let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                                                send(UiEvent::Info(String::new()));
                                            }
                                            Err(e) => send(UiEvent::Info(format!("Herunterladen fehlgeschlagen: {e}"))),
                                        }
                                    });
                                }
                                Cmd::SendFile { chat, path, caption } => {
                                    let dir = media_dir.clone();
                                    tokio::spawn(async move {
                                        let path = match path {
                                            Some(p) => p,
                                            None => match rfd::AsyncFileDialog::new().set_title("Datei senden").pick_file().await {
                                                Some(f) => f.path().to_path_buf(),
                                                None => return,
                                            },
                                        };
                                        if let Err(e) = send_file(&client, &send, &dir, &chat, &path, caption).await {
                                            log(format!("Datei senden fehlgeschlagen: {e}"));
                                            send(UiEvent::Info(format!("Datei senden fehlgeschlagen: {e}")));
                                        }
                                    });
                                }
                                Cmd::SendSticker { chat, media } => {
                                    tokio::spawn(async move {
                                        let msg = wa::Message {
                                            sticker_message: whatsapp_rust::waproto::buffa::MessageField::some(wa::message::StickerMessage {
                                                url: Some(media.url.clone()),
                                                direct_path: Some(media.direct_path.clone()),
                                                media_key: Some(media.key.clone()),
                                                file_sha256: Some(media.sha.clone()),
                                                file_enc_sha256: Some(media.enc_sha.clone()),
                                                file_length: Some(media.len),
                                                mimetype: Some("image/webp".into()),
                                                width: Some(media.w),
                                                height: Some(media.h),
                                                is_animated: Some(media.animated),
                                                ..Default::default()
                                            }),
                                            ..Default::default()
                                        };
                                        let temp = format!("local-st-{}", chrono::Utc::now().timestamp_millis());
                                        send(UiEvent::Message(ChatMessage {
                                            chat: chat.clone(),
                                            is_group: chat.is_group(),
                                            from_me: true,
                                            sender_name: String::new(),
                                            sender: String::new(),
                                            text: String::new(),
                                            ts: chrono::Utc::now().timestamp(),
                                            id: temp.clone(),
                                            status: 0,
                                            media: Some(media),
                                            quote: None,
                                        }));
                                        let key = chat.to_string();
                                        match client.send_message(&chat, msg).await {
                                            Ok(r) => send(UiEvent::Sent { chat: key, temp, id: r.message_id }),
                                            Err(e) => send(UiEvent::SendFailed { chat: key, temp, err: e.to_string() }),
                                        }
                                    });
                                }
                                Cmd::Quit => {
                                    handle.shutdown().await;
                                    break;
                                }
                            }
                        }
                    }
                }
            });
        })
        .expect("backend thread");

    cmd_tx
}
