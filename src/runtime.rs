use crate::{
    audio::{self, AudioQueue},
    config::Config,
    provider,
};
use eframe::egui;
use parking_lot::Mutex;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tokio::{
    runtime::Runtime,
    sync::{Notify, watch},
    task::JoinHandle,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Opening,
    WaitingApp,
    Listening,
    Connecting,
    Live,
    Retry,
    Stopped,
}
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Caption {
    pub source: String,
    pub text: String,
    pub speaker: String,
}
#[derive(Clone)]
pub struct PendingCaption {
    pub started: Instant,
    pub updated: Instant,
    pub body_changed: Instant,
    pub expects_translation: bool,
    pub caption: Caption,
    pub displayed: Caption,
    language: String,
    closed: bool,
}
pub struct FinalCaption {
    pub started: Instant,
    pub finished: Instant,
    pub body_changed: Instant,
    pub expects_translation: bool,
    pub caption: Caption,
}
#[derive(Default)]
pub struct Transcript {
    pub pending: VecDeque<PendingCaption>,
    pub history: VecDeque<FinalCaption>,
    pub overlay_draft: Option<VecDeque<PendingCaption>>,
    source_turn: Option<usize>,
    target_turn: Option<usize>,
    last_partial: Option<Instant>,
}
pub enum CaptionEvent<'a> {
    Text {
        source: bool,
        expects_translation: bool,
        text: &'a str,
        speaker: String,
        language: &'a str,
    },
    End,
}
pub(crate) struct ChatDisplay {
    tx: watch::Sender<(String, String)>,
    streaming: bool,
    show_source: bool,
}
impl Transcript {
    fn append_event(&mut self, event: CaptionEvent<'_>) -> bool {
        let CaptionEvent::Text {
            source,
            expects_translation,
            text,
            speaker,
            language,
        } = event
        else {
            return false;
        };
        let first = self.append(source, text, &speaker, language);
        if source && let Some(i) = self.source_turn {
            self.pending[i].expects_translation = expects_translation;
        }
        first
    }
    fn close_pending(&mut self, chat: Option<&ChatDisplay>) {
        for p in &mut self.pending {
            p.closed = true;
        }
        self.commit_ready(chat);
    }

