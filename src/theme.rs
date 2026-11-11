//! Farben, Schriften und Emoji-Auswahl. Ein Akzentton, ruhige Flächen, hell und dunkel.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius};

#[derive(Clone, Copy)]
pub struct Palette {
    pub rail: Color32,
    pub side: Color32,
    pub chat: Color32,
    pub bar: Color32,
    pub bubble_me: Color32,
    pub bubble_them: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub border: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub input: Color32,
    pub chip_on: Color32,
    pub chip_on_text: Color32,
    pub tick_read: Color32,
    pub pattern: Color32,
    pub danger: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

impl Palette {
    /// Farben wie WhatsApp Web (dunkel).
    pub fn dark() -> Self {
        Self {
            rail: rgb(0x1d, 0x1f, 0x1f),
            side: rgb(0x16, 0x17, 0x17),
            chat: rgb(0x16, 0x17, 0x17),
            bar: rgb(0x16, 0x17, 0x17),
            bubble_me: rgb(0x14, 0x4d, 0x37),
            bubble_them: rgb(0x24, 0x26, 0x26),
            text: rgb(0xfa, 0xfa, 0xfa),
            dim: rgb(0xa5, 0xa5, 0xa5),
            accent: rgb(0x21, 0xc0, 0x63),
            on_accent: rgb(0x0b, 0x14, 0x10),
            border: rgb(0x2a, 0x2c, 0x2c),
            hover: rgb(0x24, 0x26, 0x26),
            selected: rgb(0x2e, 0x2f, 0x2f),
            input: rgb(0x2e, 0x2f, 0x2f),
            chip_on: rgb(0x10, 0x35, 0x29),
            chip_on_text: rgb(0xd9, 0xfd, 0xd3),
            tick_read: rgb(0x53, 0xbd, 0xeb),
            pattern: rgb(0x1c, 0x1e, 0x1e),
            danger: rgb(0xf1, 0x5c, 0x6d),
        }
    }

    /// Farben wie WhatsApp Web (hell).
    pub fn light() -> Self {
        Self {
            rail: rgb(0xf7, 0xf5, 0xf3),
            side: rgb(0xff, 0xff, 0xff),
            chat: rgb(0xf5, 0xf1, 0xeb),
            bar: rgb(0xff, 0xff, 0xff),
            bubble_me: rgb(0xd9, 0xfd, 0xd3),
            bubble_them: rgb(0xff, 0xff, 0xff),
            text: rgb(0x11, 0x1b, 0x21),
            dim: rgb(0x66, 0x77, 0x81),
            accent: rgb(0x1d, 0xaa, 0x61),
            on_accent: rgb(0xff, 0xff, 0xff),
            border: rgb(0xe9, 0xed, 0xef),
            hover: rgb(0xf5, 0xf6, 0xf6),
            selected: rgb(0xf0, 0xf2, 0xf5),
            input: rgb(0xf0, 0xf2, 0xf5),
            chip_on: rgb(0xd9, 0xfd, 0xd3),
            chip_on_text: rgb(0x0a, 0x33, 0x2c),
            tick_read: rgb(0x53, 0xbd, 0xeb),
            pattern: rgb(0xee, 0xe9, 0xe1),
            danger: rgb(0xd4, 0x2a, 0x3e),
        }
    }

