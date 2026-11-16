use super::*;

pub(crate) fn data_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
        })
        .or_else(|| std::env::var_os("APPDATA").map(std::path::PathBuf::from))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = base.join("wa-snap");
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
    }
    dir
}

/// Datenordner nur für den eigenen Nutzer lesbar machen (Ordner 0700, Dateien 0600),
/// halbe Downloads (`.part`) und alte Temp-Dateien entfernen. Läuft einmal beim Start.
pub(crate) fn harden_data_dir() {
    use std::os::unix::fs::PermissionsExt;
    fn walk(p: &std::path::Path, depth: usize) {
        let Ok(rd) = std::fs::read_dir(p) else { return };
        for e in rd.flatten() {
            let path = e.path();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
                if depth < 3 {
                    walk(&path, depth + 1);
                }
            } else if path.extension().map(|x| x == "part").unwrap_or(false) {
                let _ = std::fs::remove_file(&path);
            } else if meta.permissions().mode() & 0o077 != 0 {
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
    let dir = data_dir();
    if let Err(e) = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)) {
        eprintln!("{}: {e}", dir.display());
    }
    let _ = std::fs::remove_dir_all(dir.join("tmp"));
    walk(&dir, 0);
}

/// Datei atomar ersetzen: erst `<name>.tmp` schreiben und syncen, dann umbenennen.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    {
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

pub(crate) fn settings_path() -> std::path::PathBuf {
    data_dir().join("settings.json")
}

pub(crate) fn load_settings() -> Settings {
    let path = settings_path();
    let Ok(t) = std::fs::read_to_string(&path) else { return Settings::default() };
    match serde_json::from_str(&t) {
        Ok(s) => s,
        Err(e) => {
            // Kaputte Datei nicht still überschreiben, sondern beiseitelegen.
            let bak = data_dir().join(format!("settings.json.bak-{}", now_ts()));
            let _ = std::fs::rename(&path, &bak);
            backend::log(format!("settings.json unlesbar ({e}), gesichert als {}", bak.display()));
            Settings::default()
        }
    }
}

pub(crate) fn autostart_file() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("autostart/wa-snap.desktop"))
}

pub(crate) fn set_autostart(on: bool) -> std::io::Result<()> {
    let Some(file) = autostart_file() else { return Ok(()) };
    if on {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let exe = std::env::current_exe()?;
        let entry = include_str!("../assets/wa-snap.desktop")
            .replace("@EXEC@", &format!("\"{}\" --hidden", exe.display()));
        std::fs::write(file, entry)
    } else if file.exists() {
        std::fs::remove_file(file)
    } else {
        Ok(())
    }
}

pub(crate) fn set_flag(list: &mut Vec<String>, key: String, on: bool) {
    let pos = list.iter().position(|k| k == &key);
    match (on, pos) {
        (true, None) => list.push(key),
        (false, Some(i)) => {
            list.remove(i);
        }
        _ => {}
    }
}

pub(crate) fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn fmt_time(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// Kurze Zeit für die Chatliste: heute Uhrzeit, gestern "Gestern", sonst Datum.
pub(crate) fn fmt_list_time(ts: i64) -> String {
    use chrono::TimeZone;
    if ts <= 0 {
        return String::new();
    }
    let Some(d) = chrono::Local.timestamp_opt(ts, 0).single() else { return String::new() };
    let today = chrono::Local::now().date_naive();
    let day = d.date_naive();
    if day == today {
        d.format("%H:%M").to_string()
    } else if today.signed_duration_since(day).num_days() == 1 {
        t!("Gestern", "Yesterday").to_owned()
    } else {
        d.format("%d.%m.%y").to_string()
    }
}

pub(crate) fn day_label(ts: i64) -> String {
    use chrono::TimeZone;
    let Some(d) = chrono::Local.timestamp_opt(ts, 0).single() else { return String::new() };
    let today = chrono::Local::now().date_naive();
    let day = d.date_naive();
    match today.signed_duration_since(day).num_days() {
        0 => t!("Heute", "Today").to_owned(),
        1 => t!("Gestern", "Yesterday").to_owned(),
        _ => d.format("%d.%m.%Y").to_string(),
    }
}

pub(crate) fn day_key(ts: i64) -> i32 {
    use chrono::{Datelike, TimeZone};
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.date_naive().num_days_from_ce())
        .unwrap_or(0)
}

