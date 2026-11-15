# WhatsSnap 0.14.0 – Stand der Arbeiten

Geprüft auf Fedora 44, KDE Plasma (Wayland), PipeWire, ffmpeg 8.1. Stand: 11.11.2026.

Legende: **erledigt** · **teilweise** · **weggelassen**

## Wie getestet wurde

- `cargo build --release` ohne Warnungen, `cargo test --release`: 38 Tests grün
  (vorher 14). Neu sind u. a. Tests für Anruf-Blockgröße, Ogg/Opus gegen `ffprobe`,
  echtes Video-Dekodieren mit `ffmpeg`, Öffnen-Allowlist, Log-Kürzel, Instanz-Sperre,
  Löschen lokaler Daten, Abmelde-Merker, Kompaktierung, Einfüge-Reihenfolge.
- Sichttest per Screenshot mit einer Kopie des Verlaufs in einem getrennten Datenordner:
  Bildbetrachter und Videoplayer öffnen und zeigen Bild bzw. laufendes Video.
- Rauchtest mit der echten Sitzung: Start, „Verbunden“, keine Abstürze im Log, keine
  Nummern im neuen Log, Dateirechte 0600/0700, zweiter Start öffnet nur das Fenster,
  Schließen über X und Wiederöffnen.
- Audio-Kette lokal: `pw-record` mit 16 kHz liefert gültige Daten, `pw-play` mit 16 kHz
  spielt einen Testton ohne Fehler.

**Nicht verfügbar:** `cargo clippy`. Rust kommt hier aus den Fedora-Paketen ohne rustup;
clippy bräuchte `sudo dnf install clippy`. Es wurde also nicht ausgeführt.

**Grundsätzlich ungetestet**, weil dafür jemand klicken bzw. ein Handy bedienen muss:
alles, was unten als „ungetestet“ steht. Die Oberfläche lässt sich von hier aus nur
starten, schließen und fotografieren, nicht bedienen.

## Teil A – Fehler

### 1. Anrufe: man versteht nichts — erledigt (Ursache gefunden), am Handy ungetestet

Ursache: falsche Abtastrate. Die Bibliothek arbeitet mit 16 kHz mono und Blöcken von
960 Samples = 60 ms (`whatsapp_rust::voip::AudioSource`). WhatsSnap nahm mit 48 kHz auf
und spielte mit 48 kHz ab. Das Gegenüber hörte die eigene Stimme dreimal zu langsam,
hier kam das Gegenüber dreimal zu schnell an.

- Aufnahme und Wiedergabe laufen jetzt mit 16 kHz (`pw-record`/`pw-play`, `parec`/`pacat`,
  `arecord`/`aplay`). Format war und ist `s16` Little Endian; das war nicht der Fehler.
- Warteschlangen von 50 Blöcken (bis 3 s Verzögerung) auf 3 (Mikrofon) und 5 (Lautsprecher)
  verkürzt; was sich staut, wird verworfen.
- Gleichanteil-Filter am Mikrofon. Der Eingang dieses Rechners übersteuert direkt nach dem
  Einschalten für etwa eine Sekunde (gemessen: Vollausschlag beim ersten Öffnen, danach normal).
- Mitschnitt: `WA_SNAP_CALL_DUMP=1 wa-snap` schreibt `/tmp/wa-snap-call-mic.wav` und
  `/tmp/wa-snap-call-speaker.wav`.
- Opus, Jitter-Puffer und Paketverlust liegen in der Bibliothek, nicht in WhatsSnap; daran
  wurde nichts geändert. Echo-Unterdrückung gibt es nicht: mit Lautsprechern statt Kopfhörer
  kann das Gegenüber sich selbst hören.

Ungetestet: ein echter Anruf. Ob es jetzt klar klingt, zeigt nur der Test am Handy.

### 2. Sprachnachrichten — erledigt, am Handy ungetestet

