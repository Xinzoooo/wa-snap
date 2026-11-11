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
    Muted { users: Vec<String>, on: bool, until: Option<i64> },
    Pinned { users: Vec<String>, on: bool },
    Archived { users: Vec<String>, on: bool },
    ChatRead { users: Vec<String>, read: bool },
    /// Rückmeldung für die Statuszeile.
    Info(String),
    /// Alle Gruppen mit Namen (direkt vom Server abgefragt).
    Groups(Vec<(Jid, String)>),
    /// Jemand tippt (oder nimmt auf) in einem Chat.
    Typing { chat: String, from: String, on: bool, recording: bool },
    /// Online-Status eines Kontakts; `last_seen` in Sekunden.
    Presence { user: String, online: bool, last_seen: Option<i64> },
    /// Stern gesetzt/entfernt (auch vom Handy).
    Starred { id: String, on: bool },
    /// Entschlüsselte Stimme einer Umfrage; leere Liste = Stimme zurückgezogen.
    PollVote { poll: String, voter: String, options: Vec<String> },
    /// Gruppenmitglieder (für @Erwähnungen): Gruppe → [(Nummer/LID, Nummer)]
    Members { group: String, members: Vec<(String, String)> },
    /// Eigene Nummer und LID (um eigene zitierte Nachrichten zu erkennen).
    Me(Vec<String>),
    /// Reaktion auf eine Nachricht; leeres Emoji = Reaktion entfernt.
    Reaction { target: String, from: String, emoji: String },
    /// Nachricht wurde für alle gelöscht.
    Revoked { id: String },
    /// Nachricht wurde bearbeitet.
    Edited { id: String, text: String },
    /// Datenschutz-Einstellungen von WhatsApp: (Kategorie, Wert), z. B. ("last", "contacts").
    Privacy(Vec<(String, String)>),
    /// Gruppendetails für die Info-Seite.
    GroupInfo(GroupDetails),
    /// Blockierte Kontakte (JIDs).
    Blocklist(Vec<String>),
    /// Datei eines Mediums liegt lokal bereit (z. B. Sprachnachricht zum Abspielen).
    FileReady { id: String, path: std::path::PathBuf },
    /// Animierter Sticker: Einzelbilder mit Dauer in Millisekunden.
    Anim { id: String, frames: Vec<([usize; 2], Vec<u8>, u32)> },
    /// Ergebnisse der GIF-Suche.
    GifResults(Vec<GifItem>),
    /// Bild/Sticker einer Nachricht dekodiert (Vorschau oder volle Größe).
    Image { id: String, size: [usize; 2], rgba: Vec<u8>, full: bool },
}

/// Was die Oberfläche dem Backend aufträgt.
pub enum Cmd {
    Send { chat: Jid, text: String, temp: String, quote: Option<Quote>, mentions: Vec<String>, expiration: u32 },
    /// Eigene Nachricht bearbeiten.
    Edit { chat: Jid, id: String, text: String },
    /// Nachricht mit Stern markieren (oder Markierung entfernen).
    Star { chat: Jid, id: String, from_me: bool, sender: String, on: bool },
    /// Ältere Nachrichten vom Handy anfordern (vor der ältesten bekannten).
    LoadOlder { chat: Jid, id: String, from_me: bool, ts: i64 },
    /// In einer Umfrage abstimmen.
    Vote { chat: Jid, poll: String, creator: String, secret: Vec<u8>, options: Vec<String> },
    /// Gespeicherte Umfragen bekannt machen, damit Stimmen entschlüsselt werden können.
    RegisterPolls(Vec<(String, Vec<u8>, String, Vec<String>)>),
    /// Mit Emoji reagieren (leer = Reaktion entfernen).
    React { chat: Jid, target: Quote, from_me: bool, emoji: String },
    /// Eigene Nachricht für alle löschen.
    Revoke { chat: Jid, id: String },
    Avatar { jid: Jid },
    /// Stummschalten; `until_ms` = Ende in Millisekunden, None = dauerhaft.
    SetMute { jid: Jid, on: bool, until_ms: Option<i64> },
    /// Eigenen Tippstatus senden.
    Typing { chat: Jid, on: bool },
    /// Online (Fenster aktiv) oder offline melden.
    SetOnline(bool),
    /// Online-Status eines Kontakts abonnieren.
    Subscribe(Jid),
    /// Nachricht weiterleiten (Medien ohne neuen Upload).
    Forward { chat: Jid, msg: ChatMessage },
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
    FetchPrivacy,
    GroupInfo(Jid),
    /// Neue Gruppe mit Telefonnummern (nur Ziffern) anlegen.
    CreateGroup { name: String, numbers: Vec<String> },
    GroupRename { jid: Jid, name: String },
    GroupAdd { jid: Jid, numbers: Vec<String> },
    GroupRemove { jid: Jid, member: String },
    GroupAdmin { jid: Jid, member: String, on: bool },
    GroupLeave(Jid),
    GroupInvite(Jid),
    /// Datenschutz ändern, Kategorie/Wert wie bei UiEvent::Privacy.
    SetPrivacy { cat: String, val: String },
    /// Standarddauer für selbstlöschende Nachrichten (Sekunden, 0 = aus).
    SetDefaultDisappearing(u32),
    /// Selbstlöschende Nachrichten für einen Chat (Sekunden, 0 = aus).
    SetChatDisappearing { chat: Jid, secs: u32 },
    /// Eigenes Profilbild setzen (ohne Pfad: Dateiauswahl).
    SetProfilePicture(Option<std::path::PathBuf>),
    FetchBlocklist,
    Block { jid: Jid, on: bool },
    /// Gerät bei WhatsApp abmelden.
    Logout,
    /// Datei eines Mediums herunterladen (falls nötig) und melden, wo sie liegt.
    FetchFile { id: String, media: MediaRef },
    /// Sprachnachricht senden (fertig kodiertes Ogg/Opus).
    SendVoice { chat: Jid, ogg: Vec<u8>, seconds: u32, waveform: Vec<u8> },
    /// Bild als eigenen Sticker senden (wird auf 512×512 WebP umgerechnet).
    MakeSticker { chat: Jid, path: Option<std::path::PathBuf> },
    /// GIFs bei Giphy suchen (eigener API-Schlüssel nötig).
    GifSearch { query: String, key: String },
    /// GIF aus der Suche senden.
    SendGif { chat: Jid, gif: GifItem },
    /// Bekannten Sticker erneut senden (ohne neuen Upload).
    SendSticker { chat: Jid, media: MediaRef },
    Quit,
}

/// Details einer Gruppe.
#[derive(Clone, Default)]
#[allow(dead_code)]
pub struct GroupDetails {
    pub group: String,
    pub subject: String,
    pub description: String,
    /// (JID, Telefonnummer-User, Admin, Ersteller)
    pub members: Vec<(String, String, bool, bool)>,
    pub created: i64,
    pub i_am_admin: bool,
}