/// Stelle, an der eine Nachricht mit Zeit `ts` eingefügt wird: hinter allen, die nicht jünger sind.
pub(crate) fn insert_pos(messages: &[ChatMessage], ts: i64) -> usize {
    match messages.last() {
        Some(last) if last.ts <= ts => messages.len(),
        _ => messages.partition_point(|x| x.ts <= ts),
    }
}

pub(crate) fn user_of(key: &str) -> String {
    key.split('@').next().unwrap_or("").split(':').next().unwrap_or("").to_owned()
}

pub(crate) fn draw_avatar(ui: &mut egui::Ui, tex: Option<&egui::TextureHandle>, name: &str, size: f32, dark: bool) {
    if let Some(t) = tex {
        ui.add(egui::Image::new((t.id(), egui::vec2(size, size))).corner_radius(size / 2.0));
        return;
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let initial = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into());
    let (bg, fg) = if dark {
        (Color32::from_rgb(0x2e, 0x2f, 0x2f), Color32::from_rgb(0xa5, 0xa5, 0xa5))
    } else {
        (Color32::from_rgb(0xdf, 0xe5, 0xe7), Color32::from_rgb(0x8a, 0x96, 0x9c))
    };
    ui.painter().circle_filled(rect.center(), size / 2.0, bg);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        egui::FontId::new(size * 0.42, theme::bold()),
        fg,
    );
}

/// Haken wie bei WhatsApp: einer = gesendet, zwei = zugestellt, zwei blau = gelesen.
pub(crate) fn draw_ticks(ui: &mut egui::Ui, status: u8, pal: &Palette) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(17.0, 11.0), egui::Sense::hover());
    let p = ui.painter();
    match status {
        0 => {
            // Uhr: wird gesendet
            p.circle_stroke(rect.center(), 4.5, egui::Stroke::new(1.2_f32, pal.dim));
            p.line_segment([rect.center(), rect.center() + egui::vec2(0.0, -3.0)], egui::Stroke::new(1.2_f32, pal.dim));
            p.line_segment([rect.center(), rect.center() + egui::vec2(2.0, 0.0)], egui::Stroke::new(1.2_f32, pal.dim));
        }
        9 => {
            p.text(rect.center(), egui::Align2::CENTER_CENTER, "!", egui::FontId::new(12.0, theme::bold()), pal.danger);
        }
        _ => {
            let col = if status >= 3 { pal.tick_read } else { pal.dim };
            let st = egui::Stroke::new(1.4_f32, col);
            let tick = |dx: f32| {
                let a = rect.left_center() + egui::vec2(dx + 1.0, 0.5);
                let b = a + egui::vec2(3.0, 3.0);
                let c = b + egui::vec2(6.5, -7.5);
                p.line_segment([a, b], st);
                p.line_segment([b, c], st);
            };
            if status >= 2 {
                tick(0.0);
                tick(5.0);
            } else {
                tick(3.0);
            }
        }
    }
}

