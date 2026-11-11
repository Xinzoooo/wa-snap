# wa-snap

**Schneller, nativer WhatsApp-Desktop-Client in Rust.**
Kein Browser, kein Electron: Die Oberfläche ist [egui](https://github.com/emilk/egui), das WhatsApp-Protokoll kommt von [`whatsapp-rust`](https://github.com/jlucaso1/whatsapp-rust). Dadurch startet das Programm schnell und braucht deutlich weniger Speicher als die Web-Versionen zum Herunterladen.

> ⚠️ **Inoffizielles Projekt.** WhatsApp bietet für private Konten keine offizielle API. Inoffizielle Clients können gegen die Nutzungsbedingungen von Meta verstoßen und zur Sperrung des Kontos führen. Benutzung auf eigene Gefahr, am besten zuerst mit einer Zweitnummer. Dieses Projekt steht in keiner Verbindung zu WhatsApp oder Meta.

## Funktionen (v0.5)

- Aussehen wie WhatsApp Web: Seitenleiste, Chatliste mit Filtern (Alle, Ungelesen, Gruppen), „Archiviert“ oben, Sprechblasen mit Zipfel, Haken, Schrift Inter; hell und dunkel
- Bunte Emojis (Twemoji) in Nachrichten, Chatliste und Emoji-Auswahl mit Kategorien; reine Emoji-Nachrichten werden groß angezeigt
- Haken wie bei WhatsApp: gesendet, zugestellt, gelesen (blau)
- Beim Öffnen eines Chats werden Lesebestätigungen gesendet, der Ungelesen-Zähler am Handy geht mit
- Stumm, Anheften und Archiv über Rechtsklick auf einen Chat oder das Menü im Chat-Kopf, abgeglichen mit WhatsApp (Nummer und LID werden beide zugeordnet)
- Benachrichtigungen kommen auch, wenn das Fenster minimiert ist; Test-Knopf in den Einstellungen
- Suche in der Chatliste und im geöffneten Chat
- Verknüpfung per QR-Code, die Sitzung bleibt gespeichert; beim Verknüpfen wird der Verlauf der letzten 365 Tage angefordert
- Verlauf wird lokal gespeichert und ist nach dem Neustart wieder da
- Namen aus dem Adressbuch, Gruppennamen direkt vom Server, Profilbilder mit Zwischenspeicher
- Mehrzeilig schreiben (Shift+Enter), senden mit Enter
- WhatsApp-Profil (Name, Info) in den Einstellungen änderbar; Autostart, Schriftgröße
- Neuer Chat per Telefonnummer (Nummer in die Suche tippen)
- Medien erscheinen als Hinweis, z. B. „📷 Foto“
- Diagnose-Log: `~/.local/share/wa-snap/wa-snap.log`

**Noch nicht vorhanden:** Bilder/Dokumente/Sprachnachrichten anzeigen und senden, GIFs und Sticker, Antworten/Reaktionen, Datenschutz-Einstellungen, Gruppen anlegen, Anrufe. Fehler bitte als Issue melden.

## Installation auf Fedora (empfohlen)

Ein Befehl, danach findest du wa-snap in der Anwendungssuche und kannst es an die Taskleiste anheften:

```bash
sudo dnf install https://raw.githubusercontent.com/Xinzoooo/wa-snap/main/dist/wa-snap-0.5.0-1.x86_64.rpm
```

Deinstallieren: `sudo dnf remove wa-snap`. Deine Anmeldedaten bleiben in `~/.local/share/wa-snap/` und werden dabei nicht gelöscht.

## Aus dem Quellcode bauen

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

## Im Anwendungsmenü und in der Taskleiste (Linux)

```bash
wa-snap --install-desktop
```

Das legt einen Startmenü-Eintrag mit Icon an. Danach findest du wa-snap im Anwendungsmenü und kannst es per Rechtsklick an die Taskleiste anheften.

## Daten

Die Sitzung liegt in `~/.local/share/wa-snap/whatsapp.db` (Linux). Die Datei enthält deine Schlüssel: nicht weitergeben und nie in ein Repo hochladen. Zum Abmelden löschst du die Datei und entfernst das Gerät in der WhatsApp-App.

## Lizenz

MIT, siehe [LICENSE](LICENSE).

Mitgeliefert werden die Schrift [Inter](https://github.com/rsms/inter) (SIL Open Font License 1.1, [assets/fonts/OFL.txt](assets/fonts/OFL.txt)) und die Emoji-Grafiken von [Twemoji](https://github.com/jdecked/twemoji) (© Twitter, Inc. und weitere, CC-BY 4.0, [assets/TWEMOJI-LICENSE.txt](assets/TWEMOJI-LICENSE.txt)). `tools/pack_emoji.py` erzeugt daraus `assets/emoji.bin`.