/// Ein Treffer der GIF-Suche.
#[derive(Clone)]
pub struct GifItem {
    pub id: String,
    pub title: String,
    pub mp4: String,
    pub w: u32,
    pub h: u32,
    /// Vorschaubild (RGBA) und dessen Größe
    pub preview: Option<([usize; 2], Vec<u8>)>,
    /// Vorschau als JPEG für die Nachricht
    pub thumb: Vec<u8>,
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
    pub x: MsgExtra,
}

/// Weitere Eigenschaften einer Nachricht (Umfrage, Standort, Kontakt, Link, Markierungen).
#[derive(Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MsgExtra {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Extra>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub edited: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub view_once: bool,
    /// Selbstlöschend nach so vielen Sekunden (0 = aus).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub expiration: u32,
    /// Erwähnte Nummern/LIDs (ohne @server).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
    /// Vom Absender gelöscht, Inhalt aber behalten (Einstellung „Gelöschte Nachrichten behalten“).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    /// Frühere Fassungen bearbeiteter Nachrichten (älteste zuerst).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prev: Vec<String>,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(tag = "t")]
pub enum Extra {
    Poll { name: String, options: Vec<String>, secret: Vec<u8>, multi: bool },
    Location { lat: f64, lon: f64, name: String, address: String, live: bool },
    Contact { name: String, vcard: String },
    Link { url: String, title: String, description: String },
}

impl Extra {
    pub fn label(&self) -> String {
        match self {
            Extra::Poll { name, .. } => format!("📊 {name}"),
            Extra::Location { name, live, .. } => {
                let base = if *live { "📍 Live-Standort" } else { "📍 Standort" };
                if name.is_empty() { base.to_owned() } else { format!("{base}: {name}") }
            }
            Extra::Contact { name, .. } => format!("👤 {name}"),
            Extra::Link { .. } => String::new(),
        }
    }
}

/// Umfragen, deren Stimmen das Backend entschlüsseln kann: ID → (Geheimnis, Ersteller, Optionen).
static POLLS: std::sync::Mutex<Option<HashMap<String, (Vec<u8>, String, Vec<String>)>>> = std::sync::Mutex::new(None);

fn remember_poll(id: &str, secret: &[u8], creator: &str, options: &[String]) {
    if secret.is_empty() {
        return;
    }
    if let Ok(mut g) = POLLS.lock() {
        g.get_or_insert_with(HashMap::new).insert(id.to_owned(), (secret.to_vec(), creator.to_owned(), options.to_vec()));
    }
}

/// Umfrage, Standort, Kontakt, Link, Einmal-Ansicht, Selbstlöschen und Erwähnungen lesen.
pub fn extra_of(msg: &wa::Message) -> MsgExtra {
    let b = msg.get_base_message();
    let mut x = MsgExtra { view_once: msg.is_view_once(), ..Default::default() };
    let secret = msg
        .message_context_info
        .as_option()
        .and_then(|c| c.message_secret.clone())
        .or_else(|| b.message_context_info.as_option().and_then(|c| c.message_secret.clone()))
        .unwrap_or_default();
    let poll = b
        .poll_creation_message
        .as_option()
        .or_else(|| b.poll_creation_message_v2.as_option())
        .or_else(|| b.poll_creation_message_v3.as_option());
    if let Some(p) = poll {
        x.kind = Some(Extra::Poll {
            name: p.name.clone().unwrap_or_default(),
            options: p.options.iter().filter_map(|o| o.option_name.clone()).collect(),
            secret,
            multi: p.selectable_options_count.unwrap_or(0) != 1,
        });
    } else if let Some(l) = b.location_message.as_option() {
        x.kind = Some(Extra::Location {
            lat: l.degrees_latitude.unwrap_or(0.0),
            lon: l.degrees_longitude.unwrap_or(0.0),
            name: l.name.clone().unwrap_or_default(),
            address: l.address.clone().unwrap_or_default(),
            live: false,
        });
    } else if let Some(l) = b.live_location_message.as_option() {
        x.kind = Some(Extra::Location {
            lat: l.degrees_latitude.unwrap_or(0.0),
            lon: l.degrees_longitude.unwrap_or(0.0),
            name: String::new(),
            address: String::new(),
            live: true,
        });
    } else if let Some(c) = b.contact_message.as_option() {
        x.kind = Some(Extra::Contact { name: c.display_name.clone().unwrap_or_default(), vcard: c.vcard.clone().unwrap_or_default() });
    } else if let Some(e) = b.extended_text_message.as_option() {
        let url = e.matched_text.clone().unwrap_or_default();
        let title = e.title.clone().unwrap_or_default();
        if !url.is_empty() && !title.is_empty() {
            x.kind = Some(Extra::Link { url, title, description: e.description.clone().unwrap_or_default() });
        }
    }
    let ci = b
        .extended_text_message
        .as_option()
        .and_then(|m| m.context_info.as_option())
        .or_else(|| b.image_message.as_option().and_then(|m| m.context_info.as_option()))
        .or_else(|| b.video_message.as_option().and_then(|m| m.context_info.as_option()))
        .or_else(|| b.document_message.as_option().and_then(|m| m.context_info.as_option()));
    if let Some(ci) = ci {
        x.expiration = ci.expiration.unwrap_or(0);
        x.mentions = ci.mentioned_jid.iter().map(|j| user_of(j)).collect();
    }
    x
}

