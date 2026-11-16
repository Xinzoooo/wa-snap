//! Audio für Sprachanrufe: Mikrofon und Lautsprecher als rohe 16-Bit-PCM-Ströme über
//! PipeWire, PulseAudio oder ALSA.
//!
//! WhatsApp-Anrufe laufen mit 16 kHz mono; die Bibliothek erwartet und liefert Blöcke
//! à 960 Samples = 60 ms (siehe `whatsapp_rust::voip::AudioSource`). Mit 48 kHz wäre die
//! eigene Stimme beim Gegenüber dreimal zu langsam und das Gegenüber hier dreimal zu schnell.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use whatsapp_rust::voip::{AudioSink, AudioSource};

/// Abtastrate der Anrufe.
pub const CALL_RATE: u32 = 16_000;
/// Samples je Block (60 ms bei 16 kHz).
const FRAME: usize = 960;
/// Höchstens so viele Blöcke stauen (Mikrofon ~180 ms, Lautsprecher ~300 ms); was darüber
/// hinausgeht, wird verworfen, damit das Gespräch nicht immer weiter nachhinkt.
const MIC_QUEUE: usize = 3;
const SPK_QUEUE: usize = 5;

/// Mitschnitt zur Fehlersuche: mit `WA_SNAP_CALL_DUMP=1` landen beide Richtungen als WAV in /tmp.
struct Dump(Option<(std::fs::File, u32)>);

impl Dump {
    fn new(name: &str) -> Self {
        if std::env::var_os("WA_SNAP_CALL_DUMP").is_none() {
            return Dump(None);
        }
        let path = std::env::temp_dir().join(name);
        let file = std::fs::File::create(&path).and_then(|mut f| {
            f.write_all(&wav_header(0))?;
            Ok(f)
        });
        match file {
            Ok(f) => {
                crate::backend::log(format!("Anruf-Mitschnitt: {}", path.display()));
                Dump(Some((f, 0)))
            }
            Err(e) => {
                crate::backend::log(format!("Anruf-Mitschnitt nicht möglich: {e}"));
                Dump(None)
            }
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        if let Some((f, n)) = self.0.as_mut() {
            if f.write_all(bytes).is_ok() {
                *n += bytes.len() as u32;
            }
        }
    }
}

impl Drop for Dump {
    fn drop(&mut self) {
        use std::io::Seek;
        if let Some((mut f, n)) = self.0.take() {
            if f.seek(std::io::SeekFrom::Start(0)).is_ok() {
                let _ = f.write_all(&wav_header(n));
            }
        }
    }
}

/// WAV-Kopf für 16 Bit mono mit [`CALL_RATE`].
fn wav_header(data_len: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data_len).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes()); // PCM
    h.extend_from_slice(&1u16.to_le_bytes()); // mono
    h.extend_from_slice(&CALL_RATE.to_le_bytes());
    h.extend_from_slice(&(CALL_RATE * 2).to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data_len.to_le_bytes());
    h
}

/// Ersten funktionierenden Befehl starten (prüft kurz, ob er sofort wieder beendet ist).
fn spawn_raw(cands: &[(&str, &[&str])], record: bool) -> Result<Child, String> {
    let mut last = String::from("kein Audioprogramm gefunden (pw-record/parec/arecord)");
    for (cmd, args) in cands {
        let mut c = Command::new(cmd);
        c.args(*args).stderr(Stdio::null());
        if record {
            c.stdout(Stdio::piped()).stdin(Stdio::null());
        } else {
            c.stdin(Stdio::piped()).stdout(Stdio::null());
        }
        match c.spawn() {
            Ok(mut child) => {
                std::thread::sleep(std::time::Duration::from_millis(150));
                if let Ok(Some(st)) = child.try_wait() {
                    last = format!("{cmd}: beendet ({st})");
                    continue;
                }
                return Ok(child);
            }
            Err(e) => last = format!("{cmd}: {e}"),
        }
    }
    Err(last)
}

/// Laufende Audio-Prozesse eines Anrufs; beim Auflegen beenden.
#[derive(Clone, Default)]
pub struct Procs(Arc<Mutex<Vec<Child>>>);

