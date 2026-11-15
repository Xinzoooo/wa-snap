//! Schutzfunktionen an einer Stelle: was mit dem Standardprogramm geöffnet werden darf,
//! Telefonnummern im Log unkenntlich machen, Benachrichtigungstext entschärfen,
//! Dateirechte, Einzelinstanz-Sperre und das Löschen lokaler Daten.

use std::path::{Path, PathBuf};

/// Dateitypen, die ohne Rückfrage mit dem Standardprogramm geöffnet werden:
/// Bilder, Video, Audio, PDF, Office-Dokumente und reiner Text.
const SAFE_EXT: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "heic", "heif", "avif", "tif", "tiff", // Bilder
    "mp4", "m4v", "mov", "webm", "mkv", "avi", "3gp", // Video
    "mp3", "m4a", "aac", "ogg", "oga", "opus", "wav", "flac", "amr", // Audio
    "pdf", "txt", "csv", "md", "rtf", // Text
    "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "epub", // Office
    "vcf", "ics",
];

/// Inhalte, die ein Programm starten oder Code ausführen können, egal wie die Datei heißt.
const RISKY_MIME: &[&str] = &[
    "application/x-executable",
    "application/vnd.microsoft.portable-executable",
    "application/x-mach-binary",
    "application/x-sharedlib",
    "application/java-archive",
    "application/x-rpm",
    "application/vnd.debian.binary-package",
    "application/x-shellscript",
    "application/x-msdownload",
    "application/vnd.android.package-archive",
    "text/html",
    "application/x-sh",
];

/// Darf die Datei ohne Rückfrage geöffnet werden? `Err` nennt den Grund für den Warn-Dialog.
/// Geprüft wird die Endung UND der Inhalt: eine „rechnung.pdf“, die in Wahrheit ein
/// Programm oder Skript ist, fällt durch.
pub fn open_verdict(path: &Path) -> Result<(), String> {
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    if !SAFE_EXT.contains(&ext.as_str()) {
        return Err(if ext.is_empty() {
            t!("Datei ohne Endung: der Typ ist unbekannt.", "File without an extension: its type is unknown.").into()
        } else {
            tf!("Dateien vom Typ „.{ext}“ können Programme ausführen oder sind unbekannt.", "Files of type “.{ext}” can run programs or are unknown.")
        });
    }
    let mut head = [0u8; 8192];
    let n = std::fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    let head = &head[..n];
    if head.starts_with(b"#!") || head.starts_with(b"\x7fELF") || head.starts_with(b"MZ") {
        return Err(t!("Der Inhalt ist ein Programm oder Skript, obwohl der Name etwas anderes sagt.", "The content is a program or script although the name says otherwise.").into());
    }
    if let Some(kind) = infer::get(head) {
        if RISKY_MIME.contains(&kind.mime_type()) {
            return Err(tf!("Der Inhalt ist „{}“, obwohl der Name etwas anderes sagt.", "The content is “{}” although the name says otherwise.", kind.mime_type()));
        }
        // Bild-Endung, aber z. B. ein Archiv oder Office-Dokument darin: nicht stillschweigend öffnen.
        let claims_media = matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "mp4" | "m4v" | "mov" | "webm" | "mp3" | "m4a" | "ogg" | "opus" | "wav" | "pdf");
        let is_media = matches!(kind.matcher_type(), infer::MatcherType::Image | infer::MatcherType::Video | infer::MatcherType::Audio) || kind.mime_type() == "application/pdf";
        if claims_media && !is_media {
            return Err(tf!("Der Inhalt ({}) passt nicht zur Endung „.{ext}“.", "The content ({}) does not match the extension “.{ext}”.", kind.mime_type()));
        }
    }
    Ok(())
}

