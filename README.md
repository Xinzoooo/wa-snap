# wa-snap

Schneller, nativer WhatsApp-Desktop-Client in Rust. Kein Browser und kein
Electron: Die Oberfläche ist [egui](https://github.com/emilk/egui), das
Protokoll kommt von [`whatsapp-rust`](https://github.com/jlucaso1/whatsapp-rust).

> **Inoffiziell.** WhatsApp bietet für private Konten keine offizielle API.
> Inoffizielle Clients können gegen die Nutzungsbedingungen von Meta
> verstoßen und zur Sperrung des Kontos führen. Benutzung auf eigene Gefahr,
> am besten zuerst mit einer Zweitnummer.

## Stand (v0.1)

Funktioniert (kompiliert und startet; der Login mit echtem Konto ist noch
nicht getestet):

- Verknüpfung per QR-Code (WhatsApp → Verknüpfte Geräte → Gerät hinzufügen)
- Sitzung wird gespeichert, kein erneutes Scannen beim nächsten Start
- Chatliste mit Suche, Ungelesen-Zähler, sortiert nach letzter Nachricht
- Nachrichten empfangen und Textnachrichten senden (Enter)
- Medien erscheinen als Platzhalter, z. B. `[image]`

Noch nicht vorhanden:

- Nachrichtenverlauf nach Neustart (nur die Sitzung wird gespeichert)
- Kontakt- und Gruppennamen aus dem Adressbuch (Namen kommen aus den Nachrichten)
- Bilder, Dokumente, Sprachnachrichten, Anrufe, Emojis in der Schrift
- Uhrzeiten werden in UTC angezeigt

## Bauen

Voraussetzung ist eine stabile Rust-Toolchain (getestet mit 1.97).

```bash
cargo run --release
```

Der erste Build dauert einige Minuten. Die Sitzungsdatenbank liegt unter
`~/.local/share/wa-snap/whatsapp.db`; sie enthält deine Schlüssel und gehört
nicht in ein öffentliches Repo (`.gitignore` schließt sie aus).

## Lizenz

MIT