    pub fn of(dark: bool) -> Self {
        if dark {
            Self::dark()
        } else {
            Self::light()
        }
    }
}

/// Feste Farben für Absendernamen in Gruppen (wie bei WhatsApp je Person anders).
pub fn sender_color(name: &str, dark: bool) -> Color32 {
    const DARK: [Color32; 8] = [
        rgb(0x53, 0xbd, 0xeb), rgb(0xfc, 0x92, 0x75), rgb(0xa5, 0xb3, 0x37), rgb(0xe5, 0x76, 0xbd),
        rgb(0x3e, 0xc8, 0xa8), rgb(0xff, 0xd2, 0x79), rgb(0xa7, 0x91, 0xff), rgb(0x7f, 0xc9, 0x5a),
    ];
    const LIGHT: [Color32; 8] = [
        rgb(0x02, 0x7e, 0xb5), rgb(0xc4, 0x53, 0x2d), rgb(0x6c, 0x7a, 0x00), rgb(0xb4, 0x2e, 0x84),
        rgb(0x00, 0x8a, 0x6c), rgb(0xa3, 0x6b, 0x00), rgb(0x5b, 0x46, 0xd6), rgb(0x3b, 0x86, 0x1c),
    ];
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let i = (h % 8) as usize;
    if dark { DARK[i] } else { LIGHT[i] }
}

/// Schriftfamilie für fette Texte (Namen, Überschriften).
pub fn bold() -> egui::FontFamily {
    egui::FontFamily::Name("bold".into())
}

pub fn apply(ctx: &egui::Context, dark: bool, font_size: f32) {
    let p = Palette::of(dark);
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.override_text_color = Some(p.text);
    v.panel_fill = p.chat;
    v.window_fill = p.side;
    v.extreme_bg_color = p.input;
    v.selection.bg_fill = p.accent;
    v.selection.stroke.color = p.on_accent;
    v.hyperlink_color = p.accent;
    let r = CornerRadius::same(8);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_stroke.color = p.border;
    v.widgets.inactive.bg_fill = p.input;
    v.widgets.inactive.weak_bg_fill = p.input;
    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.hovered.weak_bg_fill = p.hover;
    ctx.set_visuals(v);

    ctx.style_mut(|st| {
        st.spacing.item_spacing = egui::vec2(8.0, 6.0);
        st.spacing.button_padding = egui::vec2(12.0, 6.0);
        for (ts, f) in st.text_styles.iter_mut() {
            match ts {
                egui::TextStyle::Body | egui::TextStyle::Button | egui::TextStyle::Monospace => {
                    f.size = font_size
                }
                egui::TextStyle::Small => f.size = font_size - 3.0,
                egui::TextStyle::Heading => f.size = font_size + 6.0,
                _ => {}
            }
        }
    });
}

/// Schrift: Inter (mitgeliefert, ähnlich wie WhatsApp Web), dahinter Emoji-Schriften
/// von egui und, falls vorhanden, Systemschriften für CJK/Arabisch.
pub fn install_fonts(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/google-noto-sans-cjk-fonts/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
        "/usr/share/fonts/google-noto/NotoSansArabic-Regular.ttf",
    ];
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "inter".into(),
        Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.otf"))),
    );
    fonts.font_data.insert(
        "inter-semibold".into(),
        Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/Inter-SemiBold.otf"))),
    );
    let mut fallback: Vec<String> = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (i, path) in CANDIDATES.iter().enumerate() {
        if let Ok(bytes) = std::fs::read(path) {
            let name = format!("system-{i}");
            fonts.font_data.insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));
            fallback.push(name);
        }
    }
    let mut regular = vec!["inter".to_owned()];
    regular.extend(fallback.iter().cloned());
    let mut strong = vec!["inter-semibold".to_owned()];
    strong.extend(fallback);
    fonts.families.insert(egui::FontFamily::Proportional, regular);
    fonts.families.insert(bold(), strong);
    ctx.set_fonts(fonts);
}

