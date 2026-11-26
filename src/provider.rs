use crate::{
    config::Config,
    runtime::{CaptionEvent, Lane, Phase},
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    net::TcpStream,
    sync::mpsc,
    time::{MissedTickBehavior, interval, timeout},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
const GEMINI: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";
const SONIOX: &str = "wss://stt-rt.soniox.com/transcribe-websocket";
const TTS: &str = "wss://tts-rt.soniox.com/tts-websocket";
fn message(v: Value) -> Message {
    Message::text(v.to_string())
}
fn lang<'a>(p: &str, code: &'a str) -> &'a str {
    if p == "gemini" {
        return code;
    }
    match (p, code) {
        ("qwen" | "soniox", "fil") => "tl",
        ("qwen", "no") => "nb",
        ("soniox", "nb") => "no",
        _ => code.split('-').next().unwrap_or(code),
    }
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
pub fn supports_target(provider: &str, code: &str) -> bool {
    provider != "openai"
        || matches!(
            lang(provider, code),
            "en" | "es"
                | "pt"
                | "fr"
                | "ja"
                | "ru"
                | "zh"
                | "de"
                | "ko"
                | "hi"
                | "id"
                | "vi"
                | "it"
        )
}
fn target<'a>(c: &'a Config, p: &str, i: bool) -> &'a str {
    lang(
        p,
        c.s(if i {
            "/inbound/target_language"
        } else {
            "/outbound/target_language"
        }),
    )
}
fn endpoint(c: &Config, p: &str) -> Result<String> {
    match p {
        "openai" => Ok(format!(
            "wss://api.openai.com/v1/realtime/translations?model={}",
            c.s("/openai/model")
        )),
        "soniox" => Ok(SONIOX.into()),
        "qwen" => {
            let base = c.s("/qwen/base_url");
            let id = c.s("/qwen/workspace_id");
            let china = c.s("/qwen/endpoint") == "beijing";
            ensure!(
                id.chars().all(|x| x.is_ascii_alphanumeric() || x == '-'),
                "Invalid Qwen workspace ID"
            );
            let url = if !base.is_empty() {
                ensure!(base.starts_with("wss://"), "Qwen URL requires wss://");
                base.into()
            } else if !id.is_empty() {
                format!(
                    "wss://{id}.{}.maas.aliyuncs.com/api-ws/v1/realtime",
                    if china {
                        "cn-beijing"
                    } else {
                        "ap-southeast-1"
                    }
                )
            } else {
                format!(
                    "wss://dashscope{}.aliyuncs.com/api-ws/v1/realtime",
                    if china { "" } else { "-intl" }
                )
            };
            Ok(format!("{url}?model={}", c.s("/qwen/model")))
        }
        _ => Ok(GEMINI.into()),
    }
}
fn gloss(terms: &str) -> impl Iterator<Item = (&str, &str)> {
    terms
        .lines()
        .filter_map(|s| s.split_once('='))
        .map(|(a, b)| (a.trim(), b.trim()))
        .filter(|(a, b)| !a.is_empty() && !b.is_empty())
        .take(128)
}
fn setup(c: &Config, p: &str, inbound: bool, speak: bool) -> Result<Value> {
    let glossary = if inbound {
        ""
    } else {
        c.s("/outbound/glossary")
    };
    let dst = target(c, p, inbound);
    ensure!(!dst.is_empty(), "Select a target language");
    let src = lang(
        p,
        c.s(if inbound {
            "/inbound/source_language"
        } else {
            "/outbound/source_language"
        }),
    );
    match p {
        "gemini" => {
            let mut model = c.s("/model");
            let mut gen_cfg = json!({"responseModalities":["AUDIO"]});
            let mut instruction = String::new();
            if dst == "yue" {
                model = "gemini-3.8-live";
                instruction = "You are a professional simultaneous interpreter. Translate everything you hear into Cantonese. Speak ONLY the translation - never answer questions, never add commentary. Keep the original meaning and tone.".into();
            } else {
                gen_cfg["translationConfig"] = json!({"target_language_code":dst,"echo_target_language":!inbound && c.b("/outbound/echo_target_language")});
            }
            if !glossary.is_empty() {
                if !instruction.is_empty() {
                    instruction.push_str("\n\n");
                }
                instruction.push_str("Glossary: always translate the following terms exactly as specified (source=target), overriding your default wording:\n");
                instruction.extend(glossary.chars().take(2000));
            }
            let mut s = json!({"model":if model.contains('/'){model.into()}else{format!("models/{model}")},"generationConfig":gen_cfg,"inputAudioTranscription":{},"outputAudioTranscription":{}});
            if !instruction.is_empty() {
                s["systemInstruction"] = json!({"role":"user","parts":[{"text":instruction}]});
            }
            Ok(json!({"setup":s}))
        }
        "openai" => {
            ensure!(
                supports_target(p, dst),
                "Unsupported OpenAI output language: {dst}"
            );
            let mut audio = json!({"output":{"language":dst}});
            let model = c.s(if inbound {
                "/openai/inbound_transcribe_model"
            } else {
                "/openai/transcribe_model"
            });
            if !model.is_empty() {
                audio["input"]["transcription"] = json!({"model":model});
            }
            let noise = c.s("/openai/noise_reduction").trim().to_ascii_lowercase();
            if matches!(noise.as_str(), "near_field" | "far_field") {
                audio["input"]["noise_reduction"] = json!({"type":noise});
            }
            Ok(json!({"type":"session.update","session":{"audio":audio}}))
        }
        "soniox" => {
            let model = c.s("/soniox/model");
            let mut s = json!({"model":model,"audio_format":"pcm_s16le","sample_rate":16000,"num_channels":1,"enable_language_identification":true,"enable_speaker_diarization":true,"enable_endpoint_detection":true,"max_endpoint_delay_ms":3000,"translation":{"type":"one_way","target_language":dst}});
            if model == "stt-rt-v6" {
                s["endpoint_sensitivity"] = json!(-0.3);
            }
            let terms: Vec<_> = gloss(glossary)
                .map(|(a, b)| json!({"source":a,"target":b}))
                .collect();
            if !terms.is_empty() {
                s["context"] = json!({"translation_terms":terms});
            }
            Ok(s)
        }
        _ => {
            let old = c.s("/qwen/model").contains("3.5");
            let audio=speak&&"zh en ar de fr es pt id it ko ru th vi ja tr hi ms nl ur nb sv da he fi pl is cs tl fa".split_whitespace().any(|l|l==dst);
            let modalities = if audio {
                json!(["text", "audio"])
            } else {
                json!(["text"])
            };
            let mut s = json!({"translation":{"language":dst}});
            if old {
                s["modalities"] = modalities;
                s["input_audio_format"] = "pcm".into();
                s["input_audio_transcription"] = json!({"model":"qwen3-asr-flash-realtime"});
                if !src.is_empty() {
                    s["input_audio_transcription"]["language"] = src.into();
                }
                if audio {
                    s["output_audio_format"] = "pcm".into();
                }
            } else {
                s["output_modalities"] = modalities;
                s["audio"] = json!({"input":{"turn_detection":{"type":"speaker_detection"}}});
            }
            if audio {
                let clone = c.s("/qwen/voice_clone");
                let voice = c.s("/qwen/voice");
                if matches!(clone, "once" | "always") || !voice.is_empty() {
                    let on = matches!(clone, "once" | "always");
                    s["voice"] = if on { "default" } else { voice }.into();
                    s["enable_voice_clone"] = true.into();
                    s["voice_clone_options"] = json!({"frequency":if on{clone}else{"never"}});
                }
            }
            let phrases: serde_json::Map<_, _> = gloss(glossary)
                .map(|(a, b)| (a.into(), Value::from(b)))
                .collect();
            if !phrases.is_empty() {
                s["translation"]["corpus"] = json!({"phrases":phrases});
            }
            Ok(json!({"type":"session.update","session":s}))
        }
    }
}
async fn connect(url: &str, p: &str, key: &str) -> Result<Socket> {
    let mut r = url.into_client_request()?;
    if p == "gemini" {
        r.headers_mut().insert(
            "x-goog-api-key",
            key.parse().context("Invalid API key characters")?,
        );
    } else {
        r.headers_mut().insert(
            "Authorization",
            format!("Bearer {key}")
                .parse()
                .context("Invalid API key characters")?,
        );
    }
    Ok(timeout(Duration::from_secs(10), connect_async(r)).await??.0)
}
fn audio_packet(p: &str, pcm: Vec<u8>) -> Message {
    if p == "soniox" {
        return Message::binary(pcm);
    }
    let data = STANDARD.encode(pcm);
    message(if p == "gemini" {
        json!({"realtime_input":{"audio":{"data":data,"mimeType":"audio/pcm;rate=16000"}}})
    } else {
        json!({"type":if p=="openai"{"session.input_audio_buffer.append"}else{"input_audio_buffer.append"},"audio":data})
    })
}
fn event(msg: Message) -> Result<Option<Value>> {
    match msg {
        Message::Text(s) => Ok(Some(serde_json::from_str(&s)?)),
        Message::Binary(b) => Ok(Some(serde_json::from_slice(&b)?)),
        Message::Close(Some(c)) => bail!("{} {}", c.code, c.reason),
        Message::Close(None) => bail!("Connection closed"),
        _ => Ok(None),
    }
}
fn check_event(v: &Value) -> Result<()> {
    let code = &v["error_code"];
    if v["type"] == "error"
        || (!code.is_null() && code != 0 && code != "0")
        || v["error"].is_object()
    {
        let detail = v["error"]["message"]
            .as_str()
            .or(v["error_message"].as_str())
            .or(v["error"]["code"].as_str())
            .unwrap_or("Service rejected the request");
        bail!("{}", detail.chars().take(240).collect::<String>());
    }
    Ok(())
}
pub async fn supervise(cfg: Config, lane: Arc<Lane>, speak: bool) {
    let p = cfg.engine(lane.inbound);
    let key = cfg.key(p);
    let mut backoff = 1;
    let mut sent_voice = None;
    let (url, frame) =
        match endpoint(&cfg, p).and_then(|url| Ok((url, setup(&cfg, p, lane.inbound, speak)?))) {
            Ok(v) => v,
            Err(e) => {
                *lane.issue.lock() = e.to_string();
                lane.phase(Phase::Stopped);
                return;
            }
        };
    loop {
        lane.phase(Phase::Listening);
        while !lane
            .last_voice
            .lock()
            .is_some_and(|v| Some(v) > sent_voice && v.elapsed() < Duration::from_secs(2))
        {
            lane.wake.notified().await;
        }
        lane.phase(Phase::Connecting);
        lane.input.keep_latest(lane.rate * 2);
        let session = async {
            let mut ws = connect(&url, p, &key).await?;
            ws.send(message(frame.clone())).await?;
            if p != "soniox" {
                timeout(Duration::from_secs(10), async {
                    while let Some(msg) = ws.next().await {
                        if let Some(v) = event(msg?)? {
                            check_event(&v)?;
                            if v.get("setupComplete").is_some()
                                || matches!(text(&v, "type"), "session.updated" | "session.created")
                            {
                                return Ok(());
                            }
                        }
                    }
                    bail!("Disconnected during setup")
                })
                .await??;
            }
            lane.issue.lock().clear();
            lane.phase(Phase::Live);
            let (tx, rx) = mpsc::channel(128);
            let run = stream(&cfg, p, &lane, speak, &mut ws, tx, &mut sent_voice);
            if speak && p == "soniox" {
                let speech_output = speech(&cfg, &lane, &key, rx, TTS);
                tokio::pin!(speech_output);
                tokio::select! {
                    result = run => {
                        result?;
                        timeout(Duration::from_secs(8),speech_output).await??;
                        Ok(())
                    }
                    result = &mut speech_output => result
                }
            } else {
                run.await
            }
        }
        .await;
        lane.finish();
        match session {
            Ok(()) => {
                backoff = 1;
            }
            Err(e) => {
                let detail = if key.is_empty() {
                    e.to_string()
                } else {
                    e.to_string().replace(&key, "[redacted]")
                };
                let lower = detail.to_lowercase();
                *lane.issue.lock() = detail.chars().take(250).collect();
                if [
                    "401",
                    "403",
                    "api key not valid",
                    "invalid api key",
                    "unauthenticated",
                    "invalidapikey",
                ]
                .iter()
                .any(|s| lower.contains(s))
                {
                    lane.phase(Phase::Stopped);
                    return;
                }
                lane.phase(Phase::Retry);
                tokio::time::sleep(Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(30);
            }
        }
    }
}
// Soniox timestamps refer to the PCM actually sent, including our endpoint
// padding. Wall clock time also includes handshake delays and idle keepalives.
#[derive(Default)]
struct SentAudio {
    samples: u64,
    audible: VecDeque<(u64, u64)>,
}
impl SentAudio {
    fn record(&mut self, pcm: &[u8]) {
        for frame in pcm.chunks(512) {
            let start = self.samples / 16; // 16 kHz mono, milliseconds
            self.samples += (frame.len() / 2) as u64;
            let end = self.samples.div_ceil(16);
            let count = frame.len() / 2;
            let energy: i64 = frame
                .chunks_exact(2)
                .map(|b| {
                    let v = i64::from(i16::from_le_bytes([b[0], b[1]]));
                    v * v
                })
                .sum();
            // Only reject near-digital silence here. Voice detection runs on capture.
            if count == 0 || energy < 16 * 16 * count as i64 {
                continue;
            }
            if let Some(last) = self.audible.back_mut()
                && start <= last.1 + 80
            {
                last.1 = end;
            } else {
                self.audible.push_back((start, end));
                if self.audible.len() > 512 {
                    self.audible.pop_front();
                }
            }
        }
    }
    fn overlaps(&self, start: u64, end: u64) -> bool {
        self.audible
            .iter()
            .rev()
            .any(|&(a, b)| a <= end.saturating_add(80) && start <= b.saturating_add(80))
    }
}
#[derive(Default)]
struct Turn {
    // Qwen interleaves source and translated items. Retain bounded per-item state.
    items: VecDeque<(String, String)>,
    speakers: VecDeque<(String, String)>,
    source_of: VecDeque<(String, String)>,
    speaker_ids: bool,
    pending: Vec<(String, String)>,
    recent_batches: VecDeque<u64>,
    recent_source: VecDeque<u64>,
    audio: Option<SentAudio>,
    last_seq: u64,
    tts_open: bool,
}
fn remember(v: &mut VecDeque<(String, String)>, id: &str, s: String) {
    if let Some(e) = v.iter_mut().find(|e| e.0 == id) {
        e.1 = s;
    } else {
        v.push_back((id.into(), s));
        if v.len() > 64 {
            v.pop_front();
        }
    }
}
fn lookup<'a>(v: &'a VecDeque<(String, String)>, id: &str) -> &'a str {
    v.iter()
        .find(|e| e.0 == id)
        .map(|e| e.1.as_str())
        .unwrap_or("")
}
impl Turn {
    fn update(
        &mut self,
        p: &str,
        v: &Value,
        l: &Lane,
        speak: bool,
        tts: &mpsc::Sender<Option<String>>,
    ) -> Result<bool> {
        check_event(v)?;
        if p == "gemini" {
            let sc = &v["serverContent"];
            l.fragment(true, text(&sc["inputTranscription"], "text"), "");
            l.fragment(false, text(&sc["outputTranscription"], "text"), "");
            if speak {
                for part in sc["modelTurn"]["parts"].as_array().into_iter().flatten() {
                    self.audio(l, text(&part["inlineData"], "data"))?;
                }
            }
            if sc["interrupted"] == true {
                l.interrupt();
            }
            if sc["turnComplete"] == true {
                l.finish();
            }
            return Ok(v.get("goAway").is_some());
        }
        if p == "soniox" {
            if let Some(seq) = v["seq"].as_u64() {
                if seq <= self.last_seq {
                    return Ok(false);
                }
                self.last_seq = seq;
            }
            let mut replayed_finals = false;
            if let Some(tokens) = v["tokens"].as_array()
                && tokens.iter().any(|t| {
                    t["is_final"] == true && t["start_ms"].is_number() && t["end_ms"].is_number()
                })
            {
                use std::hash::{Hash, Hasher};
                // Only recognize a replay when its finalized tokens include an audio position.
                let mut hash = std::hash::DefaultHasher::new();
                for t in tokens.iter().filter(|t| t["is_final"] == true) {
                    for key in [
                        "start_ms",
                        "end_ms",
                        "text",
                        "speaker",
                        "language",
                        "source_language",
                        "translation_status",
                    ] {
                        t[key].hash(&mut hash);
                    }
                }
                let id = hash.finish();
                if self.recent_batches.contains(&id) {
                    replayed_finals = true;
                } else {
                    self.recent_batches.push_back(id);
                    if self.recent_batches.len() > 64 {
                        self.recent_batches.pop_front();
                    }
                }
            }
            let mut confirmed = Vec::new();
            let mut drafts = Vec::new();
            let mut draft_sources = Vec::new();
            for t in v["tokens"].as_array().into_iter().flatten() {
                let stable = t["is_final"] == true;
                if (stable && replayed_finals) || (!stable && (!l.inbound || v["finished"] == true))
                {
                    continue;
                }
                let s = text(t, "text");
                if matches!(s, "<end>" | "<fin>") {
                    if !stable {
                        continue;
                    }
                    self.pending.clear();
                    drafts.clear();
                    draft_sources.clear();
                    confirmed.push(CaptionEvent::End);
                    if self.tts_open {
                        tts.try_send(None)?;
                        self.tts_open = false;
                    }
                    continue;
                }
                if s.is_empty() || s.starts_with('<') && s.ends_with('>') {
                    continue;
                }
                let src = t["translation_status"] != "translation";
                let language = text(t, if src { "language" } else { "source_language" });
                let mut who = match &t["speaker"] {
                    Value::String(s) => s.trim().to_string(),
                    Value::Number(n) => n.to_string(),
                    _ => String::new(),
                };
                if src {
                    if !stable && (!t["start_ms"].is_u64() || !t["end_ms"].is_u64()) {
                        continue;
                    }
                    if let (Some(start), Some(end)) = (t["start_ms"].as_u64(), t["end_ms"].as_u64())
                    {
                        use std::hash::{Hash, Hasher};
                        if self
                            .audio
                            .as_ref()
                            .is_some_and(|audio| !audio.overlaps(start, end))
                        {
                            continue;
                        }
                        let mut hash = std::hash::DefaultHasher::new();
                        (start, end, &who, language, s).hash(&mut hash);
                        let id = hash.finish();
                        if self.recent_source.contains(&id) {
                            continue;
                        }
                        if stable {
                            self.recent_source.push_back(id);
                            if self.recent_source.len() > 512 {
                                self.recent_source.pop_front();
                            }
                        }
                    }
                    let pair = (who.clone(), language.to_owned());
                    let sources = if stable {
                        &mut self.pending
                    } else {
                        &mut draft_sources
                    };
                    if !sources.contains(&pair) && sources.len() < 64 {
                        sources.push(pair);
                    }
                } else if !self
                    .pending
                    .iter()
                    .chain(draft_sources.iter().filter(|_| !stable))
                    .any(|(speaker, lang)| {
                        (language.is_empty() || lang.is_empty() || lang == language)
                            && (who.is_empty() || speaker.is_empty() || speaker == &who)
                    })
                {
                    // Translation has no timestamps of its own. Without a matching
                    // source in this segment it must not start a caption or TTS.
                    continue;
                } else if who.is_empty() {
                    let mut matching = self
                        .pending
                        .iter()
                        .chain(draft_sources.iter().filter(|_| !stable))
                        .filter(|(_, lang)| {
                            language.is_empty() || lang.is_empty() || lang == language
                        });
                    if let Some((first, _)) = matching.next()
                        && matching.all(|(other, _)| other == first)
                    {
                        who.clone_from(first);
                    }
                }
                let event = CaptionEvent::Text {
                    source: src,
                    expects_translation: t["translation_status"] == "original",
                    text: s,
                    speaker: who,
                    language,
                };
                if stable {
                    confirmed.push(event);
                } else {
                    drafts.push(event);
                }
                if stable && !src && speak {
                    tts.try_send(Some(s.into()))?;
                    self.tts_open = true;
                }
            }
            if v["tokens"].is_array() || v["finished"] == true {
                l.soniox_response(confirmed, drafts);
            }
            return Ok(v["finished"] == true);
        }
        let kind = text(v, "type");
        let id = text(v, "item_id");
        match kind {
            "session.input_transcript.delta" => l.fragment(true, text(v, "delta"), ""),
            "session.output_transcript.delta" => l.fragment(false, text(v, "delta"), ""),
            "session.output_audio.delta" | "response.audio.delta" if speak => {
                let audio = ["delta", "audio", "data"]
                    .iter()
                    .map(|key| text(v, key))
                    .find(|s| !s.is_empty())
                    .unwrap_or("");
                self.audio(l, audio)?
            }
            "input_audio_buffer.speech_started" if self.speaker_ids => {
                let s = &v["speaker_id"];
                if !id.is_empty() && !s.is_null() {
                    remember(
                        &mut self.speakers,
                        id,
                        s.as_str()
                            .map(String::from)
                            .unwrap_or_else(|| s.to_string()),
                    );
                }
            }
            "conversation.item.created" if self.speaker_ids && v["item"]["role"] == "assistant" => {
                let source = text(v, "previous_item_id");
                let target = text(&v["item"], "id");
                if !source.is_empty() && !target.is_empty() {
                    remember(&mut self.source_of, target, source.into());
                }
            }
            "response.done" if p != "qwen" => l.finish(),
            "session.finished" | "session.closed" => return Ok(true),
            k if k.starts_with("conversation.item.input_audio_transcription.")
                || k.starts_with("response.audio_transcript.")
                || k.starts_with("response.text.") =>
            {
                let src = k.starts_with("conversation");
                let key = format!("{}{id}", if src { "s" } else { "t" });
                let previous = lookup(&self.items, &key);
                let field = ["delta", "transcript", "text"]
                    .iter()
                    .find_map(|&f| v[f].as_str())
                    .unwrap_or("");
                let delta = if self.speaker_ids && k.ends_with(".delta") && !id.is_empty() {
                    field
                } else if self.speaker_ids
                    && id.is_empty()
                    && (k.ends_with(".done") || k.ends_with(".completed"))
                {
                    ""
                } else if let Some(suffix) = field.strip_prefix(previous) {
                    suffix
                } else if !k.ends_with(".done")
                    && !k.ends_with(".completed")
                    && !previous.starts_with(field)
                {
                    field
                } else {
                    ""
                };
                let accumulated = format!("{previous}{delta}");
                let source_id = if src { id } else { lookup(&self.source_of, id) };
                l.fragment(src, delta, lookup(&self.speakers, source_id));
                remember(&mut self.items, &key, accumulated);
                if id.is_empty() && (k.ends_with(".completed") || k.ends_with(".done")) {
                    remember(&mut self.items, &key, String::new());
                }
            }
            _ => {}
        }
        Ok(false)
    }
    fn audio(&self, l: &Lane, s: &str) -> Result<()> {
        if !s.is_empty() {
            l.play(&STANDARD.decode(s)?);
        }
        Ok(())
    }
}
async fn stream(
    c: &Config,
    p: &str,
    l: &Lane,
    speak: bool,
    ws: &mut Socket,
    tts: mpsc::Sender<Option<String>>,
    sent_voice: &mut Option<Instant>,
) -> Result<()> {
    let duration =
        Duration::from_millis(c.n("/audio/send_interval_ms", 50.).clamp(16., 200.) as u64);
    let mut tick = interval(duration);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let idle = if p == "soniox" && c.b("/soniox/keep_speaker_context") {
        let saved = c.n("/soniox/speaker_context_idle_sec", 60.);
        if saved > 0. { saved } else { 60. }
    } else {
        c.n("/audio/mic_idle_disconnect_sec", 15.)
    };
    let finalize = Duration::from_secs_f64(
        c.n(
            if l.inbound {
                "/audio/subtitle_finalize_silence_sec"
            } else {
                "/audio/finalize_silence_sec"
            },
            1.,
        )
        .clamp(0.2, 10.),
    );
    let mut turn = Turn {
        speaker_ids: c.speaker_ids(p),
        audio: (p == "soniox").then(SentAudio::default),
        ..Default::default()
    };
    let turn_end = Duration::from_secs_f64(
        if l.inbound {
            c.n("/audio/inbound_turn_end_silence_sec", 0.35)
        } else {
            c.n("/audio/turn_end_silence_sec", 0.55)
        }
        .clamp(duration.as_secs_f64(), 60.),
    );
    let max_padding = Duration::from_millis(if p == "soniox" { 4000 } else { 2500 });
    let mut last_send = Instant::now();
    let mut last_real = last_send;
    let mut first_audio = true;
    let mut padding_left = Duration::ZERO;
    let mut gemini_turn_open = false;
    let silence = audio_packet(
        p,
        vec![0; (l.rate as f64 * duration.as_secs_f64()) as usize * 2],
    );
    loop {
        tokio::select! {
            entry = ws.next() => {
                let msg = entry.ok_or_else(|| anyhow!("Connection ended"))??;
                let Some(v) = event(msg)? else { continue };
                let kind = text(&v, "type");
                if kind == "error" && text(&v["error"], "message").contains("still processing") {
                    continue;
                }
                if p == "gemini" && v["serverContent"]["turnComplete"] == true {
                    gemini_turn_open = false;
                    padding_left = Duration::ZERO;
                }
                if turn.update(p, &v, l, speak, &tts)? {
                    return Ok(());
                }
            }
            _ = tick.tick() => {
                let pcm = l.input.drain();
                let voice = *l.last_voice.lock();
                let real = !pcm.is_empty() && (first_audio || voice.is_some_and(|v| v.elapsed() <= turn_end || Some(v) > *sent_voice));
                let packet = if real {
                    if let Some(audio) = &mut turn.audio { audio.record(&pcm); }
                    last_real = Instant::now();
                    first_audio = false;
                    padding_left = max_padding;
                    gemini_turn_open = p == "gemini";
                    Some(audio_packet(p, pcm))
                } else if !padding_left.is_zero() && last_real.elapsed() >= Duration::from_millis(250) {
                    padding_left = padding_left.saturating_sub(duration);
                    if let (Some(audio), Message::Binary(bytes)) = (&mut turn.audio, &silence) {
                        audio.samples += (bytes.len() / 2) as u64;
                    }
                    Some(silence.clone())
                } else if gemini_turn_open && padding_left.is_zero() {
                    gemini_turn_open = false;
                    Some(message(json!({"realtime_input":{"audioStreamEnd":true}})))
                } else if p == "soniox" && last_send.elapsed() >= Duration::from_secs(5) {
                    Some(message(json!({"type": "keepalive"})))
                } else {
                    None
                };
                if let Some(packet) = packet {
                    timeout(Duration::from_secs(5), ws.send(packet)).await??;
                    last_send = Instant::now();
                    if real { *sent_voice = voice; }
                }
                if p == "soniox" {
                    l.publish_partial();
                } else {
                    l.finish_translated(finalize);
                }
                if idle <= 0. || voice.unwrap_or(last_real).elapsed().as_secs_f64() < idle.clamp(duration.as_secs_f64(), 3600.) {
                    continue;
                }
                let close = match p {
                    "soniox" => Message::text(""),
                    "qwen" => message(json!({"type": "session.finish"})),
                    "openai" => message(json!({"type": "session.close"})),
                    _ => message(json!({"realtime_input": {"audioStreamEnd": true}})),
                };
                timeout(Duration::from_secs(5), ws.send(close)).await??;
                let drain = async {
                    while let Some(m) = ws.next().await {
                        let Some(v) = event(m?)? else { continue };
                        if turn.update(p, &v, l, speak, &tts)? || v["serverContent"]["turnComplete"] == true {
                            break;
                        }
                    }
                    Ok::<_, anyhow::Error>(())
                };
                // Final transcripts can arrive during the provider's close handshake.
                let _ = timeout(Duration::from_secs(3), drain).await;
                return Ok(());
            }
        }
    }
}
async fn speech(
    c: &Config,
    l: &Lane,
    key: &str,
    mut rx: mpsc::Receiver<Option<String>>,
    url: &str,
) -> Result<()> {
    while let Some(first) = rx.recv().await {
        let Some(first) = first else { continue };
        let utterance = async {
            let ws = connect(url, "soniox", key).await?;
            let (mut sink, mut source) = ws.split();
            let id = "vrclt";
            sink.send(message(json!({
                "model": c.s("/soniox/tts_model"), "language": target(c, "soniox", l.inbound),
                "voice": c.s("/soniox/voice"), "audio_format": "pcm_s16le",
                "sample_rate": 24000, "stream_id": id
            })))
            .await?;
            let sender = async {
                let mut part = Some(first);
                loop {
                    let end = part.is_none();
                    sink.send(message(json!({
                        "stream_id": id, "text": part.take().unwrap_or_default(), "text_end": end
                    })))
                    .await?;
                    if end {
                        break;
                    }
                    part = rx.recv().await.flatten();
                }
                Ok::<_, anyhow::Error>(())
            };
            let receiver = async {
                let mut carry = Vec::new();
                let mut ended = false;
                while let Some(m) = source.next().await {
                    let Some(v) = event(m?)? else { continue };
                    check_event(&v)?;
                    if v["stream_id"] != id {
                        continue;
                    }
                    if let Some(audio) = v["audio"].as_str() {
                        carry.extend(STANDARD.decode(audio)?);
                        let aligned = carry.len() & !1;
                        l.play(&carry[..aligned]);
                        carry.drain(..aligned);
                    }
                    ended |= v["audio_end"] == true;
                    if v["terminated"] == true {
                        ensure!(ended && carry.is_empty(), "Incomplete Soniox audio");
                        return Ok(());
                    }
                }
                bail!("Soniox TTS disconnected")
            };
            tokio::try_join!(sender, receiver)?;
            Ok::<_, anyhow::Error>(())
        };
        timeout(Duration::from_secs(90), utterance).await??;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vr_revises_only_the_live_suffix_and_prioritizes_the_latest_translation() {
        use crate::{captions::recent, labels::Lang};
        let lane = Lane::new(true, 16000, true, Default::default(), None);
        let (tx, mut rx) = mpsc::channel(16);
        let mut turn = Turn::default();
        let mut feed = |tokens: Vec<Value>| {
            turn.update("soniox", &json!({"tokens": tokens}), &lane, false, &tx)
                .unwrap();
        };
        let source = |word, who, start: u32| json!({"text":word,"speaker":who,"language":"ja","translation_status":"original","is_final":true,"start_ms":start,"end_ms":start+50});
        let korean = |word, stable| json!({"text":word,"speaker":"1","source_language":"ja","translation_status":"translation","is_final":stable});
        let rows = |show| recent(&lane, Lang::Ko, 3, 7., show);
        feed(vec![
            source("こんにちは", "1", 0),
            source(&"long ".repeat(200), "2", 100),
        ]);
        assert!(rows(false).is_empty());
        feed(vec![korean("안녕", false)]);
        assert_eq!(rows(false).last().unwrap().body, "안녕");
        assert_eq!(rows(true).last().unwrap().body, "안녕");
        assert!(lane.captions.lock().pending[0].caption.text.is_empty());
        feed(vec![korean("안녕하세요", false)]);
        assert_eq!(rows(false).last().unwrap().body, "안녕하세요");
        feed(vec![korean("안녕", true), korean("하십니까", false)]);
        assert_eq!(rows(false).last().unwrap().body, "안녕하십니까");
        feed(vec![]);
        assert_eq!(rows(false).last().unwrap().body, "안녕");
        feed(vec![
            korean("하세요", true),
            json!({"is_final":true,"text":"<end>"}),
        ]);
        assert_eq!(rows(false).last().unwrap().body, "안녕하세요");
        assert_eq!(lane.captions.lock().history[0].caption.text, "안녕하세요");
        assert!(rx.try_recv().is_err());
        lane.captions.lock().history[0].body_changed = Instant::now() - Duration::from_secs(8);
        assert!(rows(false).is_empty());
        feed(vec![
            json!({"text":"여기요","language":"ko","translation_status":"none","speaker":"2","is_final":true}),
        ]);
        assert_eq!(rows(false).last().unwrap().body, "여기요");
    }

    #[test]
    fn captions_follow_source_order_and_do_not_guess_speakers() {
        let lane = Lane::new(true, 16000, true, Default::default(), None);
        let (tx, _rx) = mpsc::channel(4);
        let mut turn = Turn::default();
        for (source, who, words) in [
            (true, "1", "a"),
            (true, "2", "b"),
            (true, "1", "c"),
            (false, "2", "둘"),
            (false, "1", "하나"),
        ] {
            turn.update("soniox",&json!({"tokens":[{"text":words, "speaker":who, "is_final":true, "translation_status":if source {"original"} else {"translation"}}]}),&lane,false,&tx).unwrap();
        }
        // A translation from speaker 1 with no intervening translation speaker is still the same turn.
        turn.update("soniox",&json!({"tokens":[{"text":"?", "is_final":true, "translation_status":"translation"},{"text":"셋","speaker":1,"is_final":true,"translation_status":"translation"},{"text":"<end>","is_final":true}]}),&lane,false,&tx).unwrap();
        let t = lane.captions.lock();
        let actual: Vec<_> = t
            .history
            .iter()
            .map(|p| (&*p.caption.speaker, &*p.caption.source, &*p.caption.text))
            .collect();
        assert_eq!(
            actual,
            [
                ("1", "a", "하나"),
                ("2", "b", "둘"),
                ("1", "c", "셋"),
                ("", "", "?")
            ]
        );
    }
    #[test]
    fn shows_only_engine_supplied_speaker_ids() {
        let mut cfg = Config::default();
        assert!(cfg.speaker_ids("soniox") && cfg.speaker_ids("qwen"));
        assert!(!cfg.speaker_ids("gemini") && !cfg.speaker_ids("openai"));
        cfg.data["qwen"]["model"] = "qwen3.5-livetranslate-flash-realtime".into();
        assert!(!cfg.speaker_ids("qwen"));
        let (tx, _rx) = mpsc::channel(4);
        for (p, enabled) in [
            ("qwen", true),
            ("qwen", false),
            ("openai", false),
            ("gemini", false),
        ] {
            let lane = Lane::new(true, 16000, true, Default::default(), None);
            let mut turn = Turn {
                speaker_ids: enabled,
                ..Default::default()
            };
            for event in [
                json!({"type":"conversation.item.created","previous_item_id":"src","item":{"role":"assistant","id":"dst"}}),
                json!({"type":"input_audio_buffer.speech_started","item_id":"src","speaker_id":2}),
                if p == "gemini" {
                    json!({"serverContent":{"outputTranscription":{"text":"안녕하세요","speaker":2},"turnComplete":true}})
                } else {
                    json!({"type":"response.text.delta","item_id":"dst","delta":"안녕하세요"})
                },
            ] {
                turn.update(p, &event, &lane, false, &tx).unwrap();
            }
            lane.finish();
            let lines = crate::captions::recent(&lane, crate::labels::Lang::Ko, 3, 7., false);
            assert_eq!(lines.len(), 1);
            assert_eq!(lines[0].label, if enabled { "화자 2" } else { "" });
            assert_eq!(lines[0].body, "안녕하세요");
        }
    }
    #[tokio::test]
    async fn four_engines_stream_pcm_and_transcripts_over_local_websockets() {
        for provider in ["soniox", "gemini", "qwen", "openai"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                let packet = ws.next().await.unwrap().unwrap();
                let pcm = if provider == "soniox" {
                    packet.into_data().to_vec()
                } else {
                    let v = event(packet).unwrap().unwrap();
                    STANDARD
                        .decode(if provider == "gemini" {
                            text(&v["realtime_input"]["audio"], "data")
                        } else {
                            text(&v, "audio")
                        })
                        .unwrap()
                };
                assert_eq!(pcm, [3, 4, 5, 6]);
                let frames = match provider {
                    "gemini" => vec![
                        json!({"serverContent":{"inputTranscription":{"text":"hello"},"outputTranscription":{"text":"안녕"},"modelTurn":{"parts":[{"inlineData":{"data":"AQI="}}]},"turnComplete":true}}),
                        json!({"goAway":{}}),
                    ],
                    "soniox" => vec![
                        json!({"tokens":[{"is_final":false,"text":"draft"},{"is_final":true,"text":"hello","speaker":"1"},{"is_final":true,"translation_status":"translation","text":"안녕"},{"is_final":true,"text":"<end>"}],"finished":true}),
                    ],
                    "openai" => vec![
                        json!({"type":"session.input_transcript.delta","delta":"hello"}),
                        json!({"type":"session.output_transcript.delta","delta":"안녕"}),
                        json!({"type":"session.output_audio.delta","delta":"AQI="}),
                        json!({"type":"session.closed"}),
                    ],
                    _ => vec![
                        json!({"type":"conversation.item.input_audio_transcription.delta","delta":"hello","item_id":"a"}),
                        json!({"type":"response.text.delta","delta":"안녕","item_id":"b"}),
                        json!({"type":"response.text.done","text":"안녕","item_id":"b"}),
                        json!({"type":"response.audio.delta","delta":"AQI="}),
                        json!({"type":"response.done"}),
                        json!({"type":"session.finished"}),
                    ],
                };
                for v in frames {
                    ws.send(message(v)).await.unwrap();
                }
            });
            let c = Config::default();
            let lane = Lane::new(false, 16000, true, Default::default(), None);
            let mut socket = connect(&url, provider, "fixture").await.unwrap();
            lane.input.push(&[3, 4, 5, 6]);
            let (tx, mut rx) = mpsc::channel(8);
            timeout(
                Duration::from_secs(2),
                stream(&c, provider, &lane, true, &mut socket, tx, &mut None),
            )
            .await
            .unwrap()
            .unwrap();
            lane.finish();
            server.await.unwrap();
            let saved = lane.captions.lock();
            assert_eq!(saved.history.len(), 1, "{provider}");
            let cap = &saved.history[0].caption;
            assert_eq!((&*cap.source, &*cap.text), ("hello", "안녕"));
            if provider == "soniox" {
                assert_eq!(cap.speaker, "1");
                assert_eq!(rx.try_recv().unwrap().as_deref(), Some("안녕"));
                assert_eq!(rx.try_recv().unwrap(), None);
            } else {
                assert_eq!(lane.tts.drain(), [1, 2]);
            }
        }
    }
    #[tokio::test]
    async fn soniox_speech_drains_last_odd_sized_audio_packet() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
            for _ in 0..3 {
                assert!(ws.next().await.unwrap().is_ok());
            }
            for v in [
                json!({"stream_id":"vrclt","audio":"AQ=="}),
                json!({"stream_id":"vrclt","audio":"Ag==","audio_end":true}),
                json!({"stream_id":"vrclt","terminated":true}),
            ] {
                ws.send(message(v)).await.unwrap();
            }
        });
        let lane = Lane::new(false, 16000, true, Default::default(), None);
        let (tx, rx) = mpsc::channel(8);
        tx.send(Some("hello".into())).await.unwrap();
        tx.send(None).await.unwrap();
        drop(tx);
        timeout(
            Duration::from_secs(3),
            speech(&Config::default(), &lane, "fixture", rx, &url),
        )
        .await
        .unwrap()
        .unwrap();
        server.await.unwrap();
        assert_eq!(lane.tts.drain(), [1, 2]);
    }
    #[test]
    fn soniox_matches_delayed_translations_to_spoken_languages_and_accepts_real_repetitions() {
        let lane = Lane::new(true, 16000, true, Default::default(), None);
        let (tx, _rx) = mpsc::channel(16);
        let mut turn = Turn::default();
        let sources = json!({"tokens":[
            {"is_final":true,"text":"はい","language":"ja","speaker":"1","start_ms":10,"end_ms":350},
            {"is_final":true,"text":"好的","language":"zh","speaker":"1","start_ms":400,"end_ms":680}
        ]});
        for frame in [
            &sources,
            &sources,
            &json!({"tokens":[
                {"is_final":true,"text":"네","translation_status":"translation","source_language":"ja"},
                {"is_final":true,"text":"좋아요","translation_status":"translation","source_language":"zh"},
                {"is_final":true,"text":"<end>"}
            ]}),
            &json!({"tokens":[
                {"is_final":true,"text":"はい","language":"ja","speaker":"1","start_ms":901,"end_ms":1200},
                {"is_final":true,"text":"네","translation_status":"translation","source_language":"ja"},
                {"is_final":true,"text":"<end>"}
            ]}),
        ] {
            turn.update("soniox", frame, &lane, false, &tx).unwrap();
        }
        let t = lane.captions.lock();
        let rows = t
            .history
            .iter()
            .map(|r| (r.caption.source.as_str(), r.caption.text.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(rows, [("はい", "네"), ("好的", "좋아요"), ("はい", "네")]);
    }
    #[test]
    fn soniox_drops_silent_and_replayed_turns_before_caption_and_voice_output() {
        let lane = Lane::new(true, 16000, true, Default::default(), None);
        let (tx, mut rx) = mpsc::channel(16);
        let mut audio = SentAudio::default();
        let audible = 300_i16.to_le_bytes().repeat(8000);
        audio.record(&audible);
        audio.samples += 16_000 * 3;
        audio.record(&audible);
        let mut turn = Turn {
            audio: Some(audio),
            ..Default::default()
        };
        let source = |s, start, end, confidence| {
            json!({"is_final":true, "text":s,
            "language":"ja","speaker":"1","start_ms":start,"end_ms":end,"confidence":confidence})
        };
        let translated = |s| json!({"is_final":true,"translation_status":"translation","source_language":"ja","text":s});
        let end = json!({"is_final":true,"text":"<end>"});
        for tokens in [
            vec![
                source("出張", 10, 450, 0.9),
                translated("출장"),
                end.clone(),
            ],
            vec![source("出張", 10, 450, 0.8)],
            vec![translated("출장"), end.clone()],
            vec![
                source("ええ", 1500, 1800, 0.95),
                translated("음"),
                end.clone(),
            ],
            vec![translated("출장"), end.clone()],
            vec![source("出張", 3540, 3920, 0.9), translated("출장"), end],
        ] {
            turn.update("soniox", &json!({"tokens":tokens}), &lane, true, &tx)
                .unwrap();
        }
        let rows = lane.captions.lock();
        assert_eq!(rows.history.len(), 2);
        assert!(
            rows.history
                .iter()
                .all(|r| r.caption.source == "出張" && r.caption.text == "출장")
        );
        for _ in 0..2 {
            assert_eq!(rx.try_recv().unwrap().as_deref(), Some("출장"));
            assert_eq!(rx.try_recv().unwrap(), None);
        }
        assert!(rx.try_recv().is_err());
    }
    #[tokio::test]
    async fn silence_after_turn_cutoff_emits_padding_and_no_buffered_noise() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let lane = Lane::new(true, 16000, true, Default::default(), None);
        let server = async {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            assert_eq!(
                ws.next().await.unwrap().unwrap().into_data().as_ref(),
                &[1, 2]
            );
            tokio::time::sleep(Duration::from_millis(120)).await;
            lane.input.push(&[77, 77]);
            let next = ws.next().await.unwrap().unwrap();
            assert!(next.is_binary() && next.into_data().iter().all(|&n| n == 0));
            ws.send(message(json!({"finished":true}))).await.unwrap();
        };
        let client = async {
            let mut c = Config::default();
            c.data["audio"]["inbound_turn_end_silence_sec"] = 0.05.into();
            c.data["audio"]["turn_end_silence_sec"] = 2.0.into();
            let mut ws = connect(&endpoint, "soniox", "fixture").await.unwrap();
            *lane.last_voice.lock() = Some(Instant::now());
            lane.input.push(&[1, 2]);
            let (tx, _rx) = mpsc::channel(4);
            stream(&c, "soniox", &lane, false, &mut ws, tx, &mut None)
                .await
                .unwrap();
        };
        timeout(Duration::from_secs(3), async {
            tokio::join!(server, client);
        })
        .await
        .unwrap();
    }
    #[test]
    fn engine_protocols() {
        let mut c = Config::default();
        c.data["inbound"]["source_language"] = "en".into();
        c.data["outbound"]["source_language"] = "ko".into();
        for inbound in [false, true] {
            let opts = setup(&c, "soniox", inbound, false).unwrap();
            assert!(opts.get("language_hints").is_none());
            assert_eq!(opts["enable_language_identification"], true);
        }
        assert_eq!(
            setup(&c, "gemini", false, true).unwrap()["setup"]["generationConfig"]["translationConfig"]
                ["target_language_code"],
            "ja"
        );
        assert_eq!(
            setup(&c, "soniox", true, false).unwrap()["enable_speaker_diarization"],
            true
        );
        assert_eq!(
            setup(&c, "openai", false, true).unwrap()["session"]["audio"]["output"]["language"],
            "ja"
        );
        assert_eq!(
            setup(&c, "qwen", false, true).unwrap()["session"]["audio"]["input"]["turn_detection"]
                ["type"],
            "speaker_detection"
        );
        assert!(matches!(
            audio_packet("soniox", vec![0; 8]),
            Message::Binary(_)
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "Uses a paid Soniox session with a synthetic 48 kHz Japanese/Chinese/Japanese fixture (VRCLT_MIXED_PCM)"]
    async fn live_soniox_mixed_speech_and_silence() -> Result<()> {
        let mut cfg = Config::load(&Config::path())?;
        if std::env::var_os("VRCLT_TEST_ORIGINAL").is_some() {
            cfg.data["inbound"]["speech_filter"] = false.into();
        }
        ensure!(
            cfg.engine(true) == "soniox" && target(&cfg, "soniox", true) == "ko",
            "Test expects Soniox Korean captions"
        );
        ensure!(
            setup(&cfg, "soniox", true, false)?
                .get("language_hints")
                .is_none()
        );
        let bytes = std::fs::read(
            std::env::var_os("VRCLT_MIXED_PCM").context("Missing synthetic fixture")?,
        )?;
        let lane = Arc::new(Lane::new(true, 16000, true, Default::default(), None));
        use std::sync::atomic::Ordering::Relaxed;
        lane.speech_filter
            .store(cfg.b("/inbound/speech_filter"), Relaxed);
        let gpu = cfg.b("/inbound/speech_filter")
            && cfg.b("/inbound/nemotron_reuse")
            && std::env::var_os("VRCLT_TEST_CPU_ONLY").is_none();
        lane.nemotron.store(gpu, Relaxed);
        let session = tokio::spawn(supervise(cfg.clone(), lane.clone(), false));
        let input = lane.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let echo = Default::default();
            let mut feed = crate::audio::frame_processor(&cfg, &input, &echo)?;
            if input.speech_filter.load(Relaxed) && input.nemotron.load(Relaxed) {
                let until = Instant::now() + Duration::from_secs(180);
                while !matches!(
                    *input.gpu_filter.lock(),
                    crate::audio::reuse::Status::Active
                ) {
                    if let crate::audio::reuse::Status::Fallback(reason) = &*input.gpu_filter.lock()
                    {
                        bail!("GPU fallback: {reason}");
                    }
                    ensure!(Instant::now() < until, "GPU not ready");
                    feed(&mut [0; 768])?;
                    std::thread::sleep(Duration::from_millis(16));
                }
            }
            ensure!(
                input.last_voice.lock().is_none(),
                "Silence activated capture"
            );
            let mut next = Instant::now();
            let start = next;
            let mut times = [
                ("사과", None, None),
                ("회의", None, None),
                ("우산", None, None),
            ];
            let (mut gpu_blocks, mut cpu_assist_blocks) = (0, 0);
            for chunk in bytes.chunks(1536) {
                let mut frame = [0; 768];
                for (s, b) in frame.iter_mut().zip(chunk.chunks_exact(2)) {
                    *s = i16::from_le_bytes([b[0], b[1]]);
                }
                feed(&mut frame)?;
                let vr = crate::captions::recent(&input, crate::labels::Lang::Ko, 3, 7., false);
                let t = input.captions.lock();
                for (word, preview, confirmed) in &mut times {
                    if preview.is_none() && vr.iter().any(|r| r.body.contains(*word)) {
                        *preview = Some(start.elapsed().as_millis());
                    }
                    if confirmed.is_none()
                        && (t.pending.iter().any(|p| p.caption.text.contains(*word))
                            || t.history.iter().any(|r| r.caption.text.contains(*word)))
                    {
                        *confirmed = Some(start.elapsed().as_millis());
                    }
                }
                drop(t);
                if frame.iter().any(|v| v.unsigned_abs() > 400) {
                    match &*input.gpu_filter.lock() {
                        crate::audio::reuse::Status::Active => gpu_blocks += 1,
                        crate::audio::reuse::Status::Assisted => cpu_assist_blocks += 1,
                        _ => {}
                    }
                }
                next += Duration::from_millis(16);
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
            }
            if gpu {
                println!("Audible 16 ms frames: GPU {gpu_blocks}, CPU assist {cpu_assist_blocks}");
            }
            println!("Synthetic VR / confirmed token arrival (ms from fixture start): {times:?}");
            Ok(())
        })
        .await??;
        session.abort();
        let _ = session.await;
        let issue = lane.issue.lock().clone();
        ensure!(issue.is_empty(), "Soniox session failed: {issue}");
        let t = lane.captions.lock();
        let rows = t
            .history
            .iter()
            .map(|r| (&r.caption.source, &r.caption.text))
            .collect::<Vec<_>>();
        println!("Synthetic conversation: {rows:?}");
        for word in ["사과", "회의", "우산"] {
            ensure!(
                rows.iter().filter(|(_, dst)| dst.contains(word)).count() == 1,
                "Expected one translation for {word}"
            );
        }
        let noisy = std::env::var_os("VRCLT_TEST_NOISY_REPETITION").is_some();
        if noisy {
            ensure!(
                rows.iter()
                    .filter(|(src, dst)| src.contains("출장") || dst.contains("출장"))
                    .count()
                    == 2,
                "Expected both spoken instances of 출장"
            );
        }
        ensure!(
            t.pending.is_empty() && rows.len() == if noisy { 5 } else { 3 },
            "Extra or pending utterances after silence"
        );
        ensure!(
            *lane.phase.lock() == Phase::Listening,
            "Session did not return to idle"
        );
        Ok(())
    }
}
