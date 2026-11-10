# wa-snap

**Schneller, nativer WhatsApp-Desktop-Client in Rust.**
Kein Browser, kein Electron: Die Oberfläche ist [egui](https://github.com/emilk/egui), das WhatsApp-Protokoll kommt von [`whatsapp-rust`](https://github.com/jlucaso1/whatsapp-rust). Dadurch startet das Programm schnell und braucht deutlich weniger Speicher als die Web-Versionen zum Herunterladen.

> ⚠️ **Inoffizielles Projekt.** WhatsApp bietet für private Konten keine offizielle API. Inoffizielle Clients können gegen die Nutzungsbedingungen von Meta verstoßen und zur Sperrung des Kontos führen. Benutzung auf eigene Gefahr, am besten zuerst mit einer Zweitnummer. Dieses Projekt steht in keiner Verbindung zu WhatsApp oder Meta.

## Funktionen (v0.1)

- Verknüpfung per QR-Code, die Sitzung bleibt gespeichert
- Chatliste mit Suche, Ungelesen-Zähler und Sortierung nach letzter Nachricht
- Nachrichten empfangen und Textnachrichten senden (Enter)
- Medien erscheinen als Platzhalter, z. B. `[image]`
- Dunkles Design, läuft unter Linux, Windows und macOS (getestet nur unter Linux)

**Noch nicht vorhanden:** Nachrichtenverlauf nach Neustart, Namen aus dem Adressbuch und Gruppennamen, Bilder/Dokumente/Sprachnachrichten, Anrufe, Zeitzone (Uhrzeiten sind UTC). Der Login mit einem echten Konto ist bisher nicht ausführlich getestet. Fehler bitte als Issue melden.

## Download und Installation

Du brauchst [Rust](https://rustup.rs) (stabile Version) und einen C-Compiler (unter Fedora: `sudo dnf install gcc`, unter Debian/Ubuntu: `sudo apt install build-essential`).

**Variante 1: mit einem Befehl installieren**

```bash
cargo install --git https://github.com/Xinzoooo/wa-snap
wa-snap
```

**Variante 2: herunterladen und starten**

```bash
git clone https://github.com/Xinzoooo/wa-snap.git
cd wa-snap
cargo run --release
```

Der erste Build dauert einige Minuten, danach startet das Programm sofort. Beim ersten Start erscheint ein QR-Code. Scanne ihn am Handy unter **WhatsApp → Verknüpfte Geräte → Gerät hinzufügen**.

## Daten

Die Sitzung liegt in `~/.local/share/wa-snap/whatsapp.db` (Linux). Die Datei enthält deine Schlüssel: nicht weitergeben und nie in ein Repo hochladen. Zum Abmelden löschst du die Datei und entfernst das Gerät in der WhatsApp-App.

## Lizenz

MIT, siehe [LICENSE](LICENSE).
