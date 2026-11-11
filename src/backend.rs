//! Hintergrund-Thread: hält die WhatsApp-Verbindung (tokio) und spricht
//! per Channels mit der Oberfläche. Die UI blockiert nie auf Netzwerk.

use std::sync::mpsc::Sender;
use std::sync::Arc;

use whatsapp_rust::prelude::*;
use whatsapp_rust::wacore::proto_helpers::{MessageBuilderExt, MessageExt};
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
    SendFailed(String),
    /// Stumm/Angeheftet/Archiviert wie bei WhatsApp (vom Handy oder von hier).
    Muted { jid: String, on: bool },
    Pinned { jid: String, on: bool },
    Archived { jid: String, on: bool },
    /// Rückmeldung zu einer Einstellungsänderung (Text für die Statuszeile).
    Info(String),
    /// Alle Gruppen mit Namen (direkt vom Server abgefragt).
    Groups(Vec<(Jid, String)>),
}

static LOG_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Diagnose-Log (`wa-snap.log` im Datenordner), damit sich Sync-Probleme nachvollziehen lassen.
pub fn log(msg: impl AsRef<str>) {
    use std::io::Write;
    let Some(p) = LOG_PATH.get() else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{} {}", chrono::Local::now().format("%F %T"), msg.as_ref());
    }
}

pub struct HistChat {
    pub jid: Jid,
    pub name: String,
    pub is_group: bool,
    pub last_ts: i64,
    pub unread: u32,
    pub msgs: Vec<ChatMessage>,
}

/// Was die Oberfläche dem Backend aufträgt.
pub enum Cmd {
    Send { chat: Jid, text: String },
    Avatar { jid: Jid },
    SetMute { jid: Jid, on: bool },
    SetPin { jid: Jid, on: bool },
    SetArchive { jid: Jid, on: bool },
    SetPushName(String),
    SetAbout(String),
    Quit,
}

#[derive(Clone)]
pub struct ChatMessage {
    pub chat: Jid,
    pub is_group: bool,
    pub from_me: bool,
    pub sender_name: String,
    pub text: String,
    pub ts: i64,
    pub id: String,
}

