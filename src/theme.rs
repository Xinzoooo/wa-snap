//! Farben, Schriften und Emoji-Auswahl. Ein Akzentton, ruhige Flächen, hell und dunkel.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius};

#[derive(Clone, Copy)]
pub struct Palette {
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
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

impl Palette {
    pub fn dark() -> Self {
        Self {
            side: rgb(0x14, 0x1a, 0x20),
            chat: rgb(0x0e, 0x13, 0x18),
            bar: rgb(0x14, 0x1a, 0x20),
            bubble_me: rgb(0x1b, 0x5e, 0x50),
            bubble_them: rgb(0x1f, 0x27, 0x2f),
            text: rgb(0xe6, 0xeb, 0xef),
            dim: rgb(0x8a, 0x97, 0xa3),
            accent: rgb(0x22, 0xb5, 0x94),
            on_accent: rgb(0x06, 0x1a, 0x15),
            border: rgb(0x24, 0x2d, 0x36),
            hover: rgb(0x1a, 0x22, 0x2a),
            selected: rgb(0x20, 0x2a, 0x34),
            input: rgb(0x1b, 0x23, 0x2b),
        }
    }

    pub fn light() -> Self {
        Self {
            side: rgb(0xff, 0xff, 0xff),
            chat: rgb(0xf3, 0xf5, 0xf7),
            bar: rgb(0xff, 0xff, 0xff),
            bubble_me: rgb(0xd5, 0xf2, 0xe7),
            bubble_them: rgb(0xff, 0xff, 0xff),
            text: rgb(0x17, 0x21, 0x2b),
            dim: rgb(0x6b, 0x7a, 0x88),
            accent: rgb(0x0f, 0x9d, 0x82),
            on_accent: rgb(0xff, 0xff, 0xff),
            border: rgb(0xe3, 0xe8, 0xed),
            hover: rgb(0xf1, 0xf4, 0xf6),
            selected: rgb(0xe6, 0xf4, 0xef),
            input: rgb(0xee, 0xf1, 0xf4),
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
    let r = CornerRadius::same(10);
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

/// Zusätzliche Systemschriften (z. B. CJK), falls vorhanden. Beste Mühe, kein Fehler wenn nicht.
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
    let mut added = false;
    for (i, path) in CANDIDATES.iter().enumerate() {
        if let Ok(bytes) = std::fs::read(path) {
            let name = format!("system-{i}");
            fonts
                .font_data
                .insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));
            if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                f.push(name);
                added = true;
            }
        }
    }
    if added {
        ctx.set_fonts(fonts);
    }
}

pub const EMOJIS: &[&str] = &[
    "😀", "😃", "😄", "😁", "😆", "😅", "😂", "🤣", "😊", "🙂", "😉", "😍", "😘", "😎", "🤔", "😐",
    "😴", "😢", "😭", "😡", "🥳", "🤩", "😬", "🙄", "👍", "👎", "👏", "🙏", "💪", "👋", "🤝", "✌",
    "❤", "💔", "🔥", "✨", "🎉", "🎂", "⭐", "💯", "✅", "❌", "⚠", "❓", "☀", "🌙", "☕", "🍕",
    "🍺", "🎮", "🚗", "🏠", "📱", "💻", "📷", "🎵", "⚽", "🐶", "🐱", "🌹", "👀", "😇", "🤗", "😋",
];
