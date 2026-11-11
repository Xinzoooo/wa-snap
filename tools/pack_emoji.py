#!/usr/bin/env python3
"""Packt die Emoji-Grafiken für WhatsSnap.

Quellen:
  * Microsoft Fluent Emoji 3D (https://github.com/microsoft/fluentui-emoji, MIT) – Hauptsatz
  * Twemoji (https://github.com/jdecked/twemoji, Grafiken CC-BY 4.0) – Ersatz für fehlende
  * Emojibase (https://github.com/milesj/emojibase, MIT) – Reihenfolge, Kategorien,
    deutsche Namen und Suchbegriffe für die Auswahl

Ausgabe:
  assets/emoji.bin       Bilder. Format: b"WEM1", u32 Anzahl, je Eintrag: u16 Schlüssellänge,
                         Schlüssel (UTF-8, Emoji ohne U+FE0F), u32 Offset, u32 Länge; danach PNGs.
  assets/emoji_list.json Auswahl: [[Kategorie, [[Emoji, Name, Stichwörter, [Hauttöne…]], …]], …]

Aufruf: pack_emoji.py <fluent/assets> <twemoji/assets/72x72> <emojibase/de/compact.json> <assets-ordner> [größe]
"""
import io, json, os, struct, sys
from PIL import Image

fluent_dir, twe_dir, ebase_file, out_dir = sys.argv[1:5]
size = int(sys.argv[5]) if len(sys.argv) > 5 else 48

def key_of(cps):
    return "".join(chr(c) for c in cps if c != 0xFE0F)

images = {}  # Schlüssel -> Dateipfad

# Twemoji zuerst (wird von Fluent überschrieben)
for name in os.listdir(twe_dir):
    if name.endswith(".png"):
        images[key_of(int(h, 16) for h in name[:-4].split("-"))] = os.path.join(twe_dir, name)

TONES = ["Default", "Light", "Medium-Light", "Medium", "Medium-Dark", "Dark"]
fluent = 0
for folder in os.listdir(fluent_dir):
    base = os.path.join(fluent_dir, folder)
    try:
        meta = json.load(open(os.path.join(base, "metadata.json"), encoding="utf-8"))
    except Exception:
        continue
    def png_in(d):
        d = os.path.join(d, "3D")
        if os.path.isdir(d):
            for f in os.listdir(d):
                if f.endswith(".png"):
                    return os.path.join(d, f)
        return None
    direct = png_in(base)
    if direct:
        images[key_of(int(h, 16) for h in meta["unicode"].split())] = direct
        fluent += 1
    tones = meta.get("unicodeSkintones") or []
    for i, uc in enumerate(tones):
        p = png_in(os.path.join(base, TONES[i])) if i < len(TONES) else None
        if p:
            images[key_of(int(h, 16) for h in uc.split())] = p
            fluent += 1

entries, blob = [], bytearray()
for key in sorted(images):
    img = Image.open(images[key]).convert("RGBA").resize((size, size), Image.LANCZOS)
    img = img.quantize(colors=128, method=Image.Quantize.FASTOCTREE)
    buf = io.BytesIO()
    img.save(buf, "PNG", optimize=True)
    data = buf.getvalue()
    entries.append((key.encode("utf-8"), len(blob), len(data)))
    blob += data
with open(os.path.join(out_dir, "emoji.bin"), "wb") as f:
    f.write(b"WEM1" + struct.pack("<I", len(entries)))
    for k, off, ln in entries:
        f.write(struct.pack("<H", len(k)) + k + struct.pack("<II", off, ln))
    f.write(blob)

# Auswahl-Liste in WhatsApp-Kategorien
CATS = [
    ("Smileys und Personen", {0, 1}),
    ("Tiere und Natur", {3}),
    ("Essen und Trinken", {4}),
    ("Aktivitäten", {6}),
    ("Reisen und Orte", {5}),
    ("Objekte", {7}),
    ("Symbole", {8}),
    ("Flaggen", {9}),
]
data = json.load(open(ebase_file, encoding="utf-8"))
data = [d for d in data if "group" in d and "order" in d]
data.sort(key=lambda d: d["order"])
out = []
for name, groups in CATS:
    items = []
    for d in data:
        if d["group"] not in groups:
            continue
        k = key_of(ord(c) for c in d["unicode"])
        if k not in images:
            continue
        skins = [s["unicode"] for s in d.get("skins", []) if key_of(ord(c) for c in s["unicode"]) in images]
        tags = " ".join(d.get("tags", []))
        items.append([d["unicode"], d.get("label", ""), tags, skins])
    out.append([name, items])
json.dump(out, open(os.path.join(out_dir, "emoji_list.json"), "w", encoding="utf-8"), ensure_ascii=False, separators=(",", ":"))
print(len(entries), "Bilder (", fluent, "aus Fluent ),", len(blob) // 1024, "KiB;",
      sum(len(c[1]) for c in out), "Emojis in der Auswahl")