/// Klartext einer Nachricht; bei Medien ein kurzer Platzhalter.
fn describe(message: &wa::Message, media_type: &str) -> String {
    if let Some(t) = message.text_content() {
        return t.to_owned();
    }
    if let Some(c) = message.get_caption() {
        return format!("[{media_type}] {c}");
    }
    format!("[{media_type}]")
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
        if jid.server == whatsapp_rust::Server::Broadcast || jid.server == whatsapp_rust::Server::Newsletter {
            continue;
        }
        let is_group = jid.server == whatsapp_rust::Server::Group;
        let mut msgs = Vec::new();
        // Nur die letzten Nachrichten übernehmen, das hält den Start schnell.
        let skip = c.messages.len().saturating_sub(60);
        for hm in &c.messages[skip..] {
            let wm = &hm.message;
            let Some(inner) = wm.message.as_option() else { continue };
            let text = match inner.text_content().or_else(|| inner.get_caption()) {
                Some(t) => t.to_owned(),
                None => continue,
            };
            let key = &wm.key;
            msgs.push(ChatMessage {
                chat: jid.clone(),
                is_group,
                from_me: key.from_me.unwrap_or(false),
                sender_name: wm
                    .push_name
                    .clone()
                    .or_else(|| wm.participant.as_deref().map(user_of))
                    .unwrap_or_default(),
                text,
                ts: wm.message_timestamp.unwrap_or(0) as i64,
                id: key.id.clone().unwrap_or_default(),
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

pub fn spawn(
    dir: std::path::PathBuf,
    ui_tx: Sender<UiEvent>,
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
                log("--- Start ---");
                let avatar_dir = dir.join("avatars");
                let store = match SqliteStore::new(&dir.join("whatsapp.db").to_string_lossy()).await {
                    Ok(s) => s,
                    Err(e) => {
                        send(UiEvent::Status(format!("Datenbank-Fehler: {e}")));
                        return;
                    }
                };

                let (s_qr, s_conn, s_out, s_msg, s_ev) = (
                    send.clone(),
                    send.clone(),
                    send.clone(),
                    send.clone(),
                    send.clone(),
                );

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
                        async move {
                            let info = &ctx.info;
                            log(format!(
                                "Nachricht {} von_mir={} typ='{}'",
                                info.source.chat, info.source.is_from_me, info.media_type
                            ));
                            if !is_visible(&ctx.message, &info.media_type) {
                                return;
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
                                text: describe(&ctx.message, &info.media_type),
                                ts: info.timestamp.timestamp(),
                                id: info.id.to_string(),
                            };
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
                        ],
                        move |event, _client| {
                            let s = s_ev.clone();
                            async move {
                                match &*event {
                                    Event::MuteUpdate(m) => {
                                        let end = m.action.mute_end_timestamp.unwrap_or(0);
                                        let now_ms = chrono::Utc::now().timestamp_millis();
                                        let on = m.action.muted.unwrap_or(false)
                                            && (end <= 0 || end > now_ms);
                                        s(UiEvent::Muted { jid: m.jid.to_string(), on });
                                    }
                                    Event::PinUpdate(p) => {
                                        s(UiEvent::Pinned {
                                            jid: p.jid.to_string(),
                                            on: p.action.pinned.unwrap_or(false),
                                        });
                                    }
                                    Event::ArchiveUpdate(a) => {
                                        s(UiEvent::Archived {
                                            jid: a.jid.to_string(),
                                            on: a.action.archived.unwrap_or(false),
                                        });
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

                loop {
                    tokio::select! {
                        _ = &mut handle => break,
                        cmd = cmd_rx.recv() => match cmd {
                            Some(Cmd::Send { chat, text }) => {
                                let client = client.clone();
                                let send = send.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = client
                                        .send_message(&chat, wa::Message::text(text))
                                        .await
                                    {
                                        send(UiEvent::SendFailed(e.to_string()));
                                    }
                                });
                            }
                            Some(Cmd::Avatar { jid }) => {
                                let client = client.clone();
                                let send = send.clone();
                                let dir = avatar_dir.clone();
                                tokio::spawn(async move {
                                    if let Some(ev) = load_avatar(client, dir, jid).await {
                                        send(ev);
                                    }
                                });
                            }
                            Some(Cmd::SetMute { jid, on }) => {
                                let (client, send) = (client.clone(), send.clone());
                                tokio::spawn(async move {
                                    let r = if on {
                                        client.chat_actions().mute_chat(&jid).await
                                    } else {
                                        client.chat_actions().unmute_chat(&jid).await
                                    };
                                    if let Err(e) = r {
                                        log(format!("Stumm fehlgeschlagen: {e}"));
                                        send(UiEvent::Info(format!("Stummschalten fehlgeschlagen: {e}")));
                                    }
                                });
                            }
                            Some(Cmd::SetPin { jid, on }) => {
                                let (client, send) = (client.clone(), send.clone());
                                tokio::spawn(async move {
                                    let r = if on {
                                        client.chat_actions().pin_chat(&jid).await
                                    } else {
                                        client.chat_actions().unpin_chat(&jid).await
                                    };
                                    if let Err(e) = r {
                                        log(format!("Anheften fehlgeschlagen: {e}"));
                                        send(UiEvent::Info(format!("Anheften fehlgeschlagen: {e}")));
                                    }
                                });
                            }
                            Some(Cmd::SetArchive { jid, on }) => {
                                let (client, send) = (client.clone(), send.clone());
                                tokio::spawn(async move {
                                    let r = if on {
                                        client.chat_actions().archive_chat(&jid, None).await
                                    } else {
                                        client.chat_actions().unarchive_chat(&jid, None).await
                                    };
                                    if let Err(e) = r {
                                        log(format!("Archivieren fehlgeschlagen: {e}"));
                                        send(UiEvent::Info(format!("Archivieren fehlgeschlagen: {e}")));
                                    }
                                });
                            }
                            Some(Cmd::SetPushName(n)) => {
                                let (client, send) = (client.clone(), send.clone());
                                tokio::spawn(async move {
                                    let m = match client.profile().set_push_name(&n).await {
                                        Ok(_) => "Profilname gespeichert".to_owned(),
                                        Err(e) => format!("Profilname: {e}"),
                                    };
                                    log(&m);
                                    send(UiEvent::Info(m));
                                });
                            }
                            Some(Cmd::SetAbout(t)) => {
                                let (client, send) = (client.clone(), send.clone());
                                tokio::spawn(async move {
                                    let m = match client.profile().set_status_text(&t).await {
                                        Ok(_) => "Info-Text gespeichert".to_owned(),
                                        Err(e) => format!("Info-Text: {e}"),
                                    };
                                    log(&m);
                                    send(UiEvent::Info(m));
                                });
                            }
                            Some(Cmd::Quit) | None => {
                                handle.shutdown().await;
                                break;
                            }
                        }
                    }
                }
            });
        })
        .expect("backend thread");

    cmd_tx
}
