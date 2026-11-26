mod assets;
mod model;

use super::{Decimator, FRAME};
use anyhow::{Result, bail, ensure};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::{Duration, Instant},
};

// 512 ms of new speech with 64 ms of context on either side at 16 kHz.
const HOP: usize = 8192;
const CONTEXT: usize = 1024;
const WINDOW: usize = HOP + 2 * CONTEXT;
const CROSS: usize = 512;

#[derive(Clone, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Preparing,
    Active,
    Assisted,
    Fallback(String),
}
impl Status {
    pub fn description(&self, l: crate::labels::Lang) -> String {
        match self {
            Self::Idle => String::new(),
            Self::Preparing => l
                .t(
                    "GPU 준비 중 / 현재 CPU 필터",
                    "Preparing GPU / CPU filter active",
                    "GPU 準備中 / CPU 処理中",
                    "GPU 准备中 / 正在使用 CPU",
                )
                .into(),
            Self::Active => "NVIDIA Nemotron RE-USE (GPU)".into(),
            Self::Assisted => l
                .t(
                    "GPU 혼잡 / 현재 구간은 CPU 처리",
                    "GPU busy / CPU handling this segment",
                    "GPU 混雑 / この区間は CPU 処理",
                    "GPU 繁忙 / 此段由 CPU 处理",
                )
                .into(),
            Self::Fallback(e) => format!(
                "{} / {}",
                l.t(
                    "CPU 필터로 전환",
                    "Using CPU fallback",
                    "CPU フィルターに切替",
                    "已切换至 CPU"
                ),
                e
            ),
        }
    }
}

pub fn worker_main() -> Result<()> {
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    // Exercise the speech path at startup, including backends that skip silence.
    let mut samples: Vec<f32> = (0..WINDOW)
        .map(|i| (i as f32 * 0.06).sin() * 0.08)
        .collect();
    let mut enhanced = vec![0i16; WINDOW];
    let mut model = model::Model::load()?;
    model.enhance(&samples, &mut enhanced)?; // compile the graph before switching live audio
    output.write_all(b"READY\n")?;
    output.flush()?;
    let mut pcm = vec![0u8; WINDOW * 2];
    while input.read_exact(&mut pcm).is_ok() {
        for (v, b) in samples.iter_mut().zip(pcm.chunks_exact(2)) {
            *v = i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.;
        }
        model.enhance(&samples, &mut enhanced)?;
        for (b, v) in pcm.chunks_exact_mut(2).zip(&enhanced) {
            b.copy_from_slice(&v.to_le_bytes());
        }
        output.write_all(&pcm)?;
        output.flush()?;
    }
    Ok(())
}

const GRACE_FRAMES: usize = 24; // 384 ms after a complete window is available.

