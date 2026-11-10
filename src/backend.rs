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
    SendFailed(String),
}

/// Was die Oberfläche dem Backend aufträgt.
pub enum Cmd {
    Send { chat: Jid, text: String },
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
    if media_type.is_empty() {
        "[nicht unterstützter Nachrichtentyp]".to_owned()
    } else {
        format!("[{media_type}]")
    }
}

pub fn spawn(
    db_path: String,
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

                let store = match SqliteStore::new(&db_path).await {
                    Ok(s) => s,
                    Err(e) => {
                        send(UiEvent::Status(format!("Datenbank-Fehler: {e}")));
                        return;
                    }
                };

                let (s_qr, s_conn, s_out, s_msg) =
                    (send.clone(), send.clone(), send.clone(), send.clone());

                let bot = Bot::builder()
                    .with_backend(store)
                    .on_qr_code(move |code, _timeout| {
                        let s = s_qr.clone();
                        async move { s(UiEvent::Qr(code)) }
                    })
                    .on_connected(move |_client| {
                        let s = s_conn.clone();
                        async move { s(UiEvent::Connected) }
                    })
                    .on_logged_out(move |_info| {
                        let s = s_out.clone();
                        async move { s(UiEvent::LoggedOut) }
                    })
                    .on_message(move |ctx| {
                        let s = s_msg.clone();
                        async move {
                            let info = &ctx.info;
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
