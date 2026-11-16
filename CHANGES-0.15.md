# WhatsSnap 0.15.0 – Stand der Arbeiten

Geprüft auf Fedora 44, KDE Plasma (Wayland), ffmpeg 8.1.3, whatsapp-rust 0.7.0.
Stand: 16.11.2026.

Legende: **erledigt** · **teilweise** · **weggelassen**

## Wie getestet wurde

- `cargo build` und `cargo build --release` ohne Warnungen, `cargo test --release`:
  40 Tests grün (vorher 38). Neu: Drossel für Media-Neu-Anforderungen,
  ffmpeg/ffprobe gegen präparierte Playlists und concat-Listen.
- Handtests mit echter Sitzung und Handy (vom Nutzer geklickt, Log ausgewertet):
  Foto im Betrachter, Video mit Ton, Verlauf vom Handy nachladen.
- **Nicht verfügbar:** `cargo clippy` (nicht installiert, `sudo dnf install clippy`).

## 0. Repository

`git pull` ging nicht direkt: lokal lagen die Einzel-Commits von 0.14.0, auf GitHub
dieselben Änderungen zusammengefasst plus 0.14.1. Die Bäume unterschieden sich nur um
die 0.14.1-Änderungen; zusammengeführt per Merge-Commit mit dem GitHub-Stand
(kein Reset, nichts verworfen). Die alte `dist/wa-snap-0.14.0-1.x86_64.rpm` liegt danach
ungetrackt in `dist/` und kann von Hand gelöscht werden.

## 1. Abmeldung am 15.11. um 23:54:38 — teilweise (Ursache eingegrenzt, nicht belegt)

Befund aus Log und Quelltext von whatsapp-rust 0.7.0:

- WhatsSnap war vom 11.11. 20:01 bis 15.11. 23:54 aus. 25 s nach dem Start, mitten im
  Nachholen der Offline-Nachrichten (≈ 260 Medien, „Verbunden“ war noch nicht gemeldet),
  kam `LoggedOut`. Die Starts um 00:06 und 00:20 wurden sofort abgewiesen: das Gerät
  war da schon vom Konto entfernt.
- `LoggedOut` entsteht in der Bibliothek nur durch (a) `<stream:error>` mit Code 516
  (Gerät entfernt), 401 oder `<conflict>` außer „replaced“, (b) `<failure>` beim
  Verbinden mit Abmeldegrund, (c) einen eigenen Aufruf von `logout()`. (c) war es
  nicht (das hätte „Abgemeldet (auf Wunsch)“ geloggt). Es war also der Server bzw. das Handy.
- In diesen 25 s hat WhatsSnap über die Verbindung nur wenig geschickt: zwei
  Retry-Receipts für nicht lesbare Nachrichten (macht die Bibliothek selbst), keine
  Media-Neu-Anforderung. Medien laden per HTTP vom CDN, nicht über die Verbindung.
  Ein Auslöser in WhatsSnap ließ sich nicht finden.
- Den genauen Grund kann niemand mehr nachlesen: der Handler verwarf ihn, und
  Warnungen der Bibliothek gingen nirgends hin.

Geändert:

- Abmeldung: Grund, ob beim Verbinden, Tag der Stanza, `code`, `reason`, `conflict`-Typ
  und Unterknoten stehen im Log.
- Warnungen und Fehler von whatsapp-rust/wacore (`log`-Crate) landen anonymisiert im
  Log (höchstens 200 Zeilen pro Sitzung, je 400 Zeichen).
- Media-Neu-Anforderungen (jede ist eine verschlüsselte Nachricht an ein Handy):
  je Medium einmal pro Sitzung, höchstens 40 pro Sitzung, nur eine gleichzeitig,
  3 s Abstand. Vorher wurde dasselbe Medium mehrfach angefragt (im Log 00:22 und 00:24).
- Neu-Anforderung bei zusammengeführten Einzelchats: die Chat-Angabe ist jetzt die LID,
  nicht die Nummer. Grund: Alle 13 Neu-Anforderungen im Log liefen ins Zeitlimit
  (30 s). Zusammengeführte Chats tragen die Nummern-JID, das Handy führt die Nachricht
  aber meist unter der LID. **Ungetestet** – ob das die Zeitlimits behebt, zeigt erst
  das nächste abgelaufene Medium. Für Gruppen ändert sich nichts.

