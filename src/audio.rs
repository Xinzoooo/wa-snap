//! Sprachnachrichten: Ogg/Opus dekodieren und kodieren (libopus ist eingebaut),
//! abspielen und aufnehmen über PipeWire (`pw-play`/`pw-record`), ersatzweise
//! PulseAudio (`paplay`/`parecord`) oder ALSA (`aplay`/`arecord`).

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const RATE: u32 = 48_000;

/// Ogg/Opus-Datei zu 16-Bit-PCM (48 kHz). Gibt (Samples, Kanäle) zurück.
/// Längste Sprachnachricht, die dekodiert wird (WhatsApp selbst erlaubt deutlich weniger).
pub const MAX_VOICE_SECS: usize = 30 * 60;

pub fn decode_ogg_opus(bytes: &[u8]) -> Result<(Vec<i16>, usize), String> {
    let mut reader = ogg::PacketReader::new(Cursor::new(bytes));
    let head = reader.read_packet().map_err(|e| e.to_string())?.ok_or("leere Datei")?;
    if !head.data.starts_with(b"OpusHead") || head.data.len() < 19 {
        return Err("keine Opus-Datei".into());
    }
    let channels = head.data[9].clamp(1, 2) as usize;
    let pre_skip = u16::from_le_bytes([head.data[10], head.data[11]]) as usize;
    let ch = if channels == 2 { opus::Channels::Stereo } else { opus::Channels::Mono };
    let mut dec = opus::Decoder::new(RATE, ch).map_err(|e| e.to_string())?;
    let mut pcm: Vec<i16> = Vec::new();
    let mut buf = vec![0i16; 5760 * channels];
    let limit = MAX_VOICE_SECS * RATE as usize * channels;
    while let Some(p) = reader.read_packet().map_err(|e| e.to_string())? {
        if p.data.starts_with(b"OpusTags") {
            continue;
        }
        match dec.decode(&p.data, &mut buf, false) {
            Ok(n) => pcm.extend_from_slice(&buf[..n * channels]),
            Err(_) => continue,
        }
        // Winzige Pakete können sich auf das Tausendfache aufblähen: harte Obergrenze.
        if pcm.len() > limit {
            return Err(format!("länger als {} Minuten", MAX_VOICE_SECS / 60));
        }
    }
    let skip = (pre_skip * channels).min(pcm.len());
    pcm.drain(..skip);
    Ok((pcm, channels))
}

