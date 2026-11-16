//! Videos in der App abspielen. Dekodiert wird von `ffmpeg` als Unterprozess: rohe
//! RGBA-Bilder kommen über eine Pipe, der Ton läuft über eine zweite ffmpeg-Instanz
//! direkt in `pw-play` (ersatzweise `pacat`/`aplay`).
//!
//! Warum ffmpeg als Programm statt gstreamer oder eingebundener Bibliothek: Es braucht
//! beim Bauen keine Entwicklungspakete, ein Absturz des Dekoders reißt die App nicht mit,
//! und für GIFs wird es ohnehin schon aufgerufen. Pause und Spulen starten den Dekoder
//! an der neuen Stelle neu; das ist einfacher und robuster als ein eigenes Steuerprotokoll.
//!
//! Es werden nie mehr als [`QUEUE`] Bilder vorgehalten: ist die Warteschlange voll,
//! blockiert der Lesethread und damit über die Pipe auch ffmpeg.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::time::Instant;

/// Vor jedes `-i` bei ffmpeg/ffprobe: nur lokale Dateien und nur echte Container.
///
/// Empfangene Dateien sind fremd. Ohne Sperre könnte eine als `.mp4` getarnte
/// HLS-Playlist oder concat-Liste Netzwerkadressen abrufen oder andere lokale Dateien
/// einlesen. `-protocol_whitelist` verbietet Netzwerkprotokolle (gilt auch für alles,
/// was der Demuxer nachlädt); `-format_whitelist` lässt Playlist-, concat- und
/// Bildfolgen-Demuxer gar nicht erst zu – sonst kämen relative Dateinamen über `file` durch.
pub const SAFE_INPUT: [&str; 4] = [
    "-protocol_whitelist",
    "file,pipe",
    "-format_whitelist",
    "mov,matroska,ogg,mp3,wav,flac,aac,amr,gif,avi,mpegts,mpeg,asf,flv",
];

/// Bilder, die höchstens im Voraus dekodiert im Speicher liegen.
const QUEUE: usize = 3;
/// Größte Kantenlänge, auf die das Video zum Anzeigen verkleinert wird.
const MAX_W: u32 = 1280;
const MAX_H: u32 = 720;
/// Mehr Bilder pro Sekunde zeigt der Player nicht (spart CPU bei 60-fps-Videos).
const MAX_FPS: f32 = 30.0;

/// Eckdaten eines Videos (aus `ffprobe`).
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    pub duration: f32,
    pub fps: f32,
    pub has_audio: bool,
}

/// Ausgabe von `ffprobe -of default=nw=1` auswerten.
pub fn parse_probe(text: &str) -> Option<Info> {
    let (mut w, mut h, mut dur, mut fps, mut audio, mut rot) = (0u32, 0u32, 0f32, 0f32, false, 0i32);
    let mut kind = "";
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        match k.trim() {
            "codec_type" => {
                kind = if v == "video" { "video" } else if v == "audio" { "audio" } else { "" };
                if kind == "audio" {
                    audio = true;
                }
            }
            "width" if kind == "video" && w == 0 => w = v.parse().unwrap_or(0),
            "height" if kind == "video" && h == 0 => h = v.parse().unwrap_or(0),
            "avg_frame_rate" | "r_frame_rate" if kind == "video" && fps == 0.0 => {
                fps = match v.split_once('/') {
                    Some((a, b)) => {
                        let (a, b): (f32, f32) = (a.parse().unwrap_or(0.0), b.parse().unwrap_or(0.0));
                        if b > 0.0 { a / b } else { 0.0 }
                    }
                    None => v.parse().unwrap_or(0.0),
                };
            }
            "rotation" if kind == "video" => rot = v.parse().unwrap_or(0),
            "duration" => {
                let d: f32 = v.parse().unwrap_or(0.0);
                dur = dur.max(d);
            }
            _ => {}
        }
    }
    if w == 0 || h == 0 {
        return None;
    }
    // Hochkant gefilmte Handy-Videos: ffmpeg dreht beim Dekodieren selbst.
    if rot.rem_euclid(180) == 90 {
        std::mem::swap(&mut w, &mut h);
    }
    if !(1.0..=240.0).contains(&fps) {
        fps = 25.0;
    }
    Some(Info { width: w, height: h, duration: dur.max(0.0), fps, has_audio: audio })
}