/// Ordner der Datei im Dateimanager öffnen und die Datei markieren (ersatzweise nur den Ordner).
pub fn show_in_folder(path: &Path) {
    let uri = format!("file://{}", path.display());
    let ok = std::process::Command::new("dbus-send")
        .args(["--session", "--dest=org.freedesktop.FileManager1", "--type=method_call", "/org/freedesktop/FileManager1", "org.freedesktop.FileManager1.ShowItems", &format!("array:string:{uri}"), "string:"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        if let Some(dir) = path.parent() {
            spawn_opener(dir.as_os_str());
        }
    }
}

/// `xdg-open` starten. Das geöffnete Programm soll wieder normale Dateirechte vergeben,
/// nicht die strenge Maske von WhatsSnap erben.
pub fn spawn_opener(target: &std::ffi::OsStr) {
    use std::os::unix::process::CommandExt;
    let mut c = std::process::Command::new("xdg-open");
    c.arg(target);
    // SAFETY: `umask` ist async-signal-safe und fasst keinen Speicher des Elternprozesses an.
    unsafe {
        c.pre_exec(|| {
            libc::umask(0o022);
            Ok(())
        });
    }
    let _ = c.spawn();
}

/// Alles, was WhatsSnap ab jetzt anlegt, gehört nur dem eigenen Nutzer:
/// Dateien 0600, Ordner 0700 – auch Datenbank, WAL, Medien und Log, sofort beim Anlegen.
pub fn restrict_new_files() {
    // SAFETY: setzt nur die Dateimaske des eigenen Prozesses.
    unsafe {
        libc::umask(0o077);
    }
}

/// Ziffernfolgen ab 7 Stellen (Telefonnummern, LIDs, Gruppen-IDs) durch ein Kürzel ersetzen:
/// kurzer Prüfwert plus die letzten zwei Ziffern. Derselbe Kontakt bleibt im Log
/// wiedererkennbar, die Nummer steht aber nicht mehr drin.
pub fn anonymize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let run = &text[start..i];
            // Zeitstempel (2026-11-11 18:42:21) und kurze Zahlen bleiben lesbar.
            if run.len() >= 7 {
                let mut h: u32 = 0x811c_9dc5;
                for c in run.bytes() {
                    h = (h ^ c as u32).wrapping_mul(0x0100_0193);
                }
                out.push_str(&format!("#{:04x}…{}", h & 0xffff, &run[run.len() - 2..]));
            } else {
                out.push_str(run);
            }
        } else {
            let ch = text[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Text für Benachrichtigungen entschärfen: Plasma und GNOME deuten `<b>`, `<a href>`, `<img>`
/// im Text als Markup. Eine Nachricht darf so etwas nicht einschleusen können.
pub fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// Sperre gegen einen gleichzeitigen zweiten Start. Die zurückgegebene Datei muss bis zum
/// Programmende offen bleiben; `None` heißt: eine andere Instanz hält die Sperre.
pub fn lock_instance(dir: &Path) -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).mode(0o600).open(dir.join("wa-snap.lock")).ok()?;
    // SAFETY: gültiger Dateideskriptor einer eben geöffneten Datei.
    let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    (r == 0).then_some(f)
}

/// Was beim Löschen lokaler Daten entfernt wurde bzw. nicht entfernt werden konnte.
#[derive(Default, Debug)]
pub struct Wiped {
    pub removed: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
}

