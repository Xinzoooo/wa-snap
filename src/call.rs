//! Audio für Sprachanrufe: Mikrofon und Lautsprecher als rohe 16-Bit-PCM-Ströme
//! (48 kHz, mono, Blöcke à 960 Samples = 20 ms) über PipeWire, PulseAudio oder ALSA.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use whatsapp_rust::voip::{AudioSink, AudioSource};

const FRAME: usize = 960;

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
        ("pw-record", &["--raw", "--rate", "48000", "--channels", "1", "--format", "s16", "--latency", "20ms", "-"]),
        ("parec", &["--raw", "--format=s16le", "--rate=48000", "--channels=1", "--latency-msec=20"]),
        ("arecord", &["-q", "-t", "raw", "-f", "S16_LE", "-r", "48000", "-c", "1"]),
    ];
    let play_args: &[(&str, &[&str])] = &[
        ("pw-play", &["--raw", "--rate", "48000", "--channels", "1", "--format", "s16", "--latency", "60ms", "-"]),
        ("pacat", &["--raw", "--format=s16le", "--rate=48000", "--channels=1", "--latency-msec=60"]),
        ("aplay", &["-q", "-t", "raw", "-f", "S16_LE", "-r", "48000", "-c", "1"]),
    ];
    let mut rec = spawn_raw(rec_args, true)?;
    let mut play = match spawn_raw(play_args, false) {
        Ok(p) => p,
        Err(e) => {
            let _ = rec.kill();
            return Err(e);
        }
    };
    let (mic_tx, mic_rx) = async_channel::bounded::<Vec<i16>>(50);
    let (spk_tx, spk_rx) = async_channel::bounded::<Vec<i16>>(50);
    let mut out = rec.stdout.take().ok_or("Mikrofon ohne Ausgabe")?;
    let mut inp = play.stdin.take().ok_or("Lautsprecher ohne Eingang")?;
    std::thread::spawn(move || {
        let mut buf = vec![0u8; FRAME * 2];
        loop {
            if out.read_exact(&mut buf).is_err() {
                break;
            }
            let frame: Vec<i16> = if MUTED.load(std::sync::atomic::Ordering::Relaxed) {
                vec![0; FRAME]
            } else {
                buf.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
            };
            // volle Warteschlange: Block verwerfen statt zu verzögern
            if mic_tx.try_send(frame).is_err() && mic_tx.is_closed() {
                break;
            }
        }
    });
    std::thread::spawn(move || {
        while let Ok(frame) = spk_rx.recv_blocking() {
            let bytes: Vec<u8> = frame.iter().flat_map(|s| s.to_le_bytes()).collect();
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
