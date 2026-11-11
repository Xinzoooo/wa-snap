# WhatsSnap

**Schneller, nativer WhatsApp-Desktop-Client in Rust.**
Kein Browser, kein Electron: Die Oberfläche ist [egui](https://github.com/emilk/egui), das WhatsApp-Protokoll kommt von [`whatsapp-rust`](https://github.com/jlucaso1/whatsapp-rust). Dadurch startet das Programm schnell und braucht deutlich weniger Speicher als die Web-Versionen zum Herunterladen.

> ⚠️ **Inoffizielles Projekt.** WhatsApp bietet für private Konten keine offizielle API. Inoffizielle Clients können gegen die Nutzungsbedingungen von Meta verstoßen und zur Sperrung des Kontos führen. Benutzung auf eigene Gefahr, am besten zuerst mit einer Zweitnummer. Dieses Projekt steht in keiner Verbindung zu WhatsApp oder Meta.

## Funktionen (v0.10)

- Aussehen wie WhatsApp Web: Seitenleiste, Chatliste mit Filtern (Alle, Ungelesen, Gruppen), „Archiviert“ oben, Sprechblasen mit Zipfel, Haken, Schrift Inter; hell und dunkel
- 3D-Emojis im Stil von WhatsApp (Microsoft Fluent Emoji, fehlende aus Twemoji) in Nachrichten, Chatliste und Auswahl; alle 1900+ Emojis mit Kategorien wie bei WhatsApp, „Letzte“, deutscher Suche und Hauttönen (Rechtsklick); reine Emoji-Nachrichten werden groß angezeigt
- Haken wie bei WhatsApp: gesendet, zugestellt, gelesen (blau)
- Beim Öffnen eines Chats werden Lesebestätigungen gesendet, der Ungelesen-Zähler am Handy geht mit
- Stumm, Anheften und Archiv über Rechtsklick auf einen Chat oder das Menü im Chat-Kopf, abgeglichen mit WhatsApp (Nummer und LID werden beide zugeordnet)
- Benachrichtigungen kommen auch, wenn das Fenster minimiert ist; Test-Knopf in den Einstellungen
- Suche in der Chatliste und im geöffneten Chat
- Symbol in der Leiste (Tray): Fenster schließen lässt WhatsSnap weiterlaufen, roter Punkt bei Ungelesenem, Rechtsklick → Beenden (abschaltbar in den Einstellungen); ein zweiter Start öffnet nur das vorhandene Fenster; Autostart startet unsichtbar in der Leiste
- Formatierung wie bei WhatsApp: `*fett*`, `_kursiv_`, `~durchgestrichen~`, `` `Code` ``
- Bilder mit Strg+V aus der Zwischenablage senden (mit Vorschau)
- Entwürfe bleiben pro Chat erhalten, „Entwurf:“ in der Chatliste
- „schreibt …“, „nimmt Sprachnachricht auf …“, „online“ und „zuletzt online“; du erscheinst online, solange das Fenster aktiv ist
- Knopf „Nach unten“ mit Zähler neuer Nachrichten; Klick auf ein Zitat springt zur Originalnachricht
- Weiterleiten (auch Fotos, Sticker, Dokumente ohne neuen Upload)
- Stummschalten für 8 Stunden, 1 Woche oder immer
- Eigene Nachrichten bearbeiten (bis 15 Minuten), „Bearbeitet“-Hinweis
- @Erwähnungen in Gruppen mit Vorschlagsliste; Erwähnungen werden mit Namen hervorgehoben
- Nachrichten mit Stern markieren (abgeglichen mit WhatsApp), Übersicht über ☆ in der linken Leiste
- Umfragen anzeigen und abstimmen, Standort (Öffnen in OpenStreetMap), Kontaktkarten mit „Chat starten“, Link-Vorschau
- Einmal-Ansicht und selbstlöschende Nachrichten werden gekennzeichnet; in Chats mit Selbstlöschen gilt das auch für eigene Nachrichten
- Suche über alle Chats: passende Nachrichten erscheinen unter der Suche, Klick springt hin
- Ältere Nachrichten: Knopf oben im Chat zeigt mehr an bzw. lädt weitere vom Handy; beim Verknüpfen werden bis zu 300 Nachrichten pro Chat übernommen
- Tastenkürzel: Strg+F Suche, Strg+N neuer Chat, Esc schließt, Alt+↑/↓ wechselt den Chat
- Verknüpfung per QR-Code, die Sitzung bleibt gespeichert; beim Verknüpfen wird der Verlauf der letzten 365 Tage angefordert
- Verlauf wird lokal gespeichert und ist nach dem Neustart wieder da
- Namen aus dem Adressbuch, Gruppennamen direkt vom Server, Profilbilder mit Zwischenspeicher
- Mehrzeilig schreiben (Shift+Enter), senden mit Enter
- Einstellungen aufgebaut wie bei WhatsApp: Profil (Name, Info, Profilbild), Konto (Abmelden, lokale Daten löschen), Datenschutz (zuletzt online, online, Profilbild, Info, Gruppen, Anrufe von Unbekannten, Lesebestätigungen, Standard für selbstlöschende Nachrichten, Blockierte), Chats (Design, Schrift, Enter sendet), Benachrichtigungen (Gruppen, Ton), Speicher (automatischer Download, Medien löschen), WhatsSnap (Tray, Autostart, Online-Status, „schreibt …“, GIF-Schlüssel, Tastenkürzel), Hilfe
- Kontakt- und Gruppeninfo (Klick auf den Chat-Kopf): Medien, Beschreibung, Mitglieder, gemeinsame Gruppen, Blockieren
- Gruppen: neue Gruppe anlegen (+ → Neue Gruppe), umbenennen, Mitglieder hinzufügen/entfernen, Admins ernennen, Einladungslink, verlassen; selbstlöschende Nachrichten je Chat (⋮-Menü)
- Neuer Chat per Telefonnummer (Nummer in die Suche tippen)
- Fotos und Sticker werden im Chat angezeigt (Klick öffnet in voller Größe), animierte Sticker bewegen sich; Videos, GIFs und Dokumente laden per Klick und öffnen im Standardprogramm
- Sprachnachrichten direkt in der App abspielen (Wellenform, Pause, Klick zum Spulen) und aufnehmen (Mikrofon-Knopf, wenn das Eingabefeld leer ist); Opus ist eingebaut, abgespielt und aufgenommen wird über PipeWire (`pw-play`/`pw-record`), ersatzweise PulseAudio oder ALSA
- Videos senden; GIF-Dateien werden mit ffmpeg (falls installiert) zu WhatsApp-GIFs umgewandelt, sonst als Dokument gesendet
- GIF-Suche über Giphy (eigener kostenloser API-Schlüssel nötig, Eintrag in den Einstellungen oder direkt im GIF-Reiter)
- Eigene Sticker aus beliebigen Bildern (📎 → Sticker aus Bild, oder im Sticker-Reiter)
- Fotos und Dateien senden: 📎 oder Datei ins Fenster ziehen; Text im Eingabefeld wird zur Bildunterschrift
- Sticker: empfangene Sticker erscheinen in der Emoji-Auswahl unter „Sticker“ und lassen sich weiterschicken
- Rechtsklick auf eine Nachricht: Reagieren, Antworten, Text kopieren, für alle löschen; Reaktionen, Antworten (Zitate), gelöschte und bearbeitete Nachrichten von anderen werden angezeigt
- Diagnose-Log: `~/.local/share/wa-snap/wa-snap.log`

**Noch nicht vorhanden:** Videos direkt in der App abspielen, Status/Storys, Kanäle, Anrufe. Fehler bitte als Issue melden.

## Installation auf Fedora (empfohlen)

Ein Befehl, danach findest du WhatsSnap in der Anwendungssuche und kannst es an die Taskleiste anheften:

```bash
sudo dnf install https://raw.githubusercontent.com/Xinzoooo/wa-snap/main/dist/wa-snap-0.10.0-1.x86_64.rpm
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

Das legt einen Startmenü-Eintrag mit Icon an. Danach findest du WhatsSnap im Anwendungsmenü und kannst es per Rechtsklick an die Taskleiste anheften.

## Daten

Die Sitzung liegt in `~/.local/share/wa-snap/whatsapp.db` (Linux), heruntergeladene Medien in `~/.local/share/wa-snap/media/`. Die Datei enthält deine Schlüssel: nicht weitergeben und nie in ein Repo hochladen. Zum Abmelden löschst du die Datei und entfernst das Gerät in der WhatsApp-App.

## Lizenz

MIT, siehe [LICENSE](LICENSE).

Eingebaut sind außerdem libopus (BSD) und libwebp (BSD). Mitgeliefert werden die Schrift [Inter](https://github.com/rsms/inter) (SIL Open Font License 1.1, [assets/fonts/OFL.txt](assets/fonts/OFL.txt)) die Emoji-Grafiken von [Fluent Emoji](https://github.com/microsoft/fluentui-emoji) (© Microsoft, MIT, [assets/FLUENT-EMOJI-LICENSE.txt](assets/FLUENT-EMOJI-LICENSE.txt)) und [Twemoji](https://github.com/jdecked/twemoji) (© Twitter, Inc. und weitere, CC-BY 4.0, [assets/TWEMOJI-LICENSE.txt](assets/TWEMOJI-LICENSE.txt)). Reihenfolge, Kategorien und deutsche Suchbegriffe stammen aus [Emojibase](https://github.com/milesj/emojibase) (MIT). `tools/pack_emoji.py` erzeugt daraus `assets/emoji.bin` und `assets/emoji_list.json`.