- Kodieren/Dekodieren geprüft: 48 kHz mono, Opus 32 kbit/s, 20-ms-Pakete; die erzeugte Datei
  wird von `ffprobe` als gültiges Opus mit richtiger Dauer erkannt (Test).
- Aufnahme: Gleichanteil-Filter und 30 ms Einblenden gegen das Knacken am Anfang.
- Audiodateien, die kein Ogg/Opus sind (MP3, M4A), spielt die App jetzt über `ffmpeg` selbst
  ab statt sie an ein anderes Programm zu geben.

Ungetestet: Aufnahme mit echtem Mikrofon und Anhören am Handy.

### 3. Bilder kommen nicht an — erledigt, Empfang am Handy bestätigt

Drei Ursachen:

1. **Fehlgeschlagene Downloads wurden nie wiederholt.** Beim Start gingen alle Abrufe auf einmal
   raus, der Server antwortete mit 429 (61 Fälle im Log), und das Bild blieb bis zum Neustart
   auf „Lädt …“. Seit 0.13.2 laufen höchstens 3 Abrufe gleichzeitig. Neu: ein fehlgeschlagenes
   Bild zeigt „Laden fehlgeschlagen – klicken“ und wird nach einer Minute von selbst erneut
   versucht. Abgelaufene Medien werden beim Absender-Gerät neu angefordert.
2. **Der alte Verlauf enthält keine Medien.** Von rund 8100 Nachrichten aus zwölf Monaten hat
   keine einzige vor dem 11.11. ein Bild. Der Verlauf wurde beim Verknüpfen von einer frühen
   Version (vor 0.6) übernommen, die Medien noch verwarf. Neu: ⋮ → „Verlauf vom Handy
   nachladen“ holt die letzten rund 300 Nachrichten des Chats erneut und ergänzt Fehlendes.

3. **Medien ohne Text wurden verworfen, wenn der Typ-Vermerk des Servers fehlte.** Er fehlt bei
   Nachrichten, die das Handy nachliefert, weil die erste Zustellung nicht lesbar war. Fotos und
   Sprachnachrichten ohne Bildunterschrift fielen dadurch weg, Text kam an. Jetzt entscheidet
   der Inhalt. Nach diesem Fix kamen im Test am 11.11. ab 20:01 Fotos und Sprachnachrichten an
   (8 Einträge „Medium empfangen“ im Log, Rückmeldung: funktioniert).

Von den 49 Bildern seit dem 11.11. liegen 45 auf der Platte, 4 fehlten wegen 429.
Auto-Download war und ist eingeschaltet.

Ungetestet: „Verlauf vom Handy nachladen“ (braucht einen Klick und das Handy online), die
Neu-Anforderung abgelaufener Medien.

### 4. Bilder und Videos in der App — erledigt, Bedienung ungetestet

- Bildbetrachter als Overlay: Zoom (Mausrad um den Zeiger, Strg +/−, Tasten +/−/0,
  Doppelklick), Verschieben, vor/zurück durch die Medien des Chats (Pfeiltasten und
  Schaltflächen), Esc, Vollbild (F), „Speichern unter …“, „Extern öffnen“. Das Bild wird
  dafür in bis zu 3840 px dekodiert, im Hintergrund.
- Videoplayer: `ffmpeg` als Unterprozess, rohe RGBA-Bilder über eine Pipe, Ton über eine
  zweite ffmpeg-Instanz direkt in `pw-play`. Play/Pause, Spulen, Lautstärke, Vollbild,
  ,/. springt 5 s. GIFs laufen in Schleife. Höchstens 3 Bilder im Voraus, höchstens
  1280×720 und 30 Bilder/s.
- Warum ffmpeg als Programm statt gstreamer: keine Entwicklungspakete beim Bauen, ein
  Dekoder-Absturz reißt die App nicht mit, und für GIFs wurde es schon aufgerufen.
  Preis: Pause, Spulen und Lautstärke starten den Dekoder neu (kurze Lücke), und Bild und
  Ton laufen nur über die Uhr synchron, nicht über Zeitstempel.