/// 16-Bit-PCM (mono, 48 kHz) zu Ogg/Opus, wie WhatsApp es für Sprachnachrichten erwartet.
pub fn encode_ogg_opus(pcm: &[i16]) -> Result<Vec<u8>, String> {
    let mut enc = opus::Encoder::new(RATE, opus::Channels::Mono, opus::Application::Voip).map_err(|e| e.to_string())?;
    let _ = enc.set_bitrate(opus::Bitrate::Bits(32_000));
    let pre_skip = enc.get_lookahead().unwrap_or(312).max(0) as u16;
    let mut out = Vec::new();
    {
        let mut w = ogg::PacketWriter::new(&mut out);
        let serial = 0x5741_534e; // "WASN"
        let mut head = b"OpusHead".to_vec();
        head.push(1); // Version
        head.push(1); // Kanäle
        head.extend_from_slice(&pre_skip.to_le_bytes());
        head.extend_from_slice(&RATE.to_le_bytes());
        head.extend_from_slice(&0u16.to_le_bytes()); // Verstärkung
        head.push(0); // Kanalzuordnung
        w.write_packet(head, serial, ogg::PacketWriteEndInfo::EndPage, 0).map_err(|e| e.to_string())?;
        let vendor = b"WhatsSnap";
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        w.write_packet(tags, serial, ogg::PacketWriteEndInfo::EndPage, 0).map_err(|e| e.to_string())?;
        const FRAME: usize = 960; // 20 ms
        let frames = pcm.len().div_ceil(FRAME).max(1);
        let mut granule = pre_skip as u64;
        let mut buf = vec![0u8; 4000];
        for i in 0..frames {
            let mut frame = [0i16; FRAME];
            let start = i * FRAME;
            let end = (start + FRAME).min(pcm.len());
            if start < end {
                frame[..end - start].copy_from_slice(&pcm[start..end]);
            }
            let n = enc.encode(&frame, &mut buf).map_err(|e| e.to_string())?;
            granule += FRAME as u64;
            let info = if i + 1 == frames {
                ogg::PacketWriteEndInfo::EndStream
            } else if (i + 1) % 50 == 0 {
                ogg::PacketWriteEndInfo::EndPage
            } else {
                ogg::PacketWriteEndInfo::NormalPacket
            };
            w.write_packet(buf[..n].to_vec(), serial, info, granule).map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

/// Einfache Lautstärkekurve (64 Werte 0..=100) für die Anzeige und für WhatsApp.
pub fn waveform(pcm: &[i16], channels: usize) -> Vec<u8> {
    let n = 64;
    let step = (pcm.len() / channels.max(1) / n).max(1);
    let mut v: Vec<f32> = (0..n)
        .map(|i| {
            let s = i * step * channels;
            let e = ((i + 1) * step * channels).min(pcm.len());
            if s >= e {
                return 0.0;
            }
            let sum: f64 = pcm[s..e].iter().map(|x| (*x as f64).powi(2)).sum();
            ((sum / (e - s) as f64).sqrt() as f32).max(1.0)
        })
        .collect();
    let max = v.iter().cloned().fold(1.0, f32::max);
    for x in v.iter_mut() {
        *x = (*x / max * 100.0).clamp(0.0, 100.0);
    }
    v.into_iter().map(|x| x as u8).collect()
}

fn wav_bytes(pcm: &[i16], channels: usize) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&(channels as u16).to_le_bytes());
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * channels as u32 * 2).to_le_bytes());
    b.extend_from_slice(&((channels * 2) as u16).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

/// WAV lesen, auch wenn der Kopf nach einem Abbruch nicht fertig geschrieben wurde.
/// Gibt Mono-PCM mit 48 kHz zurück (andere Raten werden einfach umgerechnet).
pub fn read_wav_mono(path: &Path) -> Result<Vec<i16>, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut b = Vec::new();
    f.read_to_end(&mut b).map_err(|e| e.to_string())?;
    if b.len() < 44 || &b[..4] != b"RIFF" {
        return Err("keine WAV-Datei".into());
    }
    let mut p = 12;
    let (mut ch, mut rate, mut bits) = (1usize, RATE, 16u16);
    let mut data = &b[b.len()..];
    while p + 8 <= b.len() {
        let id = &b[p..p + 4];
        let len = u32::from_le_bytes([b[p + 4], b[p + 5], b[p + 6], b[p + 7]]) as usize;
        let body = p + 8;
        if id == b"fmt " && body + 16 <= b.len() {
            ch = u16::from_le_bytes([b[body + 2], b[body + 3]]).max(1) as usize;
            rate = u32::from_le_bytes([b[body + 4], b[body + 5], b[body + 6], b[body + 7]]);
            bits = u16::from_le_bytes([b[body + 14], b[body + 15]]);
        }
        if id == b"data" {
            let end = if len == 0 || body + len > b.len() { b.len() } else { body + len };
            data = &b[body..end];
            break;
        }
        p = body + len + (len & 1);
    }
    if bits != 16 {
        return Err(format!("{bits}-Bit-Aufnahme wird nicht unterstützt"));
    }
    let samples: Vec<i16> = data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
    let mono: Vec<i16> = samples.chunks(ch).map(|c| (c.iter().map(|x| *x as i32).sum::<i32>() / c.len() as i32) as i16).collect();
    if rate == RATE || rate == 0 {
        return Ok(mono);
    }
    let ratio = rate as f64 / RATE as f64;
    let n = (mono.len() as f64 / ratio) as usize;
    Ok((0..n).map(|i| mono[((i as f64 * ratio) as usize).min(mono.len() - 1)]).collect())
}

