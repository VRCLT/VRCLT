//! Soniox voice management. Network requests only start from an explicit UI action.
use crate::{
    audio::voice_sample::{self, Capture, CaptureMode},
    config::Config,
    labels::Lang,
    theme,
};
use anyhow::{Context, Result, bail, ensure};
use eframe::egui::{self, Ui};
use reqwest::{Client, RequestBuilder, multipart};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::Duration,
};
use tokio::runtime::Runtime;

const API: &str = "https://api.soniox.com/v1";
struct Voice {
    id: String,
    name: String,
    status: String,
}
fn voice(v: &Value, model: &str) -> Result<Voice> {
    let id = v["id"].as_str().context("Missing voice ID")?;
    ensure!(
        id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "Invalid voice ID"
    );
    let status = v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["model"] == model)
        .and_then(|m| m["status"].as_str())
        .unwrap_or("not_computed");
    Ok(Voice {
        id: id.into(),
        name: v["name"].as_str().unwrap_or(id).into(),
        status: status.into(),
    })
}
async fn request(req: RequestBuilder, key: &str) -> Result<Value> {
    let r = req.bearer_auth(key).send().await?;
    let status = r.status();
    let bytes = r.bytes().await?;
    let v: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)?
    };
    if !status.is_success() {
        let s = v["message"].as_str().unwrap_or("Soniox request failed");
        bail!(
            "Soniox {}: {}",
            status.as_u16(),
            s.replace(key, "[redacted]")
        );
    }
    Ok(v)
}
#[derive(Clone)]
enum Sample {
    Recording(Arc<Vec<u8>>),
    File(PathBuf),
}
enum Change {
    List,
    Created(String),
    Prepared(String),
    Deleted(String),
}
struct Library {
    rows: Vec<Voice>,
    limit: Option<u32>,
    change: Change,
    warning: String,
}
enum Command {
    List,
    Create(String, Sample),
    Delete(String),
    Prepare(String),
}
async fn perform(api: &str, cmd: Command, key: String, model: String) -> Result<Library> {
    ensure!(!key.is_empty(), "Soniox API key required");
    let client = Client::builder().timeout(Duration::from_secs(90)).build()?;
    let mut warning = String::new();
    let change = match cmd {
        Command::Create(name, sample) => {
            ensure!(
                (1..=128).contains(&name.trim().chars().count()),
                "Enter a voice name (1–128 characters)"
            );
            let payload = match sample {
                Sample::Recording(wav) => (wav.as_ref().clone(), "vrclt-voice-sample.wav".into()),
                Sample::File(path) => {
                    tokio::task::spawn_blocking(move || -> Result<_> {
                        use std::io::Read;
                        let file = std::fs::File::open(&path)?;
                        let mut data = vec![];
                        file.take(35 * 1024 * 1024 + 1).read_to_end(&mut data)?;
                        ensure!(
                            !data.is_empty() && data.len() <= 35 * 1024 * 1024,
                            "Choose an audio sample of up to 35 MB"
                        );
                        Ok((
                            data,
                            path.file_name()
                                .context("Invalid file name")?
                                .to_string_lossy()
                                .into_owned(),
                        ))
                    })
                    .await??
                }
            };
            let form = multipart::Form::new().text("name", name).part(
                "file",
                multipart::Part::bytes(payload.0).file_name(payload.1),
            );
            let created =
                request(client.post(format!("{api}/voices")).multipart(form), &key).await?;
            let id = voice(&created, &model)?.id;
            if let Err(error) = prepare(api, &client, &key, &model, &id).await {
                warning = error.to_string();
            }
            Change::Created(id)
        }
        Command::Delete(id) => {
            request(client.delete(format!("{api}/voices/{id}")), &key).await?;
            Change::Deleted(id)
        }
        Command::Prepare(id) => {
            prepare(api, &client, &key, &model, &id).await?;
            Change::Prepared(id)
        }
        Command::List => Change::List,
    };
    let mut limit = None;
    if matches!(change, Change::List) {
        if let Ok(v) = request(client.get(format!("{api}/tts-models")), &key).await {
            if let Some(m) = v["models"]
                .as_array()
                .and_then(|a| a.iter().find(|v| v["id"] == model))
            {
                limit = if m["supports_voice_cloning"] == false {
                    Some(0)
                } else {
                    m["voice_cloning_max_audio_duration_ms"]
                        .as_u64()
                        .filter(|n| *n > 0)
                        .map(|n| (n / 1000).min(voice_sample::LIMIT_SECONDS as u64) as u32)
                };
            }
        }
    }
    let mut result = vec![];
    let mut cursor = String::new();
    for _ in 0..20 {
        let mut r = client
            .get(format!("{api}/voices"))
            .query(&[("limit", "100")]);
        if !cursor.is_empty() {
            r = r.query(&[("cursor", &cursor)]);
        }
        let v = match request(r, &key).await {
            Ok(v) => v,
            Err(e) if !matches!(change, Change::List) => {
                warning = e.to_string();
                break;
            }
            Err(e) => return Err(e),
        };
        for row in v["voices"].as_array().into_iter().flatten() {
            result.push(voice(row, &model)?);
        }
        cursor = v["next_page_cursor"].as_str().unwrap_or("").into();
        if cursor.is_empty() {
            break;
        }
    }
    Ok(Library {
        rows: result,
        limit,
        change,
        warning,
    })
}
async fn prepare(api: &str, client: &Client, key: &str, model: &str, id: &str) -> Result<()> {
    let mut submitted = false;
    for _ in 0..60 {
        let url = format!("{api}/voices/{id}");
        let v = request(client.get(&url), key).await?;
        match voice(&v, model)?.status.as_str() {
            "ready" => return Ok(()),
            "failed" => bail!("Soniox could not prepare this voice"),
            "not_computed" if !submitted => {
                request(
                    client
                        .post(format!("{url}/recompute"))
                        .json(&json!({"model":model})),
                    key,
                )
                .await?;
                submitted = true;
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    bail!("Voice is still processing. Refresh the list later.")
}
#[derive(Clone, Copy, PartialEq)]
enum Page {
    Library,
    Record,
    File,
}
#[derive(Default)]
pub struct Voices {
    page: Option<Page>,
    name: String,
    path: String,
    available: Vec<Voice>,
    selected: String,
    loaded: bool,
    rx: Option<Receiver<Result<Library>>>,
    key: String,
    error: String,
    delete: Option<String>,
    capture: Option<Capture>,
    recorded: Option<Arc<Vec<u8>>>,
    limit: Option<u32>,
}
impl Voices {
    fn run(&mut self, cmd: Command, rt: &Runtime, c: &Config, ctx: egui::Context) {
        let key = c.key("soniox");
        let model = c.s("/soniox/tts_model").to_owned();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.error.clear();
        rt.spawn(async move {
            let _ = tx.send(perform(API, cmd, key, model).await);
            ctx.request_repaint();
        });
    }
    pub fn close(&mut self) {
        self.capture.take();
        self.page = None;
        self.delete = None;
    }
    pub fn panel(&mut self, ui: &mut Ui, c: &mut Config, l: Lang, rt: &Runtime) {
        let key = c.key("soniox");
        if self.key != key {
            *self = Self {
                key,
                ..Default::default()
            };
        }
        let current = c.s("/soniox/voice");
        let display = self
            .available
            .iter()
            .find(|v| v.id == current)
            .map(|v| v.name.as_str())
            .unwrap_or_else(|| {
                if current.len() == 36 {
                    l.t(
                        "저장된 복제 음성",
                        "Saved cloned voice",
                        "保存したクローン音声",
                        "已保存的克隆声音",
                    )
                } else {
                    current
                }
            });
        let display = display.to_owned();
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::bold(
                l.t("번역 목소리", "Translation voice", "翻訳の音声", "翻译声音"),
                16.,
            ));
            theme::hint(ui, &display);
            if ui
                .button(l.t("목소리 선택", "Choose voice", "音声を選ぶ", "选择声音"))
                .clicked()
            {
                self.page = Some(Page::Library);
                self.selected = current.into();
                if !self.key.is_empty() && self.rx.is_none() {
                    self.run(Command::List, rt, c, ui.ctx().clone());
                }
            }
            if ui
                .button(l.t(
                    "새 목소리 녹음",
                    "Record a voice",
                    "新しい音声を録音",
                    "录制新声音",
                ))
                .clicked()
            {
                self.page = Some(Page::Record);
                if !self.key.is_empty() && !self.loaded && self.rx.is_none() {
                    self.run(Command::List, rt, c, ui.ctx().clone());
                }
            }
        });
    }
    pub fn dialog(&mut self, ctx: &egui::Context, c: &mut Config, l: Lang, rt: &Runtime) {
        let Some(mut page) = self.page else { return };
        if let Some(result) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.rx = None;
            match result {
                Ok(lib) => {
                    self.available = lib.rows;
                    self.loaded = true;
                    if let Some(limit) = lib.limit {
                        self.limit = Some(limit);
                    }
                    self.error = lib.warning;
                    match lib.change {
                        Change::List => {}
                        Change::Prepared(id) => self.selected = id,
                        Change::Created(id) => {
                            self.selected = id;
                            self.recorded = None;
                            self.path.clear();
                            self.name.clear();
                            page = Page::Library;
                        }
                        Change::Deleted(id) => {
                            if c.s("/soniox/voice") == id {
                                c.data["soniox"]["voice"] = "Daniel".into();
                            }
                            self.selected.clear();
                        }
                    }
                }
                Err(e) => self.error = e.to_string(),
            }
        }
        if let Some(result) = self
            .capture
            .as_ref()
            .and_then(|cap| cap.result.try_recv().ok())
        {
            self.capture = None;
            match result {
                Ok(wav) if voice_sample::seconds(&wav) >= voice_sample::MIN_SECONDS => {
                    self.recorded = Some(Arc::new(wav));
                    self.error.clear();
                }
                Ok(_) => {
                    self.error = l
                        .t(
                            "3초 이상 녹음해 주세요.",
                            "Record at least three seconds.",
                            "3秒以上録音してください。",
                            "请至少录制三秒。",
                        )
                        .into()
                }
                Err(e) => {
                    self.error = format!(
                        "{}  {e}",
                        l.t(
                            "마이크를 확인해 주세요.",
                            "Check the microphone.",
                            "マイクを確認してください。",
                            "请检查麦克风。"
                        )
                    )
                }
            }
        }
        let mut close = false;
        let idle = self.rx.is_none();
        let frame = egui::Frame::new()
            .fill(theme::PAPER)
            .inner_margin(22)
            .corner_radius(12)
            .stroke(egui::Stroke::new(1_f32, theme::LINE));
        let modal = egui::Modal::new(egui::Id::new("soniox-voice-studio"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width((ctx.screen_rect().width() - 84.).clamp(280., 670.));
                ui.horizontal(|ui| {
                    ui.label(theme::bold(
                        l.t(
                            "Soniox 목소리",
                            "Soniox voices",
                            "Soniox の音声",
                            "Soniox 声音",
                        ),
                        23.,
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(l.t("닫기", "Close", "閉じる", "关闭")).clicked() {
                            close = true;
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(self.capture.is_none(), |ui| {
                        for (tab, label) in [
                            (
                                Page::Library,
                                l.t("내 목소리", "My voices", "自分の音声", "我的声音"),
                            ),
                            (
                                Page::Record,
                                l.t("마이크 녹음", "Microphone", "マイク録音", "麦克风录音"),
                            ),
                            (
                                Page::File,
                                l.t("파일 사용", "Audio file", "音声ファイル", "音频文件"),
                            ),
                        ] {
                            if ui.selectable_label(page == tab, label).clicked() {
                                page = tab;
                                self.delete = None;
                                if tab == Page::Library
                                    && !self.loaded
                                    && !self.key.is_empty()
                                    && idle
                                {
                                    self.run(Command::List, rt, c, ctx.clone());
                                }
                            }
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("voice-body")
                    .max_height((ctx.screen_rect().height() - 195.).max(160.))
                    .min_scrolled_height((ctx.screen_rect().height() - 195.).clamp(160., 480.))
                    .show(ui, |ui| {
                        if page == Page::Library {
                            if self.available.is_empty() {
                                theme::hint(
                                    ui,
                                    if !self.loaded && !self.key.is_empty() && self.rx.is_some() {
                                        l.t(
                                            "목소리를 불러오는 중…",
                                            "Loading voices…",
                                            "音声を読み込み中…",
                                            "正在加载声音…",
                                        )
                                    } else if self.key.is_empty() {
                                        l.t(
                                            "목록을 불러오려면 Soniox API 키를 입력하세요.",
                                            "Enter a Soniox API key to load voices.",
                                            "音声一覧の取得には API キーが必要です。",
                                            "输入 Soniox API 密钥以加载声音。",
                                        )
                                    } else {
                                        l.t(
                                            "등록된 목소리가 없습니다.",
                                            "No saved voices.",
                                            "保存した音声はありません。",
                                            "暂无已保存的声音。",
                                        )
                                    },
                                );
                            }
                            for voice in &self.available {
                                ui.horizontal(|ui| {
                                    if ui
                                        .selectable_label(self.selected == voice.id, &voice.name)
                                        .clicked()
                                    {
                                        self.selected = voice.id.clone();
                                        self.delete = None;
                                    }
                                    if c.s("/soniox/voice") == voice.id {
                                        ui.colored_label(
                                            theme::MINT,
                                            l.t("사용 중", "In use", "使用中", "使用中"),
                                        );
                                    } else if voice.status != "ready" {
                                        theme::hint(ui, &voice.status);
                                    }
                                });
                            }
                            ui.add_space(8.);
                            ui.add_enabled_ui(idle && !self.key.is_empty(), |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    let picked =
                                        self.available.iter().find(|v| v.id == self.selected);
                                    if let Some(v) = picked {
                                        let id = v.id.clone();
                                        let ready = v.status == "ready";
                                        if ui
                                            .add_enabled(
                                                v.status != "failed",
                                                egui::Button::new(if ready {
                                                    l.t(
                                                        "이 목소리 사용",
                                                        "Use voice",
                                                        "この音声を使う",
                                                        "使用此声音",
                                                    )
                                                } else {
                                                    l.t(
                                                        "음성 준비",
                                                        "Prepare voice",
                                                        "音声を準備",
                                                        "准备声音",
                                                    )
                                                }),
                                            )
                                            .clicked()
                                        {
                                            if ready {
                                                c.data["soniox"]["voice"] = id.clone().into();
                                                close = true;
                                            } else {
                                                self.run(
                                                    Command::Prepare(id.clone()),
                                                    rt,
                                                    c,
                                                    ctx.clone(),
                                                );
                                            }
                                        }
                                        if ui
                                            .button(l.t("삭제", "Delete", "削除", "删除"))
                                            .clicked()
                                        {
                                            self.delete = Some(id);
                                        }
                                    }
                                    if ui
                                        .button(l.t("새로 고침", "Refresh", "更新", "刷新"))
                                        .clicked()
                                    {
                                        self.run(Command::List, rt, c, ctx.clone());
                                    }
                                });
                                if let Some(id) = self.delete.clone() {
                                    ui.horizontal_wrapped(|ui| {
                                        let name = self
                                            .available
                                            .iter()
                                            .find(|v| v.id == id)
                                            .map(|v| v.name.as_str())
                                            .unwrap_or("");
                                        ui.label(format!(
                                            "{}  {name}",
                                            l.t(
                                                "삭제할 목소리",
                                                "Voice to delete",
                                                "削除する音声",
                                                "要删除的声音"
                                            )
                                        ));
                                        if ui
                                            .button(l.t(
                                                "영구 삭제",
                                                "Delete permanently",
                                                "完全に削除",
                                                "永久删除",
                                            ))
                                            .clicked()
                                        {
                                            self.delete = None;
                                            self.run(Command::Delete(id), rt, c, ctx.clone());
                                        }
                                        if ui
                                            .button(l.t("취소", "Cancel", "キャンセル", "取消"))
                                            .clicked()
                                        {
                                            self.delete = None;
                                        }
                                    });
                                }
                            });
                        } else {
                            self.create_form(ui, c, l, rt, page);
                        }
                        if self.rx.is_some() {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                theme::hint(
                                    ui,
                                    l.t(
                                        "Soniox 연결 중",
                                        "Contacting Soniox",
                                        "Soniox に接続中",
                                        "正在连接 Soniox",
                                    ),
                                );
                            });
                        }
                        if !self.error.is_empty() {
                            ui.colored_label(theme::ALERT, &self.error);
                        }
                    });
            });
        self.page = Some(page);
        if close || modal.should_close() {
            self.close();
        }
    }
    fn create_form(&mut self, ui: &mut Ui, c: &Config, l: Lang, rt: &Runtime, page: Page) {
        let limit = self.limit.unwrap_or(voice_sample::LIMIT_SECONDS);
        if page == Page::Record {
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::bold(
                    l.t("읽을 대사", "Reading script", "読み上げる文章", "朗读稿"),
                    18.,
                ));
                theme::hint(ui, l.t("약 1분", "About 1 minute", "約1分", "约1分钟"));
            });
            egui::Frame::new()
                .fill(theme::INK)
                .inner_margin(14)
                .corner_radius(7)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(l.t(
                            include_str!("voices/ko.txt"),
                            include_str!("voices/en.txt"),
                            include_str!("voices/ja.txt"),
                            include_str!("voices/zh.txt"),
                        ))
                        .size(17.),
                    );
                });
            let mic = c.s("/outbound/mic_device");
            let seconds = self
                .capture
                .as_ref()
                .map(|c| c.progress().frames as u32 / 48000)
                .or_else(|| self.recorded.as_ref().map(|w| voice_sample::seconds(w)))
                .unwrap_or(0);
            ui.horizontal_wrapped(|ui| {
                let label = if self.capture.is_some() {
                    l.t("녹음 마치기", "Stop recording", "録音を終了", "结束录音")
                } else if self.recorded.is_some() {
                    l.t("다시 녹음", "Record again", "録り直す", "重新录制")
                } else {
                    l.t("녹음 시작", "Start recording", "録音を開始", "开始录音")
                };
                if ui
                    .add_enabled(
                        limit >= voice_sample::MIN_SECONDS,
                        egui::Button::new(label).min_size(egui::vec2(112., 36.)),
                    )
                    .clicked()
                {
                    if let Some(cap) = &self.capture {
                        cap.stop();
                    } else {
                        self.error.clear();
                        self.capture = Some(Capture::start(
                            mic.into(),
                            CaptureMode::Record(limit),
                            ui.ctx().clone(),
                        ));
                    }
                }
                ui.label(format!(
                    "{}:{:02} / {}:{:02}",
                    seconds / 60,
                    seconds % 60,
                    limit / 60,
                    limit % 60
                ));
                theme::hint(
                    ui,
                    if mic.is_empty() {
                        l.default_device()
                    } else {
                        mic
                    },
                );
            });
            if let Some(cap) = &self.capture {
                theme::meter(ui, cap.progress().level, false);
                ui.ctx().request_repaint_after(Duration::from_millis(80));
            }
        } else {
            ui.label(l.t(
                "오디오 샘플 파일",
                "Audio sample file",
                "音声サンプルファイル",
                "音频样本文件",
            ));
            ui.add(
                egui::TextEdit::singleline(&mut self.path)
                    .desired_width(f32::INFINITY)
                    .hint_text(l.t(
                        "파일을 끌어 놓거나 경로를 붙여 넣으세요",
                        "Drop a file or paste its path",
                        "ファイルをドロップ、またはパスを貼り付け",
                        "拖入文件或粘贴路径",
                    )),
            );
            if let Some(path) =
                ui.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()))
            {
                if self.name.is_empty() {
                    self.name = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .chars()
                        .take(128)
                        .collect();
                }
                self.path = path.to_string_lossy().into_owned();
            }
        }
        ui.label(l.t("목소리 이름", "Voice name", "音声名", "声音名称"));
        ui.add(
            egui::TextEdit::singleline(&mut self.name)
                .char_limit(128)
                .desired_width(300.),
        );
        theme::hint(ui,l.t("본인 또는 허락받은 목소리를 사용하세요. 음성 만들기를 누르면 샘플이 Soniox에 전송됩니다.","Use a voice you own or have permission to use. Creating a voice sends the sample to Soniox.","自分の声か許可を得た声を使用してください。作成時に Soniox に送信されます。","请使用本人或获准的声音。创建声音时会将样本发送给 Soniox。"));
        let sample = if page == Page::Record {
            self.recorded
                .clone()
                .filter(|w| voice_sample::seconds(w) <= limit)
                .map(Sample::Recording)
        } else {
            (!self.path.trim().is_empty())
                .then(|| Sample::File(self.path.trim().trim_matches('"').into()))
        };
        if self.key.is_empty() {
            theme::hint(
                ui,
                l.t(
                    "음성을 만들려면 Soniox API 키를 입력하세요.",
                    "Enter a Soniox API key to create a voice.",
                    "音声を作成するには API キーを入力してください。",
                    "创建声音需要 Soniox API 密钥。",
                ),
            );
        }
        if ui
            .add_enabled(
                self.rx.is_none()
                    && self.capture.is_none()
                    && sample.is_some()
                    && !self.name.trim().is_empty()
                    && !self.key.is_empty()
                    && limit > 0,
                egui::Button::new(l.t(
                    "Soniox에 음성 만들기",
                    "Create voice in Soniox",
                    "Soniox で音声を作成",
                    "在 Soniox 创建声音",
                )),
            )
            .clicked()
        {
            self.run(
                Command::Create(self.name.trim().into(), sample.unwrap()),
                rt,
                c,
                ui.ctx().clone(),
            );
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    #[tokio::test]
    async fn uploads_recorded_sample_and_refreshes_library() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", server.local_addr().unwrap());
        let recording: Vec<_> = b"RIFFtest-recording-fixture".to_vec();
        let expected = recording.clone();
        let worker = std::thread::spawn(move || {
            for step in 0..3 {
                let (mut sock, _) = server.accept().unwrap();
                let mut reader = BufReader::new(&sock);
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line);
                }
                let name = headers.lines().next().unwrap();
                assert!(
                    headers
                        .to_ascii_lowercase()
                        .contains("authorization: bearer test-only")
                );
                let voice = json!({"id":"sample-id","name":"test voice","models":[{"model":"test-model","status":"ready"}]});
                let result = match step {
                    0 => {
                        assert!(name.starts_with("POST /v1/voices "));
                        let n: usize = headers
                            .lines()
                            .find_map(|s| {
                                s.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(|n| n.parse().unwrap())
                            })
                            .unwrap();
                        let mut body = vec![0; n];
                        reader.read_exact(&mut body).unwrap();
                        assert!(body.windows(expected.len()).any(|w| w == expected));
                        assert!(String::from_utf8_lossy(&body).contains("vrclt-voice-sample.wav"));
                        voice
                    }
                    1 => {
                        assert!(name.starts_with("GET /v1/voices/sample-id "));
                        voice
                    }
                    _ => {
                        assert!(name.starts_with("GET /v1/voices?"));
                        json!({"voices":[voice]})
                    }
                }
                .to_string();
                write!(sock,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",result.len(),result).unwrap();
            }
        });
        let result = perform(
            &base,
            Command::Create("test voice".into(), Sample::Recording(Arc::new(recording))),
            "test-only".into(),
            "test-model".into(),
        )
        .await
        .unwrap();
        worker.join().unwrap();
        assert!(matches!(result.change,Change::Created(ref id) if id=="sample-id"));
        assert_eq!(result.rows[0].status, "ready");
        assert!(result.warning.is_empty());
    }
}
