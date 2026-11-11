//! Symbol im Systemabschnitt der Leiste (KDE/StatusNotifierItem): App läuft nach dem
//! Schließen des Fensters weiter, Klick öffnet es wieder, Punkt zeigt Ungelesenes.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;

use ksni::blocking::TrayMethods;

/// Anzahl ungelesener Chats/Nachrichten für das Symbol (UI und Backend schreiben hinein).
pub static BADGE: AtomicU32 = AtomicU32::new(0);

pub enum TrayMsg {
    Open,
    Quit,
}

struct WaTray {
    tx: Sender<TrayMsg>,
    unread: u32,
}

/// Symbol zeichnen: grüner Kreis mit weißer Sprechblase, roter Punkt bei Ungelesenem.
fn render(size: i32, unread: bool) -> ksni::Icon {
    let s = size as f32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut px = [0u8, 0, 0, 0]; // ARGB
            let d = ((fx - s / 2.0).powi(2) + (fy - s / 2.0).powi(2)).sqrt();
            if d <= s / 2.0 {
                px = [255, 0x21, 0xc0, 0x63];
                // Sprechblase: Ellipse plus Zipfel unten links
                let ex = (fx - s * 0.5) / (s * 0.27);
                let ey = (fy - s * 0.46) / (s * 0.22);
                let tail = fx > s * 0.26 && fx < s * 0.42 && fy > s * 0.58 && fy < s * 0.78 && (fx - s * 0.26) > (fy - s * 0.58) * 0.8;
                if ex * ex + ey * ey <= 1.0 || tail {
                    px = [255, 255, 255, 255];
                }
            }
            if unread {
                let bd = ((fx - s * 0.80).powi(2) + (fy - s * 0.20).powi(2)).sqrt();
                if bd <= s * 0.20 {
                    px = [255, 0xe5, 0x39, 0x35];
                }
            }
            data.extend_from_slice(&px);
        }
    }
    ksni::Icon { width: size, height: size, data }
}

impl ksni::Tray for WaTray {
    fn id(&self) -> String {
        "wa-snap".into()
    }
    fn title(&self) -> String {
        "WhatsSnap".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [22, 32, 48, 64].iter().map(|&s| render(s, self.unread > 0)).collect()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "WhatsSnap".into(),
            description: match self.unread {
                0 => t!("Keine ungelesenen Nachrichten", "No unread messages").into(),
                1 => t!("1 ungelesene Nachricht", "1 unread message").into(),
                n => tf!("{n} ungelesene Nachrichten", "{n} unread messages"),
            },
            ..Default::default()
        }
    }
    fn status(&self) -> ksni::Status {
        if self.unread > 0 { ksni::Status::NeedsAttention } else { ksni::Status::Active }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(TrayMsg::Open);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: t!("Öffnen", "Open").into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(TrayMsg::Open);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: t!("Beenden", "Quit").into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(TrayMsg::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Symbol starten. Gibt false zurück, wenn die Leiste keine Tray-Symbole kann.
pub fn start(tx: Sender<TrayMsg>) -> bool {
    let tray = WaTray { tx, unread: 0 };
    match tray.spawn() {
        Ok(handle) => {
            // Ungelesen-Zähler regelmäßig übernehmen.
            std::thread::spawn(move || {
                let mut last = u32::MAX;
                loop {
                    let n = BADGE.load(Ordering::Relaxed);
                    if n != last {
                        last = n;
                        if handle.update(|t| t.unread = n).is_none() {
                            break;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(800));
                }
            });
            true
        }
        Err(e) => {
            crate::backend::log(format!("Tray-Symbol nicht verfügbar: {e}"));
            false
        }
    }
}
