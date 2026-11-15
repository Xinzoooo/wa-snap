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
    /// Ausgehende Datei (Foto, Video, Dokument, Sprachnachricht), solange der Server sie noch
    /// nicht bestätigt hat. Die Datei selbst liegt als `<temp>.<endung>` im Medienordner.
    #[serde(rename = "outfile")]
    OutFile {
        temp: String,
        chat: String,
        is_group: bool,
        ts: i64,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        caption: String,
        media: crate::backend::MediaRef,
    },
    /// Ausgehende Nachricht erledigt (gesendet oder verworfen).
    #[serde(rename = "outdone")]
    OutDone { temp: String },
    /// Dieselbe Person unter zwei Kennungen: interne LID und Telefonnummer (nur die Ziffern).
    #[serde(rename = "alias")]
    Alias { lid: String, pn: String },
    /// Stimme in einer Umfrage; `voter` leer = ich.
    #[serde(rename = "vote")]
    Vote { poll: String, voter: String, options: Vec<String> },
}

/// Ab so vielen überholten Einträgen wird die Datei beim Start neu geschrieben.
const COMPACT_MIN: usize = 500;

/// Überholte Einträge entfernen. Alle anderen Zeilen (auch unlesbare) bleiben unverändert.
/// `recs` gehört zeilenweise zu `text`.
///
/// - Reaktionen: Schaltet jemand eine Reaktion immer wieder an und aus, entsteht pro Wechsel
///   eine Zeile; es zählt nur die letzte je Nachricht und Absender.
/// - Zustellstatus: von „gesendet → zugestellt → gelesen“ zählt nur der Endstand.
/// - Status-Beiträge (die 24-Stunden-Meldungen) älter als einen Tag: sie werden nie wieder gezeigt.
///
/// Bearbeitungen bleiben vollständig: daraus entstehen die „früheren Fassungen“.
fn compact(path: &Path, text: &str, recs: &mut Vec<Option<Rec>>) {
    compact_at(path, text, recs, chrono::Utc::now().timestamp())
}