fn spawn_first(cands: &[(&str, Vec<String>)]) -> Result<Child, String> {
    let mut last = String::from("kein Audioprogramm gefunden (pw-play/paplay/aplay)");
    for (cmd, args) in cands {
        match Command::new(cmd).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(c) => return Ok(c),
            Err(e) => last = format!("{cmd}: {e}"),
        }
    }
    Err(last)
}

enum PlayState {
    Pending,
    Running(Child),
    Failed(String),
    Stopped,
}

/// Läuft gerade eine Wiedergabe?
pub struct Playback {
    pub id: String,
    state: Arc<Mutex<PlayState>>,
    wav: PathBuf,
    started: Instant,
    pub offset: f32,
    pub duration: f32,
}

impl Playback {
    pub fn position(&self) -> f32 {
        (self.offset + self.started.elapsed().as_secs_f32()).min(self.duration)
    }

    pub fn finished(&mut self) -> bool {
        let done = match self.state.lock().as_deref_mut() {
            Ok(PlayState::Running(c)) => matches!(c.try_wait(), Ok(Some(_))),
            Ok(PlayState::Failed(_)) | Ok(PlayState::Stopped) | Err(_) => true,
            Ok(PlayState::Pending) => false,
        };
        done || self.position() >= self.duration + 0.5
    }

    /// Fehlermeldung, falls der Player nicht starten konnte.
    pub fn error(&self) -> Option<String> {
        match self.state.lock().as_deref() {
            Ok(PlayState::Failed(e)) => Some(e.clone()),
            _ => None,
        }
    }

    pub fn stop(self) {
        if let Ok(mut g) = self.state.lock() {
            if let PlayState::Running(c) = &mut *g {
                let _ = c.kill();
                let _ = c.wait();
                let _ = std::fs::remove_file(&self.wav);
            }
            *g = PlayState::Stopped;
        }
    }
}

/// Dekodierte Sprachnachricht ab `offset` Sekunden abspielen.
/// WAV-Datei schreiben und Player starten passiert in einem eigenen Thread,
/// damit lange Nachrichten das Fenster nicht einfrieren.
pub fn play(id: &str, pcm: &std::sync::Arc<Vec<i16>>, channels: usize, offset: f32, tmp: &Path) -> Result<Playback, String> {
    let duration = pcm.len() as f32 / channels as f32 / RATE as f32;
    let start = ((offset.max(0.0) * RATE as f32) as usize * channels).min(pcm.len());
    std::fs::create_dir_all(tmp).map_err(|e| e.to_string())?;
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let wav = tmp.join(format!("wiedergabe-{}-{n}.wav", std::process::id()));
    let state: Arc<Mutex<PlayState>> = Arc::new(Mutex::new(PlayState::Pending));
    let (st, pcm, path) = (state.clone(), pcm.clone(), wav.clone());
    std::thread::spawn(move || {
        let r = std::fs::write(&path, wav_bytes(&pcm[start..], channels)).map_err(|e| e.to_string()).and_then(|_| {
            let w = path.to_string_lossy().to_string();
            spawn_first(&[("pw-play", vec![w.clone()]), ("paplay", vec![w.clone()]), ("aplay", vec!["-q".into(), w])])
        });
        let Ok(mut g) = st.lock() else { return };
        match (r, &*g) {
            // Schon gestoppt, bevor der Player lief: sofort wieder beenden.
            (Ok(mut c), PlayState::Stopped) => {
                let _ = c.kill();
                let _ = c.wait();
                let _ = std::fs::remove_file(&path);
            }
            (Ok(c), _) => *g = PlayState::Running(c),
            (Err(e), _) => {
                let _ = std::fs::remove_file(&path);
                *g = PlayState::Failed(e)
            }
        }
    });
    Ok(Playback { id: id.to_owned(), state, wav, started: Instant::now(), offset, duration })
}

/// Laufende Aufnahme.
pub struct Recording {
    child: Child,
    pub path: PathBuf,
    pub started: Instant,
}