- RPM: `Requires: /usr/bin/ffmpeg`, `/usr/bin/ffprobe` (passt für RPM Fusion und ffmpeg-free).
- Animierte Sticker im Chat wie bisher.

Getestet: Screenshot mit geöffnetem Bild und mit laufendem Video, ffmpeg startet und endet.
Ungetestet: jede Bedienung (Zoom, Verschieben, Blättern, Spulen, Lautstärke, Vollbild,
Speichern), Ton und Lippensynchronität, lange oder hochauflösende Videos.

## Teil B – Audit

| Nr. | Punkt | Status |
|---|---|---|
| 5 | Dateien öffnen nur per Allowlist | erledigt |
| 6 | Log ohne Nummern, Debug-Schalter, Rotation | erledigt |
| 7 | Neue Dateien sofort 0600 | erledigt |
| 8 | Benachrichtigungen escapen | erledigt |
| 9 | Abmelden und lokale Daten | erledigt (nur Code und Test) |
| 10 | cargo audit | teilweise |
| 11 | SQLCipher | weggelassen |
| 12 | Duplikate über Index, geordnetes Einfügen | erledigt |
| 13 | Ausgang für Medien | erledigt, ungetestet |
| 14 | Echo/Receipt vor „Sent“ | erledigt, ungetestet |
| 15 | Umfrage-Geheimnisse | war schon erledigt |
| 16 | flock vor dem Socket | erledigt |
| 17 | Start-`expect`s, verschluckte Fehler | teilweise |
| 18 | Suche | teilweise |
| 19 | dir_size, status_groups | erledigt |
| 20 | Zwischenablage im Hintergrund | erledigt, ungetestet |
| 21 | PCM freigeben | erledigt |
| 22 | Verlauf kompaktieren | teilweise |
| 23 | main.rs aufteilen | teilweise |

**5.** `src/safety.rs`: ohne Rückfrage öffnen nur Bilder, Video, Audio, PDF, Office, Text.
Zusätzlich wird der Inhalt geprüft (Crate `infer` plus `#!`, ELF, MZ): ein Skript namens
`rechnung.pdf` oder ein Archiv namens `urlaub.png` fällt durch. Sonst Dialog mit „Im Ordner
anzeigen“, „Abbrechen“, „Trotzdem öffnen“. Tests vorhanden; der Dialog selbst ist ungetestet.
SVG steht bewusst nicht auf der Liste (kann Skripte enthalten).

**6.** Ziffernfolgen ab 7 Stellen werden im Log zu `#abcd…78` (Prüfwert plus letzte zwei
Ziffern). Zeilen pro Nachricht nur mit „Ausführliches Log“ (Einstellungen → WhatsSnap,
Standard aus). Rotation alle 200 Zeilen geprüft, ab 512 KB nach `wa-snap.log.old`.
„Lokale Daten löschen“ entfernt jetzt auch Log und `tmp/`. Alte Logzeilen von vor dem
Update enthalten weiterhin volle Nummern, bis die Datei rotiert.

**7.** `umask 077` beim Start: alles Neue ist 0600 bzw. 0700, auch Datenbank, WAL, Medien.
Mit `xdg-open` gestartete Programme bekommen wieder die normale Maske.

**8.** `<`, `>`, `&` im Benachrichtigungstext werden ersetzt.

**9.** Die Bibliothek leert den Store beim Abmelden nicht (steht so in ihrem Quelltext).
Neu: nach dem Abmelden wird vorgemerkt, die Sitzungsdatenbank beim nächsten Start zu löschen
(sofort geht nicht, sie ist noch geöffnet), und ein Dialog fragt, ob auch Verlauf und Medien
weg sollen. Wie verlangt nicht echt getestet, nur per Unit-Test.

