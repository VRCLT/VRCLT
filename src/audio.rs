use crate::{
    config::Config,
    platform,
    runtime::{Lane, Phase},
};
use anyhow::{Context, Result, bail};
use parking_lot::{Condvar, Mutex};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use wasapi::*;
pub mod mic_search;
pub mod reuse;
mod source_gate;
mod speech;
pub mod voice_sample;

struct ComApartment;
impl ComApartment {
    fn open() -> Result<Self> {
        initialize_mta().ok()?;
        Ok(Self)
    }
}
impl Drop for ComApartment {
    fn drop(&mut self) {
        deinitialize();
    }
}
const INPUT_RATE: usize = 48000;
const FRAME: usize = 768; // 16 ms also gives one exact 256-sample VAD frame.
const TAPS: usize = 49;
pub fn rms(samples: &[i16]) -> f32 {
    let sum: i64 = samples.iter().map(|&n| i64::from(n) * i64::from(n)).sum();
    (sum as f32 / samples.len().max(1) as f32).sqrt()
}
// Windowed-sinc decimation. Preserve the filter tail across WASAPI packets.
struct Decimator {
    kernel: [f32; TAPS],
    buf: [f32; FRAME + TAPS - 1],
    stride: usize,
}
impl Decimator {
    fn new(rate: usize) -> Self {
        let cutoff = 0.46 * rate as f32 / INPUT_RATE as f32;
        let mut kernel = std::array::from_fn(|i| {
            let x = i as f32 - (TAPS - 1) as f32 / 2.;
            let sinc = if x == 0. {
                2. * cutoff
            } else {
                (2. * std::f32::consts::PI * cutoff * x).sin() / (std::f32::consts::PI * x)
            };
            let window =
                0.5 - 0.5 * (2. * std::f32::consts::PI * i as f32 / (TAPS - 1) as f32).cos();
            sinc * window
        });
        let sum: f32 = kernel.iter().sum();
        for k in &mut kernel {
            *k /= sum;
        }
        Self {
            kernel,
            buf: [0.; FRAME + TAPS - 1],
            stride: INPUT_RATE / rate,
        }
    }
    fn process(&mut self, input: &[i16; FRAME], out: &mut [i16]) {
        for (dest, &sample) in self.buf[TAPS - 1..].iter_mut().zip(input) {
            *dest = sample as f32;
        }
        for (i, dest) in out.iter_mut().enumerate() {
            let start = i * self.stride;
            *dest = self.buf[start..start + TAPS]
                .iter()
                .zip(self.kernel.iter())
                .map(|(a, b)| a * b)
                .sum::<f32>()
                .round() as i16;
        }
        self.buf.copy_within(FRAME.., 0);
    }
}
fn vb_cable_endpoint(device: &Device, name: &str) -> bool {
    crate::setup::vb_cable_name(name)
        || device
            .get_interface_friendlyname()
            .is_ok_and(|interface| crate::setup::vb_cable_name(&interface))
}
pub fn devices() -> Result<crate::setup::Environment> {
    let _com = ComApartment::open()?;
    let e = DeviceEnumerator::new()?;
    let names = |dir| -> Result<(Vec<String>, Vec<String>)> {
        let mut names = Vec::new();
        let mut cables = Vec::new();
        for d in (&e.get_device_collection(dir)?).into_iter().flatten() {
            let Ok(name) = d.get_friendlyname() else {
                continue;
            };
            if vb_cable_endpoint(&d, &name) {
                cables.push(name.clone());
                if dir == Direction::Capture {
                    continue;
                }
            }
            names.push(name);
        }
        Ok((names, cables))
    };
    let default_name = |dir| {
        e.get_default_device(dir)
            .and_then(|d| d.get_friendlyname())
            .unwrap_or_default()
    };
    let (inputs, cable_mics) = names(Direction::Capture)?;
    let (outputs, cable_speakers) = names(Direction::Render)?;
    Ok(crate::setup::Environment::read(
        inputs,
        outputs,
        cable_mics,
        cable_speakers,
        default_name(Direction::Capture),
        default_name(Direction::Render),
    ))
}
fn device(dir: Direction, want: &str) -> Result<Device> {
    let e = DeviceEnumerator::new()?;
    let automatic =
        want.is_empty() || (dir == Direction::Capture && crate::setup::vb_cable_name(want));
    if automatic && dir != Direction::Capture {
        return Ok(e.get_default_device(dir)?);
    }
    if automatic && let Ok(default) = e.get_default_device(dir) {
        if !vb_cable_endpoint(&default, &default.get_friendlyname()?) {
            return Ok(default);
        }
    }
    let norm = |s: &str| s.to_lowercase().replace(" (", "(");
    let needle = norm(want);
    let mut partial = None;
    for d in &e.get_device_collection(dir)? {
        let Ok(d) = d else { continue };
        let Ok(name) = d.get_friendlyname() else {
            continue;
        };
        if dir == Direction::Capture && vb_cable_endpoint(&d, &name) {
            continue;
        }
        if automatic {
            return Ok(d);
        }
        let name = norm(&name);
        if name == needle {
            return Ok(d);
        }
        if partial.is_none() && name.contains(&needle) {
            partial = Some(d);
        }
    }
    if let Some(d) = partial {
        return Ok(d);
    }
    if automatic {
        bail!("No microphone available");
    }
    bail!("Audio device unavailable: {want}")
}
fn init(client: &mut AudioClient, dir: Direction, rate: usize) -> Result<()> {
    init_format(
        client,
        dir,
        &WaveFormat::new(16, 16, SampleType::Int, rate, 1, None),
    )
}
fn init_format(client: &mut AudioClient, dir: Direction, format: &WaveFormat) -> Result<()> {
    client.initialize_client(
        format,
        dir,
        StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: 0,
        },
        None,
    )?;
    Ok(())
}
pub struct AudioQueue {
    samples: Mutex<VecDeque<u8>>,
    ready: Condvar,
    closed: AtomicBool,
    capacity: usize,
}
impl AudioQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
            closed: AtomicBool::new(false),
            capacity,
        }
    }
    pub fn push(&self, pcm: &[u8]) {
        let mut q = self.samples.lock();
        if !self.is_open() {
            return;
        }
        let pcm = &pcm[pcm.len().saturating_sub(self.capacity)..];
        let excess = (q.len() + pcm.len()).saturating_sub(self.capacity);
        q.drain(..excess);
        let was_empty = q.is_empty();
        q.extend(pcm);
        if was_empty {
            self.ready.notify_one();
        }
    }
    pub fn clear(&self) {
        self.samples.lock().clear();
    }
    pub fn drain(&self) -> Vec<u8> {
        self.samples.lock().drain(..).collect()
    }
    pub fn keep_latest(&self, bytes: usize) {
        let mut samples = self.samples.lock();
        let excess = samples.len().saturating_sub(bytes);
        samples.drain(..excess);
    }
    fn wait_for_audio(&self, lane: &Lane) {
        let mut queue = self.samples.lock();
        while queue.is_empty() && lane.alive() && self.is_open() {
            self.ready.wait_for(&mut queue, Duration::from_millis(200));
        }
    }
    pub fn is_open(&self) -> bool {
        !self.closed.load(Ordering::Acquire)
    }
    pub fn close(&self) {
        let mut samples = self.samples.lock();
        self.closed.store(true, Ordering::Release);
        samples.clear();
        self.ready.notify_all();
    }
    fn render_block(&self, buf: &mut [u8], lane: &Lane, output: &Playback) {
        self.fill(buf);
        let gain = match output {
            Playback::Unity => 1.,
            Playback::Translated | Playback::Monitor { .. } => lane.voice_gain(),
        };
        if gain != 1. {
            for sample in buf.chunks_exact_mut(2) {
                let v = i16::from_le_bytes([sample[0], sample[1]]);
                sample.copy_from_slice(&((f64::from(v) * gain) as i16).to_le_bytes());
            }
        }
    }
    fn fill(&self, buf: &mut [u8]) {
        let mut q = self.samples.lock();
        let n = buf.len().min(q.len());
        let (a, b) = q.as_slices();
        let first = a.len().min(n);
        buf[..first].copy_from_slice(&a[..first]);
        buf[first..n].copy_from_slice(&b[..n - first]);
        q.drain(..n);
        buf[n..].fill(0);
    }
}
pub enum Playback {
    Unity,
    Translated,
    Monitor { primary: String },
}
pub fn playback(
    lane: &Lane,
    q: &AudioQueue,
    want: &str,
    rate: usize,
    output: &Playback,
) -> Result<()> {
    let _com = ComApartment::open()?;
    q.wait_for_audio(lane);
    if !lane.alive() || !q.is_open() {
        return Ok(());
    }
    let endpoint = device(Direction::Render, want)?;
    if let Playback::Monitor { primary } = output {
        let id = endpoint.get_id()?;
        if device(Direction::Render, primary)
            .and_then(|d| Ok(d.get_id()?))
            .ok()
            .as_ref()
            == Some(&id)
        {
            // The primary stream already plays here, including when it uses the system default.
            lane.playback_status(q, true, String::new());
            q.close();
            return Ok(());
        }
    }
    let mut c = endpoint.get_iaudioclient()?;
    init(&mut c, Direction::Render, rate)?;
    let event = c.set_get_eventhandle()?;
    let render = c.get_audiorenderclient()?;
    let mut buf = vec![0; c.get_buffer_size()? as usize * 2];
    c.start_stream()?;
    lane.playback_status(q, matches!(output, Playback::Monitor { .. }), String::new());
    while lane.alive() && q.is_open() {
        if event.wait_for_event(100).is_err() {
            continue;
        }
        let len = c.get_available_space_in_frames()? as usize * 2;
        if len == 0 {
            continue;
        }
        buf.resize(len, 0);
        q.render_block(&mut buf, lane, output);
        render.write_to_device(&buf, None)?;
    }
    c.stop_stream()?;
    Ok(())
}

