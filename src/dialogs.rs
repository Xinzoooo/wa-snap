//! Rückfragen, die zur Sicherheit gestellt werden: riskante Datei öffnen, nach dem
//! Abmelden lokale Daten löschen.

use eframe::egui::{self, RichText};

use crate::backend::Cmd;
use crate::{data_dir, safety, App};

/// Datei, vor deren Öffnen gewarnt wird, mit dem Grund.
pub struct OpenWarn {
    pub path: std::path::PathBuf,
    pub reason: String,
}

/// Name der Merkdatei: beim nächsten Start wird die Sitzungsdatenbank gelöscht.
const LOGOUT_MARK: &str = "abgemeldet.merker";

/// Nach dem Abmelden vormerken, dass die Schlüssel gelöscht werden. Sofort geht das nicht,
/// weil die Datenbank noch geöffnet ist; die Bibliothek selbst leert sie beim Abmelden nicht.
pub fn mark_logged_out(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::write(dir.join(LOGOUT_MARK), b"")
}

/// Beim Start: liegt die Merkdatei, die Sitzungsdatenbank entfernen. Gibt zurück, ob gelöscht wurde.
pub fn finish_logout(dir: &std::path::Path) -> bool {
    let mark = dir.join(LOGOUT_MARK);
    if !mark.exists() {
        return false;
    }
    for f in ["whatsapp.db", "whatsapp.db-wal", "whatsapp.db-shm"] {
        let _ = std::fs::remove_file(dir.join(f));
    }
    let _ = std::fs::remove_file(mark);
    true
}

impl App {
    pub fn safety_dialogs(&mut self, ctx: &egui::Context) {
        if let Some(w) = self.open_warn.as_ref() {
            let mut choice = 0;
            egui::Window::new(t!("Diese Datei wirklich öffnen?", "Really open this file?"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_width(420.0);
                    let name = w.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    ui.label(RichText::new(name).strong());
                    ui.add_space(4.0);
                    ui.label(&w.reason);
                    ui.label(t!("Öffne sie nur, wenn du dem Absender vertraust und die Datei erwartet hast.", "Only open it if you trust the sender and expected this file."));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(t!("Im Ordner anzeigen", "Show in folder")).clicked() {
                            choice = 1;
                        }
                        if ui.button(t!("Abbrechen", "Cancel")).clicked() {
                            choice = 3;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(RichText::new(t!("Trotzdem öffnen", "Open anyway")).color(ui.visuals().error_fg_color)).clicked() {
                                choice = 2;
                            }
                        });
                    });
                });
            if choice != 0 {
                if let Some(w) = self.open_warn.take() {
                    match choice {
                        1 => safety::show_in_folder(&w.path),
                        2 => {
                            crate::backend::log(format!("Riskante Datei auf Wunsch geöffnet: {}", w.path.display()));
                            safety::spawn_opener(w.path.as_os_str());
                        }
                        _ => {}
                    }
                }
            }
        }

        if self.logout_prompt {
            let mut choice = 0;
            egui::Window::new(t!("Abgemeldet", "Logged out"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_width(420.0);
                    ui.label(t!("WhatsSnap ist vom Handy getrennt. Die Anmeldeschlüssel werden beim nächsten Start von diesem PC entfernt.", "WhatsSnap is unlinked from your phone. The login keys will be removed from this PC on the next start."));
                    ui.add_space(6.0);
                    ui.label(t!("Auch Verlauf und Medien auf diesem PC löschen?", "Also delete history and media on this PC?"));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new(t!("Ja, alles löschen", "Yes, delete everything")).color(ui.visuals().error_fg_color)).clicked() {
                            choice = 1;
                        }
                        if ui.button(t!("Verlauf behalten", "Keep history")).clicked() {
                            choice = 2;
                        }
                    });
                });
            if choice != 0 {
                self.logout_prompt = false;
                if choice == 1 {
                    self.wipe_local_data();
                }
            }
        }
    }

    /// Verlauf, Medien, Profilbilder, Log und Zwischendateien löschen und die Anzeige leeren.
    pub fn wipe_local_data(&mut self) {
        let dir = data_dir();
        let w = safety::wipe_local(&dir, false);
        // Die Verlaufsdatei ist weg: neu öffnen, sonst gingen neue Einträge ins Leere.
        self.store = crate::store::Store::open(&dir.join("history.jsonl")).0;
        self.chats.clear();
        self.msg_index.clear();
        self.reactions.clear();
        self.outbox.clear();
        self.outbox_files.clear();
        self.alias.clear();
        self.heights.clear();
        self.selected = None;
        self.viewer = None;
        self.images.clear();
        self.anims.clear();
        self.avatars.clear();
        self.avatar_requested.clear();
        self.img_requested.clear();
        self.decoded.clear();
        self.info = Some(if w.failed.is_empty() {
            t!("Lokale Daten gelöscht (Verlauf, Medien, Profilbilder, Log).", "Local data deleted (history, media, profile photos, log).").into()
        } else {
            tf!("Nicht alles ließ sich löschen: {}", "Not everything could be deleted: {}", w.failed.iter().map(|(p, e)| format!("{}: {e}", p.display())).collect::<Vec<_>>().join("; "))
        });
        let _ = self.cmd.send(Cmd::FetchChannels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_is_removed_on_next_start_after_logout() {
        let d = std::env::temp_dir().join(format!("wasnap-logout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for f in ["whatsapp.db", "whatsapp.db-wal", "history.jsonl"] {
            std::fs::write(d.join(f), "x").unwrap();
        }
        assert!(!finish_logout(&d), "ohne Merker bleibt alles");
        assert!(d.join("whatsapp.db").exists());
        mark_logged_out(&d).unwrap();
        assert!(finish_logout(&d));
        assert!(!d.join("whatsapp.db").exists() && !d.join("whatsapp.db-wal").exists());
        assert!(d.join("history.jsonl").exists(), "der Verlauf wird nur auf Wunsch gelöscht");
        assert!(!finish_logout(&d), "der Merker ist verbraucht");
        let _ = std::fs::remove_dir_all(&d);
    }
}
