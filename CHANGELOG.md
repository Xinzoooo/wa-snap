# Changelog

## 0.15.1
- Intern: `main.rs` in Module aufgeteilt, clippy-Korrekturen

## 0.15.0
- Sicherheit: Videos und Töne werden mit ffmpeg nur noch als lokale Datei und nur als echter Container geöffnet (keine Playlists, keine Netzwerkadressen)
- Diagnose: Bei einer Abmeldung durch WhatsApp stehen Grund und Server-Antwort im Log, ebenso Warnungen der WhatsApp-Bibliothek
- Fehlende Medien werden gedrosselt neu angefordert (nicht mehr mehrfach dasselbe Medium)
- Neu-Anforderung von Medien in zusammengeführten Einzelchats nutzt die richtige Kennung (LID)

## 0.14.1
- Nach einer Abmeldung lässt sich direkt neu verknüpfen

## 0.14.0
- Sprachanrufe annehmen, ablehnen und starten
- Medien-Betrachter für Bilder, Videos und GIFs (Zoom, Spulen, Blättern, Vollbild)
- Ausgang: nicht gesendete Fotos, Videos, Dateien und Sprachnachrichten überstehen einen Neustart
- Sicherheit: nur harmlose Dateitypen öffnen ohne Rückfrage, Log ohne Telefonnummern, Dateirechte 0600/0700, Instanz-Sperre
- Performance: entprellte Suche, Zwischenablage im Hintergrund, Kompaktierung alter Daten

## 0.13.x
- Kanäle komplett: Abos synchron mit dem Handy, Suche, Link, Reaktionen, eigener Kanal
- Englische Oberfläche
- Download-Drossel, Zusammenführung von Nummer und LID, Tray-Fix, Härtung nach Audit

## 0.12.0
- Sprachanrufe, Status ansehen und posten, Kanäle lesen, Chat-Export

## 0.11.0
- Gelöschte Nachrichten behalten (Schalter), frühere Fassungen bearbeiteter Nachrichten, mehr als 3 angeheftete Chats

## 0.10.0
- 3D-Emojis (Fluent) mit voller Auswahl, Einstellungen wie bei WhatsApp inkl. Datenschutz, Kontakt- und Gruppeninfo, Gruppen verwalten, Profilbild, Blockieren, Abmelden

## 0.9.0
- Sprachnachrichten abspielen und aufnehmen, animierte und eigene Sticker, Videos und GIFs senden, GIF-Suche

## 0.8.0
- Bearbeiten, Erwähnungen, Stern, Umfragen, Standort, Kontakte, Link-Vorschau, Suche über alle Chats

## 0.7.0
- Name WhatsSnap, Tray-Symbol, Formatierung, Bilder per Strg+V, Entwürfe, „schreibt …“/online, Weiterleiten, Tastenkürzel

## 0.6.0 und früher
- Fotos, Sticker und Dokumente, Antworten, Reaktionen, Design wie WhatsApp Web, Benachrichtigungen, lokaler Verlauf, Verknüpfen per QR-Code