**10.** `cargo audit` meldet 4 Schwachstellen und 11 Hinweise auf nicht mehr gepflegte
Crates. `cargo update` brachte nur `webbrowser` 1.2.7. Offen, weil nur über größere Sprünge
fremder Crates behebbar:

- `libsqlite3-sys` 0.37.0 (RUSTSEC-2026-0459, mittel): kommt über diesel aus
  `whatsapp-rust-sqlite-storage`; Fix ab 0.39.
- `crypto-bigint` 0.5.5 (RUSTSEC-2026-0387, Zeitverhalten): kommt über `webrtc-dtls` aus dem
  Anruf-Teil der Bibliothek; Fix ab 0.7.6.
- `quick-xml` 0.30.0 (RUSTSEC-2026-0194/-0195, hoch, Denial of Service): kommt über die
  Barrierefreiheits-Anbindung von egui (`accesskit` → `zbus_xml`). Verarbeitet wird damit nur
  XML vom lokalen D-Bus, nichts aus Nachrichten. Abschalten ginge nur mit dem Verzicht auf
  Screenreader-Unterstützung; das wurde nicht gemacht.

Alle drei brauchen eine neuere `whatsapp-rust`- bzw. egui-Version.

**11.** Weggelassen. `whatsapp-rust-sqlite-storage` öffnet die Datenbank selbst
(`SqliteStore::new(pfad)`) und bietet keine Stelle, an der vor dem ersten Zugriff ein
Schlüssel gesetzt werden könnte. Dazu müsste das Storage-Crate geändert oder ersetzt werden,
und eine Migration der bestehenden Sitzung ließe sich hier nicht testen, ohne die echte
Sitzung zu riskieren. Ein Fehler dabei hieße: neu verknüpfen.

**12.** Duplikatprüfung über `msg_index` (ganzer Verlauf, vorher nur die letzten 400),
Einfügen per `partition_point`. Test für die Reihenfolge vorhanden.

**13.** Fotos, Videos, Dateien und Sprachnachrichten stehen bis zur Bestätigung im Verlauf
(`outfile`), erscheinen nach einem Neustart als „nicht gesendet“ und lassen sich erneut
senden oder verwerfen. Einschränkungen: ein erneut gesendetes GIF geht als normales Video
raus; Sticker, weitergeleitete Nachrichten und Giphy-GIFs sind nicht erfasst.
Komplett ungetestet (braucht Netzabbruch und Klicks).

**14.** Zustellbestätigungen für eine noch unbekannte ID werden gemerkt (bis 500) und nach
„Sent“ angewendet. Kommt das eigene Echo vor „Sent“, wird der vorläufige Eintrag entfernt
statt eine zweite Nachricht zu erzeugen. Ungetestet (lässt sich nicht gezielt auslösen).

**15.** Die Geheimnisse stehen schon im Verlauf (Teil der Umfrage-Nachricht) und werden beim
Start dem Backend gemeldet. Nichts zu tun.

**16.** `flock` auf `$XDG_RUNTIME_DIR/wa-snap.lock` vor dem Socket; Test vorhanden.

**17.** Die drei `expect` beim Start sind durch Meldungen ersetzt. Verschluckte IO-Fehler
werden an den neu angefassten Stellen geloggt (Ausgang, Medien löschen, Abmelde-Merker).
Im restlichen Code gibt es weiterhin viele `let _ = …` bei Dateizugriffen.

**18.** Suche über alle Chats: 180 ms Tipp-Pause, Ergebnis wird zwischengespeichert, höchstens
300 Treffer. Suche im Chat: Trefferliste wird zwischengespeichert. **Nicht** umgesetzt: ein
eigener Hintergrund-Thread. Gerechnet wird weiter im Fenster-Thread, aber nur noch einmal
pro Suchtext statt in jedem Bild.

**19.** Ordnergröße höchstens alle 5 s, Status-Gruppen nur bei neuem Status oder nach einer
Minute.

