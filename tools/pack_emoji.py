#!/usr/bin/env python3
"""Packt die Twemoji-PNGs (https://github.com/jdecked/twemoji, Grafiken CC-BY 4.0)
verkleinert in eine Datei assets/emoji.bin, die ins Programm eingebettet wird.

Format: b"WEM1", u32 Anzahl, je Eintrag: u16 Schlüssellänge, Schlüssel (UTF-8,
Emoji-Folge ohne U+FE0F), u32 Offset, u32 Länge; danach die PNG-Daten.
Aufruf: pack_emoji.py <ordner-mit-72x72-pngs> <ziel.bin> [kantenlänge]
"""
import io, os, struct, sys
from PIL import Image

src, dst = sys.argv[1], sys.argv[2]
size = int(sys.argv[3]) if len(sys.argv) > 3 else 48
entries, blob = [], bytearray()
for name in sorted(os.listdir(src)):
    if not name.endswith(".png"):
        continue
    cps = [int(h, 16) for h in name[:-4].split("-")]
    key = "".join(chr(c) for c in cps if c != 0xFE0F).encode("utf-8")
    img = Image.open(os.path.join(src, name)).convert("RGBA").resize((size, size), Image.LANCZOS)
    buf = io.BytesIO()
    img = img.quantize(colors=64, method=Image.Quantize.FASTOCTREE)
    img.save(buf, "PNG", optimize=True)
    data = buf.getvalue()
    entries.append((key, len(blob), len(data)))
    blob += data
with open(dst, "wb") as f:
    f.write(b"WEM1" + struct.pack("<I", len(entries)))
    for key, off, ln in entries:
        f.write(struct.pack("<H", len(key)) + key + struct.pack("<II", off, ln))
    f.write(blob)
print(len(entries), "Emojis,", len(blob) // 1024, "KiB")