Getestet: Build, Tests (Drossel-Test). Mit echter Sitzung: Neustart verbunden, die
Neu-Anforderungen kommen im Log jetzt nacheinander im Abstand von 33 s
(30 s Zeitlimit + 3 s) statt gebündelt. Die neue Abmelde-Zeile ist **ungetestet**
(es gab seitdem keine Abmeldung).

## 2. ffmpeg absichern — erledigt

Vor jedem `-i` von ffmpeg/ffprobe (Videoplayer, Ton des Videos, Prüfen, fremde
Audioformate, GIF → MP4, Video-Vorschaubild): `-protocol_whitelist file,pipe` und
`-format_whitelist mov,matroska,ogg,mp3,wav,flac,aac,amr,gif,avi,mpegts,mpeg,asf,flv`.

Warum beides: Mit nur `-protocol_whitelist file,pipe` öffnete eine concat-Liste mit
relativem Dateinamen trotzdem eine andere lokale Datei (über `file`). Erst die
Formatliste lässt HLS-, concat- und Bildfolgen-Demuxer gar nicht zu.

Getestet (`video::tests::rejects_playlists_and_concat`): HLS-Playlist mit `http://`
als `.mp4` und als `.m3u8`, concat-Liste mit relativem und mit absolutem Pfad – alle vier
werden von `probe()` und `decode_with_ffmpeg()` abgelehnt. Gegenprobe: ohne die Sperre
schlägt der Test fehl. Ein echtes MP4 und die vorhandenen empfangenen mp4/ogg-Dateien
werden weiter erkannt. Hinweis: ffmpeg 8.1 erkennt eine als `.mp4` getarnte HLS-Datei
ohnehin nicht mehr; ältere ffmpeg-Versionen schon.

Restrisiko: Seltene Container außerhalb der Liste (z. B. `.caf`, `.wv`) spielt
WhatsSnap nicht mehr ab.

## 3. Tests mit dem Handy — teilweise

| Test | Ergebnis |
|---|---|
| a) Foto im Betrachter: Zoom, Verschieben, Pfeiltasten, Speichern unter | geht (lief noch mit 0.14.1; am Betrachter wurde nichts geändert) |
| b) Video mit Ton: Abspielen, Pause, Spulen, Lautstärke, F = Vollbild | geht (neuer Build) |
| c) Verlauf vom Handy nachladen | geht; Medien, die das Handy nicht mehr hat, bleiben unscharf (403/410, Zeitlimit) |
| d) Ausgang bei Netz aus, Neustart, „Erneut senden“ | **weggelassen**, auf Wunsch abgebrochen – ungetestet |
| e) Warn-Dialog bei .sh und .html | nicht am Handy getestet; abgedeckt durch die Unit-Tests `dangerous_extensions_need_confirmation` und `disguised_content_is_caught` (.sh, .html, getarntes Skript als .pdf) |

## 4. Status und Kanäle trennen — weggelassen

Sollte nur nach Rückfrage gemacht werden; die Rückfrage hat nicht stattgefunden.

## 5. Code aufräumen — weggelassen

- clippy: nicht installiert.
- Aufteilen von `src/main.rs` (> 6000 Zeilen): auf später verschoben, weil danach ein
  Rauchtest mit dem Handy nötig ist und die Tests abgebrochen wurden. Die Aufteilung
  ändert für Nutzer nichts.

## Bekannte Restprobleme

- Ursache der Abmeldung vom 15.11. nicht belegt; beim nächsten Mal steht sie im Log.
- Media-Neu-Anforderungen liefen bisher alle ins Zeitlimit; Fix für Einzelchats ungetestet,
  für Gruppen keine Änderung.
- `Kanäle: IQ request failed` einmal beim Start (00:21:18); danach nicht wieder. Nicht untersucht.
- Ausgang bei Netzausfall (3d) in 0.15 ungetestet.