/// Verlauf, Medien, Profilbilder, Log und Zwischendateien auf diesem PC löschen.
/// Mit `session` zusätzlich die Sitzungsdatenbank (nur nach dem Abmelden sinnvoll).
/// Die Einstellungen bleiben.
pub fn wipe_local(dir: &Path, session: bool) -> Wiped {
    let mut w = Wiped::default();
    let mut targets: Vec<PathBuf> = ["history.jsonl", "history.jsonl.tmp", "wa-snap.log", "wa-snap.log.old", "media", "avatars", "tmp"].iter().map(|n| dir.join(n)).collect();
    if session {
        targets.extend(["whatsapp.db", "whatsapp.db-wal", "whatsapp.db-shm"].iter().map(|n| dir.join(n)));
    }
    for p in targets {
        let r = match std::fs::symlink_metadata(&p) {
            Err(_) => continue,
            Ok(m) if m.is_dir() => std::fs::remove_dir_all(&p),
            Ok(_) => std::fs::remove_file(&p),
        };
        match r {
            Ok(()) => w.removed.push(p),
            Err(e) => w.failed.push((p, e.to_string())),
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wasnap-safety-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn harmless_files_open_directly() {
        let d = tmp("ok");
        let png = d.join("bild.png");
        std::fs::write(&png, b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").unwrap();
        assert!(open_verdict(&png).is_ok());
        let txt = d.join("notiz.txt");
        std::fs::write(&txt, "Hallo").unwrap();
        assert!(open_verdict(&txt).is_ok());
        let pdf = d.join("a.PDF");
        std::fs::write(&pdf, b"%PDF-1.7\n").unwrap();
        assert!(open_verdict(&pdf).is_ok());
    }

    #[test]
    fn dangerous_extensions_need_confirmation() {
        let d = tmp("ext");
        for name in ["x.sh", "x.jar", "x.py", "x.html", "x.desktop", "x.rpm", "x.exe", "ohne-endung", "x.svg"] {
            let p = d.join(name);
            std::fs::write(&p, "egal").unwrap();
            assert!(open_verdict(&p).is_err(), "{name} darf nicht stillschweigend geöffnet werden");
        }
    }

    #[test]
    fn disguised_content_is_caught() {
        let d = tmp("tarn");
        let script = d.join("rechnung.pdf");
        std::fs::write(&script, "#!/bin/sh\nrm -rf ~\n").unwrap();
        assert!(open_verdict(&script).is_err());
        let elf = d.join("foto.jpg");
        std::fs::write(&elf, b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0").unwrap();
        assert!(open_verdict(&elf).is_err());
        let zip = d.join("urlaub.png");
        std::fs::write(&zip, b"PK\x03\x04\x14\0\0\0\x08\0").unwrap();
        assert!(open_verdict(&zip).is_err(), "Archiv mit Bild-Endung");
    }

    #[test]
    fn numbers_are_hidden_but_stable() {
        let a = anonymize("Nachricht 4915112345678@s.whatsapp.net von_mir=false");
        assert!(!a.contains("4915112345678"));
        assert!(a.contains("…78@s.whatsapp.net"), "{a}");
        assert_eq!(a, anonymize("Nachricht 4915112345678@s.whatsapp.net von_mir=false"), "derselbe Kontakt, dasselbe Kürzel");
        assert_ne!(anonymize("4915112345678"), anonymize("4915112345178"));
    }

    #[test]
    fn short_numbers_and_text_stay() {
        assert_eq!(anonymize("code=429, 23 Gruppen, Größe 512000 ü"), "code=429, 23 Gruppen, Größe 512000 ü");
        let g = anonymize("120363041234567890@g.us");
        assert!(g.starts_with('#') && g.ends_with("90@g.us"), "{g}");
    }

    #[test]
    fn markup_is_escaped() {
        assert_eq!(escape_markup("<b>hi</b> & <img src=x>"), "&lt;b&gt;hi&lt;/b&gt; &amp; &lt;img src=x&gt;");
        assert_eq!(escape_markup("normal 👍"), "normal 👍");
    }

    #[test]
    fn second_lock_is_refused() {
        let d = tmp("lock");
        let first = lock_instance(&d);
        assert!(first.is_some());
        assert!(lock_instance(&d).is_none(), "zweite Instanz darf die Sperre nicht bekommen");
        drop(first);
        assert!(lock_instance(&d).is_some(), "nach dem Ende ist die Sperre wieder frei");
    }

    #[test]
    fn wipe_removes_data_but_keeps_settings_and_session() {
        let d = tmp("wipe");
        for f in ["history.jsonl", "wa-snap.log", "settings.json", "whatsapp.db", "whatsapp.db-wal"] {
            std::fs::write(d.join(f), "x").unwrap();
        }
        for sub in ["media", "avatars", "tmp"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
            std::fs::write(d.join(sub).join("a"), "x").unwrap();
        }
        let w = wipe_local(&d, false);
        assert!(w.failed.is_empty(), "{:?}", w.failed);
        for gone in ["history.jsonl", "wa-snap.log", "media", "avatars", "tmp"] {
            assert!(!d.join(gone).exists(), "{gone} sollte weg sein");
        }
        assert!(d.join("settings.json").exists());
        assert!(d.join("whatsapp.db").exists(), "ohne Abmelden bleibt die Sitzung");
        let w = wipe_local(&d, true);
        assert!(w.failed.is_empty());
        assert!(!d.join("whatsapp.db").exists() && !d.join("whatsapp.db-wal").exists());
        assert!(d.join("settings.json").exists());
    }
}
