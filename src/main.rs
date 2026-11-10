mod backend;

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};

use backend::{ChatMessage, Cmd, UiEvent};
use eframe::egui::{self, Color32, RichText};
use whatsapp_rust::Jid;

const GREEN: Color32 = Color32::from_rgb(0x1f, 0xa8, 0x55);
const BUBBLE_ME: Color32 = Color32::from_rgb(0x0b, 0x5a, 0x3a);
const BUBBLE_THEM: Color32 = Color32::from_rgb(0x2a, 0x33, 0x3a);

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

struct App {
    rx: Receiver<UiEvent>,
    cmd: tokio::sync::mpsc::UnboundedSender<Cmd>,
    conn: Conn,
    status: String,
    qr: Option<String>,
    chats: HashMap<String, Chat>,
    selected: Option<String>,
    search: String,
    input: String,
    error: Option<String>,
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

fn fmt_time(ts: i64) -> String {
    let secs = ts.rem_euclid(86_400);
    // UTC-Anzeige; lokale Zeitzone bräuchte eine zusätzliche Abhängigkeit.
    format!("{:02}:{:02}", secs / 3600, (secs % 3600) / 60)
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let (tx, rx) = channel();
        let ctx = cc.egui_ctx.clone();
        let db = data_dir().join("whatsapp.db");
        let cmd = backend::spawn(db.to_string_lossy().into_owned(), tx, move || {
            ctx.request_repaint()
        });
        Self {
            rx,
            cmd,
            conn: Conn::Starting,
            status: "Starte …".into(),
            qr: None,
            chats: HashMap::new(),
            selected: None,
            search: String::new(),
            input: String::new(),
            error: None,
        }
    }

    fn ingest(&mut self, m: ChatMessage) {
        let key = m.chat.to_string();
        let selected = self.selected.as_deref() == Some(key.as_str());
        let chat = self.chats.entry(key).or_insert_with(|| Chat {
            jid: m.chat.clone(),
            name: m.chat.user.to_string(),
            is_group: m.is_group,
            messages: Vec::new(),
            unread: 0,
            last_ts: 0,
        });
        // Gesprächsname: bei Einzelchats der Name des Gegenübers.
        if !m.is_group && !m.from_me && !m.sender_name.is_empty() {
            chat.name = m.sender_name.clone();
        }
        if chat.messages.iter().any(|x| !m.id.is_empty() && x.id == m.id) {
            return; // Duplikat (z. B. erneute Zustellung)
        }
        if !m.from_me && !selected {
            chat.unread += 1;
        }
        chat.last_ts = chat.last_ts.max(m.ts);
        chat.messages.push(m);
        chat.messages.sort_by_key(|x| x.ts);
    }