fn compact_at(path: &Path, text: &str, recs: &mut Vec<Option<Rec>>, now: i64) {
    let mut last: std::collections::HashMap<(&str, &str), usize> = std::collections::HashMap::new();
    // je Nachricht: (geltender Stand, Zeile, die ihn gesetzt hat)
    let mut status: std::collections::HashMap<&str, (u8, usize)> = std::collections::HashMap::new();
    let mut dead = vec![false; recs.len()];
    let mut n = 0;
    for (i, r) in recs.iter().enumerate() {
        match r {
            Some(Rec::React { target, from, .. }) => {
                if let Some(old) = last.insert((target.as_str(), from.as_str()), i) {
                    dead[old] = true;
                    n += 1;
                }
            }
            // Gleiche Regel wie beim Anwenden: nur ein höherer Stand oder ein Fehler (9) zählt.
            Some(Rec::Status { id, st }) => match status.get_mut(id.as_str()) {
                Some((cur, line)) => {
                    if *st > *cur || *st == 9 {
                        dead[*line] = true;
                        (*cur, *line) = (*st, i);
                    } else {
                        dead[i] = true;
                    }
                    n += 1;
                }
                None => {
                    status.insert(id.as_str(), (*st, i));
                }
            },
            Some(Rec::Msg { chat, ts, .. }) if chat == "status@broadcast" && *ts < now - 86_400 => {
                dead[i] = true;
                n += 1;
            }
            _ => {}
        }
    }
    drop(last);
    drop(status);
    if n < COMPACT_MIN {
        return;
    }
    let mut out = String::with_capacity(text.len());
    for (l, _) in text.lines().zip(&dead).filter(|(_, d)| !**d) {
        out.push_str(l);
        out.push('\n');
    }
    // Erst vollständig daneben schreiben, dann umbenennen: ein Absturz lässt die alte Datei heil.
    let tmp = path.with_extension("jsonl.tmp");
    let written = (|| -> std::io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(out.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    let mut i = 0;
    recs.retain(|_| {
        i += 1;
        !dead[i - 1]
    });
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
        let mut recs: Vec<Option<Rec>> = text.lines().map(|l| serde_json::from_str(l).ok()).collect();
        compact(path, &text, &mut recs);
        let recs: Vec<Rec> = recs.into_iter().flatten().collect();
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
    fn superseded_reactions_are_compacted() {
        let dir = std::env::temp_dir().join(format!("wa-snap-test-compact-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");
        let mut text = String::from("{\"t\":\"del\",\"id\":\"A\"}\n{kaputt\n");
        for i in 0..COMPACT_MIN + 1 {
            let e = if i % 2 == 0 { "👍" } else { "" };
            text.push_str(&format!("{{\"t\":\"re\",\"target\":\"M1\",\"from\":\"49151\",\"emoji\":\"{e}\"}}\n"));
        }
        text.push_str("{\"t\":\"re\",\"target\":\"M2\",\"from\":\"49151\",\"emoji\":\"❤\"}\n");
        std::fs::write(&path, &text).unwrap();
        let (mut s, recs) = Store::open(&path);
        assert_eq!(recs.len(), 3, "Löschung + je eine Reaktion für M1 und M2");
        match &recs[1] {
            Rec::React { target, emoji, .. } => {
                assert_eq!(target, "M1");
                assert_eq!(emoji, "👍", "die letzte Reaktion gewinnt");
            }
            _ => panic!("zweiter Eintrag sollte die Reaktion sein"),
        }
        s.append(&Rec::Del { id: "Z".into() });
        drop(s);
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after.lines().count(), 5, "kaputte Zeile bleibt, neue hängt sauber an");
        assert!(after.contains("{kaputt"));
        let (_s, recs) = Store::open(&path);
        assert_eq!(recs.len(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_updates_and_old_status_posts_are_folded() {
        let dir = std::env::temp_dir().join(format!("wa-snap-test-fold-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");
        let now = 1_800_000_000i64;
        let mut text = String::new();
        // 300 Nachrichten mit je drei Statuszeilen (zugestellt, gelesen, noch einmal zugestellt)
        for i in 0..300 {
            for st in [2, 3, 2] {
                text.push_str(&format!("{{\"t\":\"st\",\"id\":\"M{i}\",\"st\":{st}}}\n"));
            }
        }
        // Fehler bleibt Fehler-Stand, auch wenn danach ein niedrigerer Wert kommt
        text.push_str("{\"t\":\"st\",\"id\":\"F\",\"st\":3}\n{\"t\":\"st\",\"id\":\"F\",\"st\":9}\n{\"t\":\"st\",\"id\":\"F\",\"st\":2}\n");
        for (id, ts) in [("ALT", now - 90_000), ("NEU", now - 3_600)] {
            text.push_str(&format!("{{\"t\":\"msg\",\"chat\":\"status@broadcast\",\"is_group\":false,\"from_me\":false,\"sender\":\"A\",\"text\":\"x\",\"ts\":{ts},\"id\":\"{id}\"}}\n"));
        }
        text.push_str(&format!("{{\"t\":\"msg\",\"chat\":\"49151@s.whatsapp.net\",\"is_group\":false,\"from_me\":false,\"sender\":\"A\",\"text\":\"bleibt\",\"ts\":{},\"id\":\"CHAT\"}}\n", now - 900_000));
        text.push_str("{\"t\":\"ed\",\"id\":\"CHAT\",\"text\":\"eins\"}\n{\"t\":\"ed\",\"id\":\"CHAT\",\"text\":\"zwei\"}\n");
        std::fs::write(&path, &text).unwrap();
        let mut recs: Vec<Option<Rec>> = text.lines().map(|l| serde_json::from_str(l).ok()).collect();
        compact_at(&path, &text, &mut recs, now);
        let recs: Vec<Rec> = recs.into_iter().flatten().collect();
        let st: Vec<(&str, u8)> = recs.iter().filter_map(|r| if let Rec::Status { id, st } = r { Some((id.as_str(), *st)) } else { None }).collect();
        assert_eq!(st.len(), 301, "je Nachricht ein Stand");
        assert!(st.iter().filter(|(id, _)| *id != "F").all(|(_, s)| *s == 3), "gelesen gewinnt gegen späteres zugestellt");
        assert!(st.contains(&("F", 9)));
        let ids: Vec<&str> = recs.iter().filter_map(|r| if let Rec::Msg { id, .. } = r { Some(id.as_str()) } else { None }).collect();
        assert_eq!(ids, ["NEU", "CHAT"], "alter Status-Beitrag weg, normale alte Nachricht bleibt");
        assert_eq!(recs.iter().filter(|r| matches!(r, Rec::Edit { .. })).count(), 2, "Bearbeitungen bleiben");
        // Die Datei auf der Platte entspricht dem Ergebnis
        let (_s, again) = Store::open(&path);
        assert_eq!(again.len(), recs.len());
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