/// Emoji-Auswahl nach Kategorien (Reihenfolge wie bei WhatsApp).
pub const EMOJI_CATS: &[(&str, &[&str])] = &[
    ("Smileys & Personen", &[
        "😀", "😃", "😄", "😁", "😆", "😅", "😂", "🤣", "🥲", "☺", "😊", "😇", "🙂", "🙃", "😉", "😌",
        "😍", "🥰", "😘", "😗", "😙", "😚", "😋", "😛", "😝", "😜", "🤪", "🤨", "🧐", "🤓", "😎", "🥸",
        "🤩", "🥳", "😏", "😒", "😞", "😔", "😟", "😕", "🙁", "☹", "😣", "😖", "😫", "😩", "🥺", "😢",
        "😭", "😤", "😠", "😡", "🤬", "🤯", "😳", "🥵", "🥶", "😱", "😨", "😰", "😥", "😓", "🤗", "🤔",
        "🤭", "🤫", "🤥", "😶", "😐", "😑", "😬", "🙄", "😯", "😦", "😧", "😮", "😲", "🥱", "😴", "🤤",
        "😪", "😵", "🤐", "🥴", "🤢", "🤮", "🤧", "😷", "🤒", "🤕", "🤑", "🤠", "😈", "👿", "🤡", "💩",
        "👻", "💀", "👽", "🤖", "🎃", "😺", "😸", "😹", "😻", "😼", "😽", "🙀", "😿", "😾",
    ]),
    ("Gesten", &[
        "👋", "🤚", "🖐", "✋", "🖖", "👌", "🤌", "🤏", "✌", "🤞", "🤟", "🤘", "🤙", "👈", "👉", "👆",
        "👇", "☝", "👍", "👎", "✊", "👊", "🤛", "🤜", "👏", "🙌", "👐", "🤲", "🤝", "🙏", "✍", "💅",
        "💪", "🦾", "🧠", "👀", "👁", "👅", "👄", "💋",
    ]),
    ("Herzen", &[
        "❤", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "🤎", "💔", "❣", "💕", "💞", "💓", "💗", "💖",
        "💘", "💝", "💟", "♥",
    ]),
    ("Tiere & Natur", &[
        "🐶", "🐱", "🐭", "🐹", "🐰", "🦊", "🐻", "🐼", "🐨", "🐯", "🦁", "🐮", "🐷", "🐸", "🐵", "🐔",
        "🐧", "🐦", "🐤", "🦆", "🦅", "🦉", "🐺", "🐴", "🦄", "🐝", "🐛", "🦋", "🐌", "🐞", "🐢", "🐍",
        "🐙", "🦀", "🐠", "🐬", "🐳", "🦈", "🌵", "🌲", "🌳", "🌴", "🌱", "🌿", "🍀", "🍁", "🍂", "🌷",
        "🌹", "🌺", "🌸", "🌼", "🌻", "🌞", "🌙", "⭐", "🌟", "✨", "⚡", "🔥", "🌈", "☀", "⛅", "☁",
        "🌧", "⛈", "❄", "☃", "💧", "🌊",
    ]),
    ("Essen & Trinken", &[
        "🍏", "🍎", "🍐", "🍊", "🍋", "🍌", "🍉", "🍇", "🍓", "🍒", "🍑", "🥭", "🍍", "🥥", "🥝", "🍅",
        "🥑", "🥦", "🥕", "🌽", "🌶", "🥔", "🥐", "🍞", "🥨", "🧀", "🥚", "🍳", "🥓", "🥩", "🍗", "🍖",
        "🌭", "🍔", "🍟", "🍕", "🥪", "🌮", "🌯", "🥗", "🍝", "🍜", "🍣", "🍱", "🍦", "🍰", "🎂", "🍫",
        "🍿", "🍩", "🍪", "☕", "🍵", "🥤", "🍺", "🍻", "🥂", "🍷", "🥃", "🍸", "🍹", "🍾",
    ]),
    ("Aktivitäten", &[
        "⚽", "🏀", "🏈", "⚾", "🎾", "🏐", "🎱", "🏓", "🏸", "🥊", "⛳", "🎣", "🎿", "🏂", "🏋", "🚴",
        "🏆", "🥇", "🥈", "🥉", "🏅", "🎖", "🎫", "🎟", "🎪", "🎭", "🎨", "🎬", "🎤", "🎧", "🎼", "🎹",
        "🥁", "🎷", "🎺", "🎸", "🎻", "🎲", "🎯", "🎳", "🎮", "🧩",
    ]),
    ("Reisen & Orte", &[
        "🚗", "🚕", "🚙", "🚌", "🏎", "🚓", "🚑", "🚒", "🚚", "🚜", "🛵", "🏍", "🚲", "🛴", "🚨", "🚆",
        "🚇", "✈", "🚀", "🛸", "🚁", "⛵", "🚤", "🚢", "⚓", "⛽", "🚦", "🗺", "🗽", "🏰", "🏟", "🎡",
        "🎢", "⛱", "🏖", "🏝", "⛰", "🏔", "🏕", "🏠", "🏡", "🏢", "🏥", "🏦", "🏨", "🏫", "⛪", "🌃",
    ]),
    ("Objekte", &[
        "⌚", "📱", "💻", "⌨", "🖥", "🖨", "🖱", "💾", "💿", "📷", "📸", "📹", "🎥", "📞", "☎", "📺",
        "📻", "⏰", "⌛", "🔋", "🔌", "💡", "🔦", "🕯", "💸", "💵", "💶", "💰", "💳", "💎", "🔧", "🔨",
        "🛠", "🔩", "⚙", "🔫", "💣", "🔪", "🛡", "🔮", "💊", "💉", "🧬", "🧪", "🧹", "🧻", "🚽", "🛁",
        "🔑", "🗝", "🚪", "🛋", "🛏", "🎁", "🎈", "🎉", "🎊", "✉", "📦", "📝", "📅", "📌", "📎", "✂",
        "🔒", "🔓",
    ]),
    ("Symbole", &[
        "💯", "✅", "☑", "✔", "❌", "❎", "➕", "➖", "➗", "✖", "❓", "❔", "❗", "❕", "‼", "⁉",
        "⚠", "🚫", "⛔", "📛", "🔞", "♻", "🆗", "🆕", "🆒", "🆓", "🔝", "🔜", "💤", "💢", "💥", "💫",
        "💦", "💨", "🕳", "💬", "💭", "🗯", "♠", "♣", "♦", "🔴", "🟠", "🟡", "🟢", "🔵", "🟣", "⚫",
        "⚪", "🟥", "🟩", "🟦", "⬛", "⬜", "🔶", "🔷", "▶", "⏸", "⏹", "⏺", "⏭", "⏮", "🔀", "🔁",
    ]),
    ("Flaggen", &["🏁", "🚩", "🏳", "🏴", "🏳️‍🌈", "🇩🇪", "🇦🇹", "🇨🇭", "🇪🇺", "🇺🇸", "🇬🇧", "🇫🇷", "🇮🇹", "🇪🇸", "🇹🇷", "🇺🇦"]),
];