    fn update_overlay_draft(&mut self, fragments: Vec<CaptionEvent<'_>>) -> bool {
        let previous = self.overlay_draft.take();
        if fragments.is_empty() {
            return previous.is_some();
        }
        // Speculate on a copy of this response's uncommitted turns. Soniox may
        // revise the entire suffix on its next response, including the speaker.
        let mut draft = Transcript {
            pending: self.pending.clone(),
            source_turn: self.source_turn,
            target_turn: self.target_turn,
            ..Default::default()
        };
        for event in fragments {
            draft.append_event(event);
        }
        if let Some(old) = previous.as_ref() {
            for (index, row) in draft.pending.iter_mut().enumerate() {
                if let Some(prev) = old.get(index)
                    && prev.caption.speaker == row.caption.speaker
                    && prev.language == row.language
                    && prev.caption.text == row.caption.text
                    && (prev.caption.source == row.caption.source || !row.caption.text.is_empty())
                    && (prev.started == row.started || index >= self.pending.len())
                {
                    row.body_changed = prev.body_changed;
                }
            }
        }
        self.overlay_draft = Some(draft.pending);
        true
    }
    // Return true for the first visible text of a row or its first translation.
    fn append(&mut self, source: bool, text: &str, speaker: &str, language: &str) -> bool {
        let now = Instant::now();
        let cursor = if source {
            self.source_turn
        } else {
            self.target_turn
        };
        let matches = |p: &PendingCaption| {
            !p.closed
                && p.caption.speaker == speaker
                && (language.is_empty() || p.language.is_empty() || p.language == language)
        };
        let same_turn = cursor.filter(|&i| matches(&self.pending[i]));
        if !source
            && same_turn.is_none()
            && let Some(i) = cursor
        {
            self.pending[i].closed = true;
        }
        let index = same_turn
            .or_else(|| {
                self.pending.iter().position(|p| {
                    matches(p)
                        && if source {
                            p.caption.source.is_empty()
                        } else {
                            p.caption.text.is_empty()
                        }
                })
            })
            .unwrap_or_else(|| {
                self.pending.push_back(PendingCaption {
                    started: now,
                    updated: now,
                    body_changed: now,
                    expects_translation: false,
                    closed: false,
                    language: language.to_owned(),
                    displayed: Caption::default(),
                    caption: Caption {
                        speaker: speaker.into(),
                        ..Default::default()
                    },
                });
                self.pending.len() - 1
            });
        let p = &mut self.pending[index];
        let first_text =
            p.caption.text.trim().is_empty() && (!source || p.caption.source.trim().is_empty());
        if p.language.is_empty() {
            p.language.push_str(language);
        }
        p.updated = now;
        if !text.trim().is_empty() && (!source || p.caption.text.trim().is_empty()) {
            p.body_changed = now;
        }
        if source {
            self.source_turn = Some(index);
            p.caption.source.push_str(text);
        } else {
            self.target_turn = Some(index);
            p.caption.text.push_str(text);
        }
        p.closed |= p.caption.text.len() > 1600;
        first_text && !text.trim().is_empty()
    }
    fn commit_ready(&mut self, chat: Option<&ChatDisplay>) {
        let mut bytes: usize = self
            .pending
            .iter()
            .map(|p| p.caption.source.len() + p.caption.text.len())
            .sum();
        while self
            .pending
            .front()
            .is_some_and(|p| p.closed || self.pending.len() > 64 || bytes > 32768)
        {
            let turn = self.pending.pop_front().unwrap();
            let c = turn.caption;
            bytes -= c.source.len() + c.text.len();
            self.source_turn = self.source_turn.and_then(|i| i.checked_sub(1));
            self.target_turn = self.target_turn.and_then(|i| i.checked_sub(1));
            if c.text.trim().is_empty() && c.source.trim().is_empty() {
                continue;
            }
            if let Some(chat) = chat
                && !chat.streaming
                && !c.text.is_empty()
            {
                chat.tx.send_replace((c.source.clone(), c.text.clone()));
            }
            self.history.push_back(FinalCaption {
                started: turn.started,
                finished: Instant::now(),
                body_changed: turn.body_changed,
                expects_translation: turn.expects_translation,
                caption: c,
            });
            if self.history.len() > 80 {
                self.history.pop_front();
            }
        }
    }
    fn publish(&mut self, interval: Duration, chat: Option<&ChatDisplay>, force: bool) -> bool {
        if !force
            && self
                .last_partial
                .is_some_and(|last| last.elapsed() < interval)
        {
            return false;
        }
        let mut changed = false;
        for p in &mut self.pending {
            if p.displayed != p.caption {
                p.displayed.clone_from(&p.caption);
                changed = true;
            }
        }
        if !force && !changed {
            return false;
        }
        self.last_partial = Some(Instant::now());
        if let Some(chat) = chat.filter(|chat| chat.streaming) {
            let turns = self
                .pending
                .iter()
                .rev()
                .map(|p| (&p.caption, p.started))
                .chain(self.history.iter().rev().map(|p| (&p.caption, p.finished)));
            let mut rows = VecDeque::new();
            let mut next = Instant::now();
            let mut size = 0;
            for (c, at) in turns.filter(|(c, _)| !c.text.trim().is_empty()).take(6) {
                if next.saturating_duration_since(at) > Duration::from_secs(15) {
                    break;
                }
                let src = if chat.show_source {
                    c.source.trim()
                } else {
                    ""
                };
                let dst = c.text.trim();
                size += src.chars().count()
                    + dst.chars().count()
                    + usize::from(!src.is_empty())
                    + usize::from(!rows.is_empty()) * 2;
                if !rows.is_empty() && size > 144 {
                    break;
                }
                rows.push_front((src, dst));
                next = at;
            }
            if !rows.is_empty() {
                let src = rows
                    .iter()
                    .map(|p| p.0)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                let dst = rows.iter().map(|p| p.1).collect::<Vec<_>>().join(" ");
                let pair = (src, dst);
                chat.tx.send_if_modified(|old| {
                    if *old == pair {
                        false
                    } else {
                        *old = pair;
                        true
                    }
                });
            }
        }
        true
    }
}
pub struct Lane {
    pub inbound: bool,
    pub rate: usize,
    pub translating: bool,
    pub speech_filter: AtomicBool,
    pub nemotron: AtomicBool,
    pub gpu_filter: Mutex<crate::audio::reuse::Status>,
    pub level: AtomicU32,
    pub wake: Notify,
    pub input: AudioQueue,
    pub tts: Arc<AudioQueue>,
    pub monitor: Mutex<Option<Arc<AudioQueue>>>,
    voice_volume: AtomicU32,
    pub last_voice: Mutex<Option<Instant>>,
    pub captions: Mutex<Transcript>,
    vr_caption_thread: Mutex<std::sync::Weak<thread::Thread>>,
    pub phase: Mutex<Phase>,
    pub capture: Mutex<Phase>,
    pub issue: Mutex<String>,
    pub audio_issue: Mutex<String>,
    pub output_issue: Mutex<String>,
    pub monitor_issue: Mutex<String>,
    alive: AtomicBool,
    ctx: egui::Context,
    chat: Option<ChatDisplay>,
    partial_interval: Duration,
}
impl Lane {
    pub(crate) fn new(
        inbound: bool,
        rate: usize,
        translating: bool,
        ctx: egui::Context,
        chat: Option<ChatDisplay>,
    ) -> Self {
        Self {
            inbound,
            rate,
            translating,
            speech_filter: AtomicBool::new(false),
            nemotron: AtomicBool::new(false),
            gpu_filter: Mutex::default(),
            level: AtomicU32::new(0),
            wake: Notify::new(),
            input: AudioQueue::new(rate * 2 * 6),
            tts: Arc::new(AudioQueue::new(24000 * 2 * 8)),
            monitor: Mutex::default(),
            voice_volume: AtomicU32::new(100),
            last_voice: Mutex::default(),
            captions: Mutex::default(),
            vr_caption_thread: Mutex::default(),
            phase: Mutex::new(Phase::Listening),
            capture: Mutex::new(Phase::Opening),
            issue: Mutex::default(),
            audio_issue: Mutex::default(),
            output_issue: Mutex::default(),
            monitor_issue: Mutex::default(),
            alive: AtomicBool::new(true),
            ctx,
            chat,
            partial_interval: Duration::ZERO,
        }
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }
    pub fn repaint(&self) {
        self.ctx.request_repaint();
    }
    pub fn wake_vr_on_caption(&self, worker: &Arc<thread::Thread>) {
        *self.vr_caption_thread.lock() = Arc::downgrade(worker);
    }
    fn caption_changed(&self) {
        if let Some(worker) = self.vr_caption_thread.lock().upgrade() {
            worker.unpark();
        }
        self.repaint();
    }
    pub fn capture_phase(&self, p: Phase) {
        let mut v = self.capture.lock();
        if *v != p {
            *v = p;
            self.repaint();
        }
    }
    pub fn phase(&self, p: Phase) {
        *self.phase.lock() = p;
        self.repaint();
    }
    pub fn sleep(&self, d: Duration) {
        let end = Instant::now() + d;
        while self.alive() && Instant::now() < end {
            thread::sleep(Duration::from_millis(50));
        }
    }
    pub fn play(&self, pcm: &[u8]) {
        if self.alive() {
            self.tts.push(pcm);
            if let Some(q) = self.monitor.lock().as_ref() {
                q.push(pcm);
            }
        }
    }
    pub fn interrupt(&self) {
        self.tts.clear();
        if let Some(q) = self.monitor.lock().as_ref() {
            q.clear();
        }
    }
    pub fn set_voice_volume(&self, percent: i32) {
        self.voice_volume
            .store(percent.clamp(0, 200) as u32, Ordering::Relaxed);
    }
    pub fn voice_gain(&self) -> f64 {
        f64::from(self.voice_volume.load(Ordering::Relaxed)) / 100.
    }
    fn replace_monitor(&self, next: Option<Arc<AudioQueue>>) {
        let mut active = self.monitor.lock();
        if let Some(previous) = active.take() {
            previous.close();
        }
        *active = next;
        self.monitor_issue.lock().clear();
    }
    pub fn playback_status(&self, queue: &AudioQueue, monitor: bool, message: String) {
        if monitor {
            let active = self.monitor.lock();
            if active
                .as_ref()
                .is_some_and(|q| std::ptr::eq(q.as_ref(), queue))
            {
                *self.monitor_issue.lock() = message;
            }
        } else {
            *self.output_issue.lock() = message;
        }
        self.repaint();
    }
    pub fn finish(&self) {
        let mut t = self.captions.lock();
        if t.overlay_draft.take().is_none() && t.pending.is_empty() {
            return;
        }
        t.close_pending(self.chat.as_ref());
        t.publish(self.partial_interval, self.chat.as_ref(), true);
        drop(t);
        self.caption_changed();
    }
    pub fn finish_translated(&self, silence: Duration) {
        let mut t = self.captions.lock();
        if !t
            .pending
            .iter()
            .any(|p| !p.closed && !p.caption.text.is_empty() && p.updated.elapsed() >= silence)
        {
            if t.publish(self.partial_interval, self.chat.as_ref(), false) {
                drop(t);
                self.caption_changed();
            }
            return;
        }
        for p in &mut t.pending {
            p.closed |= !p.caption.text.is_empty() && p.updated.elapsed() >= silence;
        }
        t.commit_ready(self.chat.as_ref());
        t.publish(self.partial_interval, self.chat.as_ref(), true);
        drop(t);
        self.caption_changed();
    }
    pub fn publish_partial(&self) {
        if self
            .captions
            .lock()
            .publish(self.partial_interval, self.chat.as_ref(), false)
        {
            self.caption_changed();
        }
    }
    pub fn fragment(&self, source: bool, text: &str, speaker: &str) {
        self.language_fragment(source, text, speaker, "");
    }
    pub fn soniox_response(&self, confirmed: Vec<CaptionEvent<'_>>, drafts: Vec<CaptionEvent<'_>>) {
        let mut t = self.captions.lock();
        if !self.alive() {
            return;
        }
        let chat = self.chat.as_ref();
        let has_confirmed = !confirmed.is_empty();
        let mut force = false;
        for event in confirmed {
            match event {
                event @ CaptionEvent::Text { .. } => {
                    let first = t.append_event(event);
                    t.commit_ready(chat);
                    force |= self.inbound && first;
                }
                CaptionEvent::End => {
                    t.close_pending(chat);
                    force = true;
                }
            }
        }
        let changed = t.update_overlay_draft(drafts);
        let published = has_confirmed && t.publish(self.partial_interval, chat, force);
        drop(t);
        if published || changed || (has_confirmed && self.inbound) {
            self.caption_changed();
        }
    }
    pub fn language_fragment(&self, source: bool, text: &str, speaker: &str, language: &str) {
        if text.is_empty() {
            return;
        }
        let mut t = self.captions.lock();
        if !self.alive() {
            return;
        }
        let first_text = t.append(source, text, speaker.trim(), language);
        t.commit_ready(self.chat.as_ref());
        let published = t.publish(
            self.partial_interval,
            self.chat.as_ref(),
            self.inbound && first_text,
        );
        if published || self.inbound {
            drop(t);
            self.caption_changed();
        }
    }
}
pub struct Route {
    pub lane: Arc<Lane>,
    tasks: Vec<JoinHandle<()>>,
    workers: Vec<thread::JoinHandle<()>>,
}
impl Route {
    fn audio_outputs(
        cfg: &Config,
        inbound: bool,
        translating: bool,
        cable: crate::setup::Cable,
    ) -> (bool, bool) {
        if inbound {
            return (cfg.b("/inbound/play_audio"), false);
        }
        if cable != crate::setup::Cable::Ready {
            return (false, false);
        }
        let text = cfg.b("/outbound/text_only");
        (
            translating && !text && cfg.b("/outbound/voice_output"),
            text || !translating || cfg.b("/outbound/passthrough_while_translating"),
        )
    }
    pub fn start(
        rt: &Runtime,
        cfg: Config,
        inbound: bool,
        translating: bool,
        ctx: egui::Context,
        echo: Arc<Mutex<Option<Instant>>>,
        captions: Option<watch::Sender<(String, String)>>,
        cable: crate::setup::Cable,
    ) -> Self {
        let (speak, pass) = Self::audio_outputs(&cfg, inbound, translating, cable);
        let rate = if cfg.engine(inbound) == "openai" {
            24000
        } else {
            16000
        };
        let mut lane = Lane::new(
            inbound,
            rate,
            translating,
            ctx,
            captions.map(|tx| ChatDisplay {
                tx,
                streaming: cfg.b("/osc/stream_sentences"),
                show_source: cfg.b("/osc/show_source"),
            }),
        );
        lane.partial_interval = Duration::from_secs_f64(if inbound {
            cfg.n("/audio/subtitle_partial_interval_sec", 0.15)
                .clamp(0.05, 10.)
        } else {
            0.3
        });
        let lane = Arc::new(lane);
        lane.speech_filter.store(
            cfg.b(Config::speech_filter_path(inbound)),
            Ordering::Relaxed,
        );
        lane.nemotron
            .store(cfg.b(Config::nemotron_path(inbound)), Ordering::Relaxed);
        let mut this = Self {
            lane: lane.clone(),
            tasks: Vec::new(),
            workers: Vec::new(),
        };
        let output = if inbound {
            cfg.s("/inbound/audio_device")
        } else {
            cfg.s("/outbound/tts_device")
        };
        if speak {
            lane.set_voice_volume(cfg.voice_volume());
            this.player(
                output,
                lane.tts.clone(),
                24000,
                if inbound {
                    audio::Playback::Unity
                } else {
                    audio::Playback::Translated
                },
            );
            if !inbound {
                this.set_monitor(&cfg);
            }
        }
        let raw = pass.then(|| Arc::new(AudioQueue::new(48000 * 2 / 8)));
        if let Some(q) = raw.clone() {
            this.player(output, q, 48000, audio::Playback::Unity);
        }
        if translating {
            this.tasks
                .push(rt.spawn(provider::supervise(cfg.clone(), lane.clone(), speak)));
        }
        this.workers.push(thread::spawn(move || {
            while lane.alive() {
                lane.audio_issue.lock().clear();
                if let Err(e) = audio::capture(&cfg, &lane, &echo, raw.as_deref()) {
                    *lane.audio_issue.lock() = format!("{e:#}");
                    lane.capture_phase(Phase::Retry);
                    lane.repaint();
                    lane.sleep(Duration::from_secs(3));
                }
            }
        }));
        this
    }
    pub fn set_monitor(&mut self, cfg: &Config) {
        let name = cfg.s("/outbound/monitor_device");
        let enabled = self.lane.translating
            && !self.lane.inbound
            && !cfg.b("/outbound/text_only")
            && cfg.b("/outbound/voice_output")
            && !name.is_empty();
        let queue = enabled.then(|| Arc::new(AudioQueue::new(24000 * 2 * 8)));
        self.lane.replace_monitor(queue.clone());
        self.workers.retain(|worker| !worker.is_finished());
        if let Some(q) = queue {
            self.player(
                name,
                q,
                24000,
                audio::Playback::Monitor {
                    primary: cfg.s("/outbound/tts_device").to_owned(),
                },
            );
        }
    }
    fn player(&mut self, name: &str, q: Arc<AudioQueue>, rate: usize, output: audio::Playback) {
        let name = name.to_owned();
        let lane = self.lane.clone();
        self.workers.push(thread::spawn(move || {
            while lane.alive() && q.is_open() {
                let Err(e) = audio::playback(&lane, &q, &name, rate, &output) else {
                    break;
                };
                lane.playback_status(
                    &q,
                    matches!(output, audio::Playback::Monitor { .. }),
                    format!("{e:#}"),
                );
                q.clear();
                lane.sleep(Duration::from_secs(3));
            }
        }));
    }
}
impl Drop for Route {
    fn drop(&mut self) {
        self.lane.alive.store(false, Ordering::Release);
        for task in self.tasks.drain(..) {
            task.abort();
        }
        // COM activation may be slow on unplug. Cleanup must never stall the UI.
        let workers = std::mem::take(&mut self.workers);
        thread::spawn(move || {
            for w in workers {
                let _ = w.join();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_cable_starts_neither_outbound_audio_player() {
        use crate::setup::Cable;
        let mut cfg = Config::default();
        cfg.data["inbound"]["play_audio"] = true.into();
        cfg.data["outbound"]["voice_output"] = true.into();
        cfg.data["outbound"]["passthrough_while_translating"] = true.into();
        for cable in [Cable::Unknown, Cable::Missing, Cable::Inactive] {
            for text in [false, true] {
                cfg.data["outbound"]["text_only"] = text.into();
                for enabled in [true, false] {
                    assert_eq!(
                        Route::audio_outputs(&cfg, false, enabled, cable),
                        (false, false)
                    );
                }
                assert_eq!(Route::audio_outputs(&cfg, true, true, cable), (true, false));
            }
        }
        assert_eq!(
            Route::audio_outputs(&cfg, false, true, Cable::Ready),
            (false, true)
        );
        cfg.data["outbound"]["text_only"] = false.into();
        assert_eq!(
            Route::audio_outputs(&cfg, false, true, Cable::Ready),
            (true, true)
        );
    }
    #[test]
    fn first_inbound_words_and_translation_wake_the_overlay_before_the_next_tick() {
        let mut lane = Lane::new(true, 16000, true, Default::default(), None);
        lane.partial_interval = Duration::from_secs(10);
        lane.fragment(false, "이전 문장", "");
        lane.finish();
        let (tx, rx) = std::sync::mpsc::channel();
        let render = thread::spawn(move || {
            let worker = Arc::new(thread::current());
            tx.send(worker.clone()).unwrap();
            thread::park_timeout(Duration::from_secs(3));
            tx.send(worker).unwrap();
        });
        let worker = rx.recv().unwrap();
        lane.wake_vr_on_caption(&worker);
        lane.fragment(true, "Hello", "2");
        rx.recv_timeout(Duration::from_secs(1))
            .expect("caption must wake a parked VR renderer");
        assert_eq!(lane.captions.lock().pending[0].displayed.source, "Hello");
        lane.fragment(false, "안녕", "2");
        lane.fragment(false, "하세요", "2");
        assert_eq!(lane.captions.lock().pending[0].displayed.text, "안녕");
        lane.finish();
        assert_eq!(
            lane.captions.lock().history.back().unwrap().caption.text,
            "안녕하세요"
        );
        render.join().unwrap();
    }
    #[test]
    fn finals_publish_once_and_partial_updates_follow_the_saved_interval() {
        let (tx, mut rx) = watch::channel((String::new(), String::new()));
        let mut lane = Lane::new(
            false,
            16000,
            true,
            Default::default(),
            Some(ChatDisplay {
                tx,
                streaming: true,
                show_source: false,
            }),
        );
        lane.partial_interval = Duration::from_secs(2);
        lane.fragment(false, "첫", "");
        assert_eq!(rx.borrow_and_update().1, "첫");
        lane.fragment(false, " 문장", "");
        assert_eq!(lane.captions.lock().pending[0].displayed.text, "첫");
        assert!(!rx.has_changed().unwrap());
        lane.finish();
        assert_eq!(rx.borrow_and_update().1, "첫 문장");
        lane.finish_translated(Duration::from_millis(1));
        lane.finish();
        assert!(!rx.has_changed().unwrap());
        lane.fragment(false, "둘째 문장", "");
        lane.finish();
        assert_eq!(rx.borrow_and_update().1, "첫 문장 둘째 문장");
    }
    #[test]
    fn changing_monitor_preserves_primary_audio_and_text_mode_stays_silent() {
        let mut cfg = Config::default();
        cfg.data["outbound"]["text_only"] = false.into();
        cfg.data["outbound"]["voice_output"] = true.into();
        let lane = Arc::new(Lane::new(false, 16000, true, Default::default(), None));
        let mut route = Route {
            lane: lane.clone(),
            tasks: vec![],
            workers: vec![],
        };
        lane.play(&[1, 2]);
        cfg.data["outbound"]["monitor_device"] = "test monitor one".into();
        route.set_monitor(&cfg);
        let old = lane.monitor.lock().clone().unwrap();
        cfg.data["outbound"]["monitor_device"] = "test monitor two".into();
        route.set_monitor(&cfg);
        assert!(!old.is_open());
        let next = lane.monitor.lock().clone().unwrap();
        assert!(next.is_open());
        lane.playback_status(&old, true, "old error".into());
        assert!(lane.monitor_issue.lock().is_empty());
        cfg.data["outbound"]["monitor_device"] = "".into();
        route.set_monitor(&cfg);
        assert!(!next.is_open());
        assert!(lane.monitor.lock().is_none());
        assert_eq!(lane.tts.drain(), [1, 2]);
        assert!(lane.alive());
        cfg.data["outbound"]["monitor_device"] = "test monitor".into();
        cfg.data["outbound"]["text_only"] = true.into();
        route.set_monitor(&cfg);
        assert!(lane.monitor.lock().is_none());
    }
}