    fn pump(&mut self) {
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
                UiEvent::Message(m) => self.ingest(m),
                UiEvent::SendFailed(e) => self.error = Some(format!("Senden fehlgeschlagen: {e}")),
            }
        }
    }

    fn send_current(&mut self) {
        let text = self.input.trim().to_owned();
        let Some(key) = self.selected.clone() else { return };
        if text.is_empty() {
            return;
        }
        if let Some(chat) = self.chats.get_mut(&key) {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            chat.messages.push(ChatMessage {
                chat: chat.jid.clone(),
                is_group: chat.is_group,
                from_me: true,
                sender_name: String::new(),
                text: text.clone(),
                ts,
                id: String::new(),
            });
            chat.last_ts = ts;
            let _ = self.cmd.send(Cmd::Send { chat: chat.jid.clone(), text });
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
        p.rect_filled(rect, 4.0, Color32::WHITE);
        let colors = code.to_colors();
        for y in 0..n {
            for x in 0..n {
                if colors[y * n + x] == qrcode::Color::Dark {
                    let min = rect.min + egui::vec2((x as f32 + quiet) * cell, (y as f32 + quiet) * cell);
                    p.rect_filled(
                        egui::Rect::from_min_size(min, egui::vec2(cell, cell)),
                        0.0,
                        Color32::BLACK,
                    );
                }
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump();

        if self.conn != Conn::Online && self.chats.is_empty() {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    match self.conn {
                        Conn::NeedsQr => {
                            ui.heading("Mit WhatsApp verknüpfen");
                            ui.label("Handy: WhatsApp → Verknüpfte Geräte → Gerät hinzufügen");
                            ui.add_space(12.0);
                            if let Some(q) = self.qr.clone() {
                                self.qr_view(ui, &q);
                            }
                        }
                        Conn::LoggedOut => {
                            ui.heading("Abgemeldet");
                            ui.label("Die Verknüpfung wurde auf dem Handy entfernt. Bitte neu starten und erneut scannen.");
                        }
                        _ => {
                            ui.heading("wa-snap");
                            ui.label(&self.status);
                            ui.spinner();
                        }
                    }
                });
            });
            return;
        }

        egui::SidePanel::left("chats")
            .resizable(true)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Chats");
                    let (txt, col) = match self.conn {
                        Conn::Online => ("● online", GREEN),
                        Conn::LoggedOut => ("● abgemeldet", Color32::LIGHT_RED),
                        _ => ("● verbinde", Color32::YELLOW),
                    };
                    ui.label(RichText::new(txt).color(col).small());
                });
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Suchen …"));
                ui.separator();
                let needle = self.search.to_lowercase();
                let mut keys: Vec<&String> = self
                    .chats
                    .iter()
                    .filter(|(_, c)| needle.is_empty() || c.name.to_lowercase().contains(&needle))
                    .map(|(k, _)| k)
                    .collect();
                keys.sort_by_key(|k| std::cmp::Reverse(self.chats[*k].last_ts));
                let keys: Vec<String> = keys.into_iter().cloned().collect();
                let mut clicked = None;
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show_rows(
                    ui,
                    44.0,
                    keys.len(),
                    |ui, range| {
                        for key in &keys[range] {
                            let c = &self.chats[key];
                            let last = c.messages.last().map(|m| m.text.as_str()).unwrap_or("");
                            let sel = self.selected.as_deref() == Some(key.as_str());
                            let title = if c.unread > 0 {
                                format!("{}  ({})", c.name, c.unread)
                            } else {
                                c.name.clone()
                            };
                            let preview: String = last.chars().take(34).collect();
                            let resp = ui.add_sized(
                                [ui.available_width(), 40.0],
                                egui::SelectableLabel::new(
                                    sel,
                                    RichText::new(format!("{title}\n{preview}")),
                                ),
                            );
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
                }
            });

        egui::TopBottomPanel::bottom("input").show(ctx, |ui| {
            if let Some(e) = self.error.clone() {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            ui.horizontal(|ui| {
                let enabled = self.selected.is_some() && self.conn == Conn::Online;
                let resp = ui.add_enabled(
                    enabled,
                    egui::TextEdit::singleline(&mut self.input)
                        .hint_text("Nachricht schreiben …")
                        .desired_width(ui.available_width() - 80.0),
                );
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let btn = ui.add_enabled(enabled, egui::Button::new("Senden"));
                if enter || btn.clicked() {
                    self.send_current();
                    resp.request_focus();
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(key) = self.selected.clone() else {
                ui.centered_and_justified(|ui| ui.label("Wähle links einen Chat."));
                return;
            };
            let Some(chat) = self.chats.get(&key) else { return };
            ui.heading(&chat.name);
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show_rows(ui, 38.0, chat.messages.len(), |ui, range| {
                    for m in &chat.messages[range] {
                        let (bg, align) = if m.from_me {
                            (BUBBLE_ME, egui::Align::Max)
                        } else {
                            (BUBBLE_THEM, egui::Align::Min)
                        };
                        ui.with_layout(egui::Layout::top_down(align), |ui| {
                            egui::Frame::new()
                                .fill(bg)
                                .corner_radius(8.0)
                                .inner_margin(6.0)
                                .show(ui, |ui| {
                                    ui.set_max_width(ui.available_width() * 0.75);
                                    if chat.is_group && !m.from_me {
                                        ui.label(RichText::new(&m.sender_name).small().color(GREEN));
                                    }
                                    ui.label(&m.text);
                                    ui.label(RichText::new(fmt_time(m.ts)).small().weak());
                                });
                        });
                    }
                });
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.cmd.send(Cmd::Quit);
    }
}

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 680.0])
            .with_title("wa-snap"),
        ..Default::default()
    };
    eframe::run_native("wa-snap", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
