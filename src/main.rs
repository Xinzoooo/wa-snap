mod backend;

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};

use backend::{ChatMessage, Cmd, UiEvent};
use eframe::egui::{self, Color32, RichText};
use whatsapp_rust::Jid;

const GREEN: Color32 = Color32::from_rgb(0x00, 0xa8, 0x84);
const BUBBLE_ME: Color32 = Color32::from_rgb(0x00, 0x5c, 0x4b);
const BUBBLE_THEM: Color32 = Color32::from_rgb(0x20, 0x2c, 0x33);
const BG_LIST: Color32 = Color32::from_rgb(0x11, 0x1b, 0x21);
const BG_CHAT: Color32 = Color32::from_rgb(0x0b, 0x14, 0x1a);
const BG_BAR: Color32 = Color32::from_rgb(0x20, 0x2c, 0x33);
const TEXT_DIM: Color32 = Color32::from_rgb(0x86, 0x96, 0xa0);

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
    names: HashMap<String, (String, bool)>,
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
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_default()
}

fn avatar(ui: &mut egui::Ui, name: &str, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let initial = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into());
    let hue = name.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) % 6;
    let colors = [
        Color32::from_rgb(0x00, 0x80, 0x69),
        Color32::from_rgb(0x53, 0x6e, 0xb5),
        Color32::from_rgb(0x9b, 0x59, 0xb6),
        Color32::from_rgb(0xc0, 0x6c, 0x2e),
        Color32::from_rgb(0x2e, 0x86, 0x9e),
        Color32::from_rgb(0xb0, 0x4a, 0x5a),
    ];
    ui.painter().circle_filled(rect.center(), size / 2.0, colors[hue as usize]);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        egui::FontId::proportional(size * 0.45),
        Color32::WHITE,
    );
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
            names: HashMap::new(),
            selected: None,
            search: String::new(),
            input: String::new(),
            error: None,
        }
    }

    fn display_name(&self, chat: &Chat) -> String {
        if let Some((n, _)) = self.names.get(chat.jid.user.as_str()) {
            return n.clone();
        }
        if chat.is_group {
            return format!("Gruppe {}", chat.jid.user);
        }
        chat.name.clone()
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
                UiEvent::Contact { user, name, from_book } => {
                    // Adressbuch-Namen überschreiben Push-Namen, nicht umgekehrt.
                    let keep = matches!(self.names.get(&user), Some((_, true))) && !from_book;
                    if !keep {
                        self.names.insert(user, (name, from_book));
                    }
                }
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
            .default_width(340.0)
            .frame(egui::Frame::new().fill(BG_LIST))
            .show(ctx, |ui| {
                egui::Frame::new().fill(BG_BAR).inner_margin(10.0).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("WhatsApp").size(20.0).strong().color(GREEN));
                        let (txt, col) = match self.conn {
                            Conn::Online => ("● online", GREEN),
                            Conn::LoggedOut => ("● abgemeldet", Color32::LIGHT_RED),
                            _ => ("● verbinde", Color32::YELLOW),
                        };
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(txt).color(col).small());
                        });
                    });
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Suchen oder neuen Chat beginnen")
                            .desired_width(ui.available_width() - 8.0),
                    );
                });
                ui.add_space(6.0);

                let needle = self.search.to_lowercase();
                let mut rows: Vec<(String, String, i64)> = self
                    .chats
                    .iter()
                    .map(|(k, c)| (k.clone(), self.display_name(c), c.last_ts))
                    .filter(|(_, n, _)| needle.is_empty() || n.to_lowercase().contains(&needle))
                    .collect();
                rows.sort_by_key(|r| std::cmp::Reverse(r.2));

                let mut clicked = None;
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show_rows(
                    ui,
                    64.0,
                    rows.len(),
                    |ui, range| {
                        for (key, name, ts) in &rows[range] {
                            let c = &self.chats[key];
                            let sel = self.selected.as_deref() == Some(key.as_str());
                            let (rect, resp) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 64.0),
                                egui::Sense::click(),
                            );
                            let bg = if sel {
                                BG_BAR
                            } else if resp.hovered() {
                                Color32::from_rgb(0x17, 0x22, 0x29)
                            } else {
                                BG_LIST
                            };
                            ui.painter().rect_filled(rect, 0.0, bg);
                            let mut inner = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect.shrink2(egui::vec2(12.0, 8.0)))
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                            );
                            avatar(&mut inner, name, 46.0);
                            inner.add_space(10.0);
                            let right_w = 44.0;
                            let text_w = (inner.available_width() - right_w).max(20.0);
                            inner.allocate_ui(egui::vec2(text_w, 46.0), |ui| {
                                ui.vertical(|ui| {
                                    ui.add(egui::Label::new(RichText::new(name).size(15.0).strong()).truncate());
                                    let last = c.messages.last().map(|m| m.text.as_str()).unwrap_or("");
                                    ui.add(
                                        egui::Label::new(RichText::new(last).small().color(TEXT_DIM))
                                            .truncate(),
                                    );
                                });
                            });
                            inner.vertical(|ui| {
                                ui.label(
                                    RichText::new(fmt_time(*ts))
                                        .small()
                                        .color(if c.unread > 0 { GREEN } else { TEXT_DIM }),
                                );
                                if c.unread > 0 {
                                    ui.label(
                                        RichText::new(format!(" {} ", c.unread))
                                            .small()
                                            .strong()
                                            .color(Color32::BLACK)
                                            .background_color(GREEN),
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
                }
            });

        egui::TopBottomPanel::bottom("input")
            .frame(egui::Frame::new().fill(BG_BAR).inner_margin(10.0))
            .show(ctx, |ui| {
                if let Some(e) = self.error.clone() {
                    ui.colored_label(Color32::LIGHT_RED, e);
                }
                ui.horizontal(|ui| {
                    let enabled = self.selected.is_some() && self.conn == Conn::Online;
                    let resp = ui.add_enabled(
                        enabled,
                        egui::TextEdit::singleline(&mut self.input)
                            .hint_text("Nachricht eingeben")
                            .desired_width(ui.available_width() - 90.0)
                            .margin(egui::vec2(10.0, 8.0)),
                    );
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let btn = ui.add_enabled(
                        enabled,
                        egui::Button::new(RichText::new("Senden").strong().color(Color32::WHITE))
                            .fill(GREEN)
                            .min_size(egui::vec2(72.0, 34.0)),
                    );
                    if enter || btn.clicked() {
                        self.send_current();
                        resp.request_focus();
                    }
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG_CHAT))
            .show(ctx, |ui| {
                let Some(key) = self.selected.clone() else {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("Wähle links einen Chat").color(TEXT_DIM).size(18.0));
                    });
                    return;
                };
                let Some(chat) = self.chats.get(&key) else { return };
                let title = self.display_name(chat);
                egui::Frame::new().fill(BG_BAR).inner_margin(10.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        avatar(ui, &title, 38.0);
                        ui.label(RichText::new(&title).size(17.0).strong());
                    });
                });
                // Nur die letzten Nachrichten zeichnen: schnell auch bei langen Chats.
                let start = chat.messages.len().saturating_sub(300);
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.add_space(8.0);
                        for m in &chat.messages[start..] {
                            let (bg, align) = if m.from_me {
                                (BUBBLE_ME, egui::Align::Max)
                            } else {
                                (BUBBLE_THEM, egui::Align::Min)
                            };
                            ui.with_layout(egui::Layout::top_down(align), |ui| {
                                egui::Frame::new()
                                    .fill(bg)
                                    .corner_radius(10.0)
                                    .inner_margin(egui::Margin::symmetric(10, 6))
                                    .outer_margin(egui::Margin::symmetric(14, 2))
                                    .show(ui, |ui| {
                                        ui.set_max_width((ui.available_width() * 0.72).max(120.0));
                                        if chat.is_group && !m.from_me {
                                            ui.label(RichText::new(&m.sender_name).small().strong().color(GREEN));
                                        }
                                        ui.label(&m.text);
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                            ui.label(RichText::new(fmt_time(m.ts)).size(10.5).color(TEXT_DIM));
                                        });
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
