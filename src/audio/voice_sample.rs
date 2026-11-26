//! Independent WASAPI shared-mode input for local voice recording and microphone level checks.
use super::{ComApartment, INPUT_RATE, device, init};
use anyhow::{Result, bail};
use eframe::egui;
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
use wasapi::Direction;

const BYTES_PER_SECOND: usize = INPUT_RATE * 2;
pub const LIMIT_SECONDS: u32 = 120;
pub const MIN_SECONDS: u32 = 3;

pub enum CaptureMode {
    Record(u32),
    Meter,
}
#[derive(Default, Clone, Copy)]
pub struct Progress {
    pub frames: usize,
    pub level: u32,
    pub peak: u32,
}
pub struct Capture {
    progress: Arc<Mutex<Progress>>,
    stop: Arc<AtomicBool>,
    pub result: mpsc::Receiver<Result<Vec<u8>>>,
}
impl Capture {
    pub fn start(mic: String, mode: CaptureMode, ctx: egui::Context) -> Self {
        let (tx, result) = mpsc::channel();
        let progress = Arc::new(Mutex::new(Progress::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (state, done) = (progress.clone(), stop.clone());
        std::thread::spawn(move || {
            let _ = tx.send(read(&mic, mode, &state, &done));
            ctx.request_repaint();
        });
        Self {
            progress,
            stop,
            result,
        }
    }
    pub fn progress(&self) -> Progress {
        *self.progress.lock()
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn seconds(wav: &[u8]) -> u32 {
    (wav.len().saturating_sub(44) / BYTES_PER_SECOND) as u32
}
fn wav(pcm: Vec<u8>) -> Vec<u8> {
    let size = pcm.len() as u32;
    let mut out = Vec::with_capacity(pcm.len() + 44);
    out.extend(b"RIFF");
    out.extend((36 + size).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend((INPUT_RATE as u32).to_le_bytes());
    out.extend((BYTES_PER_SECOND as u32).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(size.to_le_bytes());
    out.extend(pcm);
    out
}
fn read(
    mic: &str,
    mode: CaptureMode,
    progress: &Mutex<Progress>,
    stop: &AtomicBool,
) -> Result<Vec<u8>> {
    let _com = ComApartment::open()?;
    let client = device(Direction::Capture, mic)?.get_iaudioclient()?;
    capture_client(client, mode, progress, stop)
}
fn capture_client(
    mut client: wasapi::AudioClient,
    mode: CaptureMode,
    progress: &Mutex<Progress>,
    stop: &AtomicBool,
) -> Result<Vec<u8>> {
    init(&mut client, Direction::Capture, INPUT_RATE)?;
    let event = client.set_get_eventhandle()?;
    let reader = client.get_audiocaptureclient()?;
    let mut packet = Vec::new();
    let mut pcm = Vec::new();
    let limit = match mode {
        CaptureMode::Record(seconds) => {
            Some(seconds.min(LIMIT_SECONDS) as usize * BYTES_PER_SECOND)
        }
        CaptureMode::Meter => None,
    };
    client.start_stream()?;
    let mut last = Instant::now();
    while !stop.load(Ordering::Acquire) && limit.is_none_or(|n| pcm.len() < n) {
        if last.elapsed() > Duration::from_secs(4) {
            bail!("Microphone stopped responding");
        }
        if event.wait_for_event(100).is_err() {
            continue;
        }
        while !stop.load(Ordering::Acquire) {
            let frames = reader.get_next_packet_size()?.unwrap_or(0) as usize;
            if frames == 0 {
                break;
            }
            packet.resize(frames * 2, 0);
            let (frames, info) = reader.read_from_device(&mut packet)?;
            let bytes = &mut packet[..frames as usize * 2];
            if bytes.is_empty() {
                break;
            }
            last = Instant::now();
            if info.flags.silent {
                bytes.fill(0);
            }
            let sum: f64 = bytes
                .chunks_exact(2)
                .map(|b| f64::from(i16::from_le_bytes([b[0], b[1]])).powi(2))
                .sum();
            let level = (sum / frames as f64).sqrt() as u32;
            let used = if let Some(limit) = limit {
                let n = (limit - pcm.len()).min(bytes.len());
                pcm.extend_from_slice(&bytes[..n]);
                n
            } else {
                bytes.len()
            };
            let mut state = progress.lock();
            state.frames += used / 2;
            state.level = level;
            state.peak = state.peak.max(level);
            if limit == Some(pcm.len()) {
                break;
            }
        }
    }
    client.stop_stream()?;
    Ok(if limit.is_some() {
        wav(pcm)
    } else {
        Vec::new()
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Uses only this process's silent WASAPI render stream as the recording source"]
    fn native_record_limit_and_manual_stop() {
        let _com = ComApartment::open().unwrap();
        for manual in [true, false] {
            let done = Arc::new(AtomicBool::new(false));
            let stop = done.clone();
            let state = Arc::new(Mutex::new(Progress::default()));
            let observed = state.clone();
            let recorder = std::thread::spawn(move || {
                let _com = ComApartment::open().unwrap();
                let input =
                    wasapi::AudioClient::new_application_loopback_client(std::process::id(), true)
                        .unwrap();
                capture_client(
                    input,
                    CaptureMode::Record(if manual { LIMIT_SECONDS } else { MIN_SECONDS }),
                    &observed,
                    &stop,
                )
                .unwrap()
            });
            let mut output = device(Direction::Render, "")
                .unwrap()
                .get_iaudioclient()
                .unwrap();
            init(&mut output, Direction::Render, 24000).unwrap();
            let event = output.set_get_eventhandle().unwrap();
            let render = output.get_audiorenderclient().unwrap();
            output.start_stream().unwrap();
            let started = Instant::now();
            while !recorder.is_finished() {
                assert!(
                    started.elapsed() < Duration::from_secs(12),
                    "loopback timed out"
                );
                event.wait_for_event(500).unwrap();
                let bytes = output.get_available_space_in_frames().unwrap() as usize * 2;
                if bytes > 0 {
                    render.write_to_device(&vec![0; bytes], None).unwrap();
                }
                if manual && state.lock().frames >= INPUT_RATE * 3 {
                    done.store(true, Ordering::Release);
                }
            }
            output.stop_stream().unwrap();
            let wav = recorder.join().unwrap();
            assert_eq!(seconds(&wav), MIN_SECONDS);
            if !manual {
                assert_eq!(wav.len(), 44 + BYTES_PER_SECOND * 3);
            }
        }
    }
    #[test]
    fn voice_sample_wav_and_duration() {
        let pcm: Vec<_> = [0u8, 1, 128, 255]
            .into_iter()
            .cycle()
            .take(BYTES_PER_SECOND * 3)
            .collect();
        let bytes = wav(pcm.clone());
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 48000);
        assert_eq!(&bytes[44..], pcm);
        assert_eq!(seconds(&bytes), MIN_SECONDS);
    }
}