pub fn start_recording(tmp: &Path) -> Result<Recording, String> {
    let _ = std::fs::create_dir_all(tmp);
    let path = tmp.join(format!("aufnahme-{}.wav", chrono::Utc::now().timestamp_millis()));
    let p = path.to_string_lossy().to_string();
    let child = spawn_first(&[
        ("pw-record", vec!["--rate".into(), RATE.to_string(), "--channels".into(), "1".into(), "--format".into(), "s16".into(), p.clone()]),
        ("parecord", vec!["--rate=48000".into(), "--channels=1".into(), "--format=s16le".into(), "--file-format=wav".into(), p.clone()]),
        ("arecord", vec!["-q".into(), "-f".into(), "S16_LE".into(), "-r".into(), "48000".into(), "-c".into(), "1".into(), p]),
    ])?;
    Ok(Recording { child, path, started: Instant::now() })
}

impl Recording {
    pub fn seconds(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }

    /// Aufnahme beenden (sauber per SIGINT, damit die Datei vollständig ist) und als PCM lesen.
    pub fn finish(mut self) -> Result<Vec<i16>, String> {
        let _ = Command::new("kill").args(["-INT", &self.child.id().to_string()]).status();
        for _ in 0..30 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let pcm = read_wav_mono(&self.path);
        let _ = std::fs::remove_file(&self.path);
        pcm
    }

    pub fn cancel(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_roundtrip() {
        // 1 Sekunde 440-Hz-Ton
        let pcm: Vec<i16> = (0..RATE as usize).map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin() * 8000.0) as i16).collect();
        let ogg = encode_ogg_opus(&pcm).unwrap();
        assert!(ogg.starts_with(b"OggS"));
        let (back, ch) = decode_ogg_opus(&ogg).unwrap();
        assert_eq!(ch, 1);
        let diff = (back.len() as i64 - pcm.len() as i64).abs();
        assert!(diff < 2000, "Länge {} statt {}", back.len(), pcm.len());
        let energy: f64 = back.iter().map(|x| (*x as f64).abs()).sum::<f64>() / back.len() as f64;
        assert!(energy > 1000.0, "zu leise: {energy}");
    }

    /// Viele winzige Pakete dürfen nicht zu unbegrenztem Speicher führen.
    #[test]
    fn opus_bomb_is_rejected() {
        let mut enc = opus::Encoder::new(RATE, opus::Channels::Mono, opus::Application::Voip).unwrap();
        let mut buf = vec![0u8; 4000];
        let n = enc.encode(&[0i16; 960], &mut buf).unwrap();
        let pkt = buf[..n].to_vec();
        let mut out = Vec::new();
        {
            let mut w = ogg::PacketWriter::new(&mut out);
            let mut head = b"OpusHead".to_vec();
            head.extend_from_slice(&[1, 1, 0, 0]);
            head.extend_from_slice(&RATE.to_le_bytes());
            head.extend_from_slice(&[0, 0, 0]);
            w.write_packet(head, 1, ogg::PacketWriteEndInfo::EndPage, 0).unwrap();
            let frames = MAX_VOICE_SECS * 50 + 500; // 20-ms-Pakete, etwas über dem Limit
            for i in 0..frames {
                let info = if i % 200 == 199 { ogg::PacketWriteEndInfo::EndPage } else { ogg::PacketWriteEndInfo::NormalPacket };
                w.write_packet(pkt.clone(), 1, info, (i as u64 + 1) * 960).unwrap();
            }
        }
        assert!(out.len() < 8_000_000, "Testdatei sollte klein sein: {}", out.len());
        assert!(decode_ogg_opus(&out).is_err());
    }

    #[test]
    fn wav_roundtrip() {
        let pcm: Vec<i16> = (0..4800).map(|i| (i % 300) as i16 * 10).collect();
        let dir = std::env::temp_dir().join("wasnap-test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("t.wav");
        std::fs::write(&p, wav_bytes(&pcm, 1)).unwrap();
        assert_eq!(read_wav_mono(&p).unwrap(), pcm);
    }
}
