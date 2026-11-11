//! Bunte Emojis (Twemoji, CC-BY 4.0) statt der einfarbigen Schrift-Emojis von egui.
//! Die Bilder stecken gepackt im Programm (`assets/emoji.bin`, erzeugt von
//! `tools/pack_emoji.py`) und werden erst bei Bedarf als Textur geladen.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use eframe::egui;

static DATA: &[u8] = include_bytes!("../assets/emoji.bin");

struct Index {
    entries: HashMap<String, (usize, usize)>,
    first: HashSet<char>,
    blob: usize,
}

fn index() -> &'static Index {
    static IDX: OnceLock<Index> = OnceLock::new();
    IDX.get_or_init(|| {
        let mut entries = HashMap::new();
        let mut first = HashSet::new();
        let rd32 = |p: usize| u32::from_le_bytes([DATA[p], DATA[p + 1], DATA[p + 2], DATA[p + 3]]) as usize;
        if DATA.len() < 8 || &DATA[..4] != b"WEM1" {
            return Index { entries, first, blob: 0 };
        }
        let n = rd32(4);
        let mut p = 8;
        for _ in 0..n {
            let kl = u16::from_le_bytes([DATA[p], DATA[p + 1]]) as usize;
            let key = String::from_utf8_lossy(&DATA[p + 2..p + 2 + kl]).into_owned();
            let off = rd32(p + 2 + kl);
            let len = rd32(p + 6 + kl);
            p += 10 + kl;
            if let Some(c) = key.chars().next() {
                first.insert(c);
            }
            entries.insert(key, (off, len));
        }
        Index { entries, first, blob: p }
    })
}

/// Ein Eintrag der Emoji-Auswahl.
pub struct Item {
    pub e: String,
    pub name: String,
    pub tags: String,
    pub skins: Vec<String>,
}

/// Emoji-Auswahl in WhatsApp-Kategorien (aus `assets/emoji_list.json`).
pub fn picker() -> &'static Vec<(String, Vec<Item>)> {
    static P: OnceLock<Vec<(String, Vec<Item>)>> = OnceLock::new();
    P.get_or_init(|| {
        let raw: Vec<(String, Vec<(String, String, String, Vec<String>)>)> =
            serde_json::from_str(include_str!("../assets/emoji_list.json")).unwrap_or_default();
        raw.into_iter()
            .map(|(cat, items)| {
                (cat, items.into_iter().map(|(e, name, tags, skins)| Item { tags: tags.to_lowercase(), name, e, skins }).collect())
            })
            .collect()
    })
}

/// Schlüssel (ohne U+FE0F), wie ihn `Cache::get` erwartet.
pub fn key(e: &str) -> String {
    e.chars().filter(|c| *c != '\u{FE0F}').collect()
}

pub enum Seg<'a> {
    Text(&'a str),
    Emoji(String),
}

/// Text in Textstücke und Emojis zerlegen (längste passende Emoji-Folge gewinnt).
pub fn segments(text: &str) -> Vec<Seg<'_>> {
    let idx = index();
    let mut out = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    let mut text_start = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        let candidate = (c as u32) >= 0x2000 && idx.first.contains(&c)
            || (c.is_ascii_digit() || c == '#' || c == '*') && idx.first.contains(&c);
        if candidate {
            let mut key = String::new();
            let mut best: Option<(usize, String)> = None;
            let mut j = i;
            while j < chars.len() && j - i < 14 {
                let cj = chars[j].1;
                if cj != '\u{FE0F}' {
                    key.push(cj);
                }
                // Einzelne Ziffern/#/* sind nur mit Tastenkappe (U+20E3) ein Emoji.
                let ascii_only = key.chars().all(|k| k.is_ascii());
                if !ascii_only && idx.entries.contains_key(&key) {
                    best = Some((j + 1, key.clone()));
                }
                j += 1;
            }
            if let Some((end, k)) = best {
                // ©, ®, ™ nur als Emoji, wenn U+FE0F folgt (wie bei WhatsApp).
                let plain_symbol = matches!(c, '\u{2122}' | '\u{2139}' | '\u{2194}'..='\u{2199}')
                    && end == i + 1;
                if !plain_symbol {
                    if pos > text_start {
                        out.push(Seg::Text(&text[text_start..pos]));
                    }
                    out.push(Seg::Emoji(k));
                    i = end;
                    text_start = chars.get(i).map(|x| x.0).unwrap_or(text.len());
                    continue;
                }
            }
        }
        i += 1;
    }
    if text_start < text.len() {
        out.push(Seg::Text(&text[text_start..]));
    }
    out
}

pub fn has_emoji(text: &str) -> bool {
    let idx = index();
    text.chars().any(|c| (c as u32) >= 0x2000 && idx.first.contains(&c))
}

/// Geladene Emoji-Texturen.
#[derive(Default)]
pub struct Cache {
    tex: HashMap<String, Option<egui::TextureHandle>>,
}

impl Cache {
    pub fn get(&mut self, ctx: &egui::Context, key: &str) -> Option<egui::TextureId> {
        if let Some(t) = self.tex.get(key) {
            return t.as_ref().map(|t| t.id());
        }
        let idx = index();
        let loaded = idx.entries.get(key).and_then(|&(off, len)| {
            let start = idx.blob + off;
            let bytes = DATA.get(start..start + len)?;
            let img = image::load_from_memory(bytes).ok()?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            Some(ctx.load_texture(format!("emoji-{key}"), ci, egui::TextureOptions::LINEAR))
        });
        let id = loaded.as_ref().map(|t| t.id());
        self.tex.insert(key.to_owned(), loaded);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_text_and_emoji() {
        let s = segments("Hi 👍🏽 ok 🇩🇪!");
        let kinds: Vec<String> = s
            .iter()
            .map(|x| match x {
                Seg::Text(t) => format!("T:{t}"),
                Seg::Emoji(e) => format!("E:{}", e.chars().count()),
            })
            .collect();
        assert_eq!(kinds, vec!["T:Hi ", "E:2", "T: ok ", "E:2", "T:!"]);
    }

    #[test]
    fn plain_digits_are_text() {
        assert!(matches!(segments("123").as_slice(), [Seg::Text("123")]));
    }

    #[test]
    fn heart_with_variation_selector() {
        assert!(matches!(segments("❤️").as_slice(), [Seg::Emoji(_)]));
    }
}