/// WhatsApp-Formatierung: *fett*, _kursiv_, ~durchgestrichen~, `Code`/```Code```.
/// Ergebnis: Textstücke mit Stil-Bits (1 fett, 2 kursiv, 4 durchgestrichen, 8 Code).
pub(crate) fn fmt_spans(t: &str, base: u8) -> Vec<(String, u8)> {
    let chars: Vec<char> = t.chars().collect();
    let mut out: Vec<(String, u8)> = Vec::new();
    let mut buf = String::new();
    let boundary = |c: Option<&char>| c.map(|c| !c.is_alphanumeric()).unwrap_or(true);
    let mut i = 0;
    let mut base = base;
    while i < chars.len() {
        let c = chars[i];
        // Erwähnungen sind mit U+E000 … U+E001 eingerahmt (siehe with_mention_names)
        if c == '\u{E000}' || c == '\u{E001}' {
            if !buf.is_empty() {
                out.push((std::mem::take(&mut buf), base));
            }
            if c == '\u{E000}' {
                base |= 16;
            } else {
                base &= !16;
            }
            i += 1;
            continue;
        }
        if base & 8 == 0 && matches!(c, '*' | '_' | '~' | '`') {
            // ```Block```
            let triple = c == '`' && chars.get(i + 1) == Some(&'`') && chars.get(i + 2) == Some(&'`');
            let mlen = if triple { 3 } else { 1 };
            let open_ok = (i == 0 || boundary(chars.get(i - 1))) && chars.get(i + mlen).map(|n| !n.is_whitespace()).unwrap_or(false);
            if open_ok {
                let mut j = i + mlen;
                let mut close = None;
                while j < chars.len() {
                    if chars[j] == '\n' && !triple {
                        break;
                    }
                    let is_close = if triple {
                        chars[j] == '`' && chars.get(j + 1) == Some(&'`') && chars.get(j + 2) == Some(&'`')
                    } else {
                        chars[j] == c
                    };
                    if is_close && j > i + mlen && !chars[j - 1].is_whitespace() && boundary(chars.get(j + mlen)) {
                        close = Some(j);
                        break;
                    }
                    j += 1;
                }
                if let Some(j) = close {
                    if !buf.is_empty() {
                        out.push((std::mem::take(&mut buf), base));
                    }
                    let inner: String = chars[i + mlen..j].iter().collect();
                    let bit = match c {
                        '*' => 1,
                        '_' => 2,
                        '~' => 4,
                        _ => 8,
                    };
                    if bit == 8 {
                        out.push((inner, base | 8));
                    } else {
                        out.extend(fmt_spans(&inner, base | bit));
                    }
                    i = j + mlen;
                    continue;
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    if !buf.is_empty() {
        out.push((buf, base));
    }
    out
}

pub(crate) fn has_format(t: &str) -> bool {
    t.contains(['*', '_', '~', '`', '\u{E000}'])
}

pub(crate) fn styled(t: String, style: u8, size: f32) -> RichText {
    let mut r = RichText::new(t).size(size);
    if style & 8 != 0 {
        r = r.code();
    }
    if style & 1 != 0 {
        r = r.family(theme::bold());
    }
    if style & 2 != 0 {
        r = r.italics();
    }
    if style & 4 != 0 {
        r = r.strikethrough();
    }
    if style & 16 != 0 {
        r = r.color(Color32::from_rgb(0x21, 0xc0, 0x63)).family(theme::bold());
    }
    r
}

/// Text mit bunten Emojis in einer Umbruch-Zeile ausgeben. Nur Emojis (bis 3) werden groß gezeigt.
pub(crate) fn rich_label(ui: &mut egui::Ui, text: &str, size: f32, cache: &mut emoji::Cache) {
    if !emoji::has_emoji(text) && !has_format(text) {
        ui.add(egui::Label::new(RichText::new(text).size(size)).selectable(true));
        return;
    }
    let segs = emoji::segments(text);
    let esz = emoji_size(&segs, size);
    let old = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = 0.0;
    for s in segs {
        match s {
            emoji::Seg::Text(t) => {
                for (part, style) in fmt_spans(t, 0) {
                    ui.add(egui::Label::new(styled(part, style, size)).selectable(true));
                }
            }
            emoji::Seg::Emoji(k) => match cache.get(ui.ctx(), &k) {
                Some(id) => {
                    ui.add(egui::Image::new((id, egui::vec2(esz, esz))));
                }
                None => {
                    ui.label(RichText::new(k).size(size));
                }
            },
        }
    }
    ui.spacing_mut().item_spacing.x = old;
}

pub(crate) fn emoji_size(segs: &[emoji::Seg<'_>], size: f32) -> f32 {
    let only = segs.iter().all(|s| match s {
        emoji::Seg::Emoji(_) => true,
        emoji::Seg::Text(t) => t.trim().is_empty(),
    });
    let n = segs.iter().filter(|s| matches!(s, emoji::Seg::Emoji(_))).count();
    if only && n <= 3 { size * 2.6 } else { size * 1.3 }
}

/// Ungefähre Breite von Text mit Emojis (für die Blasenbreite).
pub(crate) fn rich_width(ui: &egui::Ui, text: &str, size: f32, max_w: f32) -> f32 {
    let font = egui::FontId::proportional(size);
    if !emoji::has_emoji(text) {
        return ui.fonts(|f| f.layout(text.to_owned(), font, Color32::WHITE, max_w).size().x);
    }
    let segs = emoji::segments(text);
    let esz = emoji_size(&segs, size);
    let mut line = 0.0_f32;
    let mut widest = 0.0_f32;
    for s in &segs {
        match s {
            emoji::Seg::Emoji(_) => line += esz,
            emoji::Seg::Text(t) => {
                let mut parts = t.split('\n').peekable();
                while let Some(part) = parts.next() {
                    line += ui.fonts(|f| f.layout_no_wrap(part.to_owned(), font.clone(), Color32::WHITE).size().x);
                    if parts.peek().is_some() {
                        widest = widest.max(line);
                        line = 0.0;
                    }
                }
            }
        }
    }
    widest.max(line).min(max_w)
}

/// Eine Zeile Text mit bunten Emojis direkt zeichnen (Chatliste).
pub(crate) fn paint_rich_line(
    painter: &egui::Painter,
    ctx: &egui::Context,
    cache: &mut emoji::Cache,
    left_center: egui::Pos2,
    text: &str,
    font: egui::FontId,
    color: Color32,
) {
    let mut x = left_center.x;
    let max_x = painter.clip_rect().right();
    let esz = font.size * 1.2;
    for s in emoji::segments(text) {
        if x > max_x {
            break;
        }
        match s {
            emoji::Seg::Text(t) => {
                let g = painter.layout_no_wrap(t.to_owned(), font.clone(), color);
                let w = g.size().x;
                painter.galley(egui::pos2(x, left_center.y - g.size().y / 2.0), g, color);
                x += w;
            }
            emoji::Seg::Emoji(k) => {
                if let Some(id) = cache.get(ctx, &k) {
                    let r = egui::Rect::from_min_size(egui::pos2(x, left_center.y - esz / 2.0), egui::vec2(esz, esz));
                    painter.image(id, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                }
                x += esz + 1.0;
            }
        }
    }
}

/// Schalter wie bei WhatsApp (Text links, grüner Schalter rechts).
pub(crate) fn toggle(ui: &mut egui::Ui, on: &mut bool, label: &str, pal: &Palette) -> egui::Response {
    let w = ui.available_width().min(520.0);
    let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(w, 40.0), egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let p = ui.painter();
    let galley = p.layout(label.to_owned(), egui::FontId::proportional(14.5), pal.text, w - 70.0);
    p.galley(egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, pal.text);
    let t = ui.ctx().animate_bool(resp.id, *on);
    let sw = egui::Rect::from_center_size(egui::pos2(rect.right() - 24.0, rect.center().y), egui::vec2(38.0, 20.0));
    let bg = if *on { pal.accent } else { pal.border.gamma_multiply(2.0) };
    p.rect_filled(sw, 10.0, bg);
    let knob = egui::pos2(egui::lerp(sw.left() + 10.0..=sw.right() - 10.0, t), sw.center().y);
    p.circle_filled(knob, 7.5, if *on { pal.on_accent } else { pal.text });
    resp
}

/// Runder Filter-Knopf ("Alle", "Ungelesen", …).
pub(crate) fn chip(ui: &mut egui::Ui, label: &str, on: bool, pal: &Palette) -> bool {
    let font = egui::FontId::new(13.5, if on { theme::bold() } else { egui::FontFamily::Proportional });
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, if on { pal.chip_on_text } else { pal.dim });
    let size = egui::vec2(galley.size().x + 24.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let p = ui.painter();
    if on {
        p.rect_filled(rect, 15.0, pal.chip_on);
    } else {
        p.rect_filled(rect, 15.0, if resp.hovered() { pal.hover } else { Color32::TRANSPARENT });
        p.rect_stroke(rect, 15.0, egui::Stroke::new(1.0_f32, pal.border), egui::StrokeKind::Inside);
    }
    p.galley(rect.center() - galley.size() / 2.0, galley, pal.text);
    resp.clicked()
}

/// Kleiner runder Symbolknopf ohne Rahmen.
pub(crate) fn icon_button(ui: &mut egui::Ui, icon: &str, size: f32, color: Color32, pal: &Palette, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size + 14.0, size + 14.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().circle_filled(rect.center(), rect.width() / 2.0, pal.hover);
    }
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(size), color);
    resp.on_hover_text(tip)
}

/// Dezentes Hintergrundmuster im Chat (wie die Kritzeleien bei WhatsApp, nur ruhiger).
pub(crate) fn paint_pattern(ui: &egui::Ui, rect: egui::Rect, pal: &Palette) {
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, pal.chat);
    let step = 46.0;
    let mut y = rect.top() - (rect.top() % step);
    let mut row = 0;
    while y < rect.bottom() + step {
        let off = if row % 2 == 0 { 0.0 } else { step / 2.0 };
        let mut x = rect.left() - (rect.left() % step) + off;
        while x < rect.right() + step {
            let c = egui::pos2(x, y);
            match (row + (x / step) as i32) % 3 {
                0 => {
                    p.circle_stroke(c, 5.0, egui::Stroke::new(1.3_f32, pal.pattern));
                }
                1 => {
                    p.line_segment([c + egui::vec2(-5.0, 0.0), c + egui::vec2(5.0, 0.0)], egui::Stroke::new(1.3_f32, pal.pattern));
                    p.line_segment([c + egui::vec2(0.0, -5.0), c + egui::vec2(0.0, 5.0)], egui::Stroke::new(1.3_f32, pal.pattern));
                }
                _ => {
                    p.rect_stroke(
                        egui::Rect::from_center_size(c, egui::vec2(8.0, 8.0)),
                        2.0,
                        egui::Stroke::new(1.3_f32, pal.pattern),
                        egui::StrokeKind::Middle,
                    );
                }
            }
            x += step;
        }
        y += step;
        row += 1;
    }
}

pub(crate) fn fmt_last_seen(ts: i64) -> String {
    let label = day_label(ts);
    let t = fmt_time(ts);
    match label.as_str() {
        "Heute" => tf!("heute um {t}", "today at {t}"),
        "Gestern" => tf!("gestern um {t}", "yesterday at {t}"),
        d => tf!("am {d} um {t}", "on {d} at {t}"),
    }
}

/// 1234 → "1.234", 1234567 → "1,2 Mio."
pub(crate) fn fmt_count(n: u64) -> String {
    if n >= 1_000_000 {
        tf!("{} Mio.", "{}M", format!("{:.1}", n as f64 / 1e6).replace('.', ","))
    } else if n >= 1000 {
        format!("{}.{:03}", n / 1000, n % 1000)
    } else {
        n.to_string()
    }
}

pub(crate) fn apply_lang(lang: &str) {
    i18n::set_en(match lang {
        "en" => true,
        "de" => false,
        _ => !i18n::system_is_german(),
    });
}

pub(crate) fn dir_size(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| rd.flatten().map(|e| e.metadata().map(|m| if m.is_dir() { dir_size(&e.path()) } else { m.len() }).unwrap_or(0)).sum())
        .unwrap_or(0)
}

pub(crate) fn fmt_size(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / 1048576.0)
    } else {
        format!("{} KB", (n / 1024).max(1))
    }
}

/// Anzeigegröße eines Bildes/Stickers in der Blase.
pub(crate) fn media_size(md: &backend::MediaRef, tex: Option<&(egui::TextureHandle, bool)>, max_w: f32) -> egui::Vec2 {
    let (w, h) = match tex {
        Some((t, _)) => (t.size()[0] as f32, t.size()[1] as f32),
        None if md.w > 0 && md.h > 0 => (md.w as f32, md.h as f32),
        None => (300.0, 300.0),
    };
    let limit = if md.kind == "sticker" { 150.0 } else { 320.0_f32.min(max_w - 16.0) };
    let scale = (limit / w).min(380.0 / h);
    egui::vec2(w * scale, h * scale)
}
