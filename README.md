# wa-snap

**Schneller, nativer WhatsApp-Desktop-Client in Rust.**
Kein Browser, kein Electron: Die Oberfläche ist [egui](https://github.com/emilk/egui), das WhatsApp-Protokoll kommt von [`whatsapp-rust`](https://github.com/jlucaso1/whatsapp-rust). Dadurch startet das Programm schnell und braucht deutlich weniger Speicher als die Web-Versionen zum Herunterladen.

> ⚠️ **Inoffizielles Projekt.** WhatsApp bietet für private Konten keine offizielle API. Inoffizielle Clients können gegen die Nutzungsbedingungen von Meta verstoßen und zur Sperrung des Kontos führen. Benutzung auf eigene Gefahr, am besten zuerst mit einer Zweitnummer. Dieses Projekt steht in keiner Verbindung zu WhatsApp oder Meta.

## Funktionen (v0.4)

- Verknüpfung per QR-Code, die Sitzung bleibt gespeichert
- Modernes, ruhiges Design, hell und dunkel umschaltbar; Chatliste mit Profilbildern, Suche, Ungelesen-Zähler, Datumstrenner
- Verlauf wird lokal gespeichert und ist nach dem Neustart wieder da
- Mehrzeilig schreiben (Shift+Enter) und Emoji-Auswahl
- Namen aus dem Adressbuch (sobald WhatsApp sie synchronisiert hat), sonst Push-Namen
- Interne Sync-Nachrichten werden ausgeblendet, Uhrzeit in deiner Zeitzone
- Nachrichten empfangen und Textnachrichten senden (Enter)
- Bestehende Chats, Gruppennamen und der letzte Verlauf werden nach der Verknüpfung aus dem Verlauf-Sync übernommen
- Desktop-Benachrichtigungen (an/aus, mit oder ohne Textvorschau; Fehler landen im Log, Ersatzweg über `notify-send`)
- Stumm, Anheften und Archiv werden mit WhatsApp abgeglichen (Änderungen vom Handy kommen an, Änderungen hier gehen zurück); Profilname und Info-Text lassen sich bei WhatsApp ändern
- Gruppenliste und Gruppennamen werden beim Start direkt vom Server geholt
- Beim Verknüpfen wird der Verlauf der letzten 365 Tage angefordert
- Diagnose-Log: `~/.local/share/wa-snap/wa-snap.log`
- Einstellungen: WhatsApp-Profil, Benachrichtigungen, Autostart (Linux), Schriftgröße
- Neuer Chat per Telefonnummer (Nummer in die Suche tippen)
- Medien erscheinen als Platzhalter, z. B. `[image]`; Profilbilder werden geladen und zwischengespeichert
- Dunkles Design, läuft unter Linux, Windows und macOS (getestet nur unter Linux)

**Noch nicht vorhanden:** Bilder/Dokumente/Sprachnachrichten, bunte Emojis, GIFs und Sticker (die Oberfläche zeichnet Emojis einfarbig), Anrufe, Antworten/Reaktionen, Haken für gelesen, Gruppen anlegen. Der Login mit einem echten Konto ist bisher nicht ausführlich getestet. Fehler bitte als Issue melden.

## Installation auf Fedora (empfohlen)

Ein Befehl, danach findest du wa-snap in der Anwendungssuche und kannst es an die Taskleiste anheften:

```bash
sudo dnf install https://raw.githubusercontent.com/Xinzoooo/wa-snap/main/dist/wa-snap-0.4.0-1.x86_64.rpm
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