pub fn probe(path: &Path) -> Result<Info, String> {
    let out = Command::new("ffprobe")
        .args(SAFE_INPUT)
        .args(["-v", "error", "-show_entries", "stream=codec_type,width,height,avg_frame_rate,r_frame_rate,duration:stream_side_data=rotation:format=duration", "-of", "default=nw=1"])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;
    parse_probe(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| "kein abspielbares Video".to_owned())
}

/// Anzeigegröße: passt in [`MAX_W`]×[`MAX_H`], Seitenverhältnis bleibt, beide Kanten gerade.
pub fn fit(w: u32, h: u32) -> (u32, u32) {
    let s = (MAX_W as f32 / w as f32).min(MAX_H as f32 / h as f32).min(1.0);
    let even = |x: f32| ((x.round() as u32).max(2) / 2) * 2;
    (even(w as f32 * s), even(h as f32 * s))
}

/// Ein dekodiertes Bild mit seinem Zeitpunkt im Video.
pub struct Frame {
    pub at: f32,
    pub rgba: Vec<u8>,
}

/// Laufende Dekoder-Prozesse; beim Verwerfen werden sie beendet.
struct Running {
    video: Child,
    audio: Vec<Child>,
    frames: Receiver<Frame>,
    started: Instant,
    from: f32,
    /// Nächstes Bild, das noch nicht fällig war.
    pending: Option<Frame>,
    ended: bool,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.video.kill();
        let _ = self.video.wait();
        for c in &mut self.audio {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

pub struct Player {
    pub path: PathBuf,
    pub info: Info,
    pub size: (u32, u32),
    fps: f32,
    run: Option<Running>,
    /// Position, solange pausiert ist.
    pos: f32,
    pub volume: f32,
    pub error: Option<String>,
    /// Nach einem Sprung im Pausenzustand: erstes Bild zeigen, dann wieder anhalten.
    hold_after_first: bool,
}

impl Player {
    /// Player mit schon ermittelten Eckdaten anlegen (das Vermessen läuft im Hintergrund).
    pub fn with_info(path: &Path, info: Info) -> Player {
        let size = fit(info.width, info.height);
        let fps = info.fps.min(MAX_FPS);
        Player { path: path.to_owned(), info, size, fps, run: None, pos: 0.0, volume: 1.0, error: None, hold_after_first: false }
    }

    pub fn playing(&self) -> bool {
        self.run.as_ref().map(|r| !r.ended).unwrap_or(false)
    }

    pub fn position(&self) -> f32 {
        match &self.run {
            Some(r) if !r.ended => (r.from + r.started.elapsed().as_secs_f32()).min(self.info.duration.max(0.01)),
            _ => self.pos,
        }
    }

    pub fn pause(&mut self) {
        self.pos = self.position();
        self.run = None;
    }

    /// Ab `at` Sekunden abspielen (startet die Dekoder neu).
    pub fn play_from(&mut self, at: f32) {
        self.run = None;
        let at = at.clamp(0.0, (self.info.duration - 0.05).max(0.0));
        self.pos = at;
        match self.spawn(at) {
            Ok(r) => {
                self.run = Some(r);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub fn toggle(&mut self) {
        if self.playing() {
            self.pause();
        } else {
            let at = if self.pos >= self.info.duration - 0.1 { 0.0 } else { self.pos };
            self.play_from(at);
        }
    }

    /// Springen; pausiert bleibt pausiert, zeigt aber das Bild an der neuen Stelle.
    pub fn seek(&mut self, at: f32) {
        let was = self.playing();
        self.play_from(at);
        if !was {
            self.hold_after_first = true;
        }
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.5);
        if self.playing() {
            let at = self.position();
            self.play_from(at);
        }
    }

    fn spawn(&self, at: f32) -> Result<Running, String> {
        let (w, h) = self.size;
        let ss = format!("{at:.3}");
        let vf = format!("fps={},scale={w}:{h}", self.fps);
        let mut video = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-ss", &ss])
            .args(SAFE_INPUT)
            .arg("-i")
            .arg(&self.path)
            .args(["-an", "-sn", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        let mut out = video.stdout.take().ok_or("ffmpeg ohne Ausgabe")?;
        let (tx, rx) = sync_channel::<Frame>(QUEUE);
        let (len, fps) = ((w * h * 4) as usize, self.fps);
        std::thread::spawn(move || {
            let mut n = 0u32;
            loop {
                let mut buf = vec![0u8; len];
                if out.read_exact(&mut buf).is_err() {
                    break;
                }
                let f = Frame { at: at + n as f32 / fps, rgba: buf };
                n += 1;
                // Blockiert, solange die Oberfläche nicht nachkommt oder pausiert: begrenzt den Speicher.
                if tx.send(f).is_err() {
                    break;
                }
            }
        });
        let mut audio = Vec::new();
        if self.info.has_audio && self.volume > 0.0 {
            audio = spawn_audio(&self.path, &ss, self.volume);
        }
        Ok(Running { video, audio, frames: rx, started: Instant::now(), from: at, pending: None, ended: false })
    }

    /// Das Bild, das jetzt gezeigt werden soll, falls ein neues fällig ist.
    pub fn poll(&mut self) -> Option<Frame> {
        let hold = std::mem::take(&mut self.hold_after_first);
        let now = self.position();
        let r = self.run.as_mut()?;
        let mut due = None;
        loop {
            let next = match r.pending.take() {
                Some(f) => f,
                None => match r.frames.try_recv() {
                    Ok(f) => f,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        r.ended = true;
                        break;
                    }
                },
            };
            if next.at <= now + 0.005 || hold {
                due = Some(next);
                if hold {
                    break;
                }
            } else {
                r.pending = Some(next);
                break;
            }
        }
        if hold {
            if due.is_some() {
                // Nur das erste Bild der neuen Stelle zeigen, dann wieder anhalten.
                self.run = None;
            } else {
                self.hold_after_first = true;
            }
        } else if self.run.as_ref().map(|r| r.ended).unwrap_or(false) {
            self.pos = self.info.duration;
            self.run = None;
        }
        due
    }

    /// Wartet der Player noch auf das Standbild nach einem Sprung?
    pub fn seeking(&self) -> bool {
        self.hold_after_first
    }
}

/// Ton: ffmpeg dekodiert nach PCM, dessen Ausgabe geht direkt in den Audio-Player.
fn spawn_audio(path: &Path, ss: &str, volume: f32) -> Vec<Child> {
    let af = format!("volume={volume:.2}");
    let dec = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-ss", ss])
        .args(SAFE_INPUT)
        .arg("-i")
        .arg(path)
        .args(["-vn", "-sn", "-af", &af, "-f", "s16le", "-acodec", "pcm_s16le", "-ac", "2", "-ar", "48000", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut dec) = dec else { return Vec::new() };
    let Some(pipe) = dec.stdout.take() else {
        let _ = dec.kill();
        let _ = dec.wait();
        return Vec::new();
    };
    let sinks: [(&str, &[&str]); 3] = [
        ("pw-play", &["--raw", "--rate", "48000", "--channels", "2", "--format", "s16", "-"]),
        ("pacat", &["--raw", "--format=s16le", "--rate=48000", "--channels=2"]),
        ("aplay", &["-q", "-t", "raw", "-f", "S16_LE", "-r", "48000", "-c", "2"]),
    ];
    let mut pipe = Some(Stdio::from(pipe));
    for (cmd, args) in sinks {
        let Some(input) = pipe.take() else { break };
        match Command::new(cmd).args(args).stdin(input).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(sink) => return vec![dec, sink],
            // Programm fehlt: die Pipe ist verbraucht, ohne Ton weiter.
            Err(_) => break,
        }
    }
    let _ = dec.kill();
    let _ = dec.wait();
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Präparierte Dateien (HLS-Playlist mit http://, concat-Liste mit relativem und
    /// absolutem Pfad) dürfen weder geprüft noch dekodiert werden. Läuft nur mit ffmpeg.
    #[test]
    fn rejects_playlists_and_concat() {
        if Command::new("ffprobe").arg("-version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("wasnap-evil-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = dir.join("ok.mp4");
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=64x64:rate=5:duration=1", "-pix_fmt", "yuv420p"])
            .arg(&ok)
            .status();
        let hls = "#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:10,\nhttp://127.0.0.1:9/x.ts\n#EXT-X-ENDLIST\n";
        let cases = [
            ("hls.mp4", hls.to_owned()),
            ("hls.m3u8", hls.to_owned()),
            ("rel.mp4", "ffconcat version 1.0\nfile 'ok.mp4'\n".to_owned()),
            ("abs.mp4", format!("ffconcat version 1.0\nfile '{}'\n", ok.display())),
        ];
        for (name, body) in cases {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            assert!(probe(&p).is_err(), "{name}: ffprobe hat die Datei angenommen");
            assert!(crate::audio::decode_with_ffmpeg(&p).is_err(), "{name}: ffmpeg hat die Datei dekodiert");
        }
        // Ein echtes Video geht weiterhin.
        if matches!(made, Ok(s) if s.success()) {
            assert!(probe(&ok).is_ok(), "echtes MP4 abgelehnt");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_output_is_parsed() {
        let text = "codec_type=video\nwidth=1920\nheight=1080\nr_frame_rate=30000/1001\navg_frame_rate=30000/1001\nduration=12.500000\ncodec_type=audio\nr_frame_rate=0/0\navg_frame_rate=0/0\nduration=12.480000\nduration=12.512000\n";
        let i = parse_probe(text).unwrap();
        assert_eq!((i.width, i.height), (1920, 1080));
        assert!((i.fps - 29.97).abs() < 0.01);
        assert!((i.duration - 12.512).abs() < 0.001);
        assert!(i.has_audio);
    }

    #[test]
    fn portrait_rotation_swaps_sides() {
        let i = parse_probe("codec_type=video\nwidth=1920\nheight=1080\navg_frame_rate=30/1\nrotation=-90\nduration=3.0\n").unwrap();
        assert_eq!((i.width, i.height), (1080, 1920));
        assert!(!i.has_audio);
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(parse_probe("codec_type=audio\nduration=3.0\n").is_none());
        assert!(parse_probe("").is_none());
    }

    #[test]
    fn fit_keeps_aspect_and_even_sides() {
        assert_eq!(fit(1920, 1080), (1280, 720));
        assert_eq!(fit(1080, 1920), (404, 720));
        assert_eq!(fit(320, 240), (320, 240));
        let (w, h) = fit(1279, 717);
        assert!(w % 2 == 0 && h % 2 == 0);
    }

    /// Echtes Dekodieren, wenn ffmpeg vorhanden ist: Testvideo erzeugen, öffnen, Bilder lesen.
    #[test]
    fn decodes_generated_video() {
        let dir = std::env::temp_dir().join(format!("wasnap-video-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.mp4");
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=25:duration=2", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-shortest", "-pix_fmt", "yuv420p"])
            .arg(&p)
            .status();
        if !matches!(made, Ok(s) if s.success()) {
            return; // kein ffmpeg oder kein Encoder: nichts zu prüfen
        }
        let mut pl = Player::with_info(&p, probe(&p).unwrap());
        assert_eq!(pl.size, (320, 240));
        assert!((pl.info.duration - 2.0).abs() < 0.2, "{}", pl.info.duration);
        assert!(pl.info.has_audio);
        pl.volume = 0.0; // im Test keinen Ton ausgeben
        pl.seek(1.0);
        let t0 = Instant::now();
        let mut got = None;
        while got.is_none() && t0.elapsed().as_secs() < 10 {
            got = pl.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let f = got.expect("kein Bild von ffmpeg");
        assert_eq!(f.rgba.len(), 320 * 240 * 4);
        assert!((f.at - 1.0).abs() < 0.1);
        assert!(!pl.playing(), "nach einem Sprung im Pausenzustand bleibt der Player angehalten");
        assert!(f.rgba.iter().any(|b| *b > 40), "Bild ist schwarz");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
