//! Oberflächensprache: Deutsch oder Englisch. Texte stehen im Code als
//! `t!("Deutsch", "English")` bzw. `tf!("Deutsch {x}", "English {x}", …)`.

use std::sync::atomic::{AtomicBool, Ordering};

static EN: AtomicBool = AtomicBool::new(false);

pub fn en() -> bool {
    EN.load(Ordering::Relaxed)
}

pub fn set_en(on: bool) {
    EN.store(on, Ordering::Relaxed);
}

/// Systemsprache: Deutsch, wenn LANG/LC_ALL mit "de" beginnt, sonst Englisch.
pub fn system_is_german() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .map(|v| v.starts_with("de"))
        .unwrap_or(true)
}

/// Feste Texte aus Datendateien (z. B. Emoji-Kategorien) übersetzen.
pub fn tr(s: &str) -> &str {
    if !en() {
        return s;
    }
    match s {
        "Smileys und Personen" => "Smileys & people",
        "Tiere und Natur" => "Animals & nature",
        "Essen und Trinken" => "Food & drink",
        "Aktivitäten" => "Activity",
        "Reisen und Orte" => "Travel & places",
        "Objekte" => "Objects",
        "Symbole" => "Symbols",
        "Flaggen" => "Flags",
        other => other,
    }
}

#[macro_export]
macro_rules! t {
    ($de:literal, $en:literal) => {
        if $crate::i18n::en() { $en } else { $de }
    };
}

#[macro_export]
macro_rules! tf {
    ($de:literal, $en:literal $(, $a:expr)* $(,)?) => {
        if $crate::i18n::en() { format!($en $(, $a)*) } else { format!($de $(, $a)*) }
    };
}