impl Procs {
    pub fn stop(&self) {
        if let Ok(mut v) = self.0.lock() {
            for mut c in v.drain(..) {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

/// Stummschalten: Mikrofon liefert Stille.
pub static MUTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub struct Mic {
    rx: async_channel::Receiver<Vec<i16>>,
}

impl AudioSource for Mic {
    fn frames(&self) -> async_channel::Receiver<Vec<i16>> {
        self.rx.clone()
    }
}

pub struct Speaker {
    tx: async_channel::Sender<Vec<i16>>,
}

impl AudioSink for Speaker {
    fn playout(&self) -> async_channel::Sender<Vec<i16>> {
        self.tx.clone()
    }
}

/// Mikrofon und Lautsprecher öffnen.
pub fn open(procs: &Procs) -> Result<(Mic, Speaker), String> {
    MUTED.store(false, std::sync::atomic::Ordering::Relaxed);
    let rec_args: &[(&str, &[&str])] = &[
        ("pw-record", &["--raw", "--rate", "16000", "--channels", "1", "--format", "s16", "--latency", "20ms", "-"]),
        ("parec", &["--raw", "--format=s16le", "--rate=16000", "--channels=1", "--latency-msec=20"]),
        ("arecord", &["-q", "-t", "raw", "-f", "S16_LE", "-r", "16000", "-c", "1"]),
    ];
    let play_args: &[(&str, &[&str])] = &[
        ("pw-play", &["--raw", "--rate", "16000", "--channels", "1", "--format", "s16", "--latency", "60ms", "-"]),
        ("pacat", &["--raw", "--format=s16le", "--rate=16000", "--channels=1", "--latency-msec=60"]),
        ("aplay", &["-q", "-t", "raw", "-f", "S16_LE", "-r", "16000", "-c", "1"]),
    ];
    let mut rec = spawn_raw(rec_args, true)?;
    let mut play = match spawn_raw(play_args, false) {
        Ok(p) => p,
        Err(e) => {
            let _ = rec.kill();
            return Err(e);
        }
    };
    let (mic_tx, mic_rx) = async_channel::bounded::<Vec<i16>>(MIC_QUEUE);
    let (spk_tx, spk_rx) = async_channel::bounded::<Vec<i16>>(SPK_QUEUE);
    let mut out = rec.stdout.take().ok_or("Mikrofon ohne Ausgabe")?;
    let mut inp = play.stdin.take().ok_or("Lautsprecher ohne Eingang")?;
    std::thread::spawn(move || {
        let mut dump = Dump::new("wa-snap-call-mic.wav");
        let mut dc = crate::audio::DcBlock::default();
        let mut buf = vec![0u8; FRAME * 2];
        loop {
            if out.read_exact(&mut buf).is_err() {
                break;
            }
            let mut frame: Vec<i16> = buf.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
            dc.run(&mut frame);
            if MUTED.load(std::sync::atomic::Ordering::Relaxed) {
                frame.fill(0);
            }
            let bytes: Vec<u8> = frame.iter().flat_map(|s| s.to_le_bytes()).collect();
            dump.write(&bytes);
            // volle Warteschlange: Block verwerfen statt zu verzögern
            if mic_tx.try_send(frame).is_err() && mic_tx.is_closed() {
                break;
            }
        }
    });
    std::thread::spawn(move || {
        let mut dump = Dump::new("wa-snap-call-speaker.wav");
        while let Ok(frame) = spk_rx.recv_blocking() {
            let bytes: Vec<u8> = frame.iter().flat_map(|s| s.to_le_bytes()).collect();
            dump.write(&bytes);
            if inp.write_all(&bytes).is_err() {
                break;
            }
        }
    });
    if let Ok(mut v) = procs.0.lock() {
        v.push(rec);
        v.push(play);
    }
    Ok((Mic { rx: mic_rx }, Speaker { tx: spk_tx }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_is_60_ms_at_call_rate() {
        // Die Bibliothek verwirft jeden Block, der nicht genau 960 Samples bei 16 kHz hat.
        assert_eq!(FRAME, 960);
        assert_eq!(FRAME as u32 * 1000 / CALL_RATE, 60);
    }

    #[test]
    fn wav_header_describes_16k_mono() {
        let h = wav_header(32_000);
        assert_eq!(h.len(), 44);
        assert_eq!(u32::from_le_bytes([h[24], h[25], h[26], h[27]]), 16_000);
        assert_eq!(u16::from_le_bytes([h[22], h[23]]), 1);
        assert_eq!(u32::from_le_bytes([h[40], h[41], h[42], h[43]]), 32_000);
    }
}
