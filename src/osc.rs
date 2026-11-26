use crate::{
    config::Config,
    labels::{Lang, lang_name},
};
use eframe::egui;
use rosc::{OscMessage, OscPacket, OscType};
use std::{
    collections::VecDeque,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};
use tokio::{net::UdpSocket, sync::watch};

fn parts(src: &str, dst: &str) -> VecDeque<String> {
    let a: Vec<_> = src.trim().chars().collect();
    let b: Vec<_> = dst.trim().chars().collect();
    let count = (a.len() + b.len() + 2).div_ceil(143).max(1);
    (0..count)
        .filter_map(|i| {
            let a: String = a[i * a.len() / count..(i + 1) * a.len() / count]
                .iter()
                .collect();
            let b: String = b[i * b.len() / count..(i + 1) * b.len() / count]
                .iter()
                .collect();
            let s = [a.trim(), b.trim()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            (!s.is_empty()).then_some(s)
        })
        .collect()
}
pub struct Chatbox {
    pub captions: watch::Sender<(String, String)>,
    notices: tokio::sync::mpsc::Sender<String>,
    task: tokio::task::JoinHandle<()>,
    on: bool,
    language: String,
}
impl Chatbox {
    pub fn start(rt: &tokio::runtime::Handle, c: &Config) -> Self {
        let (captions, rx) = watch::channel(Default::default());
        let (notices, feedback) = tokio::sync::mpsc::channel(8);
        Self {
            captions,
            notices,
            task: rt.spawn(chatbox(c.clone(), rx, feedback)),
            on: c.b("/dashboard/translation_on"),
            language: c.s("/outbound/target_language").into(),
        }
    }
    pub fn sync(&mut self, c: &Config, l: Lang) {
        self.captions.send_replace(Default::default());
        let on = c.b("/dashboard/translation_on");
        let language = c.s("/outbound/target_language");
        if c.b("/control/feedback_chatbox") {
            if self.on != on {
                let label = if on {
                    l.t("번역 ON", "Translation ON", "翻訳 ON", "翻译 开")
                } else if c.b("/outbound/voice_output")
                    || c.b("/outbound/passthrough_while_translating")
                    || c.b("/outbound/text_only")
                {
                    l.t(
                        "번역 OFF (원음 송출)",
                        "Translation OFF (voice passthrough)",
                        "翻訳 OFF (原音送出)",
                        "翻译 关（原声直传）",
                    )
                } else {
                    l.t(
                        "번역 OFF (텍스트 전송 중지)",
                        "Translation OFF (text output stopped)",
                        "翻訳 OFF (テキスト送信停止)",
                        "翻译 关（文本发送停止）",
                    )
                };
                let _ = self.notices.try_send(format!("[vrclt] {label}"));
            }
            if self.language != language {
                let label = l.t("번역 언어", "Translation language", "翻訳言語", "翻译语言");
                let _ = self
                    .notices
                    .try_send(format!("[vrclt] {label}: {}", lang_name(language)));
            }
        }
        self.on = on;
        self.language = language.into();
    }
}
impl Drop for Chatbox {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn chatbox(
    c: Config,
    mut rx: watch::Receiver<(String, String)>,
    mut feedback: tokio::sync::mpsc::Receiver<String>,
) {
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0").await else {
        return;
    };
    let dest = (c.s("/osc/ip"), c.n("/osc/port", 9000.) as u16);
    let throttle = Duration::from_secs_f64(c.n("/osc/throttle_sec", 1.5).clamp(1.5, 60.));
    let chunk = Duration::from_secs_f64(
        c.n("/osc/chunk_display_sec", 4.)
            .clamp(throttle.as_secs_f64(), 120.),
    );
    let mut last = Instant::now() - chunk;
    let mut wait = throttle;
    let mut pending = VecDeque::new();
    let mut notice = None;
    let streaming = c.b("/osc/stream_sentences");
    loop {
        tokio::select! {
            biased;
            msg=feedback.recv(),if notice.is_none()=>{
                let Some(msg) = msg else {return;};
                notice=Some(msg);
                wait=throttle;
            }
            r=rx.changed()=>{
                if r.is_err(){return;}
                let pair=rx.borrow_and_update();
                let src=if c.b("/osc/show_source"){&pair.0}else{""};
                pending=if pair.1.is_empty() {VecDeque::new()} else if streaming {VecDeque::from([rolling_message(src,&pair.1)])}else{parts(src,&pair.1)};
                wait=throttle;
            }
            _=tokio::time::sleep_until((last+wait).into()),if notice.is_some() || !pending.is_empty()=>{
                let was_notice=notice.is_some();
                let line=notice.take().or_else(||pending.pop_front()).unwrap();let msg=OscPacket::Message(OscMessage{addr:"/chatbox/input".into(),args:vec![OscType::String(line),OscType::Bool(true),OscType::Bool(c.b("/osc/notification_sfx"))]});
                if let Ok(b)=rosc::encoder::encode(&msg){let _=sock.send_to(&b,dest).await;}last=Instant::now();wait=if was_notice {throttle} else {chunk};
            }
        }
    }
}
fn rolling_message(src: &str, dst: &str) -> String {
    let (n, m) = (src.chars().count(), dst.chars().count());
    let limit = 144 - usize::from(n > 0 && m > 0);
    let keep_dst = m.min(limit.saturating_sub(n).max(limit.div_ceil(2)));
    let source: String = src
        .chars()
        .skip(n.saturating_sub(limit - keep_dst))
        .collect();
    let target: String = dst.chars().skip(m - keep_dst).collect();
    [source, target]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
#[derive(Debug, Clone)]
pub enum Action {
    VoiceVolume(i32),
    VoiceMonitor(String),
    SpeechFilter(bool),
    Nemotron(bool),
    TextOnly(bool),
    StartSession,
    Interface(&'static str),
    SpeakerContext,
    Translation(bool),
    Hold(bool),
    ToggleTranslation,
    ToggleSubtitles,
    Language(String),
    SubtitleLanguage(String),
    CaptionDisplay(crate::vr::DisplayChange),
    PlacePanel(crate::vr::Placement),
    Microphone(String),
    Show,
    Warning(String),
}
pub async fn listen(
    health: std::sync::Arc<std::sync::atomic::AtomicU8>,
    c: Config,
    tx: Sender<Action>,
    ctx: egui::Context,
) {
    use std::sync::atomic::Ordering::Relaxed;
    let sock =
        match UdpSocket::bind(("127.0.0.1", c.n("/control/osc_listen_port", 9001.) as u16)).await {
            Ok(s) => s,
            Err(e) => {
                health.store(3, Relaxed);
                let _ = tx.send(Action::Warning(format!("OSC: {e}")));
                ctx.request_repaint();
                return;
            }
        };
    health.store(1, Relaxed);
    let enabled = format!("/avatar/parameters/{}", c.s("/control/param_enabled"));
    let lang = format!("/avatar/parameters/{}", c.s("/control/param_lang"));
    let mut buf = [0; 8192];
    while let Ok((n, _)) = sock.recv_from(&mut buf).await {
        let Ok((_, pkt)) = rosc::decoder::decode_udp(&buf[..n]) else {
            continue;
        };
        health.store(2, Relaxed);
        let mut packets = vec![pkt];
        while let Some(packet) = packets.pop() {
            match packet {
                OscPacket::Bundle(b) => packets.extend(b.content.into_iter().rev()),
                OscPacket::Message(m) => {
                    let a = match (m.addr.as_str(), m.args.first()) {
                        (addr, Some(OscType::Bool(v))) if addr == enabled => {
                            Some(Action::Translation(*v))
                        }
                        (addr, Some(OscType::Int(v))) if addr == enabled => {
                            Some(Action::Translation(*v != 0))
                        }
                        (addr, Some(OscType::Float(v))) if addr == enabled && v.is_finite() => {
                            Some(Action::Translation(*v != 0.))
                        }
                        (addr, Some(OscType::Int(i))) if addr == lang && *i >= 0 => {
                            c.data["control"]["languages"]
                                .get(*i as usize)
                                .and_then(|s| s.as_str())
                                .map(|s| Action::Language(s.into()))
                        }
                        _ => None,
                    };
                    if let Some(a) = a {
                        let _ = tx.send(a);
                        ctx.request_repaint();
                    }
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn translation_switch_emits_each_notice_once_without_microphone_audio() {
        let probe = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = Config::default();
        cfg.data["osc"]["ip"] = "127.0.0.1".into();
        cfg.data["osc"]["port"] = probe.local_addr().unwrap().port().into();
        cfg.data["dashboard"]["translation_on"] = true.into();
        let mut chat = Chatbox::start(&tokio::runtime::Handle::current(), &cfg);
        for (on, expected) in [
            (false, "[vrclt] 번역 OFF (원음 송출)"),
            (true, "[vrclt] 번역 ON"),
        ] {
            cfg.data["dashboard"]["translation_on"] = on.into();
            chat.sync(&cfg, Lang::Ko);
            chat.sync(&cfg, Lang::Ko);
            let mut bytes = [0; 1024];
            let (n, _) = tokio::time::timeout(Duration::from_secs(3), probe.recv_from(&mut bytes))
                .await
                .unwrap()
                .unwrap();
            let (_, OscPacket::Message(msg)) = rosc::decoder::decode_udp(&bytes[..n]).unwrap()
            else {
                panic!("expected OSC message")
            };
            assert_eq!(msg.addr, "/chatbox/input");
            assert_eq!(
                msg.args,
                vec![
                    OscType::String(expected.into()),
                    OscType::Bool(true),
                    OscType::Bool(false)
                ]
            );
        }
        cfg.data["control"]["feedback_chatbox"] = false.into();
        cfg.data["dashboard"]["translation_on"] = false.into();
        chat.sync(&cfg, Lang::Ko);
        let mut bytes = [0; 1024];
        assert!(
            tokio::time::timeout(Duration::from_millis(1600), probe.recv_from(&mut bytes))
                .await
                .is_err()
        );
    }
    #[test]
    fn unicode_pairs_fit() {
        let s = "안녕하세요".repeat(55);
        let d = "日本語".repeat(181);
        let p = parts(&s, &d);
        assert!(p.iter().all(|x| x.chars().count() <= 144));
        assert_eq!(
            p.iter()
                .flat_map(|x| x.split('\n').nth(1))
                .collect::<String>(),
            d
        );
    }
}