// Match the legacy game tap's equal-weight stereo downmix. Compact in place
// so a WASAPI callback doesn't have to allocate a second audio buffer.
fn game_mono(packet: &mut [u8]) -> &[u8] {
    let frames = packet.len() / 8;
    for i in 0..frames {
        let pair = &packet[i * 8..i * 8 + 8];
        let left = f32::from_le_bytes(pair[..4].try_into().unwrap());
        let right = f32::from_le_bytes(pair[4..].try_into().unwrap());
        let sample = ((left + right) * 0.5 * 32768.).round() as i16;
        packet[i * 2..i * 2 + 2].copy_from_slice(&sample.to_le_bytes());
    }
    &packet[..frames * 2]
}
pub fn capture(
    cfg: &Config,
    lane: &Arc<Lane>,
    echo: &Mutex<Option<Instant>>,
    raw: Option<&AudioQueue>,
) -> Result<()> {
    let _com = ComApartment::open()?;
    let mut pid = None;
    let mut client = if lane.inbound {
        let target = cfg.s("/inbound/process");
        let id = loop {
            if !lane.alive() {
                return Ok(());
            }
            if let Some(id) = platform::process_id(target) {
                break id;
            }
            lane.capture_phase(Phase::WaitingApp);
            lane.sleep(Duration::from_secs(1));
        };
        pid = Some(id);
        match AudioClient::new_application_loopback_client(id, true) {
            Ok(c) => c,
            Err(_) if cfg.b("/inbound/allow_system_audio") => {
                device(Direction::Render, "")?.get_iaudioclient()?
            }
            Err(e) => return Err(e).context("Per-application audio capture"),
        }
    } else {
        device(Direction::Capture, cfg.s("/outbound/mic_device"))?.get_iaudioclient()?
    };
    let bytes_per_frame = if lane.inbound {
        init_format(
            &mut client,
            Direction::Capture,
            &WaveFormat::new(32, 32, SampleType::Float, INPUT_RATE, 2, None),
        )?;
        8
    } else {
        init(&mut client, Direction::Capture, INPUT_RATE)?;
        2
    };
    let event = client.set_get_eventhandle()?;
    let reader = client.get_audiocaptureclient()?;
    let mut packet = Vec::new();
    let mut frame = [0i16; FRAME];
    let mut used = 0;
    let mut process_frame = frame_processor(cfg, lane, echo)?;
    let mut last_check = Instant::now();
    let mut last_packet = Instant::now();
    let mut next_silence = last_packet;
    let drain_detector = lane.inbound && lane.translating && cfg.b("/inbound/vad_enabled");
    client.start_stream()?;
    lane.capture_phase(Phase::Listening);
    while lane.alive() {
        if pid.is_some() && last_check.elapsed() >= Duration::from_secs(2) {
            last_check = Instant::now();
            if platform::process_id(cfg.s("/inbound/process")) != pid {
                break;
            }
        }
        let gpu_wanted = lane.translating
            && lane.speech_filter.load(Ordering::Relaxed)
            && lane.nemotron.load(Ordering::Relaxed);
        let idle = last_packet.elapsed();
        let draining = (gpu_wanted && idle < Duration::from_secs(3))
            || (drain_detector && idle < Duration::from_secs(1));
        let signalled = event
            .wait_for_event(if draining { 16 } else { 100 })
            .is_ok();
        let mut received = false;
        while signalled && lane.alive() {
            let count = reader.get_next_packet_size()?.unwrap_or(0);
            if count == 0 {
                break;
            }
            packet.resize(count as usize * bytes_per_frame, 0);
            let (n, _) = reader.read_from_device(&mut packet)?;
            if n == 0 {
                break;
            }
            received = true;
            last_packet = Instant::now();
            next_silence = last_packet + Duration::from_millis(32);
            let packet = &mut packet[..n as usize * bytes_per_frame];
            let pcm = if lane.inbound {
                game_mono(packet)
            } else {
                &packet[..]
            };
            if let Some(q) = raw {
                q.push(pcm);
            }
            for s in pcm.chunks_exact(2) {
                frame[used] = i16::from_le_bytes([s[0], s[1]]);
                used += 1;
                if used != FRAME {
                    continue;
                }
                used = 0;
                process_frame(&mut frame)?;
            }
        }
        // Loopback may stop emitting packets when the game goes quiet. Flush the
        // tail and drain queued restored speech at the PCM clock, including short last words.
        if !received && draining {
            let now = Instant::now();
            while next_silence <= now {
                frame[used..].fill(0);
                used = 0;
                process_frame(&mut frame)?;
                next_silence += Duration::from_millis(16);
            }
        } else if !received && lane.translating {
            // WASAPI loopback can remain completely silent during model setup
            // or while the user changes the selection. Poll only until it settles.
            let poll = match *lane.gpu_filter.lock() {
                reuse::Status::Idle => gpu_wanted,
                reuse::Status::Preparing => true,
                _ => !gpu_wanted,
            };
            if poll {
                process_frame(&mut [0; FRAME])?;
            }
        }
    }
    client.stop_stream()?;
    Ok(())
}
pub(crate) fn frame_processor<'a>(
    cfg: &Config,
    lane: &'a Lane,
    echo: &'a Mutex<Option<Instant>>,
) -> Result<impl FnMut(&mut [i16; FRAME]) -> Result<()> + 'a> {
    let mut filter = Decimator::new(lane.rate);
    let mut filtered = vec![0i16; lane.rate * 16 / 1000];
    let initially_enabled = lane.translating && lane.speech_filter.load(Ordering::Relaxed);
    let mut focus = initially_enabled
        .then(speech::SpeechFilter::new)
        .transpose()?;
    let use_vad = if lane.inbound {
        cfg.b("/inbound/vad_enabled")
    } else {
        initially_enabled
    };
    let mut vad =
        (lane.translating && !lane.inbound && use_vad).then(earshot::Detector::default_boxed);
    let mut onset = speech::SpeechOnset::default();
    let mut vad_filter = Decimator::new(16000);
    let mut vad_frame = [0; 256];
    let mut preroll = VecDeque::<u8>::new();
    let mut encoded = Vec::with_capacity(filtered.len() * 2);
    let threshold = cfg.n("/audio/voice_rms_threshold", 90.).max(1.) as f32;
    let vad_threshold = if lane.inbound {
        cfg.n("/inbound/vad_threshold", 0.5).clamp(0., 1.) as f32
    } else {
        0.5
    };
    let hangover = Duration::from_secs_f64(
        cfg.n(
            if lane.inbound {
                "/inbound/vad_hangover_sec"
            } else {
                "/audio/voice_hangover_sec"
            },
            0.35,
        )
        .clamp(0., 8.),
    );
    let hold = Duration::from_secs_f64(cfg.n("/audio/echo_guard_hold_sec", 1.2).clamp(0., 10.));
    let mut source_gate = (lane.inbound && lane.translating && use_vad)
        .then(|| source_gate::SourceGate::new(vad_threshold, hangover))
        .transpose()?;
    let echo_factor = cfg.n("/audio/echo_guard_multiplier", 4.).max(1.) as f32;
    let barge = cfg.n("/audio/echo_guard_barge_in_multiplier", 3.) as f32;
    let mut last_voice = None::<Instant>;
    let mut gpu_requested = false;
    let mut was_gpu = false;
    let mut gpu = None;
    *lane.gpu_filter.lock() = reuse::Status::Idle;
    Ok(move |frame: &mut [i16; FRAME]| -> Result<()> {
        let level = rms(frame);
        lane.level.store(level as u32, Ordering::Relaxed);
        if !lane.translating {
            return Ok(());
        }
        let enabled = lane.speech_filter.load(Ordering::Relaxed);
        let source_open = match &mut source_gate {
            Some(gate) => Some(gate.process(frame)?),
            None => None,
        };
        let input_level = if source_open.is_some() {
            rms(frame)
        } else {
            level
        };
        if enabled != focus.is_some() {
            focus = enabled.then(speech::SpeechFilter::new).transpose()?;
            if !lane.inbound {
                vad = enabled.then(earshot::Detector::default_boxed);
            }
            preroll.clear();
            last_voice = None;
            onset = Default::default();
        }
        let requested = enabled && lane.nemotron.load(Ordering::Relaxed);
        if requested != gpu_requested {
            gpu_requested = requested;
            gpu = None;
            let status = if requested {
                match reuse::Gpu::start() {
                    Ok(worker) => {
                        gpu = Some(worker);
                        reuse::Status::Preparing
                    }
                    Err(e) => reuse::Status::Fallback(e.to_string()),
                }
            } else {
                reuse::Status::Idle
            };
            *lane.gpu_filter.lock() = status;
            lane.repaint();
        }
        let output = if let Some(g) = &mut gpu {
            match g.process(frame) {
                Ok(mode) => mode,
                Err(e) => {
                    *lane.gpu_filter.lock() = reuse::Status::Fallback(e.to_string());
                    lane.repaint();
                    gpu = None;
                    reuse::Output::Preparing
                }
            }
        } else {
            reuse::Output::Preparing
        };
        let aligned = output != reuse::Output::Preparing;
        let active = output == reuse::Output::Restored;
        if aligned != was_gpu {
            was_gpu = aligned;
            preroll.clear();
            last_voice = None;
            onset = Default::default();
        }
        if aligned {
            let mut status = lane.gpu_filter.lock();
            let next = if active {
                reuse::Status::Active
            } else {
                reuse::Status::Assisted
            };
            if *status != next {
                *status = next;
                lane.repaint();
            }
        }
        let gate_level = if aligned {
            gpu.as_ref().unwrap().source_level()
        } else {
            input_level
        };
        let voice_energy = match &mut focus {
            _ if active => true,
            Some(filter) => filter.process(frame)?,
            None => true,
        };
        filter.process(frame, &mut filtered);
        let now = Instant::now();
        let guarded = !lane.inbound && echo.lock().is_some_and(|v| now - v < hold);
        let recent = last_voice.is_some_and(|at| now - at <= hangover);
        if guarded && (barge <= 0. || gate_level < threshold * barge) {
            // A rejected echo must not re-enter the next request as pre-roll or hangover.
            preroll.clear();
            onset = Default::default();
            return Ok(());
        }
        let gate = threshold
            * if guarded { echo_factor.min(barge) } else { 1. }
            * if recent { 0.4 } else { 1. };
        let source_only = source_open.is_some() && !enabled;
        let speech = if let Some((_, detected)) = source_open {
            // Restoration may be delayed. Its stored source level is from the
            // original, already admitted PCM on that same playout timeline.
            if aligned { gate_level > 0. } else { detected }
        } else if let Some(detector) = &mut vad {
            let detected = if lane.rate == 16000 {
                detector.predict_i16(&filtered) >= vad_threshold
            } else {
                vad_filter.process(frame, &mut vad_frame);
                detector.predict_i16(&vad_frame) >= vad_threshold
            };
            // A restorer can amplify faint noise into speech-like output. Use the level
            // of the original sound at this exact point on the playout timeline.
            let voiced = detected && voice_energy && gate_level >= gate;
            if enabled {
                onset.accept(voiced, recent)
            } else {
                voiced
            }
        } else {
            gate_level >= gate
        };
        if speech {
            last_voice = Some(now);
            *lane.last_voice.lock() = last_voice;
            if lane.inbound {
                *echo.lock() = last_voice;
            }
        }
        encoded.clear();
        for v in &filtered {
            encoded.extend_from_slice(&v.to_le_bytes());
        }
        if source_only && source_open.is_some_and(|(open, _)| open)
            || speech
            || (!source_only && last_voice.is_some_and(|t| now - t <= hangover))
        {
            if !preroll.is_empty() {
                lane.input.push(preroll.make_contiguous());
                preroll.clear();
            }
            lane.input.push(&encoded);
            if speech {
                lane.wake.notify_one();
            }
        } else if !source_only {
            preroll.extend(&encoded);
            let excess = preroll.len().saturating_sub(lane.rate * 2 / 5);
            preroll.drain(..excess);
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translated_and_monitor_blocks_share_live_volume_and_raw_audio_stays_at_unity() {
        let lane = Lane::new(false, 16000, true, Default::default(), None);
        let monitor = Arc::new(AudioQueue::new(64));
        *lane.monitor.lock() = Some(monitor.clone());
        let headphone = Playback::Monitor {
            primary: "Cable".into(),
        };
        let source = [20_000_i16, -20_000].map(i16::to_le_bytes).concat();
        let raw = AudioQueue::new(64);
        for (percent, pos, neg) in [
            (0, 0_i16, 0_i16),
            (73, 14600, -14600),
            (100, 20000, -20000),
            (200, i16::MAX, i16::MIN),
        ] {
            lane.play(&source);
            raw.push(&source);
            lane.set_voice_volume(percent);
            for (queue, mode) in [(&*lane.tts, &Playback::Translated), (&*monitor, &headphone)] {
                let mut block = [0; 4];
                queue.render_block(&mut block, &lane, mode);
                assert_eq!(&block[..], [pos, neg].map(i16::to_le_bytes).concat());
            }
            let mut block = [0; 4];
            raw.render_block(&mut block, &lane, &Playback::Unity);
            assert_eq!(&block[..], source);
        }
    }

    #[test]
    #[ignore = "Requires two distinct WASAPI render endpoints in VRCLT_TEST_OUTPUT and VRCLT_TEST_MONITOR"]
    fn wasapi_renders_primary_and_monitor_and_suppresses_a_duplicate_endpoint() {
        let primary = std::env::var("VRCLT_TEST_OUTPUT").unwrap();
        let headset = std::env::var("VRCLT_TEST_MONITOR").unwrap();
        let lane = Lane::new(false, 16000, true, Default::default(), None);
        let monitor = Arc::new(AudioQueue::new(24000));
        *lane.monitor.lock() = Some(monitor.clone());
        let mode = Playback::Monitor {
            primary: primary.clone(),
        };
        let silence = vec![0; 4800];
        lane.play(&silence);
        std::thread::scope(|scope| {
            let main =
                scope.spawn(|| playback(&lane, &lane.tts, &primary, 24000, &Playback::Translated));
            let secondary = scope.spawn(|| playback(&lane, &monitor, &headset, 24000, &mode));
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline
                && !(lane.tts.samples.lock().is_empty() && monitor.samples.lock().is_empty())
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            let both_played = monitor.is_open()
                && lane.tts.samples.lock().is_empty()
                && monitor.samples.lock().is_empty();
            monitor.close();
            lane.tts.close();
            main.join().unwrap().unwrap();
            secondary.join().unwrap().unwrap();
            assert!(both_played);
        });
        let duplicate = AudioQueue::new(4800);
        duplicate.push(&silence);
        playback(&lane, &duplicate, &primary, 24000, &mode).unwrap();
        assert!(!duplicate.is_open());
    }
    #[test]
    #[ignore = "Requires a Windows audio endpoint and process-loopback support"]
    fn wasapi_process_loopback() {
        let _com = ComApartment::open().unwrap();
        let mut capture =
            AudioClient::new_application_loopback_client(std::process::id(), true).unwrap();
        init_format(
            &mut capture,
            Direction::Capture,
            &WaveFormat::new(32, 32, SampleType::Float, INPUT_RATE, 2, None),
        )
        .unwrap();
        let input_event = capture.set_get_eventhandle().unwrap();
        let reader = capture.get_audiocaptureclient().unwrap();
        capture.start_stream().unwrap();
        let mut playback = device(Direction::Render, "")
            .unwrap()
            .get_iaudioclient()
            .unwrap();
        init(&mut playback, Direction::Render, 24000).unwrap();
        let output_event = playback.set_get_eventhandle().unwrap();
        let render = playback.get_audiorenderclient().unwrap();
        playback.start_stream().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut received = 0;
        while received == 0 && Instant::now() < deadline {
            output_event.wait_for_event(500).unwrap();
            let n = playback.get_available_space_in_frames().unwrap() as usize;
            if n > 0 {
                render.write_to_device(&vec![0u8; n * 2], None).unwrap();
            }
            if input_event.wait_for_event(20).is_ok() {
                let frames = reader.get_next_packet_size().unwrap().unwrap_or(0);
                if frames > 0 {
                    let mut buf = vec![0; frames as usize * 8];
                    received = reader.read_from_device(&mut buf).unwrap().0;
                    assert_eq!(
                        game_mono(&mut buf[..received as usize * 8]).len(),
                        received as usize * 2
                    );
                }
            }
        }
        playback.stop_stream().unwrap();
        capture.stop_stream().unwrap();
        assert!(received > 0);
    }
    #[test]
    fn bounded_queue_keeps_latest_and_pads_playback() {
        let q = AudioQueue::new(6);
        q.push(&[1, 2, 3, 4]);
        q.push(&[5, 6, 7, 8]);
        let mut b = [0; 8];
        q.fill(&mut b);
        assert_eq!(b, [3, 4, 5, 6, 7, 8, 0, 0]);
    }
    #[test]
    #[ignore = "Requires VRCLT_BENCH_PCM pointing to raw mono 16 kHz signed 16-bit PCM"]
    fn benchmark_vad() {
        let pcm = std::fs::read(std::env::var_os("VRCLT_BENCH_PCM").unwrap()).unwrap();
        let samples: Vec<i16> = pcm
            .chunks_exact(2)
            .map(|v| i16::from_le_bytes([v[0], v[1]]))
            .collect();
        let mut detector = earshot::Detector::default();
        let start = Instant::now();
        for frame in samples.chunks_exact(256) {
            std::hint::black_box(detector.predict_i16(std::hint::black_box(frame)));
        }
        println!("vad_ms={:.3}", start.elapsed().as_secs_f64() * 1000.);
    }
    #[test]
    fn decimation_preserves_dc_across_boundaries() {
        let mut f = Decimator::new(16_000);
        let mut out = [0; 256];
        f.process(&[400; 768], &mut out);
        f.process(&[400; 768], &mut out);
        assert!(out.iter().all(|&s| s == 400));
        assert_eq!(rms(&[-400, 400]), 400.);
    }
    #[test]
    fn original_microphone_preserves_quiet_onsets_and_the_unfiltered_waveform() {
        let cfg = Config::default();
        assert!(!cfg.b("/outbound/speech_filter") && !cfg.b("/inbound/speech_filter"));
        let lane = Lane::new(false, 16000, true, Default::default(), None);
        let echo = Mutex::default();
        let mut capture = frame_processor(&cfg, &lane, &echo).unwrap();
        let mut reference = Decimator::new(16000);
        let mut expected = Vec::new();
        for (index, amplitude) in [0, 50, 150, 48, 0].into_iter().enumerate() {
            let mut block =
                std::array::from_fn(|i| if i % 64 < 32 { amplitude } else { -amplitude });
            let mut output = [0; 256];
            reference.process(&block, &mut output);
            expected.extend(output.into_iter().flat_map(i16::to_le_bytes));
            capture(&mut block).unwrap();
            if index < 2 {
                assert!(lane.last_voice.lock().is_none() && lane.input.drain().is_empty());
            }
        }
        assert!(lane.last_voice.lock().is_some());
        assert_eq!(lane.input.drain(), expected);
    }
    #[test]
    fn game_loopback_weights_both_stereo_channels_equally() {
        let mut stereo: Vec<u8> = [1_f32, 0., 0., -1., 0.75, -0.75, 1., 1.]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        let samples: Vec<_> = game_mono(&mut stereo)
            .chunks_exact(2)
            .map(|p| i16::from_le_bytes([p[0], p[1]]))
            .collect();
        assert_eq!(samples, [16384, -16384, 0, 32767]);
    }
}
