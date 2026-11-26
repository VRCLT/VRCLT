use super::ComApartment;
use anyhow::{Result, ensure};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};
use wasapi::*;

pub enum Found {
    Devices(Vec<String>),
    Speaking(Option<String>),
}
pub struct Search {
    pub result: Receiver<Result<Found>>,
    cancelled: Arc<AtomicBool>,
}
impl Search {
    pub fn start(listen: bool) -> Self {
        let (tx, result) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        thread::spawn(move || {
            let _ = tx.send(scan(listen, &stop));
        });
        Self { result, cancelled }
    }
}
impl Drop for Search {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

// Measure 50 ms RMS blocks, and require three in a row above the endpoint's own floor.
struct Level {
    floor: f32,
    peak: f32,
    recent: [f32; 3],
    sum: f64,
    frames: usize,
}
impl Default for Level {
    fn default() -> Self {
        Self {
            floor: f32::INFINITY,
            peak: 0.,
            recent: [0.; 3],
            sum: 0.,
            frames: 0,
        }
    }
}
impl Level {
    fn block(&mut self, rms: f32) {
        self.floor = self.floor.min(rms);
        self.recent.rotate_left(1);
        self.recent[2] = rms;
        self.peak = self
            .peak
            .max(self.recent.into_iter().fold(f32::INFINITY, f32::min));
    }
    fn sample(&mut self, v: i16) {
        self.sum += f64::from(v).powi(2);
        self.frames += 1;
        if self.frames == 800 {
            self.block((self.sum / 800.).sqrt() as f32);
            self.frames = 0;
            self.sum = 0.;
        }
    }
    fn score(&self) -> Option<f32> {
        let rise = self.peak - self.floor;
        (self.peak >= 150. && rise >= 100.).then_some(rise)
    }
}
struct Input {
    client: AudioClient,
    capture: AudioCaptureClient,
    name: String,
    level: Level,
    packet: Vec<u8>,
}
impl Input {
    fn open(device: &Device, name: String) -> Result<Self> {
        let mut client = device.get_iaudioclient()?;
        client.initialize_client(
            &WaveFormat::new(16, 16, SampleType::Int, 16000, 1, None),
            Direction::Capture,
            StreamMode::PollingShared {
                autoconvert: true,
                buffer_duration_hns: 2_000_000,
            },
            None,
        )?;
        let capture = client.get_audiocaptureclient()?;
        client.start_stream()?;
        Ok(Self {
            client,
            capture,
            name,
            level: Level::default(),
            packet: Vec::new(),
        })
    }
    fn poll(&mut self, settled: bool) -> Result<()> {
        while let Some(n) = self.capture.get_next_packet_size()? {
            if n == 0 {
                break;
            }
            self.packet.resize(n as usize * 2, 0);
            let (frames, info) = self.capture.read_from_device(&mut self.packet)?;
            if settled {
                for s in self.packet[..frames as usize * 2].chunks_exact(2) {
                    self.level.sample(if info.flags.silent {
                        0
                    } else {
                        i16::from_le_bytes([s[0], s[1]])
                    });
                }
            }
        }
        Ok(())
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        let _ = self.client.stop_stream();
    }
}

fn scan(listen: bool, stop: &AtomicBool) -> Result<Found> {
    let _com = ComApartment::open()?;
    let e = DeviceEnumerator::new()?;
    let mut names = Vec::new();
    let mut streams = Vec::new();
    for device in &e.get_device_collection(Direction::Capture)? {
        if stop.load(Ordering::Acquire) {
            return Ok(Found::Speaking(None));
        }
        let Ok(device) = device else { continue };
        let Ok(name) = device.get_friendlyname() else {
            continue;
        };
        if super::vb_cable_endpoint(&device, &name) {
            continue;
        }
        if !listen {
            names.push(name);
        } else if !crate::setup::virtual_device(&name) {
            if let Ok(input) = Input::open(&device, name) {
                streams.push(input);
            }
        }
    }
    if !listen {
        names.sort();
        names.dedup();
        return Ok(Found::Devices(names));
    }
    let start = Instant::now();
    let mut lead = None;
    let mut since = start;
    while !stop.load(Ordering::Acquire) {
        let elapsed = start.elapsed();
        streams.retain_mut(|input| input.poll(elapsed >= Duration::from_millis(500)).is_ok());
        ensure!(!streams.is_empty(), "No available microphone");
        let best = streams
            .iter()
            .filter_map(|s| Some((&s.name, s.level.score()?)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(name, _)| name);
        if best != lead.as_ref() {
            lead = best.cloned();
            since = Instant::now();
        }
        if (lead.is_some() && since.elapsed() >= Duration::from_millis(1200))
            || elapsed >= Duration::from_secs(12)
        {
            return Ok(Found::Speaking(lead));
        }
        thread::sleep(Duration::from_millis(25));
    }
    Ok(Found::Speaking(None))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Enumerates the local Windows audio endpoints"]
    fn windows_mic_pickers_exclude_the_same_cable_endpoints() {
        let env = crate::audio::devices().unwrap();
        let Found::Devices(vr) = scan(false, &AtomicBool::new(false)).unwrap() else {
            panic!()
        };
        let mut pc = env.inputs.clone();
        pc.sort();
        pc.dedup();
        assert_eq!(pc, vr);
        assert!(pc.iter().all(|name| !env.was_cable_mic(name)));
        assert!(env.cable_speakers.iter().all(|n| env.outputs.contains(n)));
        println!(
            "{} selectable microphones, {} cable capture and {} cable playback endpoints",
            pc.len(),
            env.cable_mics.len(),
            env.cable_speakers.len()
        );
    }
    #[test]
    fn sustained_voice_beats_stationary_noise_and_a_single_spike() {
        let mut voice = Level::default();
        let mut hiss = Level::default();
        let mut spike = Level::default();
        for rms in [20., 20., 430., 510., 450., 30.] {
            for _ in 0..800 {
                voice.sample(rms as i16);
            }
            hiss.block(900.);
        }
        for rms in [20., 4800., 20., 20.] {
            spike.block(rms);
        }
        assert_eq!(voice.score(), Some(410.));
        assert_eq!(hiss.score(), None);
        assert_eq!(spike.score(), None);
    }
}
