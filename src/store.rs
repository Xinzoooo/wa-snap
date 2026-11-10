//! Lokaler Verlauf: eine Textdatei mit je einem JSON-Eintrag pro Zeile (JSONL).
//! Anhängen ist schnell und übersteht Abstürze; kaputte Zeilen werden beim Laden übersprungen.

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

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
    },
    #[serde(rename = "chat")]
    Chat { jid: String, name: String, is_group: bool, last_ts: i64 },
    #[serde(rename = "name")]
    Name { user: String, name: String, book: bool },
}

pub struct Store {
    file: Option<std::fs::File>,
}

impl Store {
    /// Öffnet die Datei und liefert alle bisherigen Einträge.
    pub fn open(path: &Path) -> (Store, Vec<Rec>) {
        let recs: Vec<Rec> = std::fs::read_to_string(path)
            .map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
            .unwrap_or_default();
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok();
        (Store { file }, recs)
    }

    pub fn append(&mut self, rec: &Rec) {
        if let (Some(f), Ok(mut line)) = (self.file.as_mut(), serde_json::to_string(rec)) {
            line.push('\n');
            let _ = f.write_all(line.as_bytes());
        }
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
}
