//! Lokaler Verlauf: eine Textdatei mit je einem JSON-Eintrag pro Zeile (JSONL).
//! Anhängen ist schnell und übersteht Abstürze; kaputte Zeilen werden beim Laden übersprungen.

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

fn is_default_extra(x: &crate::backend::MsgExtra) -> bool {
    *x == crate::backend::MsgExtra::default()
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Rec {
    #[serde(rename = "msg")]
    Msg {
        chat: String,
        is_group: bool,
        from_me: bool,
        sender: String,
        text: String,
        ts: i64,
        id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        sjid: String,
        #[serde(default)]
        st: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<crate::backend::MediaRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quote: Option<crate::backend::Quote>,
        #[serde(default, skip_serializing_if = "is_default_extra")]
        x: crate::backend::MsgExtra,
    },
    #[serde(rename = "chat")]
    Chat { jid: String, name: String, is_group: bool, last_ts: i64 },
    #[serde(rename = "name")]
    Name { user: String, name: String, book: bool },
    /// Zustellstatus einer eigenen Nachricht (2 = zugestellt, 3 = gelesen).
    #[serde(rename = "st")]
    Status { id: String, st: u8 },
    /// Reaktion (leeres Emoji = entfernt); `from` leer = ich.
    #[serde(rename = "re")]
    React { target: String, from: String, emoji: String },
    /// Für alle gelöscht.
    #[serde(rename = "del")]
    Del { id: String },
    #[serde(rename = "ed")]
    Edit { id: String, text: String },
    #[serde(rename = "star")]
    Star { id: String, on: bool },
    /// Ausgehende Textnachricht, solange der Server sie noch nicht bestätigt hat.
    #[serde(rename = "out")]
    Out {
        temp: String,
        chat: String,
        is_group: bool,
        text: String,
        ts: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quote: Option<crate::backend::Quote>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mentions: Vec<String>,
        #[serde(default)]
        exp: u32,
    },
    /// Ausgehende Nachricht erledigt (gesendet oder verworfen).
    #[serde(rename = "outdone")]
    OutDone { temp: String },
    /// Stimme in einer Umfrage; `voter` leer = ich.
    #[serde(rename = "vote")]
    Vote { poll: String, voter: String, options: Vec<String> },
}

pub struct Store {
    file: Option<std::fs::File>,
    /// Erster Schreibfehler seit dem letzten Abholen (für einen Hinweis in der UI).
    error: Option<String>,
}

impl Store {
    /// Öffnet die Datei und liefert alle bisherigen Einträge.
    pub fn open(path: &Path) -> (Store, Vec<Rec>) {
        let bytes = std::fs::read(path).unwrap_or_default();
        // Nach einem Absturz kann die letzte Zeile halb geschrieben sein. Ohne Reparatur
        // würde der nächste Eintrag direkt daran hängen und ebenfalls unlesbar werden.
        if !bytes.is_empty() && !bytes.ends_with(b"\n") {
            let keep = bytes.iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0);
            if let Ok(f) = std::fs::OpenOptions::new().write(true).open(path) {
                let _ = f.set_len(keep as u64);
            }
        }
        let text = String::from_utf8_lossy(&bytes);
        let recs: Vec<Rec> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(path).ok();
        (Store { file, error: None }, recs)
    }

    /// Eintrag anhängen. Ein Fehler wird gemerkt und per `take_error` an die UI gemeldet.
    pub fn append(&mut self, rec: &Rec) {
        let r = (|| -> Result<(), String> {
            let f = self.file.as_mut().ok_or("Verlaufsdatei nicht geöffnet")?;
            let mut line = serde_json::to_string(rec).map_err(|e| e.to_string())?;
            line.push('\n');
            f.write_all(line.as_bytes()).map_err(|e| e.to_string())
        })();
        if let Err(e) = r {
            if self.error.is_none() {
                self.error = Some(e);
            }
        }
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_reload_survives_bad_lines() {
        let dir = std::env::temp_dir().join(format!("wa-snap-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");
        let _ = std::fs::remove_file(&path);
        {
            let (mut s, recs) = Store::open(&path);
            assert!(recs.is_empty());
            s.append(&Rec::Msg {
                chat: "49151@s.whatsapp.net".into(),
                is_group: false,
                from_me: true,
                sender: "Ich".into(),
                text: "Hallo\nzweite Zeile 👍".into(),
                ts: 1_700_000_000,
                id: "A1".into(),
                sjid: String::new(),
                st: 2,
                media: None,
                quote: None,
                x: Default::default(),
            });
            s.append(&Rec::Name { user: "49151".into(), name: "Anna".into(), book: true });
        }
        // kaputte Zeile dazwischen darf das Laden nicht verhindern
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{kaputt\n").unwrap();
        let (_s, recs) = Store::open(&path);
        assert_eq!(recs.len(), 2);
        match &recs[0] {
            Rec::Msg { text, ts, .. } => {
                assert_eq!(text, "Hallo\nzweite Zeile 👍");
                assert_eq!(*ts, 1_700_000_000);
            }
            _ => panic!("erste Zeile sollte eine Nachricht sein"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn half_written_last_line_is_cut_off() {
        let dir = std::env::temp_dir().join(format!("wa-snap-test-half-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");
        std::fs::write(&path, b"{\"t\":\"del\",\"id\":\"A\"}\n{\"t\":\"del\",\"id\":\"B").unwrap();
        {
            let (mut s, recs) = Store::open(&path);
            assert_eq!(recs.len(), 1);
            s.append(&Rec::Del { id: "C".into() });
            assert!(s.take_error().is_none());
        }
        let (_s, recs) = Store::open(&path);
        assert_eq!(recs.len(), 2, "der Eintrag nach dem Absturz muss lesbar bleiben");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