**20.** PNG-Kodieren und Vorschau laufen in einem eigenen Thread.

**21.** Dekodierte Sprachnachrichten: Budget 48 MB, die ältesten fliegen raus, die laufende
bleibt.

**22.** Zusätzlich zu Reaktionen werden überholte Zustellstatus-Zeilen und Status-Beiträge
älter als 24 h entfernt (atomar, ab 500 überholten Zeilen, Test vorhanden).
**Bearbeitungen bleiben absichtlich**: aus ihnen entstehen die „früheren Fassungen“.

**23.** Neuer Code liegt in eigenen Dateien: `viewer.rs`, `video.rs`, `safety.rs`,
`dialogs.rs`. `main.rs` selbst ist nicht aufgeteilt und hat weiter über 6000 Zeilen.
Ein Umbau dieser Größe ohne Möglichkeit, die Oberfläche durchzuklicken, wäre ein Risiko
für alles, was jetzt läuft.

## Außerdem geändert

- Abstürze stehen im Log (`ABSTURZ …`), ebenso jedes Schließen und Beenden.
- `Getrennt: …` bei Verbindungsabbruch.

## Bekannte Restprobleme

- Installiert ist nach `~/.local/bin/wa-snap` (ohne sudo). `/usr/bin/wa-snap` ist weiter
  0.13.1 und wird verdeckt. Sauber: `sudo dnf install ./dist/wa-snap-0.14.0-1.x86_64.rpm`,
  danach `rm ~/.local/bin/wa-snap`.
- „Archiviert“ zeigt seit der Zusammenführung von Nummer und LID 31 statt 21 Chats;
  ob das stimmt, ist nicht mit dem Handy abgeglichen.
- Speicherverbrauch bei langem Scrollen durch bildreiche Chats ist nicht gemessen
  (direkt nach dem Start rund 145 MB).
- Videoplayer: keine Untertitel, keine Wahl der Tonspur, kein Bild-für-Bild.
- Keine Echo-Unterdrückung bei Anrufen.
- Nichts ist gepusht.

## Tests am Handy

1. **Anruf:** Vom Handy einer zweiten Person anrufen lassen und annehmen, dann selbst anrufen.
   Beide Seiten: klingt es normal schnell und verständlich? Verzögerung? Mit Kopfhörer testen.
   Wenn nicht: `WA_SNAP_CALL_DUMP=1 wa-snap` starten, anrufen, die beiden WAV-Dateien aus `/tmp`
   anhören. Klingt `…-mic.wav` gut, liegt der Fehler hinter dem Mikrofon.
2. **Sprachnachricht hin:** In WhatsSnap aufnehmen und senden, am Handy anhören (Knacken am
   Anfang? Dauer richtig? Wellenform da?).
3. **Sprachnachricht zurück:** Vom Handy eine schicken, in WhatsSnap abspielen, mitten
   hineinklicken zum Spulen.
4. **Bild empfangen:** Vom Handy ein Foto schicken. Erscheint es von selbst? Anklicken:
   Betrachter mit Zoom, Verschieben, Pfeiltasten, „Speichern unter …“.
5. **Video empfangen:** Kurzes Video mit Ton schicken, anklicken. Läuft Bild und Ton,
   passen sie zusammen? Pause, Spulen, Lautstärke, F für Vollbild.
6. **Alte Bilder:** In einem Chat mit früheren Fotos ⋮ → „Verlauf vom Handy nachladen“
   (Handy entsperrt und online). Tauchen nach einigen Sekunden alte Bilder auf?
7. **Ausgang:** WLAN aus, Foto senden (rotes Ausrufezeichen), WhatsSnap beenden und neu
   starten, WLAN an, Rechtsklick → „Erneut senden“.
8. **Riskante Datei:** Sich selbst eine `.sh`- oder `.html`-Datei schicken und anklicken:
   es muss der Warn-Dialog kommen.