struct Job {
    id: u64,
    samples: Vec<i16>,
    deadline: Instant,
    reply: mpsc::Sender<(u64, Option<Vec<i16>>)>,
}
struct Worker {
    child: parking_lot::Mutex<Child>,
    jobs: SyncSender<Job>,
    state: std::sync::Arc<parking_lot::Mutex<Status>>,
}
impl Worker {
    fn launch(binary: &std::path::Path) -> Result<Self> {
        let mut child = Command::new(binary)
            .arg("--nemotron-worker")
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut to_child = child.stdin.take().unwrap();
        let mut from_child = BufReader::new(child.stdout.take().unwrap());
        let (tx, jobs) = mpsc::sync_channel::<Job>(2);
        let state = std::sync::Arc::new(parking_lot::Mutex::new(Status::Preparing));
        let report = state.clone();
        thread::spawn(move || {
            let run = || -> Result<()> {
                let mut hello = String::new();
                from_child.read_line(&mut hello)?;
                ensure!(
                    hello.trim() == "READY",
                    "{}",
                    if hello.trim().is_empty() {
                        "GPU worker exited during setup"
                    } else {
                        hello.trim()
                    }
                );
                *report.lock() = Status::Active;
                let mut bytes = vec![0u8; 2 * WINDOW];
                for job in jobs {
                    let rendered = if Instant::now() < job.deadline {
                        for (b, v) in bytes.chunks_exact_mut(2).zip(job.samples) {
                            b.copy_from_slice(&v.to_le_bytes());
                        }
                        to_child.write_all(&bytes)?;
                        from_child.read_exact(&mut bytes)?;
                        Some(
                            bytes
                                .chunks_exact(2)
                                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                                .collect(),
                        )
                    } else {
                        None
                    };
                    // A stopped lane can leave the shared model running for the other lane.
                    let _ = job.reply.send((job.id, rendered));
                }
                Ok(())
            };
            if let Err(e) = run() {
                *report.lock() = Status::Fallback(format!("{e:#}"));
            }
        });
        Ok(Self {
            child: parking_lot::Mutex::new(child),
            jobs: tx,
            state,
        })
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let child = self.child.get_mut();
        let _ = child.kill();
        let _ = child.wait();
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Output {
    Preparing,
    Restored,
    Cpu,
}
struct Block {
    id: u64,
    original: Vec<i16>,
    restored: Option<Vec<i16>>,
    silent: bool,
}
struct Playout {
    waiting: Option<Block>,
    ticks: usize,
    samples: VecDeque<i16>,
    source_levels: [f32; HOP / 256],
    tail: Option<Vec<i16>>,
    mode: Output,
}
impl Default for Playout {
    fn default() -> Self {
        Self {
            waiting: None,
            ticks: 0,
            samples: VecDeque::new(),
            source_levels: [0.; HOP / 256],
            tail: None,
            mode: Output::Restored,
        }
    }
}
impl Playout {
    fn queue(&mut self, block: Block) {
        debug_assert!(self.waiting.is_none());
        self.waiting = Some(block);
        self.ticks = GRACE_FRAMES;
    }
    fn receive(&mut self, id: u64, samples: Option<Vec<i16>>) {
        if let Some(block) = &mut self.waiting
            && block.id == id
        {
            block.restored = samples;
        }
    }
    fn advance(&mut self) {
        if self.ticks != 0 {
            self.ticks -= 1;
            return;
        }
        let Some(block) = self.waiting.take() else {
            return;
        };
        debug_assert!(self.samples.is_empty());
        let mut core = block.original;
        for (level, frame) in self.source_levels.iter_mut().zip(core.chunks_exact(256)) {
            *level = super::rms(frame);
        }
        if let Some(wet) = block.restored {
            core.copy_from_slice(&wet[CONTEXT..CONTEXT + HOP]);
            if let Some(last) = &self.tail {
                for i in 0..CROSS {
                    let t = (i + 1) as f32 / (CROSS + 1) as f32;
                    core[i] = (last[i] as f32 * (1. - t) + core[i] as f32 * t).round() as i16;
                }
            }
            self.tail = Some(wet[CONTEXT + HOP..CONTEXT + HOP + CROSS].to_vec());
            self.mode = Output::Restored;
        } else {
            self.tail = None;
            if !block.silent {
                self.mode = Output::Cpu;
            }
        }
        self.samples = core.into();
    }
}
pub(super) struct Gpu {
    worker: std::sync::Arc<Worker>,
    rx: Receiver<(u64, Option<Vec<i16>>)>,
    reply: mpsc::Sender<(u64, Option<Vec<i16>>)>,
    downsample: Decimator,
    input: Vec<i16>,
    playout: Playout,
    next_id: u64,
    in_flight: bool,
    last_sample: i16,
    started: Instant,
}
impl Gpu {
    pub fn start() -> Result<Self> {
        #[cfg(test)]
        if let Some(binary) = std::env::var_os("VRCLT_GPU_BINARY") {
            return Self::start_with(binary.as_ref());
        }
        Self::start_with(&std::env::current_exe()?)
    }
    fn start_with(binary: &std::path::Path) -> Result<Self> {
        use std::sync::{Arc, Weak};
        static SHARED: parking_lot::Mutex<Weak<Worker>> = parking_lot::Mutex::new(Weak::new());
        let mut shared = SHARED.lock();
        let worker = if let Some(worker) = shared.upgrade() {
            worker
        } else {
            let worker = Arc::new(Worker::launch(binary)?);
            *shared = Arc::downgrade(&worker);
            worker
        };
        let (reply, rx) = mpsc::channel();
        Ok(Self {
            worker,
            rx,
            reply,
            downsample: Decimator::new(16000),
            input: vec![0; CONTEXT],
            playout: Playout::default(),
            next_id: 0,
            in_flight: false,
            last_sample: 0,
            started: Instant::now(),
        })
    }
    pub fn process(&mut self, frame: &mut [i16; FRAME]) -> Result<Output> {
        match &*self.worker.state.lock() {
            Status::Preparing => {
                ensure!(
                    self.started.elapsed() < Duration::from_secs(360),
                    "GPU setup timed out"
                );
                return Ok(Output::Preparing);
            }
            Status::Fallback(err) => bail!("{err}"),
            _ => {}
        }
        while let Ok((id, samples)) = self.rx.try_recv() {
            self.in_flight = false;
            self.playout.receive(id, samples);
        }
        let mut mono = [0i16; 256];
        self.downsample.process(frame, &mut mono);
        self.input.extend_from_slice(&mono);
        if self.input.len() >= WINDOW {
            ensure!(
                self.worker.child.lock().try_wait()?.is_none(),
                "GPU worker exited"
            );
            let id = self.next_id;
            self.next_id += 1;
            let silent = self.input[..WINDOW].iter().all(|&v| v == 0);
            self.playout.queue(Block {
                id,
                original: self.input[CONTEXT..CONTEXT + HOP].to_vec(),
                restored: None,
                silent,
            });
            if !silent && !self.in_flight {
                let job = Job {
                    id,
                    samples: self.input[..WINDOW].to_vec(),
                    deadline: Instant::now() + Duration::from_millis(GRACE_FRAMES as u64 * 16),
                    reply: self.reply.clone(),
                };
                match self.worker.jobs.try_send(job) {
                    Ok(()) => self.in_flight = true,
                    Err(mpsc::TrySendError::Full(_)) => {}
                    Err(mpsc::TrySendError::Disconnected(_)) => bail!("GPU worker disconnected"),
                }
            }
            self.input.drain(..HOP);
        }
        self.playout.advance();
        for group in frame.chunks_exact_mut(3) {
            let next = self.playout.samples.pop_front().unwrap_or(0);
            let (a, b) = (i32::from(self.last_sample), i32::from(next));
            group.copy_from_slice(&[((2 * a + b) / 3) as i16, ((a + 2 * b) / 3) as i16, next]);
            self.last_sample = next;
        }
        Ok(self.playout.mode)
    }
    /// Input level of the frame just delivered, measured before enhancement.
    pub fn source_level(&self) -> f32 {
        let played = HOP - self.playout.samples.len();
        self.playout.source_levels[played.saturating_sub(1) / 256]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_gpu_blocks_are_substituted_once_and_following_blocks_can_recover() {
        let mut play = Playout::default();
        let mut emitted = Vec::new();
        for id in 0..5 {
            play.queue(Block {
                id,
                original: vec![id as i16; HOP],
                restored: None,
                silent: false,
            });
            if id > 0 {
                play.receive(id - 1, Some(vec![99; WINDOW]));
            }
            if id == 0 || id == 4 {
                play.receive(id, Some(vec![10 + id as i16; WINDOW]));
            }
            for _ in 0..=GRACE_FRAMES {
                play.advance();
            }
            assert_eq!(
                play.mode,
                if id == 0 || id == 4 {
                    Output::Restored
                } else {
                    Output::Cpu
                }
            );
            emitted.extend(play.samples.drain(..));
            assert!(play.source_levels.iter().all(|&level| level == id as f32));
        }
        assert_eq!(
            emitted,
            [10, 1, 2, 3, 14]
                .into_iter()
                .flat_map(|v| std::iter::repeat_n(v, HOP))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[ignore = "requires a Vulkan GPU, VRCLT_GPU_BINARY and VRCLT_SPEECH_PCM"]
    fn live_gpu_worker_restores_speech_and_reports_exit() {
        let exe = std::env::var_os("VRCLT_GPU_BINARY").unwrap();
        let bytes = std::fs::read(std::env::var_os("VRCLT_SPEECH_PCM").unwrap()).unwrap();
        let mut gpu = Gpu::start_with(std::path::Path::new(&exe)).unwrap();
        let mut second = Gpu::start_with(std::path::Path::new(&exe)).unwrap();
        assert!(std::sync::Arc::ptr_eq(&gpu.worker, &second.worker));
        while gpu.process(&mut [0; FRAME]).unwrap() == Output::Preparing {
            thread::sleep(Duration::from_millis(16));
        }
        let clock = Instant::now();
        let mut dry = Vec::new();
        let mut wet = Vec::new();
        for i in 0..450 {
            let mut frame = [0; FRAME];
            if i < 320 {
                for (s, b) in frame.iter_mut().zip(bytes[i * FRAME * 2..].chunks_exact(2)) {
                    *s = i16::from_le_bytes([b[0], b[1]]);
                }
            }
            dry.push(super::super::rms(&frame));
            // Both directions can contain speech concurrently in a real call.
            let mut other = frame;
            second.process(&mut other).unwrap();
            assert!(
                gpu.process(&mut frame)
                    .unwrap_or_else(|e| panic!("frame {i}, elapsed {:?}: {e}", clock.elapsed()))
                    != Output::Preparing
            );
            wet.push(super::super::rms(&frame));
            thread::sleep(
                (clock + Duration::from_millis((i as u64 + 1) * 16))
                    .saturating_duration_since(Instant::now()),
            );
        }
        let (delay, similarity) = (16..110)
            .map(|offset| {
                let y = &wet[offset..offset + 320];
                let x = &dry[..320];
                let dot: f32 = x.iter().zip(y).map(|(a, b)| a * b).sum();
                let energy = |z: &[f32]| z.iter().map(|s| s * s).sum::<f32>().sqrt();
                (offset, dot / (energy(x) * energy(y)))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        println!(
            "Live GPU speech envelope delay {} ms, similarity {similarity:.3}",
            delay * 16
        );
        assert!(similarity > 0.75, "restored speech must track the input");
        gpu.worker.child.lock().kill().unwrap();
        gpu.worker.child.lock().wait().unwrap();
        assert!((0..=HOP / 256).any(|_| gpu.process(&mut [0; FRAME]).is_err()));
    }
}
