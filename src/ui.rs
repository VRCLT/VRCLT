mod checkup;
mod subtitles;
use crate::{
    audio,
    config::{Config, ENGINES},
    labels::{LANGUAGES, Lang, engine_name, lang_name},
    osc::{self, Action},
    platform,
    runtime::{Caption, Phase, Route},
    setup::Cable,
    theme::*,
};
use eframe::egui::*;
use parking_lot::Mutex;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};
use tokio::{runtime::Runtime, task::JoinHandle};
use tray_icon::{MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
#[derive(Clone, Copy, PartialEq)]
enum Page {
    Live,
    Setup(usize),
}
pub struct Studio {
    cfg: Config,
    draft: Config,
    path: PathBuf,
    page: Page,
    lang: Lang,
    checks: checkup::Checkup,
    osc_health: Arc<std::sync::atomic::AtomicU8>,
    installer: Option<Receiver<anyhow::Result<()>>>,
    running: bool,
    pending_volume: Option<(i32, Instant)>,
    rt: Runtime,
    routes: [Option<Route>; 2],
    past_captions: [VecDeque<(Instant, Caption)>; 2],
    conversation_cleared_at: Option<Instant>,
    echo: Arc<Mutex<Option<Instant>>>,
    events: Receiver<Action>,
    tx: Sender<Action>,
    osc: Option<JoinHandle<()>>,
    chatbox: Option<osc::Chatbox>,
    devices_rx: Option<Receiver<anyhow::Result<crate::setup::Environment>>>,
    devices: crate::setup::Environment,
    devices_ready: bool,
    error: String,
    load_error: bool,
    exiting: bool,
    about: bool,
    voice_help: bool,
    voices: crate::voices::Voices,
    held: bool,
    tray: Option<TrayIcon>,
    overlay: subtitles::DesktopOverlay,
    vr: Option<crate::vr::Vr>,
    hotkeys: Option<crate::hotkeys::Hotkeys>,
}
impl Studio {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        path: PathBuf,
        profile: Option<String>,
        icon: IconData,
    ) -> Self {
        let ctx = &cc.egui_ctx;
        platform::register_console(cc);
        setup(ctx);
        let config = Config::load(&path);
        let error = config
            .as_ref()
            .err()
            .map(|e| format!("{}  ({})", e, path.display()))
            .unwrap_or_default();
        let mut cfg = config.unwrap_or_default();
        if !path.exists() && cfg.key("gemini").is_empty() {
            cfg.data["provider"] = "soniox".into();
        }
        if let Some(mode) = profile {
            cfg.profile(&mode);
        }
        let locale = platform::locale();
        let lang = Lang::new(if cfg.s("/ui/lang").is_empty() {
            &locale
        } else {
            cfg.s("/ui/lang")
        });
        let first =
            !path.exists() || (!cfg.b("/meta/setup_done") && cfg.key(cfg.engine(false)).is_empty());
        let (tx, events) = mpsc::channel();
        let (notify, paint) = (tx.clone(), ctx.clone());
        TrayIconEvent::set_event_handler(Some(move |e| {
            if matches!(
                e,
                TrayIconEvent::Click {
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let _ = notify.send(Action::Show);
                paint.send_viewport_cmd(ViewportCommand::Visible(true));
                paint.send_viewport_cmd(ViewportCommand::Focus);
                paint.request_repaint();
            }
        }));
        let tray = tray_icon::Icon::from_rgba(icon.rgba, icon.width, icon.height)
            .ok()
            .and_then(|icon| {
                TrayIconBuilder::new()
                    .with_icon(icon)
                    .with_tooltip("VRCLT")
                    .build()
                    .ok()
            });
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("async runtime");
        let overlay = subtitles::DesktopOverlay::load(&path);
        let mut app = Self {
            draft: cfg.clone(),
            cfg,
            path,
            page: Page::Live,
            lang,
            checks: Default::default(),
            osc_health: Default::default(),
            installer: None,
            running: false,
            pending_volume: None,
            rt,
            routes: [None, None],
            past_captions: Default::default(),
            conversation_cleared_at: None,
            echo: Arc::default(),
            events,
            tx,
            osc: None,
            chatbox: None,
            devices_rx: None,
            devices: Default::default(),
            devices_ready: false,
            load_error: !error.is_empty(),
            error,
            exiting: false,
            about: false,
            voice_help: false,
            voices: Default::default(),
            held: false,
            tray,
            overlay,
            vr: None,
            hotkeys: None,
        };
        app.discover(ctx);
        if !first && !app.load_error && Self::validate(&app.cfg, app.lang, app.cable()).is_ok() {
            app.start(ctx);
        } else if !app.load_error {
            app.start_vr(ctx);
        }
        app
    }
    fn discover(&mut self, ctx: &Context) {
        let (tx, rx) = mpsc::channel();
        self.devices_rx = Some(rx);
        self.checks.last_scan = Some(Instant::now());
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(audio::devices());
            ctx.request_repaint();
        });
    }
    fn cable(&self) -> Cable {
        if self.devices_ready {
            self.devices.cable()
        } else {
            Cable::Unknown
        }
    }
    fn choose_text_mode(&mut self, text: bool, ctx: &Context) {
        if !text && self.cable() != Cable::Ready {
            self.voice_help = true;
            if self.devices_rx.is_none() {
                self.discover(ctx);
            }
        } else {
            self.change_lane(0, "/outbound/text_only", text.into(), ctx);
        }
    }
    fn setup_page(&mut self, n: usize) {
        if self.page == Page::Live {
            self.flush_volume();
            self.draft = self.cfg.clone();
        }
        self.page = Page::Setup(n);
    }
    fn volume(&self) -> i32 {
        self.pending_volume
            .map_or_else(|| self.cfg.voice_volume(), |(v, _)| v)
    }
    fn preview_volume(&mut self, volume: i32) {
        let volume = volume.clamp(0, 200);
        self.pending_volume = Some((volume, Instant::now()));
        if let Some(route) = &self.routes[0] {
            route.lane.set_voice_volume(volume);
        }
    }
    fn flush_volume(&mut self) {
        let Some((volume, _)) = self.pending_volume.take() else {
            return;
        };
        let mut cfg = self.cfg.clone();
        cfg.data["outbound"]["tts_gain"] = (f64::from(volume) / 100.).into();
        if self.save(cfg) {
            self.draft.data["outbound"]["tts_gain"] = self.cfg.data["outbound"]["tts_gain"].clone();
            self.update_vr();
        } else if let Some(route) = &self.routes[0] {
            route.lane.set_voice_volume(self.cfg.voice_volume());
        }
    }
    fn change_monitor(&mut self, name: String) {
        if name == self.cfg.s("/outbound/monitor_device") {
            return;
        }
        let mut cfg = self.cfg.clone();
        cfg.data["outbound"]["monitor_device"] = name.clone().into();
        if self.save(cfg) {
            self.draft.data["outbound"]["monitor_device"] = name.into();
            if self.cable() == Cable::Ready
                && let Some(route) = &mut self.routes[0]
            {
                route.set_monitor(&self.cfg);
            }
            self.update_vr();
        }
    }
    fn save(&mut self, c: Config) -> bool {
        if self.load_error {
            return false;
        }
        match c.save(&self.path) {
            Ok(()) => {
                self.cfg = c;
                self.error.clear();
                true
            }
            Err(e) => {
                self.error = e.to_string();
                false
            }
        }
    }
    fn validate(c: &Config, l: Lang, cable: Cable) -> Result<(), (usize, String)> {
        if !c.b("/outbound/enabled")
            && !c.b("/inbound/enabled")
            && matches!(cable, Cable::Missing | Cable::Inactive)
        {
            return Err((
                0,
                l.t(
                    "통역 방향을 하나 이상 선택하세요.",
                    "Select at least one direction.",
                    "通訳の方向を選んでください。",
                    "请至少选择一个翻译方向。",
                )
                .into(),
            ));
        }
        if c.translation_active(true) && c.s("/inbound/process").is_empty() {
            return Err((
                0,
                l.t(
                    "자막을 받을 앱의 실행 파일 이름을 입력하세요.",
                    "Enter the app’s executable name.",
                    "対象アプリの実行ファイル名を入力してください。",
                    "请输入目标应用的可执行文件名。",
                )
                .into(),
            ));
        }
        for i in [false, true] {
            if c.translation_active(i) {
                let p = c.engine(i);
                let key = c.key(p);
                if key.is_empty() || key.contains("://") {
                    return Err((1, format!("{}  —  {}", engine_name(p), l.key_needed())));
                }
            }
        }
        Ok(())
    }
    fn save_session_settings(&mut self, cfg: Config) -> bool {
        let rebuild_vr = crate::vr::creation_settings_changed(&self.cfg, &cfg);
        if !self.save(cfg) {
            return false;
        }
        if rebuild_vr {
            self.vr.take();
        }
        true
    }
    fn retire_route(&mut self, i: usize) {
        if let Some(route) = self.routes[i].take() {
            let lane = route.lane.clone();
            drop(route);
            let mut transcript = lane.captions.lock();
            let past = &mut self.past_captions[i];
            past.extend(
                transcript
                    .history
                    .drain(..)
                    .map(|line| (line.started, line.caption)),
            );
            past.extend(transcript.pending.drain(..).map(|p| (p.started, p.caption)));
            past.drain(..past.len().saturating_sub(20));
        }
    }
    fn stop(&mut self) {
        for i in 0..2 {
            self.retire_route(i);
        }
        if let Some(t) = self.osc.take() {
            t.abort();
        }
        self.hotkeys.take();
        self.chatbox.take();
        self.held = false;
        self.osc_health = Arc::default();
        self.running = false;
        self.update_vr();
    }
    fn start(&mut self, ctx: &Context) {
        if let Err((n, e)) = Self::validate(&self.cfg, self.lang, self.cable()) {
            self.start_vr(ctx);
            self.setup_page(n);
            self.error = e;
            return;
        }
        self.stop();
        self.echo = Arc::default();
        self.running = true;
        for i in 0..2 {
            if i == 0 && !self.devices_ready && self.devices_rx.is_some() {
                continue;
            }
            self.route(i, ctx);
        }
        if self.cfg.b("/control/enabled") {
            self.osc = Some(self.rt.spawn(osc::listen(
                self.osc_health.clone(),
                self.cfg.clone(),
                self.tx.clone(),
                ctx.clone(),
            )));
        }
        self.hotkeys = crate::hotkeys::Hotkeys::start(&self.cfg, self.tx.clone(), ctx.clone());
        self.start_vr(ctx);
    }
    fn start_vr(&mut self, ctx: &Context) {
        if self.vr.is_some() {
            self.update_vr();
            return;
        }
        self.vr = crate::vr::Vr::start(
            self.cfg.clone(),
            self.routes
                .each_ref()
                .map(|r| r.as_ref().map(|r| r.lane.clone())),
            self.running,
            self.devices.outputs.clone(),
            self.cable(),
            self.tx.clone(),
            ctx.clone(),
        );
    }
    fn route(&mut self, i: usize, ctx: &Context) {
        self.flush_volume();
        let inbound = i == 1;
        self.retire_route(i);
        let enabled = self.cfg.b(if inbound {
            "/inbound/enabled"
        } else {
            "/outbound/enabled"
        });
        let mut captions = None;
        let cable = self.cable();
        if !inbound {
            if self.running
                && enabled
                && (self.cfg.b("/outbound/chatbox")
                    || (self.cfg.s("/app/mode") == "vrchat" && cable.text_only(&self.cfg)))
            {
                let mut chat_cfg = self.cfg.clone();
                chat_cfg.data["outbound"]["text_only"] = cable.text_only(&self.cfg).into();
                let chat = self
                    .chatbox
                    .get_or_insert_with(|| osc::Chatbox::start(self.rt.handle(), &chat_cfg));
                chat.sync(&chat_cfg, self.lang);
                captions = Some(chat.captions.clone());
            } else {
                self.chatbox.take();
            }
        }
        if self.running
            && let Some(route) =
                crate::runtime::AudioRoute::select(&self.cfg, inbound, self.held, &self.devices)
        {
            self.routes[i] = Some(Route::start(
                &self.rt,
                self.cfg.clone(),
                inbound,
                route,
                ctx.clone(),
                self.echo.clone(),
                captions,
            ));
        }
        self.update_vr();
    }
    fn update_vr(&self) {
        if let Some(vr) = &self.vr {
            vr.update(
                self.cfg.clone(),
                self.routes
                    .each_ref()
                    .map(|r| r.as_ref().map(|r| r.lane.clone())),
                self.running,
                self.devices.outputs.clone(),
                self.cable(),
            );
        }
    }
    fn change_lane(&mut self, i: usize, path: &str, value: serde_json::Value, ctx: &Context) {
        let mut cfg = self.cfg.clone();
        if let Some(v) = cfg.data.pointer_mut(path) {
            if *v == value {
                return;
            }
            *v = value;
        }
        if self.save(cfg) {
            if self.page != Page::Live {
                if let Some(draft) = self.draft.data.pointer_mut(path) {
                    *draft = self.cfg.data.pointer(path).unwrap().clone();
                }
            }
            self.route(i, ctx);
        }
    }
    fn toggle_filter_option(&mut self, inbound: bool, model: bool) {
        let path = if model {
            Config::nemotron_path(inbound)
        } else {
            Config::speech_filter_path(inbound)
        };
        let mut cfg = self.cfg.clone();
        let enabled = !cfg.b(path);
        *cfg.data.pointer_mut(path).unwrap() = enabled.into();
        if self.save(cfg) {
            if let Some(draft) = self.draft.data.pointer_mut(path) {
                *draft = enabled.into();
            }
            if let Some(route) = &self.routes[usize::from(inbound)] {
                let option = if model {
                    &route.lane.nemotron
                } else {
                    &route.lane.speech_filter
                };
                option.store(enabled, std::sync::atomic::Ordering::Relaxed);
            }
            self.update_vr();
        }
    }
    fn set_interface_language(&mut self, code: &str) {
        let mut cfg = self.cfg.clone();
        cfg.data["ui"]["lang"] = code.into();
        if self.save(cfg) {
            self.lang = Lang::new(code);
            self.draft.data["ui"]["lang"] = code.into();
            self.update_vr();
        }
    }
    fn finish_setup(&mut self, ctx: &Context) {
        if let Err((n, e)) = Self::validate(&self.draft, self.lang, self.cable()) {
            self.error = e;
            self.page = Page::Setup(n);
            return;
        }
        self.draft.data["meta"]["setup_done"] = true.into();
        if self.save_session_settings(self.draft.clone()) {
            self.page = Page::Live;
            self.start(ctx);
        }
    }
    fn title(&mut self, ctx: &Context) {
        TopBottomPanel::top("window-header")
            .exact_height(49.)
            .frame(
                Frame::new()
                    .fill(INK)
                    .inner_margin(Margin::symmetric(18, 0)),
            )
            .show(ctx, |ui| {
                let full = ui.max_rect();
                let drag = ui.interact(full, Id::new("move-window"), Sense::click_and_drag());
                if drag.double_clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(
                        !ctx.input(|i| i.viewport().maximized.unwrap_or(false)),
                    ));
                } else if drag.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                ui.horizontal_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(29., 29.), Sense::hover());
                    mark(ui.painter(), r.left_top(), 28.);
                    if ui
                        .add(Button::new(bold("VRCLT", 23.).color(TEXT)).frame(false))
                        .on_hover_text("VRCLT / Licenses")
                        .clicked()
                    {
                        self.about = true;
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(self.lang.t(
                            "통역 콘솔",
                            "Translation console",
                            "通訳コンソール",
                            "翻译控制台",
                        ))
                        .color(MUTE)
                        .size(15.),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        for (id, label) in [
                            (0, self.lang.t("닫기", "Close", "閉じる", "关闭")),
                            (
                                1,
                                self.lang.t(
                                    "최대화 / 복원",
                                    "Maximize / restore",
                                    "最大化 / 復元",
                                    "最大化 / 还原",
                                ),
                            ),
                            (2, self.lang.t("최소화", "Minimize", "最小化", "最小化")),
                        ] {
                            let (r, response) =
                                ui.allocate_exact_size(vec2(32., 29.), Sense::click());
                            let p = ui.painter();
                            if response.hovered() {
                                p.rect_filled(r, 4, NAV);
                            }
                            let c = r.center();
                            let stroke = Stroke::new(1.4_f32, TEXT);
                            match id {
                                0 => {
                                    p.line_segment([c - vec2(4., 4.), c + vec2(4., 4.)], stroke);
                                    p.line_segment([c + vec2(-4., 4.), c + vec2(4., -4.)], stroke);
                                }
                                1 => {
                                    p.rect_stroke(
                                        Rect::from_center_size(c, vec2(10., 9.)),
                                        0,
                                        stroke,
                                        StrokeKind::Inside,
                                    );
                                }
                                _ => {
                                    p.line_segment([c + vec2(-5., 2.), c + vec2(5., 2.)], stroke);
                                }
                            }
                            if response.on_hover_text(label).clicked() {
                                ctx.send_viewport_cmd(match id {
                                    0 => ViewportCommand::Close,
                                    1 => ViewportCommand::Maximized(!max),
                                    _ => ViewportCommand::Minimized(true),
                                });
                            }
                        }
                    });
                });
            });
    }
    fn footer(&mut self, ctx: &Context) {
        let l = self.lang;
        let height = if ctx.screen_rect().height() < 620. {
            52.
        } else {
            64.
        };
        TopBottomPanel::bottom("statusbar")
            .exact_height(height)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(CANVAS)
                    .inner_margin(Margin::symmetric(26, 0)),
            )
            .show(ctx, |ui| {
                let row = ui.max_rect();
                ui.painter()
                    .hline(row.x_range(), row.top(), Stroke::new(1_f32, LINE));
                ui.horizontal_centered(|ui| {
                    let color = if self.running { MINT } else { MUTE };
                    let (r, _) = ui.allocate_exact_size(vec2(9., 9.), Sense::hover());
                    ui.painter().circle_filled(r.center(), 3.5, color);
                    ui.label(
                        RichText::new(if self.running {
                            l.t("통역 실행 중", "Session running", "通訳中", "翻译中")
                        } else {
                            l.t("대기 중", "Standby", "待機中", "待机")
                        })
                        .color(color)
                        .size(13.),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().interact_size.y = 36.;
                        ui.spacing_mut().item_spacing.x = 14.;
                        ui.spacing_mut().button_padding = vec2(12., 6.);
                        let widgets = &mut ui.visuals_mut().widgets;
                        widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                        widgets.inactive.bg_stroke = Stroke::NONE;
                        widgets.hovered.weak_bg_fill = NAV;
                        widgets.open.weak_bg_fill = NAV;
                        if ui
                            .add(
                                Button::new(l.t("종료", "Quit", "終了", "退出"))
                                    .min_size(vec2(66., 36.))
                                    .corner_radius(6),
                            )
                            .on_hover_cursor(CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.exiting = true;
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        }
                        let (rule, _) = ui.allocate_exact_size(vec2(1., 16.), Sense::hover());
                        ui.painter().vline(
                            rule.center().x,
                            rule.y_range(),
                            Stroke::new(1_f32, LINE),
                        );
                        ComboBox::from_id_salt("ui-language")
                            .width(108.)
                            .icon(|ui, rect, visuals, is_open, _| {
                                let c = rect.center();
                                let dy = if is_open { -2. } else { 2. };
                                ui.painter().add(Shape::line(
                                    vec![c + vec2(-4., dy), c - vec2(0., dy), c + vec2(4., dy)],
                                    Stroke::new(1.4_f32, visuals.fg_stroke.color),
                                ));
                            })
                            .selected_text(lang_name(match l {
                                Lang::Ko => "ko",
                                Lang::Ja => "ja",
                                Lang::Zh => "zh",
                                Lang::En => "en",
                            }))
                            .show_ui(ui, |ui| {
                                ui.set_min_width(126.);
                                for code in ["ko", "en", "ja", "zh"] {
                                    if ui
                                        .selectable_label(Lang::new(code) == l, lang_name(code))
                                        .clicked()
                                    {
                                        self.set_interface_language(code);
                                    }
                                }
                            })
                            .response
                            .on_hover_cursor(CursorIcon::PointingHand)
                            .on_hover_text(l.t(
                                "화면 언어",
                                "Interface language",
                                "表示言語",
                                "界面语言",
                            ));
                    });
                });
            });
    }
    fn setup_view(&mut self, ui: &mut Ui, n: usize, ctx: &Context) {
        let l = self.lang;
        ui.horizontal(|ui| {
            if ui
                .button(l.t("← 콘솔", "← Console", "← コンソール", "← 控制台"))
                .clicked()
            {
                self.page = Page::Live;
                self.error.clear();
            }
            ui.separator();
            for i in 0..3 {
                if ui.selectable_label(i == n, l.step(i)).clicked() {
                    self.page = Page::Setup(i);
                }
            }
        });
        ui.add_space(12.);
        ui.label(bold(l.step(n), 25.));
        ui.add_space(4.);
        match n {
            0 => self.profile_form(ui),
            1 => self.engine_form(ui),
            _ => self.audio_form(ui, ctx),
        }
        ui.add_space(10.);
        ui.horizontal(|ui| {
            if primary(
                ui,
                if self.running {
                    l.t("변경 적용", "Apply changes", "変更を適用", "应用更改")
                } else {
                    l.t(
                        "저장하고 통역 시작",
                        "Save and start",
                        "保存して通訳開始",
                        "保存并开始翻译",
                    )
                },
            )
            .clicked()
            {
                self.finish_setup(ctx);
            }
            if !self.running
                && ui
                    .button(l.t("설정만 저장", "Save settings", "設定のみ保存", "仅保存设置"))
                    .clicked()
                && self.save(self.draft.clone())
            {
                self.page = Page::Live;
            }
        });
    }
    fn profile_form(&mut self, ui: &mut Ui) {
        let l = self.lang;
        hint(
            ui,
            l.t(
                "선택한 앱의 음성으로 상대방 자막을 만듭니다.",
                "Captions use audio from the app you select.",
                "選んだアプリの音声から相手の字幕を作成します。",
                "使用所选应用的音频生成对方字幕。",
            ),
        );
        for (key, name, desc) in [
            (
                "vrchat",
                "VRChat",
                l.t(
                    "번역 음성, 상대방 자막, OSC 챗박스",
                    "Voice translation, captions and OSC chatbox",
                    "翻訳音声、字幕、OSC チャットボックス",
                    "语音翻译、字幕和 OSC 聊天框",
                ),
            ),
            (
                "discord",
                "Discord",
                l.t(
                    "음성 채널에서 번역 음성과 자막 사용",
                    "Translate voice and read captions in calls",
                    "通話の音声を翻訳して字幕を表示",
                    "在语音通话中使用语音翻译和字幕",
                ),
            ),
            (
                "custom",
                l.t("다른 앱", "Another app", "その他のアプリ", "其他应用"),
                l.t(
                    "실행 파일 이름으로 캡처할 앱 지정",
                    "Choose a capture target by executable name",
                    "実行ファイル名で対象を指定",
                    "通过可执行文件名指定应用",
                ),
            ),
        ] {
            if choice(ui, key, name, desc, "", self.draft.s("/app/mode") == key) {
                self.draft.profile(key);
            }
        }
        if self.draft.s("/app/mode") == "custom" {
            edit(
                ui,
                &mut self.draft,
                "/inbound/process",
                l.t(
                    "실행 파일 이름",
                    "Executable name",
                    "実行ファイル名",
                    "可执行文件名",
                ),
                false,
            );
            self.draft.data["app"]["profiles"]["custom"]["process"] =
                self.draft.data["inbound"]["process"].clone();
        }
        ui.add_space(5.);
        ui.horizontal(|ui| {
            check(ui, &mut self.draft, "/outbound/enabled", l.out());
            check(ui, &mut self.draft, "/inbound/enabled", l.incoming());
        });
    }
    fn engine_form(&mut self, ui: &mut Ui) {
        let l = self.lang;
        if let Some(p) = engines(ui, self.draft.engine(false), l) {
            self.draft.data["provider"] = p.into();
        }
        let message = match self.draft.engine(false) {
            "soniox" => l.t(
                "화자별 자막과 번역 음성을 지원합니다. 사용량에 따라 과금됩니다.",
                "Speaker labels and translated speech. Usage-based billing.",
                "話者別字幕と翻訳音声に対応。従量課金です。",
                "提供说话人字幕和翻译语音，按用量计费。",
            ),
            "gemini" => l.t(
                "Google AI Studio의 무료 등급 한도 안에서 사용할 수 있습니다.",
                "Available within Google AI Studio’s free-tier limits.",
                "Google AI Studio の無料枠で利用できます。",
                "可在 Google AI Studio 免费层限额内使用。",
            ),
            "qwen" => l.t(
                "Alibaba Cloud Model Studio에서 발급받은 키를 사용합니다.",
                "Use an Alibaba Cloud Model Studio API key.",
                "Alibaba Cloud Model Studio のキーを使用します。",
                "使用阿里云百炼的 API 密钥。",
            ),
            _ => l.t(
                "OpenAI API 키로 GPT 실시간 음성 번역을 연결합니다.",
                "Connect GPT realtime translation with an OpenAI API key.",
                "OpenAI のキーで GPT リアルタイム翻訳に接続します。",
                "使用 OpenAI API 密钥连接 GPT 实时翻译。",
            ),
        };
        hint(ui, message);
        let p = self.draft.engine(false).to_owned();
        self.key_editor(ui, &p);
        if p == "soniox" || self.draft.engine(true) == "soniox" {
            self.voices.panel(ui, &mut self.draft, l, &self.rt);
        }
        ui.collapsing(
            l.t(
                "자막 엔진 및 상세 옵션",
                "Caption engine and advanced options",
                "字幕エンジンと詳細設定",
                "字幕引擎和高级选项",
            ),
            |ui| {
                let path = "/inbound/provider";
                let mut selected = self.draft.s(path).to_owned();
                let same = l.t(
                    "내 말 번역과 동일",
                    "Same as outgoing",
                    "送信側と同じ",
                    "与语音翻译相同",
                );
                ComboBox::from_id_salt(path)
                    .selected_text(if selected.is_empty() {
                        same
                    } else {
                        engine_name(&selected)
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut selected, String::new(), same);
                        for p in ENGINES {
                            ui.selectable_value(&mut selected, p.into(), engine_name(p));
                        }
                    });
                self.draft.data["inbound"]["provider"] = selected.clone().into();
                if !selected.is_empty() && selected != p {
                    self.key_editor(ui, &selected);
                }
                let inbound = self.draft.engine(true).to_owned();
                for (direction, engine, label) in [
                    (
                        "outbound",
                        p.as_str(),
                        l.t(
                            "내가 말하는 언어",
                            "My spoken language",
                            "自分が話す言語",
                            "我说的语言",
                        ),
                    ),
                    (
                        "inbound",
                        inbound.as_str(),
                        l.t(
                            "상대가 말하는 언어",
                            "Other speakers’ language",
                            "相手が話す言語",
                            "对方说的语言",
                        ),
                    ),
                ] {
                    if engine == "qwen" && self.draft.s("/qwen/model").contains("3.5") {
                        let mut current = self
                            .draft
                            .s(&format!("/{direction}/source_language"))
                            .to_owned();
                        let default = l.t(
                            "기본값 (영어)",
                            "Default (English)",
                            "既定 (英語)",
                            "默认 (英语)",
                        );
                        ui.horizontal(|ui| {
                            ui.label(label);
                            ComboBox::from_id_salt(("source-language", direction))
                                .selected_text(if current.is_empty() {
                                    default
                                } else {
                                    lang_name(&current)
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut current, String::new(), default);
                                    for &(code, _) in LANGUAGES {
                                        ui.selectable_value(
                                            &mut current,
                                            code.into(),
                                            lang_name(code),
                                        );
                                    }
                                });
                        });
                        self.draft.data[direction]["source_language"] = current.into();
                    }
                }
                if p == "qwen" || inbound == "qwen" {
                    let model = self.draft.s("/qwen/model").to_owned();
                    ComboBox::from_id_salt("qwen-model")
                        .selected_text(&model)
                        .show_ui(ui, |ui| {
                            for option in [
                                "qwen3.8-livetranslate-flash-realtime",
                                "qwen3.5-livetranslate-flash-realtime",
                            ] {
                                if ui.selectable_label(model == option, option).clicked() {
                                    self.draft.data["qwen"]["model"] = option.into();
                                }
                            }
                        });
                    edit(
                        ui,
                        &mut self.draft,
                        "/qwen/workspace_id",
                        "Qwen workspace ID",
                        false,
                    );
                    edit(
                        ui,
                        &mut self.draft,
                        "/qwen/endpoint",
                        "Qwen region (intl / beijing)",
                        false,
                    );
                }
                if p == "soniox" || inbound == "soniox" {
                    check(
                        ui,
                        &mut self.draft,
                        "/soniox/keep_speaker_context",
                        l.t(
                            "잠깐 멈춰도 화자 번호 유지",
                            "Keep speaker IDs during pauses",
                            "短い無音中は話者番号を保持",
                            "短暂停顿时保留说话人编号",
                        ),
                    );
                    ui.collapsing(
                        l.t(
                            "목소리 이름이나 ID 직접 지정",
                            "Enter voice name or ID",
                            "音声名や ID を指定",
                            "手动指定声音名称或 ID",
                        ),
                        |ui| {
                            edit(ui, &mut self.draft, "/soniox/voice", "", false);
                        },
                    );
                }
                edit(
                    ui,
                    &mut self.draft,
                    "/outbound/glossary",
                    l.t(
                        "번역 용어 (원문=번역)",
                        "Glossary (source=translation)",
                        "用語集 (原文=訳文)",
                        "术语 (原文=译文)",
                    ),
                    false,
                );
            },
        );
    }
    fn key_editor(&mut self, ui: &mut Ui, p: &str) {
        let l = self.lang;
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(bold(&format!("{} API Key", engine_name(p)), 16.));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.hyperlink_to(
                        l.t(
                            "키 발급 페이지 ↗",
                            "Get API key ↗",
                            "キーを取得 ↗",
                            "获取密钥 ↗",
                        ),
                        match p {
                            "gemini" => "https://aistudio.google.com/apikey",
                            "soniox" => "https://console.soniox.com/",
                            "openai" => "https://platform.openai.com/api-keys",
                            _ if self.draft.s("/qwen/endpoint") == "beijing" => {
                                "https://bailian.console.aliyun.com/?tab=model#/api-key"
                            }
                            _ => "https://modelstudio.console.alibabacloud.com/?tab=model#/api-key",
                        },
                    );
                });
            });
            edit(ui, &mut self.draft, &Config::key_path(p), "", true);
            if self.draft.s(&Config::key_path(p)).is_empty() && !self.draft.key(p).is_empty() {
                hint(
                    ui,
                    l.t(
                        "환경 변수의 키를 사용합니다.",
                        "Using key from environment.",
                        "環境変数のキーを使用します。",
                        "使用环境变量中的密钥。",
                    ),
                );
            }
        });
    }
    fn audio_form(&mut self, ui: &mut Ui, ctx: &Context) {
        let l = self.lang;
        hint(
            ui,
            l.t(
                "내 목소리를 받을 마이크와 번역 음성을 보낼 장치를 고르세요.",
                "Choose the microphone and where the translated voice should play.",
                "マイクと翻訳音声の出力先を選んでください。",
                "请选择麦克风和翻译语音的输出设备。",
            ),
        );
        card(ui, |ui| {
            if self.draft.b("/outbound/enabled") {
                let mut mic = self.draft.s("/outbound/mic_device").to_owned();
                device(
                    ui,
                    "mic",
                    l.input(),
                    &mut mic,
                    &self.devices.inputs,
                    l.auto_mic(),
                );
                self.draft.data["outbound"]["mic_device"] = mic.into();
                check(
                    ui,
                    &mut self.draft,
                    Config::speech_filter_path(false),
                    l.speech_filter(),
                );
                let path = Config::nemotron_path(false);
                let mut gpu = self.draft.b(path);
                if filter_model_choice(
                    ui,
                    ("false", "setup"),
                    &mut gpu,
                    self.draft.b(Config::speech_filter_path(false)),
                    l,
                ) {
                    *self.draft.data.pointer_mut(path).unwrap() = gpu.into();
                }
                let mut out = self.draft.s("/outbound/tts_device").to_owned();
                device(
                    ui,
                    "out",
                    l.output(),
                    &mut out,
                    &self.devices.outputs,
                    l.default_device(),
                );
                self.draft.data["outbound"]["tts_device"] = out.into();
                hint(ui,l.t("VB-Cable을 쓰면 여기는 CABLE Input, 대화 앱의 마이크는 CABLE Output으로 선택하세요.","For VB-Cable, select CABLE Input here and CABLE Output as the microphone in your chat app.","VB-Cable はここで CABLE Input、通話アプリのマイクで CABLE Output を選択します。","使用 VB-Cable 时，此处选择 CABLE Input，通话应用的麦克风选择 CABLE Output。"));
                let mut volume = self.draft.voice_volume();
                let mut monitor = self.draft.s("/outbound/monitor_device").to_owned();
                ui.add_enabled_ui(self.cable() == Cable::Ready, |ui| {
                    voice_controls(ui, l, &mut volume, &mut monitor, &self.devices.outputs)
                });
                if volume != self.draft.voice_volume() {
                    self.draft.data["outbound"]["tts_gain"] = (f64::from(volume) / 100.).into();
                }
                self.draft.data["outbound"]["monitor_device"] = monitor.into();
                if self.cable() == Cable::Missing {
                    if ui
                        .add_enabled(
                            self.installer.is_none(),
                            Button::new(l.t(
                                "VB-Audio 공식 설치 프로그램 실행",
                                "Run VB-Audio installer",
                                "VB-Audio 公式セットアップを起動",
                                "运行 VB-Audio 官方安装程序",
                            )),
                        )
                        .clicked()
                    {
                        self.install_driver(ctx);
                    }
                    ui.hyperlink_to(
                        l.t(
                            "VB-Cable 설치 페이지 ↗",
                            "Get VB-Cable ↗",
                            "VB-Cable を入手 ↗",
                            "获取 VB-Cable ↗",
                        ),
                        "https://vb-audio.com/Cable/",
                    );
                }
            }
            if self.draft.b("/inbound/enabled") {
                let p = self.draft.s("/inbound/process");
                ui.label(format!("{}   →   {}", p, l.incoming()));
            }
            if ui
                .button(l.t(
                    "장치 다시 찾기",
                    "Refresh devices",
                    "デバイスを再検出",
                    "刷新设备",
                ))
                .clicked()
            {
                self.discover(ctx);
            }
        });
        ui.collapsing(
            l.t(
                "출력 및 자막 옵션",
                "Output and captions",
                "出力と字幕",
                "输出与字幕",
            ),
            |ui| {
                if self.draft.s("/app/mode") == "vrchat" {
                    let cable = self.cable();
                    if let Some(text) = send_mode_control(ui, l, cable, &self.draft) {
                        if !text && cable != Cable::Ready {
                            self.voice_help = true;
                        } else {
                            self.draft.data["outbound"]["text_only"] = text.into();
                        }
                    }
                    check(
                        ui,
                        &mut self.draft,
                        "/outbound/chatbox",
                        l.t(
                            "OSC 챗박스",
                            "OSC chatbox",
                            "OSC チャットボックス",
                            "OSC 聊天框",
                        ),
                    );
                    check(
                        ui,
                        &mut self.draft,
                        "/control/feedback_chatbox",
                        l.t(
                            "번역 ON/OFF를 VRChat 챗박스에 표시",
                            "Show translation ON/OFF in the VRChat chatbox",
                            "翻訳 ON/OFF を VRChat チャットボックスに表示",
                            "在 VRChat 聊天框显示翻译开关状态",
                        ),
                    );
                }
                check(
                    ui,
                    &mut self.draft,
                    "/inbound/play_audio",
                    l.t(
                        "상대방 번역 음성 듣기",
                        "Play translated incoming voice",
                        "相手の翻訳音声を再生",
                        "播放对方的翻译语音",
                    ),
                );
                if self.draft.b("/inbound/play_audio") {
                    let mut value = self.draft.s("/inbound/audio_device").into();
                    device(
                        ui,
                        "inplay",
                        l.output(),
                        &mut value,
                        &self.devices.outputs,
                        l.default_device(),
                    );
                    self.draft.data["inbound"]["audio_device"] = value.into();
                }
                check(
                    ui,
                    &mut self.draft,
                    "/overlay/show_source",
                    l.t(
                        "자막에 원문 표시",
                        "Show original text in captions",
                        "字幕に原文を表示",
                        "字幕显示原文",
                    ),
                );
                check(
                    ui,
                    &mut self.draft,
                    Config::speech_filter_path(true),
                    l.speech_filter(),
                );
                let path = Config::nemotron_path(true);
                let mut gpu = self.draft.b(path);
                if filter_model_choice(
                    ui,
                    ("true", "setup"),
                    &mut gpu,
                    self.draft.b(Config::speech_filter_path(true)),
                    l,
                ) {
                    *self.draft.data.pointer_mut(path).unwrap() = gpu.into();
                }
                check(
                    ui,
                    &mut self.draft,
                    "/inbound/vad_enabled",
                    l.t(
                        "상대방 음성만 감지",
                        "Detect incoming speech",
                        "相手の音声を検出",
                        "检测对方语音",
                    ),
                );
            },
        );
    }
    fn dashboard(&mut self, ui: &mut Ui, ctx: &Context) {
        let l = self.lang;
        ui.horizontal(|ui| {
            ui.label(bold(
                l.t("대화 앱", "Application", "通話アプリ", "通话应用"),
                16.,
            ));
            let mode = self.cfg.s("/app/mode").to_owned();
            for (key, name) in [
                ("vrchat", "VRChat"),
                ("discord", "Discord"),
                ("custom", l.t("다른 앱", "Custom", "その他", "其他")),
            ] {
                if ui.selectable_label(mode == key, name).clicked() && mode != key {
                    if key == "custom" {
                        self.setup_page(0);
                        self.draft.profile(key);
                    } else {
                        let mut c = self.cfg.clone();
                        c.profile(key);
                        if self.save_session_settings(c) {
                            if self.running {
                                self.start(ctx);
                            } else {
                                self.start_vr(ctx);
                            }
                        }
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.running {
                    if ui
                        .button(l.t("통역 중지", "Stop session", "通訳を停止", "停止翻译"))
                        .clicked()
                    {
                        self.stop();
                        self.start_vr(ctx);
                    }
                } else if primary(ui, l.start()).clicked() {
                    self.start(ctx);
                }
                if ui
                    .button(l.t("연결 설정", "Connections", "接続設定", "连接设置"))
                    .clicked()
                {
                    self.setup_page(2);
                }
            });
        });
        self.checklist(ui, ctx);
        ui.label(bold(l.step(1), 16.));
        if let Some(p) = engines(ui, self.cfg.engine(false), l) {
            self.setup_page(1);
            self.draft.data["provider"] = p.into();
            self.error.clear();
        }
        ui.add_space(5.);
        ui.columns(2, |cols| {
            let mut panels: [_; 2] = std::array::from_fn(|i| card_frame().begin(&mut cols[i]));
            for (i, panel) in panels.iter_mut().enumerate() {
                let ui = &mut panel.content_ui;
                ui.set_min_width(ui.available_width());
                self.lane_controls(ui, i, ctx);
            }
            let meter_top = panels
                .iter()
                .map(|p| p.content_ui.next_widget_position().y)
                .fold(0_f32, f32::max);
            for (i, panel) in panels.iter_mut().enumerate() {
                let ui = &mut panel.content_ui;
                ui.add_space((meter_top - ui.next_widget_position().y).max(0.));
                self.lane_status(ui, i);
            }
            let bottom = panels
                .iter()
                .map(|p| p.content_ui.min_rect().bottom())
                .fold(0_f32, f32::max);
            for (mut panel, col) in panels.into_iter().zip(cols) {
                let mut bounds = panel.content_ui.min_rect();
                bounds.set_bottom(bottom);
                panel.content_ui.expand_to_include_rect(bounds);
                panel.end(col);
            }
        });
        if self.overlay.editing {
            self.overlay.geometry_controls(ui, l);
        }
        ui.add_space(9.);
        let mut console_font = self.cfg.n("/ui/caption_font_size", 17.).clamp(12., 48.) as u32;
        let mut overlay_font = self.cfg.n("/overlay/font_size", 27.).clamp(18., 72.) as u32;
        let mut show_source = self.cfg.b("/overlay/show_source");
        ui.horizontal_wrapped(|ui| {
            ui.label(bold(
                l.t("대화 자막", "Conversation", "会話字幕", "对话字幕"),
                22.,
            ));
            if self.cfg.speaker_ids(self.cfg.engine(true)) {
                ui.label(
                    RichText::new(l.t(
                        "화자 분리 켜짐",
                        "Speaker separation on",
                        "話者分離オン",
                        "说话人分离已开启",
                    ))
                    .size(13.)
                    .color(MINT),
                )
                .on_hover_text(l.t(
                    "화자 번호는 현재 연결에서 구분한 목소리입니다.",
                    "Speaker numbers identify voices in this connection.",
                    "話者番号は現在の接続内の声を区別します。",
                    "说话人编号只在当前连接中区分声音。",
                ));
            }
            ui.checkbox(
                &mut show_source,
                l.t("원문 표시", "Show original", "原文を表示", "显示原文"),
            )
            .on_hover_text(l.t(
                "원문을 번역문 위에 표시합니다.",
                "Show the original above the translation.",
                "訳文の上に原文を表示します。",
                "在译文上方显示原文。",
            ));
            subtitles::font_picker(
                ui,
                "console-font",
                l.t("콘솔 글자", "Console font", "コンソール文字", "控制台字号"),
                &mut console_font,
                12..=48,
            );
            subtitles::font_picker(
                ui,
                "overlay-font",
                l.t(
                    "오버레이 글자",
                    "Overlay font",
                    "オーバーレイ文字",
                    "浮窗字号",
                ),
                &mut overlay_font,
                18..=72,
            );
            if ui
                .button(l.t(
                    "대화 지우기",
                    "Clear conversation",
                    "会話を消去",
                    "清空对话",
                ))
                .clicked()
            {
                self.conversation_cleared_at = Some(Instant::now());
                for past in &mut self.past_captions {
                    past.clear();
                }
            }
        });
        let display_changed = show_source != self.cfg.b("/overlay/show_source")
            || overlay_font != self.cfg.n("/overlay/font_size", 27.).clamp(18., 72.) as u32;
        if display_changed
            || console_font != self.cfg.n("/ui/caption_font_size", 17.).clamp(12., 48.) as u32
        {
            let mut next = self.cfg.clone();
            next.data["ui"]["caption_font_size"] = console_font.into();
            next.data["overlay"]["font_size"] = overlay_font.into();
            next.data["overlay"]["show_source"] = show_source.into();
            if self.save(next) && display_changed {
                self.update_vr();
            }
        }
        let mut lines = Vec::new();
        for (i, route) in self.routes.iter().enumerate() {
            let mut remaining = 20;
            let mut append = |at, cap: &Caption, partial| {
                if self
                    .conversation_cleared_at
                    .is_some_and(|cutoff| at <= cutoff)
                {
                    return;
                }
                lines.push((
                    at,
                    crate::captions::Row::new(cap, l, show_source, partial, i == 0),
                ))
            };
            if let Some(route) = route {
                let t = route.lane.captions.lock();
                for line in t.history.iter().rev().take(20) {
                    append(line.started, &line.caption, false);
                    remaining -= 1;
                }
                for p in &t.pending {
                    if !p.displayed.text.is_empty() || !p.displayed.source.is_empty() {
                        append(p.started, &p.displayed, true);
                    }
                }
            }
            for (at, cap) in self.past_captions[i].iter().rev().take(remaining) {
                append(*at, cap, false);
            }
        }
        lines.sort_by_key(|line| line.0);
        subtitles::history(ui, &lines, l, self.running, console_font as f32);
        if self.running {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }
    fn lane_controls(&mut self, ui: &mut Ui, i: usize, ctx: &Context) {
        let l = self.lang;
        let inbound = i == 1;
        let name = if inbound { l.incoming() } else { l.out() };
        let path = if inbound {
            "/dashboard/subtitles_on"
        } else {
            "/dashboard/translation_on"
        };
        let direction = if inbound { "inbound" } else { "outbound" };
        let on = self.cfg.data[direction]["enabled"] == true && self.cfg.b(path);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(3., 21.), Sense::hover());
            ui.painter()
                .rect_filled(r, 2, if inbound { MINT } else { SKY });
            ui.label(bold(name, 22.));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (r, response) = ui.allocate_exact_size(vec2(44., 25.), Sense::click());
                let tint = if inbound { MINT } else { SKY };
                ui.painter()
                    .rect_filled(r, 13, if on { tint } else { LINE });
                ui.painter().circle_filled(
                    pos2(
                        if on {
                            r.right() - 12.5
                        } else {
                            r.left() + 12.5
                        },
                        r.center().y,
                    ),
                    8.5,
                    if on { INK } else { MUTE },
                );
                response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, on, name));
                if response.has_focus() {
                    ui.painter().rect_stroke(
                        r.expand(2.),
                        14,
                        Stroke::new(1_f32, SKY),
                        StrokeKind::Outside,
                    );
                }
                if response.on_hover_text(name).clicked() {
                    let mut c = self.cfg.clone();
                    c.data[direction]["enabled"] = true.into();
                    *c.data.pointer_mut(path).unwrap() = (!on).into();
                    if self.save(c) {
                        if self.running && !on && self.cfg.key(self.cfg.engine(inbound)).is_empty()
                        {
                            self.setup_page(1);
                            self.error = l.key_needed().into();
                        } else {
                            self.route(i, ctx);
                        }
                    }
                }
            });
        });
        if inbound {
            ui.horizontal(|ui| {
                ui.set_min_height(32.);
                hint(ui, l.t("입력 앱", "Audio source", "入力アプリ", "音频来源"));
                if ui.link(self.cfg.s("/inbound/process")).clicked() {
                    self.setup_page(0);
                }
            });
        } else {
            let mut mic = self.cfg.s("/outbound/mic_device").to_owned();
            ui.horizontal(|ui| {
                hint(ui, l.input());
                ComboBox::from_id_salt("console-mic")
                    .width((ui.available_width() - 12.).max(110.))
                    .truncate()
                    .selected_text(if mic.is_empty() { l.auto_mic() } else { &mic })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut mic, String::new(), l.auto_mic());
                        for n in &self.devices.inputs {
                            ui.selectable_value(&mut mic, n.clone(), n);
                        }
                    });
            });
            if mic != self.cfg.s("/outbound/mic_device") {
                self.change_lane(0, "/outbound/mic_device", mic.into(), ctx);
            }
        }
        let p = if inbound {
            "/inbound/target_language"
        } else {
            "/outbound/target_language"
        };
        let mut current = self.cfg.s(p).to_owned();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(l.t("번역할 언어", "Translate into", "翻訳先", "目标语言"))
                    .size(13.)
                    .color(MUTE),
            );
            ComboBox::from_id_salt(p)
                .width(ui.available_width().min(240.))
                .selected_text(lang_name(&current))
                .show_ui(ui, |ui| {
                    for code in ["ko", "en", "ja", "zh-Hans", "zh-Hant"] {
                        ui.selectable_value(&mut current, code.into(), lang_name(code));
                    }
                    ui.separator();
                    for &(code, _) in LANGUAGES {
                        if crate::provider::supports_target(self.cfg.engine(inbound), code)
                            && !["ko", "en", "ja", "zh-Hans", "zh-Hant"].contains(&code)
                        {
                            ui.selectable_value(&mut current, code.into(), lang_name(code));
                        }
                    }
                });
        });
        if current != self.cfg.s(p) {
            self.change_lane(i, p, current.into(), ctx);
        }
        let mut filter_on = self.cfg.b(Config::speech_filter_path(inbound));
        ui.horizontal_wrapped(|ui| {
            if ui.checkbox(&mut filter_on, l.speech_filter()).changed() {
                self.toggle_filter_option(inbound, false);
            }
            let mut gpu = self.cfg.b(Config::nemotron_path(inbound));
            if filter_model_choice(ui, (inbound, "live"), &mut gpu, filter_on, l) {
                self.toggle_filter_option(inbound, true);
            }
        });
        if !inbound && self.cfg.s("/app/mode") == "vrchat" {
            if let Some(text) = send_mode_control(ui, l, self.cable(), &self.cfg) {
                self.choose_text_mode(text, ctx);
            }
        } else if !inbound && self.cfg.b("/outbound/voice_output") && self.cable() != Cable::Ready {
            if ui
                .link(l.t(
                    "번역 음성 사용 안내",
                    "Voice mode setup",
                    "音声モードの案内",
                    "语音模式设置",
                ))
                .clicked()
            {
                self.voice_help = true;
            }
        }

        if inbound {
            ui.horizontal_wrapped(|ui| {
                let mut enabled = self.cfg.b("/ui/desktop_subtitles");
                if ui
                    .checkbox(
                        &mut enabled,
                        l.t(
                            "클릭 통과 오버레이",
                            "Click-through overlay",
                            "クリック透過字幕",
                            "点击穿透字幕",
                        ),
                    )
                    .changed()
                {
                    let mut next = self.cfg.clone();
                    next.data["ui"]["desktop_subtitles"] = enabled.into();
                    if self.save(next) && !enabled {
                        self.overlay.finish_edit();
                    }
                }
                if self.cfg.b("/ui/desktop_subtitles") {
                    let label = if self.overlay.editing {
                        l.t("조정 완료", "Done", "調整完了", "完成调整")
                    } else {
                        l.t(
                            "창 크기와 위치",
                            "Size and position",
                            "サイズと位置",
                            "大小和位置",
                        )
                    };
                    if ui.selectable_label(self.overlay.editing, label).clicked() {
                        if self.overlay.editing {
                            self.overlay.finish_edit();
                        } else {
                            self.overlay.editing = true;
                        }
                    }
                }
            });
        } else {
            let mut volume = self.volume();
            let mut monitor = self.cfg.s("/outbound/monitor_device").to_owned();
            let was = volume;
            ui.add_enabled_ui(self.cable() == Cable::Ready, |ui| {
                voice_controls(ui, l, &mut volume, &mut monitor, &self.devices.outputs)
            });
            if was != volume {
                self.preview_volume(volume);
                ctx.request_repaint_after(Duration::from_millis(220));
            }
            self.change_monitor(monitor);
            if self.cable() == Cable::Ready && self.cfg.b("/outbound/text_only") {
                hint(ui, l.text_only_audio_hint());
            }
        }
    }
    fn lane_status(&mut self, ui: &mut Ui, i: usize) {
        let l = self.lang;
        let inbound = i == 1;
        let lane = self.routes[i].as_ref().map(|r| r.lane.clone());
        let level = lane
            .as_ref()
            .map_or(0, |l| l.level.load(std::sync::atomic::Ordering::Relaxed));
        meter(ui, level, inbound);
        if let Some(lane) = &lane {
            let msg = lane.gpu_filter.lock().description(l);
            if !msg.is_empty() {
                ui.small(msg);
            }
        }
        let phase = lane.as_ref().map_or(Phase::Stopped, |lane| {
            let cap = *lane.capture.lock();
            if cap == Phase::Listening {
                *lane.phase.lock()
            } else {
                cap
            }
        });
        ui.label(
            RichText::new(match phase {
                Phase::Stopped => l.t("중지됨", "Stopped", "停止中", "已停止"),
                Phase::WaitingApp => l.t(
                    "대상 앱 실행 대기",
                    "Waiting for app",
                    "アプリの起動待ち",
                    "等待应用启动",
                ),
                Phase::Opening => l.t(
                    "장치 연결 중",
                    "Opening device",
                    "デバイス接続中",
                    "正在连接设备",
                ),
                Phase::Connecting => l.t("엔진 연결 중", "Connecting", "接続中", "正在连接"),
                Phase::Live => l.t("엔진 연결됨", "Connected", "接続済み", "已连接"),
                Phase::Retry => l.t("재연결 중", "Reconnecting", "再接続中", "重新连接中"),
                _ if lane.as_ref().is_some_and(|l| !l.translating) => l.t(
                    "VB-CABLE로 원래 목소리 전달",
                    "Original microphone to VB-CABLE",
                    "元の音声を VB-CABLE へ送信",
                    "原声传至 VB-CABLE",
                ),
                _ => l.t("음성 대기", "Listening", "音声待機中", "等待语音"),
            })
            .size(14.)
            .color(MUTE),
        );
        if let Some(lane) = lane {
            let audio = [
                lane.audio_issue.lock().clone(),
                lane.output_issue.lock().clone(),
                lane.monitor_issue.lock().clone(),
            ]
            .join(" ")
            .trim()
            .to_string();
            let network = lane.issue.lock().clone();
            if !audio.is_empty() || !network.is_empty() {
                let (s, n) = if !audio.is_empty() {
                    (audio, 2)
                } else {
                    (network, 1)
                };
                ui.colored_label(ALERT, s);
                if ui
                    .link(l.t(
                        "연결 설정 열기",
                        "Open connection settings",
                        "接続設定を開く",
                        "打开连接设置",
                    ))
                    .clicked()
                {
                    self.setup_page(n);
                }
            }
        }
    }
}
fn edit(ui: &mut Ui, c: &mut Config, path: &str, label: &str, password: bool) {
    if !label.is_empty() {
        ui.label(label);
    }
    let mut s = c
        .data
        .pointer(path)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let widget = if path.ends_with("glossary") {
        TextEdit::multiline(&mut s).desired_rows(2)
    } else {
        TextEdit::singleline(&mut s)
    };
    if ui
        .add(
            widget
                .hint_text(if password { "API key" } else { "" })
                .margin(Margin::symmetric(10, 8))
                .password(password)
                .id_salt(path)
                .desired_width(f32::INFINITY),
        )
        .changed()
    {
        if let Some(v) = c.data.pointer_mut(path) {
            *v = s.into();
        }
    }
}
fn send_mode_control(ui: &mut Ui, l: Lang, cable: Cable, cfg: &Config) -> Option<bool> {
    let text = cable.text_only(cfg);
    let mut choice = None;
    ui.horizontal_wrapped(|ui| {
        hint(ui, l.t("보내는 방식", "Send as", "送信方法", "发送方式"));
        let label = RichText::new(cable.voice_label(l));
        let mut voice = ui.selectable_label(
            !text,
            if cable == Cable::Ready {
                label
            } else {
                label.weak()
            },
        );
        if cable != Cable::Ready {
            voice = voice.on_hover_text(cable.guidance(l));
        }
        if voice.clicked() {
            choice = Some(false);
        }
        if ui
            .selectable_label(
                text,
                l.t("텍스트 모드", "Text only", "テキストモード", "文本模式"),
            )
            .clicked()
        {
            choice = Some(true);
        }
    });
    choice
}
fn filter_model_choice(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    gpu: &mut bool,
    enabled: bool,
    l: Lang,
) -> bool {
    let previous = *gpu;
    ui.add_enabled_ui(enabled, |ui| {
        ComboBox::from_id_salt(id)
            .selected_text(if !enabled {
                l.original_audio()
            } else if *gpu {
                "Nemotron RE-USE (GPU)"
            } else {
                "DeepFilterNet3 (CPU)"
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(gpu, false, "DeepFilterNet3 (CPU)");
                ui.selectable_value(gpu, true, "NVIDIA Nemotron RE-USE (GPU)");
                ui.small(l.t(
                    "처음에 약 68 MiB 다운로드. 약 1초 뒤 음성이 전달됩니다.",
                    "First use downloads 68 MiB. Audio is delayed by about one second.",
                    "初回に約68 MiBを取得。音声は約1秒遅れます。",
                    "首次下载约 68 MiB，音频约延迟一秒。",
                ));
            });
    });
    previous != *gpu
}
fn check(ui: &mut Ui, c: &mut Config, path: &str, label: &str) {
    let mut b = c.b(path);
    if ui.checkbox(&mut b, label).changed() {
        if let Some(v) = c.data.pointer_mut(path) {
            *v = b.into();
        }
    }
}
impl eframe::App for Studio {
    fn update(&mut self, ctx: &Context, _: &mut eframe::Frame) {
        while let Ok(a) = self.events.try_recv() {
            match a {
                Action::Interface(code) => self.set_interface_language(code),
                Action::VoiceVolume(delta) => {
                    self.preview_volume(self.volume() + delta);
                    self.flush_volume();
                }
                Action::VoiceMonitor(name) => self.change_monitor(name),
                Action::TextOnly(on) => {
                    if self.cfg.s("/app/mode") == "vrchat" {
                        self.choose_text_mode(on, ctx);
                        self.draft.data["outbound"]["text_only"] =
                            self.cfg.data["outbound"]["text_only"].clone();
                    }
                }
                Action::SpeakerContext => {
                    let mut c = self.cfg.clone();
                    let on = !c.b("/soniox/keep_speaker_context");
                    c.data["soniox"]["keep_speaker_context"] = on.into();
                    if self.save(c) {
                        self.draft.data["soniox"]["keep_speaker_context"] = on.into();
                        for i in 0..2 {
                            if self.cfg.engine(i == 1) == "soniox" {
                                self.route(i, ctx);
                            }
                        }
                    }
                }
                Action::Nemotron(inbound) => self.toggle_filter_option(inbound, true),
                Action::SpeechFilter(inbound) => self.toggle_filter_option(inbound, false),
                Action::StartSession => {
                    if Self::validate(&self.cfg, self.lang, self.cable()).is_err() {
                        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(ViewportCommand::Focus);
                    }
                    self.start(ctx);
                }
                Action::Show => {
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                Action::Hold(v) => {
                    if self.held != v {
                        self.held = v;
                        self.route(0, ctx);
                    }
                }
                Action::Warning(e) => self.error = e,
                Action::PlacePanel(placement) => {
                    let mut next = self.cfg.clone();
                    placement.save_to(&mut next);
                    if self.save(next) {
                        if self.page != Page::Live {
                            placement.save_to(&mut self.draft);
                        }
                        self.update_vr();
                    }
                }
                Action::Microphone(name) => {
                    if self.devices.was_cable_mic(&name) {
                        continue;
                    }
                    self.change_lane(0, "/outbound/mic_device", name.clone().into(), ctx);
                    if self.page != Page::Live && self.cfg.s("/outbound/mic_device") == name {
                        self.draft.data["outbound"]["mic_device"] = name.into();
                    }
                }
                Action::CaptionDisplay(change) => {
                    let mut next = self.cfg.clone();
                    change.apply(&mut next);
                    if self.save(next) {
                        if self.page != Page::Live {
                            for key in match change {
                                crate::vr::DisplayChange::Font(_) => &["font_size"][..],
                                crate::vr::DisplayChange::Scale(_) => &["width_m", "height_m"][..],
                                crate::vr::DisplayChange::Original => &["show_source"][..],
                            } {
                                self.draft.data["overlay"][*key] =
                                    self.cfg.data["overlay"][*key].clone();
                            }
                        }
                        self.update_vr();
                    }
                }
                Action::Translation(on) => {
                    self.change_lane(0, "/dashboard/translation_on", on.into(), ctx)
                }
                Action::ToggleTranslation => self.change_lane(
                    0,
                    "/dashboard/translation_on",
                    (!self.cfg.b("/dashboard/translation_on")).into(),
                    ctx,
                ),
                Action::ToggleSubtitles => self.change_lane(
                    1,
                    "/dashboard/subtitles_on",
                    (!self.cfg.b("/dashboard/subtitles_on")).into(),
                    ctx,
                ),
                Action::Language(s) => {
                    self.change_lane(0, "/outbound/target_language", s.into(), ctx)
                }
                Action::SubtitleLanguage(s) => {
                    self.change_lane(1, "/inbound/target_language", s.into(), ctx)
                }
            }
        }
        if let Some(result) = self.installer.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.installer = None;
            match result {
                Ok(()) => self.discover(ctx),
                Err(e) => self.error = e.to_string(),
            }
        }
        if let Some(rx) = &self.devices_rx {
            if let Ok(devices) = rx.try_recv() {
                let previous = self.cable();
                let previous_output = self
                    .devices
                    .passthrough_output(self.cfg.s("/outbound/tts_device"))
                    .map(str::to_owned);
                let mut mic_changed = false;
                match devices {
                    Ok(snapshot) => {
                        self.devices = snapshot;
                        self.devices_ready = true;
                        if self
                            .devices
                            .was_cable_mic(self.draft.s("/outbound/mic_device"))
                        {
                            self.draft.data["outbound"]["mic_device"] = "".into();
                        }
                        if self
                            .devices
                            .was_cable_mic(self.cfg.s("/outbound/mic_device"))
                        {
                            let mut c = self.cfg.clone();
                            c.data["outbound"]["mic_device"] = "".into();
                            mic_changed = self.save(c);
                        }
                    }
                    Err(e) => {
                        self.devices = Default::default();
                        self.devices_ready = false;
                        self.error = e.to_string();
                    }
                }
                self.devices_rx = None;
                if self.running
                    && (mic_changed
                        || previous_output.as_deref()
                            != self
                                .devices
                                .passthrough_output(self.cfg.s("/outbound/tts_device"))
                        || self.routes[0].is_none() && self.cfg.b("/outbound/enabled")
                        || (previous == Cable::Ready) != (self.cable() == Cable::Ready))
                {
                    self.route(0, ctx);
                } else {
                    self.update_vr();
                }
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.exiting
            && self.tray.is_some()
            && self.cfg.s("/ui/close_action") == "tray"
        {
            self.voices.close();
            self.checks.close();
            self.overlay.finish_edit();
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        if let Some((_, at)) = self.pending_volume {
            let delay = Duration::from_millis(220).saturating_sub(at.elapsed());
            if delay.is_zero() {
                self.flush_volume();
            } else {
                ctx.request_repaint_after(delay);
            }
        }
        self.title(ctx);
        self.footer(ctx);
        CentralPanel::default()
            .frame(Frame::new().fill(CANVAS).inner_margin(Margin {
                left: 26,
                right: 26,
                top: 20,
                bottom: 8,
            }))
            .show(ctx, |ui| {
                ScrollArea::vertical()
                    .id_salt((
                        "content",
                        match self.page {
                            Page::Live => 9,
                            Page::Setup(n) => n,
                        },
                    ))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if !self.error.is_empty() {
                            ui.colored_label(ALERT, &self.error);
                        }
                        match self.page {
                            Page::Setup(n) => self.setup_view(ui, n, ctx),
                            Page::Live => self.dashboard(ui, ctx),
                        }
                    });
            });
        self.checkup_dialog(ctx);
        if self.voice_help {
            let l = self.lang;
            let cable = self.cable();
            let modal = Modal::new(Id::new("voice-cable-help")).show(ctx, |ui| {
                ui.set_max_width(400.);
                ui.heading(l.t("번역 음성", "Translated voice", "翻訳音声", "翻译语音"));
                ui.label(cable.guidance(l));
                ui.label(l.t(
                    "텍스트 번역과 상대방 자막은 케이블 없이 사용할 수 있습니다.",
                    "Text translation and incoming captions work without the cable.",
                    "テキスト翻訳と相手の字幕はケーブルなしで使えます。",
                    "文字翻译和对方字幕无需虚拟声卡。",
                ));
                ui.horizontal_wrapped(|ui| {
                    if cable == Cable::Missing {
                        ui.hyperlink_to(
                            l.t(
                                "VB-Audio 공식 설치 페이지",
                                "VB-Audio download page",
                                "VB-Audio 公式配布ページ",
                                "VB-Audio 官方下载页",
                            ),
                            "https://vb-audio.com/Cable/",
                        );
                    }
                    if ui
                        .add_enabled(
                            self.devices_rx.is_none(),
                            Button::new(l.t(
                                "장치 다시 찾기",
                                "Refresh devices",
                                "機器を再検出",
                                "刷新设备",
                            )),
                        )
                        .clicked()
                    {
                        self.discover(ctx);
                    }
                    if ui.button(l.t("닫기", "Close", "閉じる", "关闭")).clicked() {
                        self.voice_help = false;
                    }
                });
            });
            if modal.should_close() {
                self.voice_help = false;
            }
        }
        self.voices
            .dialog(ctx, &mut self.draft, self.lang, &self.rt);
        if self.about {
            Window::new("VRCLT")
                .open(&mut self.about)
                .collapsible(false)
                .resizable(true)
                .default_width(470.)
                .show(ctx, |ui| {
                    ui.label(bold("VRCLT", 24.));
                    ui.label(format!("Rust desktop   {}", env!("CARGO_PKG_VERSION")));
                    ScrollArea::vertical().max_height(300.).show(ui, |ui| {
                        for (name, body) in [
                            ("VRCLT", include_str!("../LICENSE")),
                            (
                                "Nemotron RE-USE",
                                include_str!("../licenses/nemotron-reuse.txt"),
                            ),
                            ("audio.cpp", include_str!("../licenses/audiocpp.txt")),
                            ("Silero VAD", include_str!("../licenses/silero-vad.txt")),
                            ("ONNX Runtime", include_str!("../licenses/onnxruntime.txt")),
                            (
                                "DeepFilterNet3",
                                include_str!("../licenses/deepfilternet.txt"),
                            ),
                            (
                                "Pretendard JP",
                                include_str!("../assets/fonts/LICENSE-PretendardJP.txt"),
                            ),
                            (
                                "Noto Sans CJK",
                                include_str!("../assets/fonts/LICENSE-NotoSansCJK.txt"),
                            ),
                        ] {
                            ui.collapsing(name, |ui| {
                                ui.label(body);
                            });
                        }
                    });
                });
        }
        resize(ctx);
        self.overlay.show(
            ctx,
            self.lang,
            &self.cfg,
            self.routes[1].as_ref().map(|r| &r.lane),
        );
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.flush_volume();
        self.overlay.finish_edit();
        self.vr.take();
        self.stop();
    }
}

fn voice_controls(
    ui: &mut Ui,
    l: Lang,
    volume: &mut i32,
    monitor: &mut String,
    outputs: &[String],
) {
    let row_width = ui.available_width();
    ui.horizontal_wrapped(|ui| {
        let label = ui.label(RichText::new(l.voice_volume()).color(MUTE));
        ui.spacing_mut().slider_width = (row_width - label.rect.width() - 84.).clamp(88., 210.);
        ui.visuals_mut().widgets.inactive.bg_fill = LINE;
        ui.visuals_mut().selection.bg_fill = SKY;
        ui.add(Slider::new(volume, 0..=200).suffix("%").trailing_fill(true))
            .labelled_by(label.id);
    });
    ui.horizontal_wrapped(|ui| {
        let label = ui.label(RichText::new(l.monitor()).color(MUTE));
        ComboBox::from_id_salt("voice_monitor")
            .width((row_width - label.rect.width() - 20.).clamp(88., 320.))
            .truncate()
            .selected_text(if monitor.is_empty() {
                l.monitor_off()
            } else {
                monitor.as_str()
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(monitor, String::new(), l.monitor_off());
                let selected = crate::setup::selected_output(outputs, monitor).to_owned();
                for name in outputs {
                    if ui.selectable_label(selected == *name, name).clicked() {
                        monitor.clone_from(name);
                    }
                }
            })
            .response
            .labelled_by(label.id)
            .on_hover_text(if monitor.is_empty() {
                l.monitor_off()
            } else {
                monitor.as_str()
            });
    });
}