/// Was das Backend für Benachrichtigungen wissen muss. Die Oberfläche hält es aktuell.
#[derive(Default)]
pub struct NotifyState {
    pub enabled: bool,
    /// Auch bei Gruppennachrichten benachrichtigen
    pub groups: bool,
    /// Ton abspielen
    pub sound: bool,
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
        .appname("WhatsSnap")
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
                .args(["-a", "WhatsSnap", "-i", "wa-snap", title, body])
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

/// Kurzer Hinweiston (Freedesktop-Klang über PipeWire/PulseAudio/libcanberra).
pub fn play_notify_sound() {
    std::thread::spawn(|| {
        let f = "/usr/share/sounds/freedesktop/stereo/message-new-instant.oga";
        let ok = std::path::Path::new(f).exists()
            && (std::process::Command::new("pw-play").arg(f).status().map(|s| s.success()).unwrap_or(false)
                || std::process::Command::new("paplay").arg(f).status().map(|s| s.success()).unwrap_or(false));
        if !ok {
            let _ = std::process::Command::new("canberra-gtk-play").args(["-i", "message-new-instant"]).status();
        }
    });
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
        if !st.enabled || st.muted.contains(&chat_user) || viewing || age > 300 || (m.is_group && !st.groups) {
            return;
        }
        if st.sound {
            play_notify_sound();
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
            None if m.text.is_empty() => m.x.kind.as_ref().map(|k| k.label()).unwrap_or_default(),
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
    crate::tray::BADGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
        let skip = c.messages.len().saturating_sub(300);
        for hm in &c.messages[skip..] {
            let wm = &hm.message;
            let Some(inner) = wm.message.as_option() else { continue };
            let key = &wm.key;
            let media = media_of(inner);
            let x = extra_of(inner);
            let text = match (inner.text_content(), &media, &x.kind) {
                (Some(t), _, _) => t.to_owned(),
                (None, Some((_, _, cap)), _) => cap.clone().unwrap_or_default(),
                (None, None, Some(_)) => String::new(),
                (None, None, None) => continue,
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
                x,
            });
            if let Some(m) = msgs.last() {
                if let Some(Extra::Poll { secret, options, .. }) = &m.x.kind {
                    remember_poll(&m.id, secret, &m.sender, options);
                }
            }
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

/// Nachricht zum Weiterleiten bauen (Medien verweisen auf dieselbe CDN-Datei).
fn forward_message(m: &ChatMessage) -> wa::Message {
    use whatsapp_rust::waproto::buffa::MessageField as F;
    let ci = || F::some(wa::ContextInfo { is_forwarded: Some(true), forwarding_score: Some(1), ..Default::default() });
    let cap = if m.text.is_empty() { None } else { Some(m.text.clone()) };
    let Some(md) = &m.media else {
        return wa::Message {
            extended_text_message: F::some(wa::message::ExtendedTextMessage { text: Some(m.text.clone()), context_info: ci(), ..Default::default() }),
            ..Default::default()
        };
    };
    let (url, dp, key, sha, enc, len, mime) = (
        Some(md.url.clone()),
        Some(md.direct_path.clone()),
        Some(md.key.clone()),
        Some(md.sha.clone()),
        Some(md.enc_sha.clone()),
        Some(md.len),
        Some(md.mime.clone()),
    );
    match md.kind.as_str() {
        "image" => wa::Message {
            image_message: F::some(wa::message::ImageMessage {
                url, direct_path: dp, media_key: key, file_sha256: sha, file_enc_sha256: enc, file_length: len, mimetype: mime,
                width: Some(md.w), height: Some(md.h), caption: cap, context_info: ci(), ..Default::default()
            }),
            ..Default::default()
        },
        "sticker" => wa::Message {
            sticker_message: F::some(wa::message::StickerMessage {
                url, direct_path: dp, media_key: key, file_sha256: sha, file_enc_sha256: enc, file_length: len, mimetype: mime,
                width: Some(md.w), height: Some(md.h), is_animated: Some(md.animated), context_info: ci(), ..Default::default()
            }),
            ..Default::default()
        },
        "video" | "gif" => wa::Message {
            video_message: F::some(wa::message::VideoMessage {
                url, direct_path: dp, media_key: key, file_sha256: sha, file_enc_sha256: enc, file_length: len, mimetype: mime,
                width: Some(md.w), height: Some(md.h), seconds: Some(md.seconds), gif_playback: Some(md.kind == "gif"),
                caption: cap, context_info: ci(), ..Default::default()
            }),
            ..Default::default()
        },
        "audio" | "ptt" => wa::Message {
            audio_message: F::some(wa::message::AudioMessage {
                url, direct_path: dp, media_key: key, file_sha256: sha, file_enc_sha256: enc, file_length: len, mimetype: mime,
                seconds: Some(md.seconds), ptt: Some(md.kind == "ptt"), context_info: ci(), ..Default::default()
            }),
            ..Default::default()
        },
        _ => wa::Message {
            document_message: F::some(wa::message::DocumentMessage {
                url, direct_path: dp, media_key: key, file_sha256: sha, file_enc_sha256: enc, file_length: len, mimetype: mime,
                file_name: Some(md.name.clone()), title: Some(md.name.clone()), caption: cap, context_info: ci(), ..Default::default()
            }),
            ..Default::default()
        },
    }
}

fn has_cmd(cmd: &str) -> bool {
    std::process::Command::new(cmd).arg("-version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok()
}

/// GIF-Datei mit ffmpeg in ein stummes MP4 umwandeln (so verschickt WhatsApp GIFs).
fn gif_to_mp4(path: &std::path::Path, out: &std::path::Path) -> bool {
    has_cmd("ffmpeg")
        && std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-i"])
            .arg(path)
            .args(["-movflags", "faststart", "-pix_fmt", "yuv420p", "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", "-an"])
            .arg(out)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
}

/// Vorschaubild (JPEG) und Größe eines Videos über ffmpeg, falls vorhanden.
fn video_thumb(path: &std::path::Path) -> Option<(Vec<u8>, u32, u32)> {
    if !has_cmd("ffmpeg") {
        return None;
    }
    let out = std::process::Command::new("ffmpeg")
        .args(["-loglevel", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
        .output()
        .ok()?;
    let img = image::load_from_memory(&out.stdout).ok()?;
    Some((jpeg_thumb(&img), img.width(), img.height()))
}

/// Bild auf 512×512 (transparent aufgefüllt) bringen und als WebP unter 100 KB kodieren.
fn to_sticker_webp(img: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let fitted = img.resize(512, 512, image::imageops::FilterType::Lanczos3).to_rgba8();
    let mut canvas = image::RgbaImage::from_pixel(512, 512, image::Rgba([0, 0, 0, 0]));
    let x = (512 - fitted.width()) / 2;
    let y = (512 - fitted.height()) / 2;
    image::imageops::overlay(&mut canvas, &fitted, x as i64, y as i64);
    let enc = webp::Encoder::from_rgba(canvas.as_raw(), 512, 512);
    for q in [80.0, 65.0, 50.0, 35.0] {
        let data = enc.encode(q).to_vec();
        if data.len() <= 100 * 1024 {
            return Ok(data);
        }
    }
    Ok(enc.encode(25.0).to_vec())
}

/// Animierten WebP-Sticker in Einzelbilder zerlegen (höchstens 60, verkleinert).
fn decode_anim(bytes: &[u8]) -> Option<Vec<([usize; 2], Vec<u8>, u32)>> {
    use image::AnimationDecoder;
    let dec = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    if !dec.has_animation() {
        return None;
    }
    let mut out = Vec::new();
    for f in dec.into_frames().take(60).flatten() {
        let (n, d) = f.delay().numer_denom_ms();
        let ms = if d == 0 { 100 } else { (n / d).max(20) };
        let img = image::DynamicImage::ImageRgba8(f.into_buffer()).thumbnail(256, 256).to_rgba8();
        out.push(([img.width() as usize, img.height() as usize], img.into_raw(), ms));
    }
    (out.len() > 1).then_some(out)
}

/// Bei Giphy suchen. Gibt bis zu 24 Treffer mit Vorschaubild zurück.
fn giphy_search(query: &str, key: &str) -> Result<Vec<GifItem>, String> {
    let url = format!(
        "https://api.giphy.com/v1/gifs/search?api_key={}&q={}&limit=24&rating=pg-13&lang=de",
        key,
        query.replace(' ', "+")
    );
    let body = ureq::get(&url).call().map_err(|e| e.to_string())?.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for g in v["data"].as_array().cloned().unwrap_or_default() {
        let im = &g["images"];
        let mp4 = im["original"]["mp4"].as_str().or_else(|| im["fixed_width"]["mp4"].as_str()).unwrap_or("").to_owned();
        let still = im["fixed_width_small_still"]["url"].as_str().or_else(|| im["fixed_width_still"]["url"].as_str()).unwrap_or("");
        if mp4.is_empty() {
            continue;
        }
        let num = |x: &serde_json::Value| x.as_str().and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut item = GifItem {
            id: g["id"].as_str().unwrap_or("").to_owned(),
            title: g["title"].as_str().unwrap_or("").to_owned(),
            mp4,
            w: num(&im["original"]["width"]),
            h: num(&im["original"]["height"]),
            preview: None,
            thumb: Vec::new(),
        };
        if !still.is_empty() {
            if let Ok(mut r) = ureq::get(still).call() {
                if let Ok(b) = r.body_mut().read_to_vec() {
                    if let Ok(img) = image::load_from_memory(&b) {
                        item.thumb = jpeg_thumb(&img);
                        let t = img.thumbnail(160, 160).to_rgba8();
                        item.preview = Some(([t.width() as usize, t.height() as usize], t.into_raw()));
                    }
                }
            }
        }
        out.push(item);
    }
    Ok(out)
}

/// Lokale Nachricht anzeigen, Medium hochladen, Nachricht bauen und senden.
#[allow(clippy::too_many_arguments)]
async fn upload_and_send(
    client: &Client,
    send: &(impl Fn(UiEvent) + Clone),
    dir: &std::path::Path,
    chat: &Jid,
    data: Vec<u8>,
    mref: MediaRef,
    caption: String,
    mt: whatsapp_rust::wacore::download::MediaType,
    build: impl FnOnce(&whatsapp_rust::upload::UploadResponse) -> wa::Message,
) -> Result<(), String> {
    let temp = format!("local-u-{}", chrono::Utc::now().timestamp_millis());
    let _ = std::fs::create_dir_all(dir);
    let local = dir.join(format!("{}.{}", safe_name(&temp), mref.ext()));
    let _ = std::fs::write(&local, &data);
    send(UiEvent::Message(ChatMessage {
        chat: chat.clone(),
        is_group: chat.is_group(),
        from_me: true,
        sender_name: String::new(),
        sender: String::new(),
        text: caption,
        ts: chrono::Utc::now().timestamp(),
        id: temp.clone(),
        status: 0,
        media: Some(mref.clone()),
        quote: None,
        x: Default::default(),
    }));
    let key = chat.to_string();
    let fail = |e: String| {
        send(UiEvent::SendFailed { chat: key.clone(), temp: temp.clone(), err: e.clone() });
        e
    };
    let up = client.upload(data, mt, Default::default()).await.map_err(|e| fail(e.to_string()))?;
    let msg = build(&up);
    let r = client.send_message(chat, msg).await.map_err(|e| fail(e.to_string()))?;
    let _ = std::fs::rename(&local, dir.join(format!("{}.{}", safe_name(&r.message_id), mref.ext())));
    send(UiEvent::Sent { chat: key, temp, id: r.message_id });
    Ok(())
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
    // Videos und GIFs (als stummes MP4) als Video senden
    let gif = mime == "image/gif";
    if mime == "video/mp4" || gif {
        let mp4_path = if gif {
            let out = dir.join(format!("gif-{}.mp4", chrono::Utc::now().timestamp_millis()));
            let _ = std::fs::create_dir_all(dir);
            if gif_to_mp4(path, &out) { Some(out) } else { None }
        } else {
            Some(path.to_path_buf())
        };
        if let Some(mp4) = mp4_path {
            let data = tokio::fs::read(&mp4).await.map_err(|e| e.to_string())?;
            if gif {
                let _ = std::fs::remove_file(&mp4);
            }
            let (thumb, w, h) = video_thumb(path).unwrap_or_default();
            let mref = MediaRef { kind: if gif { "gif".into() } else { "video".into() }, mime: "video/mp4".into(), name, len: data.len() as u64, w, h, ..Default::default() };
            let cap = if caption.is_empty() { None } else { Some(caption.clone()) };
            return upload_and_send(client, send, dir, chat, data, mref, caption, M::Video, move |up| wa::Message {
                video_message: whatsapp_rust::waproto::buffa::MessageField::some(wa::message::VideoMessage {
                    url: Some(up.url.clone()),
                    direct_path: Some(up.direct_path.clone()),
                    media_key: Some(up.media_key.to_vec()),
                    file_sha256: Some(up.file_sha256.to_vec()),
                    file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                    file_length: Some(up.file_length),
                    media_key_timestamp: Some(up.media_key_timestamp),
                    mimetype: Some("video/mp4".into()),
                    width: (w > 0).then_some(w),
                    height: (h > 0).then_some(h),
                    gif_playback: Some(gif),
                    jpeg_thumbnail: (!thumb.is_empty()).then_some(thumb),
                    caption: cap,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .await;
        }
        // ohne ffmpeg: GIF geht als Dokument
    }
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
        x: Default::default(),
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
                                    for (j, m) in &map {
                                        let members = m
                                            .participants
                                            .iter()
                                            .map(|p| (p.jid.user.to_string(), p.phone_number.as_ref().map(|n| n.user.to_string()).unwrap_or_default()))
                                            .collect();
                                        s(UiEvent::Members { group: j.user.to_string(), members });
                                    }
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
                            let client = ctx.client.clone();
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
                            if let Some(pu) = base.poll_update_message.as_option() {
                                let poll_id = pu.poll_creation_message_key.as_option().and_then(|k| k.id.clone()).unwrap_or_default();
                                let known = POLLS.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&poll_id).cloned()));
                                let (Some((secret, creator, options)), Some(v)) = (known, pu.vote.as_option()) else {
                                    log(format!("Stimme für unbekannte Umfrage {poll_id}"));
                                    return;
                                };
                                let creator_jid = if creator.is_empty() {
                                    client.pn().unwrap_or_default()
                                } else {
                                    creator.parse::<Jid>().unwrap_or_default()
                                };
                                let ct = whatsapp_rust::features::PollVoteCiphertext {
                                    enc_payload: v.enc_payload.as_deref().unwrap_or(&[]),
                                    enc_iv: v.enc_iv.as_deref().unwrap_or(&[]),
                                };
                                match client.polls().decrypt_vote(ct, &secret, &poll_id, &creator_jid, &info.source.sender).await {
                                    Ok(hashes) => {
                                        let chosen: Vec<String> = options
                                            .iter()
                                            .filter(|o| hashes.iter().any(|h| h.as_slice() == whatsapp_rust::wacore::poll::compute_option_hash(o).as_slice()))
                                            .cloned()
                                            .collect();
                                        let voter = if info.source.is_from_me { String::new() } else { info.source.sender.user.to_string() };
                                        s(UiEvent::PollVote { poll: poll_id, voter, options: chosen });
                                    }
                                    Err(e) => log(format!("Stimme entschlüsseln fehlgeschlagen: {e}")),
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
                            let x = extra_of(&ctx.message);
                            if !is_visible(&ctx.message, &info.media_type) && x.kind.is_none() {
                                return;
                            }
                            let media = media_of(&ctx.message);
                            if let Some((_, Some(th), _)) = &media {
                                save_thumb(&info.id, th);
                            }
                            let text = match (&media, ctx.message.text_content()) {
                                (_, Some(t)) => t.to_owned(),
                                (Some((_, _, cap)), None) => cap.clone().unwrap_or_default(),
                                (None, None) if x.kind.is_some() => String::new(),
                                (None, None) => describe(&ctx.message, &info.media_type),
                            };
                            if let Some(Extra::Poll { secret, options, .. }) = &x.kind {
                                remember_poll(&info.id, secret, &info.source.sender.to_string(), options);
                            }
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
                                x,
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
                            EventKind::ChatPresence,
                            EventKind::Presence,
                            EventKind::StarUpdate,
                        ],
                        move |event, client| {
                            let s = s_ev.clone();
                            async move {
                                match &*event {
                                    Event::StarUpdate(st) => {
                                        s(UiEvent::Starred { id: st.message_id.clone(), on: st.action.starred.unwrap_or(false) });
                                    }
                                    Event::ChatPresence(c) => {
                                        use whatsapp_rust::wacore::types::presence::{ChatPresence, ChatPresenceMedia};
                                        s(UiEvent::Typing {
                                            chat: c.source.chat.user.to_string(),
                                            from: c.source.sender.user.to_string(),
                                            on: matches!(c.state, ChatPresence::Composing),
                                            recording: matches!(c.media, ChatPresenceMedia::Audio),
                                        });
                                    }
                                    Event::Presence(p) => {
                                        s(UiEvent::Presence {
                                            user: p.from.user.to_string(),
                                            online: !p.unavailable,
                                            last_seen: p.last_seen.map(|t| t.timestamp()),
                                        });
                                    }
                                    Event::MuteUpdate(m) => {
                                        let end = m.action.mute_end_timestamp.unwrap_or(0);
                                        let now_ms = chrono::Utc::now().timestamp_millis();
                                        let on = m.action.muted.unwrap_or(false)
                                            && (end <= 0 || end > now_ms);
                                        let users = aliases(&client, &m.jid).await;
                                        log(format!("Stumm-Abgleich {} -> {on} {users:?}", m.jid));
                                        s(UiEvent::Muted { users, on, until: if on && end > 0 { Some(end / 1000) } else { None } });
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
                                Cmd::Send { chat, text, temp, quote, mentions, expiration } => {
                                    tokio::spawn(async move {
                                        let key = chat.to_string();
                                        let mut ctx = match &quote {
                                            Some(q) => {
                                                let sender = if q.sender.is_empty() {
                                                    client.pn().unwrap_or_else(|| chat.clone())
                                                } else {
                                                    q.sender.parse::<Jid>().unwrap_or_else(|_| chat.clone())
                                                };
                                                whatsapp_rust::wacore::proto_helpers::build_quote_context_with_info(
                                                    q.id.clone(), &sender, &chat, &chat, &wa::Message::text(q.text.clone()),
                                                )
                                            }
                                            None => wa::ContextInfo::default(),
                                        };
                                        ctx.mentioned_jid = mentions;
                                        if expiration > 0 {
                                            ctx.expiration = Some(expiration);
                                        }
                                        let plain = quote.is_none() && ctx.mentioned_jid.is_empty() && expiration == 0;
                                        let msg = if plain { wa::Message::text(text) } else { wa::Message::text_with_context(text, ctx) };
                                        match client.send_message(&chat, msg).await {
                                            Ok(r) => send(UiEvent::Sent { chat: key, temp, id: r.message_id }),
                                            Err(e) => {
                                                log(format!("Senden fehlgeschlagen: {e}"));
                                                send(UiEvent::SendFailed { chat: key, temp, err: e.to_string() });
                                            }
                                        }
                                    });
                                }
                                Cmd::Edit { chat, id, text } => {
                                    tokio::spawn(async move {
                                        match client.edit_message(chat, id.clone(), wa::Message::text(text.clone())).await {
                                            Ok(_) => send(UiEvent::Edited { id, text }),
                                            Err(e) => send(UiEvent::Info(format!("Bearbeiten fehlgeschlagen: {e}"))),
                                        }
                                    });
                                }
                                Cmd::Star { chat, id, from_me, sender, on } => {
                                    tokio::spawn(async move {
                                        let part = if chat.is_group() && !from_me { sender.parse::<Jid>().ok() } else { None };
                                        let ca = client.chat_actions();
                                        let r = if on {
                                            ca.star_message(&chat, part.as_ref(), &id, from_me).await
                                        } else {
                                            ca.unstar_message(&chat, part.as_ref(), &id, from_me).await
                                        };
                                        report(&send, "Markieren", r.map_err(|e| e.to_string()));
                                    });
                                }
                                Cmd::LoadOlder { chat, id, from_me, ts } => {
                                    tokio::spawn(async move {
                                        match client.fetch_message_history(&chat, &id, from_me, ts * 1000, 50).await {
                                            Ok(_) => send(UiEvent::Info("Ältere Nachrichten werden vom Handy geladen …".into())),
                                            Err(e) => send(UiEvent::Info(format!("Ältere Nachrichten: {e}"))),
                                        }
                                    });
                                }
                                Cmd::Vote { chat, poll, creator, secret, options } => {
                                    tokio::spawn(async move {
                                        let creator_jid = if creator.is_empty() { client.pn().unwrap_or_default() } else { creator.parse::<Jid>().unwrap_or_default() };
                                        if let Err(e) = client.polls().vote(chat, &poll, &creator_jid, &secret, &options).await {
                                            send(UiEvent::Info(format!("Abstimmen fehlgeschlagen: {e}")));
                                        }
                                    });
                                }
                                Cmd::RegisterPolls(list) => {
                                    for (id, secret, creator, options) in list {
                                        remember_poll(&id, &secret, &creator, &options);
                                    }
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
                                Cmd::Typing { chat, on } => {
                                    tokio::spawn(async move {
                                        let cs = client.chatstate();
                                        let _ = if on { cs.send_composing(&chat).await } else { cs.send_paused(&chat).await };
                                    });
                                }
                                Cmd::SetOnline(on) => {
                                    tokio::spawn(async move {
                                        let p = client.presence();
                                        let r = if on { p.set_available().await } else { p.set_unavailable().await };
                                        if let Err(e) = r {
                                            log(format!("Online-Status: {e}"));
                                        }
                                    });
                                }
                                Cmd::Subscribe(jid) => {
                                    tokio::spawn(async move {
                                        let _ = client.presence().subscribe(jid).await;
                                    });
                                }
                                Cmd::Forward { chat, msg } => {
                                    tokio::spawn(async move {
                                        let wm = forward_message(&msg);
                                        let temp = format!("local-fw-{}", chrono::Utc::now().timestamp_millis());
                                        send(UiEvent::Message(ChatMessage {
                                            chat: chat.clone(),
                                            is_group: chat.is_group(),
                                            from_me: true,
                                            sender_name: String::new(),
                                            sender: String::new(),
                                            text: msg.text.clone(),
                                            ts: chrono::Utc::now().timestamp(),
                                            id: temp.clone(),
                                            status: 0,
                                            media: msg.media.clone(),
                                            quote: None,
                                            x: MsgExtra { kind: msg.x.kind.clone(), ..Default::default() },
                                        }));
                                        let key = chat.to_string();
                                        match client.send_message(&chat, wm).await {
                                            Ok(r) => send(UiEvent::Sent { chat: key, temp, id: r.message_id }),
                                            Err(e) => send(UiEvent::SendFailed { chat: key, temp, err: e.to_string() }),
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
                                Cmd::SetMute { jid, on, until_ms } => {
                                    tokio::spawn(async move {
                                        let ca = client.chat_actions();
                                        let r = match (on, until_ms) {
                                            (true, Some(t)) => ca.mute_chat_until(&jid, t).await,
                                            (true, None) => ca.mute_chat(&jid).await,
                                            (false, _) => ca.unmute_chat(&jid).await,
                                        };
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
                                        let m = match show_notification("WhatsSnap", "Test-Benachrichtigung") {
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
                                                let animated = media.kind == "sticker";
                                                let r = tokio::task::spawn_blocking(move || {
                                                    let b = std::fs::read(path).ok()?;
                                                    let anim = if animated { decode_anim(&b) } else { None };
                                                    Some((decode_rgba(&b, 640), anim))
                                                })
                                                .await;
                                                if let Ok(Some((still, anim))) = r {
                                                    if let Some((size, rgba)) = still {
                                                        send(UiEvent::Image { id: id.clone(), size, rgba, full: true });
                                                    }
                                                    if let Some(frames) = anim {
                                                        send(UiEvent::Anim { id, frames });
                                                    }
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
                                Cmd::GroupInfo(jid) => {
                                    tokio::spawn(async move {
                                        match client.groups().get_metadata(&jid).await {
                                            Ok(m) => {
                                                let me: Vec<String> = [client.pn(), client.lid()].into_iter().flatten().map(|j| j.user.to_string()).collect();
                                                let members: Vec<(String, String, bool, bool)> = m
                                                    .participants
                                                    .iter()
                                                    .map(|p| {
                                                        (
                                                            p.jid.to_string(),
                                                            p.phone_number.as_ref().map(|n| n.user.to_string()).unwrap_or_default(),
                                                            p.is_admin(),
                                                            p.participant_type == whatsapp_rust::features::ParticipantType::SuperAdmin,
                                                        )
                                                    })
                                                    .collect();
                                                let i_am_admin = m.participants.iter().any(|p| p.is_admin() && (me.contains(&p.jid.user.to_string()) || p.phone_number.as_ref().map(|n| me.contains(&n.user.to_string())).unwrap_or(false)));
                                                send(UiEvent::GroupInfo(GroupDetails {
                                                    group: jid.user.to_string(),
                                                    subject: m.subject.clone(),
                                                    description: m.description.clone().unwrap_or_default(),
                                                    members,
                                                    created: m.creation_time.unwrap_or(0) as i64,
                                                    i_am_admin,
                                                }));
                                            }
                                            Err(e) => send(UiEvent::Info(format!("Gruppeninfo: {e}"))),
                                        }
                                    });
                                }
                                Cmd::CreateGroup { name, numbers } => {
                                    tokio::spawn(async move {
                                        use whatsapp_rust::features::{GroupCreateOptions, GroupParticipantOptions};
                                        let parts = numbers.iter().map(|n| GroupParticipantOptions::new(Jid::pn(n.clone()))).collect();
                                        let opts = GroupCreateOptions::builder().subject(name.clone()).participants(parts).build();
                                        match client.groups().create_group(opts).await {
                                            Ok(r) => {
                                                send(UiEvent::Groups(vec![(r.metadata.id.clone(), r.metadata.subject.clone())]));
                                                send(UiEvent::Info(format!("Gruppe „{name}“ erstellt")));
                                            }
                                            Err(e) => send(UiEvent::Info(format!("Gruppe erstellen: {e}"))),
                                        }
                                    });
                                }
                                Cmd::GroupRename { jid, name } => {
                                    tokio::spawn(async move {
                                        let r = match whatsapp_rust::features::GroupSubject::new(name.clone()) {
                                            Ok(subj) => client.groups().set_subject(jid.clone(), subj).await.map_err(|e| e.to_string()),
                                            Err(e) => Err(e.to_string()),
                                        };
                                        match r {
                                            Ok(()) => send(UiEvent::Groups(vec![(jid, name)])),
                                            Err(e) => send(UiEvent::Info(format!("Umbenennen: {e}"))),
                                        }
                                    });
                                }
                                Cmd::GroupAdd { jid, numbers } => {
                                    tokio::spawn(async move {
                                        let js: Vec<Jid> = numbers.iter().map(|n| Jid::pn(n.clone())).collect();
                                        match client.groups().add_participants(jid.clone(), &js).await {
                                            Ok(_) => send(UiEvent::Info("Mitglieder hinzugefügt".into())),
                                            Err(e) => send(UiEvent::Info(format!("Hinzufügen: {e}"))),
                                        }
                                        let _ = jid;
                                    });
                                }
                                Cmd::GroupRemove { jid, member } => {
                                    tokio::spawn(async move {
                                        let Ok(m) = member.parse::<Jid>() else { return };
                                        match client.groups().remove_participants(jid, &[m]).await {
                                            Ok(_) => send(UiEvent::Info("Mitglied entfernt".into())),
                                            Err(e) => send(UiEvent::Info(format!("Entfernen: {e}"))),
                                        }
                                    });
                                }
                                Cmd::GroupAdmin { jid, member, on } => {
                                    tokio::spawn(async move {
                                        let Ok(m) = member.parse::<Jid>() else { return };
                                        let g = client.groups();
                                        let r = if on { g.promote_participants(jid, &[m]).await } else { g.demote_participants(jid, &[m]).await };
                                        if let Err(e) = r {
                                            send(UiEvent::Info(format!("Admin-Rechte: {e}")));
                                        }
                                    });
                                }
                                Cmd::GroupLeave(jid) => {
                                    tokio::spawn(async move {
                                        match client.groups().leave(jid).await {
                                            Ok(()) => send(UiEvent::Info("Gruppe verlassen".into())),
                                            Err(e) => send(UiEvent::Info(format!("Verlassen: {e}"))),
                                        }
                                    });
                                }
                                Cmd::GroupInvite(jid) => {
                                    tokio::spawn(async move {
                                        match client.groups().get_invite_link(jid, false).await {
                                            Ok(link) => send(UiEvent::Info(format!("Einladungslink (in die Zwischenablage kopiert): {link}"))),
                                            Err(e) => send(UiEvent::Info(format!("Einladungslink: {e}"))),
                                        }
                                    });
                                }
                                Cmd::FetchPrivacy => {
                                    tokio::spawn(async move {
                                        match client.fetch_privacy_settings().await {
                                            Ok(r) => {
                                                let list = r
                                                    .settings
                                                    .iter()
                                                    .map(|s| (format!("{:?}", s.category).to_lowercase(), format!("{:?}", s.value).to_lowercase()))
                                                    .collect();
                                                send(UiEvent::Privacy(list));
                                            }
                                            Err(e) => send(UiEvent::Info(format!("Datenschutz laden: {e}"))),
                                        }
                                    });
                                }
                                Cmd::SetPrivacy { cat, val } => {
                                    tokio::spawn(async move {
                                        use whatsapp_rust::wacore::iq::privacy::{PrivacyCategory as C, PrivacyValue as V};
                                        let c = match cat.as_str() {
                                            "last" => C::Last,
                                            "online" => C::Online,
                                            "profile" => C::Profile,
                                            "status" => C::Status,
                                            "groupadd" => C::GroupAdd,
                                            "readreceipts" => C::ReadReceipts,
                                            "calladd" => C::CallAdd,
                                            _ => return,
                                        };
                                        let v = match val.as_str() {
                                            "all" => V::All,
                                            "contacts" => V::Contacts,
                                            "none" => V::None,
                                            "matchlastseen" => V::MatchLastSeen,
                                            "known" => V::Known,
                                            _ => return,
                                        };
                                        match client.set_privacy_setting(c, v).await {
                                            Ok(_) => log(format!("Datenschutz {cat} = {val}")),
                                            Err(e) => send(UiEvent::Info(format!("Datenschutz ändern fehlgeschlagen: {e}"))),
                                        }
                                    });
                                }
                                Cmd::SetDefaultDisappearing(secs) => {
                                    tokio::spawn(async move {
                                        if let Err(e) = client.set_default_disappearing_mode(secs).await {
                                            send(UiEvent::Info(format!("Selbstlöschen: {e}")));
                                        }
                                    });
                                }
                                Cmd::SetChatDisappearing { chat, secs } => {
                                    tokio::spawn(async move {
                                        let r = if chat.is_group() {
                                            client.groups().set_ephemeral(&chat, secs).await.map_err(|e| e.to_string())
                                        } else {
                                            client.set_chat_disappearing_timer(chat, secs).await.map(|_| ()).map_err(|e| e.to_string())
                                        };
                                        match r {
                                            Ok(()) => send(UiEvent::Info(if secs == 0 { "Selbstlöschende Nachrichten aus".into() } else { "Selbstlöschende Nachrichten an".into() })),
                                            Err(e) => send(UiEvent::Info(format!("Selbstlöschen: {e}"))),
                                        }
                                    });
                                }
                                Cmd::SetProfilePicture(path) => {
                                    tokio::spawn(async move {
                                        let path = match path {
                                            Some(p) => p,
                                            None => match rfd::AsyncFileDialog::new()
                                                .set_title("Profilbild wählen")
                                                .add_filter("Bilder", &["png", "jpg", "jpeg", "webp"])
                                                .pick_file()
                                                .await
                                            {
                                                Some(f) => f.path().to_path_buf(),
                                                None => return,
                                            },
                                        };
                                        let jpeg = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
                                            let img = image::open(&path).map_err(|e| e.to_string())?;
                                            let side = img.width().min(img.height());
                                            let sq = img.crop_imm((img.width() - side) / 2, (img.height() - side) / 2, side, side).resize_exact(640, 640, image::imageops::FilterType::Lanczos3).to_rgb8();
                                            let mut out = Vec::new();
                                            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&sq).map_err(|e| e.to_string())?;
                                            Ok(out)
                                        })
                                        .await
                                        .map_err(|e| e.to_string())
                                        .and_then(|r| r);
                                        match jpeg {
                                            Ok(j) => match client.profile().set_profile_picture(j).await {
                                                Ok(_) => send(UiEvent::Info("Profilbild bei WhatsApp geändert".into())),
                                                Err(e) => send(UiEvent::Info(format!("Profilbild: {e}"))),
                                            },
                                            Err(e) => send(UiEvent::Info(format!("Profilbild: {e}"))),
                                        }
                                    });
                                }
                                Cmd::FetchBlocklist => {
                                    tokio::spawn(async move {
                                        match client.blocking().get_blocklist().await {
                                            Ok(list) => send(UiEvent::Blocklist(list.into_iter().map(|b| b.jid.to_string()).collect())),
                                            Err(e) => send(UiEvent::Info(format!("Blockierte laden: {e}"))),
                                        }
                                    });
                                }
                                Cmd::Block { jid, on } => {
                                    tokio::spawn(async move {
                                        let b = client.blocking();
                                        let r = if on { b.block(&jid).await } else { b.unblock(&jid).await };
                                        match r {
                                            Ok(()) => {
                                                if let Ok(list) = b.get_blocklist().await {
                                                    send(UiEvent::Blocklist(list.into_iter().map(|e| e.jid.to_string()).collect()));
                                                }
                                                send(UiEvent::Info(if on { "Kontakt blockiert".into() } else { "Blockierung aufgehoben".into() }));
                                            }
                                            Err(e) => send(UiEvent::Info(format!("Blockieren: {e}"))),
                                        }
                                    });
                                }
                                Cmd::Logout => {
                                    tokio::spawn(async move {
                                        client.logout().await;
                                        log("Abgemeldet (auf Wunsch)");
                                        send(UiEvent::LoggedOut);
                                    });
                                }
                                Cmd::FetchFile { id, media } => {
                                    let dir = media_dir.clone();
                                    tokio::spawn(async move {
                                        match ensure_file(&client, &dir, &id, &media).await {
                                            Ok(path) => send(UiEvent::FileReady { id, path }),
                                            Err(e) => send(UiEvent::Info(format!("Herunterladen fehlgeschlagen: {e}"))),
                                        }
                                    });
                                }
                                Cmd::SendVoice { chat, ogg, seconds, waveform } => {
                                    let dir = media_dir.clone();
                                    tokio::spawn(async move {
                                        use whatsapp_rust::wacore::download::MediaType as M;
                                        let mime = "audio/ogg; codecs=opus";
                                        let mref = MediaRef { kind: "ptt".into(), mime: mime.into(), len: ogg.len() as u64, seconds, ..Default::default() };
                                        let wf = waveform.clone();
                                        let r = upload_and_send(&client, &send, &dir, &chat, ogg, mref, String::new(), M::Audio, move |up| wa::Message {
                                            audio_message: whatsapp_rust::waproto::buffa::MessageField::some(wa::message::AudioMessage {
                                                url: Some(up.url.clone()),
                                                direct_path: Some(up.direct_path.clone()),
                                                media_key: Some(up.media_key.to_vec()),
                                                file_sha256: Some(up.file_sha256.to_vec()),
                                                file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                                                file_length: Some(up.file_length),
                                                media_key_timestamp: Some(up.media_key_timestamp),
                                                mimetype: Some(mime.into()),
                                                seconds: Some(seconds),
                                                ptt: Some(true),
                                                waveform: Some(wf),
                                                ..Default::default()
                                            }),
                                            ..Default::default()
                                        })
                                        .await;
                                        if let Err(e) = r {
                                            log(format!("Sprachnachricht fehlgeschlagen: {e}"));
                                        }
                                    });
                                }
                                Cmd::MakeSticker { chat, path } => {
                                    let dir = media_dir.clone();
                                    tokio::spawn(async move {
                                        use whatsapp_rust::wacore::download::MediaType as M;
                                        let path = match path {
                                            Some(p) => p,
                                            None => match rfd::AsyncFileDialog::new()
                                                .set_title("Bild für Sticker wählen")
                                                .add_filter("Bilder", &["png", "jpg", "jpeg", "webp", "gif"])
                                                .pick_file()
                                                .await
                                            {
                                                Some(f) => f.path().to_path_buf(),
                                                None => return,
                                            },
                                        };
                                        let made = tokio::task::spawn_blocking(move || {
                                            let img = image::open(&path).map_err(|e| e.to_string())?;
                                            to_sticker_webp(&img)
                                        })
                                        .await
                                        .map_err(|e| e.to_string())
                                        .and_then(|r| r);
                                        let data = match made {
                                            Ok(d) => d,
                                            Err(e) => {
                                                send(UiEvent::Info(format!("Sticker: {e}")));
                                                return;
                                            }
                                        };
                                        let mref = MediaRef { kind: "sticker".into(), mime: "image/webp".into(), len: data.len() as u64, w: 512, h: 512, ..Default::default() };
                                        let r = upload_and_send(&client, &send, &dir, &chat, data, mref, String::new(), M::Sticker, |up| wa::Message {
                                            sticker_message: whatsapp_rust::waproto::buffa::MessageField::some(wa::message::StickerMessage {
                                                url: Some(up.url.clone()),
                                                direct_path: Some(up.direct_path.clone()),
                                                media_key: Some(up.media_key.to_vec()),
                                                file_sha256: Some(up.file_sha256.to_vec()),
                                                file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                                                file_length: Some(up.file_length),
                                                media_key_timestamp: Some(up.media_key_timestamp),
                                                mimetype: Some("image/webp".into()),
                                                width: Some(512),
                                                height: Some(512),
                                                is_animated: Some(false),
                                                ..Default::default()
                                            }),
                                            ..Default::default()
                                        })
                                        .await;
                                        if let Err(e) = r {
                                            log(format!("Sticker senden fehlgeschlagen: {e}"));
                                        }
                                    });
                                }
                                Cmd::GifSearch { query, key } => {
                                    tokio::spawn(async move {
                                        match tokio::task::spawn_blocking(move || giphy_search(&query, &key)).await {
                                            Ok(Ok(list)) => send(UiEvent::GifResults(list)),
                                            Ok(Err(e)) => send(UiEvent::Info(format!("GIF-Suche: {e}"))),
                                            Err(e) => send(UiEvent::Info(format!("GIF-Suche: {e}"))),
                                        }
                                    });
                                }
                                Cmd::SendGif { chat, gif } => {
                                    let dir = media_dir.clone();
                                    tokio::spawn(async move {
                                        use whatsapp_rust::wacore::download::MediaType as M;
                                        let url = gif.mp4.clone();
                                        let data = match tokio::task::spawn_blocking(move || {
                                            ureq::get(&url).call().map_err(|e| e.to_string())?.body_mut().with_config().limit(20 * 1024 * 1024).read_to_vec().map_err(|e| e.to_string())
                                        })
                                        .await
                                        {
                                            Ok(Ok(d)) => d,
                                            Ok(Err(e)) => {
                                                send(UiEvent::Info(format!("GIF laden: {e}")));
                                                return;
                                            }
                                            Err(e) => {
                                                send(UiEvent::Info(format!("GIF laden: {e}")));
                                                return;
                                            }
                                        };
                                        let (w, h, thumb) = (gif.w, gif.h, gif.thumb.clone());
                                        let mref = MediaRef { kind: "gif".into(), mime: "video/mp4".into(), len: data.len() as u64, w, h, ..Default::default() };
                                        let r = upload_and_send(&client, &send, &dir, &chat, data, mref, String::new(), M::Video, move |up| wa::Message {
                                            video_message: whatsapp_rust::waproto::buffa::MessageField::some(wa::message::VideoMessage {
                                                url: Some(up.url.clone()),
                                                direct_path: Some(up.direct_path.clone()),
                                                media_key: Some(up.media_key.to_vec()),
                                                file_sha256: Some(up.file_sha256.to_vec()),
                                                file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                                                file_length: Some(up.file_length),
                                                media_key_timestamp: Some(up.media_key_timestamp),
                                                mimetype: Some("video/mp4".into()),
                                                width: Some(w),
                                                height: Some(h),
                                                gif_playback: Some(true),
                                                jpeg_thumbnail: (!thumb.is_empty()).then_some(thumb),
                                                ..Default::default()
                                            }),
                                            ..Default::default()
                                        })
                                        .await;
                                        if let Err(e) = r {
                                            log(format!("GIF senden fehlgeschlagen: {e}"));
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
                                            x: Default::default(),
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
