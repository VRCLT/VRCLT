use super::*;
use crate::{
    audio::voice_sample::{Capture, CaptureMode},
    setup::{Cable, Plan, Undo, virtual_device},
};

#[derive(Default)]
pub(super) struct Checkup {
    pub(super) last_scan: Option<Instant>,
    open: bool,
    preview: Option<Plan>,
    undo: Option<Undo>,
    note: String,
    finder: Option<Finder>,
}
impl Checkup {
    pub(super) fn close(&mut self) {
        self.open = false;
        self.preview = None;
        self.finder = None;
    }
}
struct Finder {
    inputs: Vec<(String, Capture, bool)>,
    manual: Option<String>,
}

#[derive(Clone, Copy)]
enum Fix {
    Key,
    Cable,
    Mic,
    Output,
    Game,
    Sound,
}
struct Check {
    title: &'static str,
    state: &'static str,
    ok: bool,
    detail: String,
    fix: Fix,
}
impl Studio {
    pub(super) fn open_checks(&mut self, ctx: &Context) {
        self.checks.open = true;
        self.checks.note.clear();
        self.checks.preview = None;
        self.discover(ctx);
    }
    fn apply_plan(&mut self, plan: &Plan, ctx: &Context) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.devices.plan(&self.cfg) == *plan,
            "{}",
            self.lang.t(
                "장치 설정이 바뀌었습니다. 자동 설정을 다시 눌러 주세요.",
                "Device settings changed. Review auto setup again.",
                "デバイスが変更されました。自動設定を再確認してください。",
                "设备设置已更改。请重新检查自动设置。"
            )
        );
        let mut after = self.cfg.clone();
        let mut local = Vec::new();
        for (path, proposed) in [
            ("/outbound/mic_device", &plan.mic),
            ("/outbound/tts_device", &plan.out),
        ] {
            if let Some(value) = proposed {
                local.push((path, after.s(path).to_owned(), value.clone()));
                *after.data.pointer_mut(path).unwrap() = value.clone().into();
            }
        }
        let registry = if plan.game_mic.is_some() || plan.osc {
            Some(crate::setup::Changes::apply(
                plan.game_mic.as_deref(),
                plan.osc,
            )?)
        } else {
            None
        };
        if !local.is_empty() && !self.save(after) {
            if let Some(changed) = &registry {
                changed.undo()?;
            }
            anyhow::bail!("{}", self.error)
        }
        if self.running && !local.is_empty() {
            self.route(0, ctx);
        }
        self.checks.undo = Some(Undo { registry, local });
        Ok(())
    }
    fn undo_plan(&mut self, ctx: &Context) -> anyhow::Result<()> {
        let Some(undo) = &self.checks.undo else {
            return Ok(());
        };
        let mut after = self.cfg.clone();
        let mut changed = false;
        for (path, was, ours) in &undo.local {
            if after.s(path) == ours {
                *after.data.pointer_mut(path).unwrap() = was.clone().into();
                changed = true;
            }
        }
        if let Some(reg) = &undo.registry {
            reg.undo()?;
        }
        if changed && !self.save(after) {
            anyhow::bail!("{}", self.error);
        }
        if changed && self.running {
            self.route(0, ctx);
        }
        self.checks.undo = None;
        Ok(())
    }
    pub(super) fn checkup_dialog(&mut self, ctx: &Context) {
        if !self.checks.open {
            return;
        }
        let l = self.lang;
        let mut close = false;
        let modal = Modal::new(Id::new("connection-check"))
            .frame(
                Frame::new()
                    .fill(PAPER)
                    .inner_margin(22)
                    .corner_radius(12)
                    .stroke(Stroke::new(1_f32, LINE)),
            )
            .show(ctx, |ui| {
                ui.set_width((ctx.screen_rect().width() - 84.).clamp(280., 665.));
                ui.horizontal(|ui| {
                    ui.label(bold(
                        if self.checks.finder.is_some() {
                            l.t(
                                "내 마이크 찾기",
                                "Find my microphone",
                                "マイクを探す",
                                "查找麦克风",
                            )
                        } else {
                            l.t("설정 체크", "Setup check", "設定チェック", "设置检查")
                        },
                        24.,
                    ));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        close = ui.button(l.t("닫기", "Close", "閉じる", "关闭")).clicked();
                    });
                });
                ui.separator();
                ScrollArea::vertical()
                    .id_salt("check-body")
                    .max_height((ctx.screen_rect().height() - 150.).max(150.))
                    .min_scrolled_height((ctx.screen_rect().height() - 150.).clamp(150., 480.))
                    .show(ui, |ui| {
                        if self.checks.finder.is_some() {
                            self.finder_view(ui, ctx);
                            return;
                        }
                        if self.devices_rx.is_some() && !self.devices_ready {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(l.t(
                                    "장치를 확인하고 있습니다.",
                                    "Checking devices…",
                                    "デバイスを確認中…",
                                    "正在检查设备…",
                                ));
                            });
                            return;
                        }
                        if self.devices_ready {
                            self.check_rows(ui, ctx);
                        } else {
                            ui.colored_label(ALERT, &self.error);
                        }
                        ui.separator();
                        let plan = self.devices.plan(&self.cfg);
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add_enabled(
                                    self.devices_ready && !self.load_error,
                                    Button::new(
                                        bold(
                                            l.t("자동 설정", "Auto setup", "自動設定", "自动设置"),
                                            16.,
                                        )
                                        .color(INK),
                                    )
                                    .fill(SKY)
                                    .min_size(vec2(104., 36.)),
                                )
                                .clicked()
                            {
                                self.checks.note.clear();
                                if plan.empty() {
                                    self.checks.note = l
                                        .t(
                                            "자동으로 바꿀 항목이 없습니다.",
                                            "No automatic changes available.",
                                            "自動で変更できる項目はありません。",
                                            "没有可自动更改的项目。",
                                        )
                                        .into();
                                } else {
                                    self.checks.preview = Some(plan.clone());
                                }
                            }
                            if ui
                                .button(l.t("다시 확인", "Recheck", "再確認", "重新检查"))
                                .clicked()
                            {
                                self.checks.preview = None;
                                self.discover(ctx);
                            }
                            if self.checks.undo.is_some()
                                && ui
                                    .button(l.t("되돌리기", "Undo", "元に戻す", "撤销"))
                                    .clicked()
                            {
                                self.checks.note = match self.undo_plan(ctx) {
                                    Ok(()) => l
                                        .t(
                                            "이전 설정으로 되돌렸습니다.",
                                            "Previous settings restored.",
                                            "以前の設定に戻しました。",
                                            "已恢复原设置。",
                                        )
                                        .into(),
                                    Err(e) => e.to_string(),
                                };
                                self.checks.preview = None;
                                self.discover(ctx);
                            }
                        });
                        if let Some(plan) = self.checks.preview.clone() {
                            ui.add_space(4.);
                            ui.label(bold(
                                l.t(
                                    "적용할 변경",
                                    "Changes to apply",
                                    "適用する変更",
                                    "即将应用的更改",
                                ),
                                16.,
                            ));
                            for (name, value) in [
                                (l.input(), plan.mic.as_deref()),
                                (l.output(), plan.out.as_deref()),
                                ("VRChat VR / Desktop", plan.game_mic.as_deref()),
                            ] {
                                if let Some(value) = value {
                                    ui.label(format!("{name}  →  {value}"));
                                }
                            }
                            if plan.osc {
                                ui.label(l.t(
                                    "VRChat OSC 켜기",
                                    "Enable VRChat OSC",
                                    "VRChat OSC を有効にする",
                                    "启用 VRChat OSC",
                                ));
                            }
                            ui.horizontal(|ui| {
                                if ui
                                    .button(l.t(
                                        "변경 적용",
                                        "Apply changes",
                                        "変更を適用",
                                        "应用更改",
                                    ))
                                    .clicked()
                                {
                                    self.checks.note = match self.apply_plan(&plan, ctx) {
                                        Ok(()) => l
                                            .t(
                                                "자동 설정을 적용했습니다.",
                                                "Changes applied.",
                                                "自動設定を適用しました。",
                                                "已应用自动设置。",
                                            )
                                            .into(),
                                        Err(e) => e.to_string(),
                                    };
                                    self.checks.preview = None;
                                    self.discover(ctx);
                                }
                                if ui
                                    .button(l.t("취소", "Cancel", "キャンセル", "取消"))
                                    .clicked()
                                {
                                    self.checks.preview = None;
                                }
                            });
                        }
                        if !self.checks.note.is_empty() {
                            ui.label(&self.checks.note);
                        }
                    });
            });
        if close || modal.should_close() {
            self.checks.close();
        }
    }
    fn check_items(&self) -> Vec<Check> {
        let l = self.lang;
        let env = &self.devices;
        let cfg = &self.cfg;
        let ready = l.t("정상", "Ready", "正常", "正常");
        let absent = l.t("장치 없음", "Unavailable", "未接続", "无设备");
        let needs = l.t("설정 필요", "Needs setup", "要設定", "需设置");
        let mut items = Vec::new();
        let mut add = |title, state, ok, detail: &str, fix| {
            items.push(Check {
                title,
                state,
                ok,
                detail: detail.to_owned(),
                fix,
            })
        };
        let missing = [false, true]
            .into_iter()
            .filter(|i| {
                cfg.b(if *i {
                    "/inbound/enabled"
                } else {
                    "/outbound/enabled"
                })
            })
            .map(|i| cfg.engine(i))
            .find(|p| cfg.key(p).is_empty());
        add(
            l.t("엔진 API 키", "Engine API key", "API キー", "引擎密钥"),
            if missing.is_some() {
                l.t("입력 필요", "Missing key", "未入力", "缺少密钥")
            } else {
                l.t("입력됨", "Configured", "入力済み", "已填写")
            },
            missing.is_none(),
            missing
                .map(engine_name)
                .unwrap_or_else(|| engine_name(cfg.engine(false))),
            Fix::Key,
        );
        if cfg.b("/outbound/enabled") {
            let cable = self.cable();
            add(
                "VB-CABLE",
                if cable == Cable::Ready {
                    ready
                } else {
                    l.t("선택 사항", "Optional", "任意", "可选")
                },
                true,
                if cable == Cable::Ready {
                    "CABLE Input / CABLE Output"
                } else {
                    cable.guidance(l)
                },
                Fix::Cable,
            );
            let mic = env.current(true, cfg.s("/outbound/mic_device"));
            let usable = !mic.is_empty() && !virtual_device(mic);
            add(
                l.input(),
                if usable {
                    ready
                } else if mic.is_empty() {
                    absent
                } else {
                    needs
                },
                usable,
                if mic.is_empty() {
                    cfg.s("/outbound/mic_device")
                } else {
                    mic
                },
                Fix::Mic,
            );
            if cable == Cable::Ready && !cfg.b("/outbound/text_only") {
                let out = env.current(false, cfg.s("/outbound/tts_device"));
                let valid = env.cable_output_selected(out);
                add(
                    l.t("번역 출력", "Voice output", "翻訳出力", "翻译输出"),
                    if valid {
                        ready
                    } else if out.is_empty() {
                        absent
                    } else {
                        needs
                    },
                    valid,
                    if out.is_empty() {
                        cfg.s("/outbound/tts_device")
                    } else {
                        out
                    },
                    Fix::Output,
                );
                if cfg.s("/app/mode") == "vrchat" {
                    let valid = env.game_mics_ready();
                    add(
                        l.t(
                            "VRChat 마이크",
                            "VRChat mic",
                            "VRChat マイク",
                            "VRChat 麦克风",
                        ),
                        if valid {
                            ready
                        } else if !env.game {
                            l.t("미확인", "Unknown", "未確認", "未确认")
                        } else {
                            needs
                        },
                        valid,
                        if valid {
                            "CABLE Output"
                        } else if !env.game {
                            l.t(
                                "VRChat을 한 번 실행하면 설정을 확인할 수 있습니다.",
                                "Run VRChat once to create its settings.",
                                "VRChat を一度起動してください。",
                                "请先运行一次 VRChat。",
                            )
                        } else {
                            l.t(
                                "VR 및 데스크톱 마이크를 CABLE Output으로 설정하세요.",
                                "Set VR and desktop microphones to CABLE Output.",
                                "VR とデスクトップで CABLE Output を選択してください。",
                                "请将 VR 和桌面麦克风设为 CABLE Output。",
                            )
                        },
                        Fix::Game,
                    );
                }
            }
        }
        if env.needs_osc(cfg) {
            let health = self.osc_health.load(std::sync::atomic::Ordering::Relaxed);
            let (state, good) = if health == 3 {
                (
                    l.t("포트 사용 중", "Port busy", "ポート使用中", "端口被占用"),
                    false,
                )
            } else if health == 2 {
                (l.t("수신 중", "Receiving", "受信中", "接收中"), true)
            } else if env.osc == Some(true) {
                (l.t("켜짐", "Enabled", "有効", "已开启"), true)
            } else if env.osc == Some(false) {
                (l.t("꺼짐", "Disabled", "無効", "未开启"), false)
            } else {
                (l.t("미확인", "Unknown", "未確認", "未确认"), false)
            };
            add(
                "VRChat OSC",
                state,
                good,
                &format!("OSC / {}", cfg.n("/control/osc_listen_port", 9001.) as u16),
                Fix::Game,
            );
        }
        if virtual_device(&env.default_output) {
            add(
                l.t(
                    "Windows 기본 출력",
                    "Windows output",
                    "Windows 出力",
                    "Windows 输出",
                ),
                needs,
                false,
                &env.default_output,
                Fix::Sound,
            );
        }
        items
    }
    pub(super) fn checklist(&mut self, ui: &mut Ui, ctx: &Context) {
        let l = self.lang;
        let delay = Duration::from_secs(10);
        if self.devices_rx.is_none()
            && self
                .checks
                .last_scan
                .is_none_or(|stamp| stamp.elapsed() >= delay)
        {
            self.discover(ctx);
        }
        ctx.request_repaint_after(delay);
        Frame::new()
            .fill(INK)
            .corner_radius(8)
            .inner_margin(Margin::symmetric(14, 9))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(bold(
                        l.t(
                            "설정 체크리스트",
                            "Setup checklist",
                            "設定チェックリスト",
                            "设置检查清单",
                        ),
                        17.,
                    ));
                    ui.label(
                        RichText::new(l.t("자동 확인", "Auto-check", "自動チェック", "自动检查"))
                            .color(MUTE)
                            .size(13.),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .button(l.t("다시 확인", "Recheck", "再確認", "重新检查"))
                            .clicked()
                            && self.devices_rx.is_none()
                        {
                            self.discover(ctx);
                        }
                        if ui
                            .add_enabled(
                                self.devices_ready && !self.load_error,
                                Button::new(l.t("자동 설정", "Auto setup", "自動設定", "自动设置")),
                            )
                            .clicked()
                        {
                            let plan = self.devices.plan(&self.cfg);
                            self.open_checks(ctx);
                            if !plan.empty() {
                                self.checks.preview = Some(plan);
                            } else {
                                self.checks.note = l
                                    .t(
                                        "자동으로 바꿀 항목이 없습니다.",
                                        "No automatic changes available.",
                                        "自動で変更できる項目はありません。",
                                        "没有可自动更改的项目。",
                                    )
                                    .into();
                            }
                        }
                    });
                });
                let items = self.check_items();
                let cols = if ui.available_width() < 810. { 4 } else { 6 };
                for group in items.chunks(cols) {
                    ui.columns(cols, |columns| {
                        for (col, item) in columns.iter_mut().zip(group.iter()) {
                            let pending = !self.devices_ready;
                            let state = if pending {
                                l.t("확인 중", "Checking", "確認中", "检查中")
                            } else {
                                item.state
                            };
                            let color = if pending {
                                MUTE
                            } else if item.ok {
                                MINT
                            } else {
                                ALERT
                            };
                            let response = col
                                .vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 3.;
                                    ui.label(RichText::new(item.title).size(14.).color(TEXT));
                                    ui.label(RichText::new(state).size(13.).color(color));
                                })
                                .response;
                            if col
                                .interact(
                                    response.rect,
                                    Id::new(("check-result", item.title)),
                                    Sense::click(),
                                )
                                .on_hover_cursor(CursorIcon::PointingHand)
                                .on_hover_text(&item.detail)
                                .clicked()
                            {
                                self.open_checks(ctx);
                            }
                        }
                    });
                }
            });
    }
    fn check_rows(&mut self, ui: &mut Ui, ctx: &Context) {
        let l = self.lang;
        for check in self.check_items() {
            ui.horizontal_wrapped(|ui| {
                ui.label(bold(check.title, 16.));
                ui.colored_label(if check.ok { MINT } else { ALERT }, check.state);
                if !check.detail.is_empty() {
                    hint(ui, &check.detail);
                }
                match check.fix {
                    Fix::Key if !check.ok => {
                        if ui
                            .link(l.t("키 설정", "Set up key", "キー設定", "设置密钥"))
                            .clicked()
                        {
                            self.checks.close();
                            self.setup_page(1);
                        }
                    }
                    Fix::Cable if self.cable() == Cable::Missing => {
                        if ui
                            .add_enabled(
                                self.installer.is_none(),
                                Button::new(l.t(
                                    "VB-Cable 설치",
                                    "Install VB-Cable",
                                    "VB-Cable を導入",
                                    "安装 VB-Cable",
                                )),
                            )
                            .clicked()
                        {
                            self.install_driver(ctx);
                        }
                    }
                    Fix::Mic => {
                        if ui
                            .link(l.t(
                                "내 마이크 찾기",
                                "Find my microphone",
                                "マイクを探す",
                                "查找麦克风",
                            ))
                            .clicked()
                        {
                            let inputs = self
                                .devices
                                .inputs
                                .iter()
                                .filter(|n| !virtual_device(n))
                                .map(|n| {
                                    (
                                        n.clone(),
                                        Capture::start(n.clone(), CaptureMode::Meter, ctx.clone()),
                                        false,
                                    )
                                })
                                .collect();
                            self.checks.finder = Some(Finder {
                                inputs,
                                manual: None,
                            });
                        }
                    }
                    Fix::Output if !check.ok => {
                        if ui
                            .link(l.t("장치 선택", "Select device", "デバイスを選ぶ", "选择设备"))
                            .clicked()
                        {
                            self.checks.close();
                            self.setup_page(2);
                        }
                    }
                    Fix::Sound => {
                        ui.hyperlink_to(
                            l.t(
                                "소리 설정 열기",
                                "Open sound settings",
                                "サウンド設定",
                                "声音设置",
                            ),
                            "ms-settings:sound",
                        );
                    }
                    _ => {}
                }
            });
        }
        if self.cfg.s("/app/mode") == "vrchat"
            && self.devices.game_open
            && (!self.devices.game_mics_ready() || self.devices.osc != Some(true))
        {
            hint(
                ui,
                l.t(
                    "VRChat 설정을 바꾸려면 게임을 종료한 뒤 다시 확인하세요.",
                    "Close VRChat and recheck to modify its settings.",
                    "VRChat を終了して再確認してください。",
                    "请关闭 VRChat 后重新检查。",
                ),
            );
        }
    }
    fn finder_view(&mut self, ui: &mut Ui, ctx: &Context) {
        let l = self.lang;
        let finder = self.checks.finder.as_mut().unwrap();
        hint(
            ui,
            l.t(
                "마이크에 말해 보세요. 신호가 가장 큰 마이크가 선택됩니다.",
                "Speak to see which microphone responds.",
                "話すと最も反応するマイクが選択されます。",
                "说话后会选择信号最强的麦克风。",
            ),
        );
        for (_, input, failed) in &mut finder.inputs {
            if input.result.try_recv().is_ok() {
                *failed = true;
            }
        }
        let threshold = self.cfg.n("/audio/voice_rms_threshold", 90.).max(1.) as u32;
        let loudest = finder
            .inputs
            .iter()
            .filter(|(_, c, failed)| !*failed && c.progress().peak >= threshold)
            .max_by_key(|(_, c, _)| c.progress().peak)
            .map(|(n, _, _)| n.clone());
        let selected = finder.manual.clone().or(loudest);
        for (name, cap, failed) in &finder.inputs {
            if ui
                .add_enabled(
                    !failed,
                    RadioButton::new(selected.as_ref() == Some(name), name),
                )
                .clicked()
            {
                finder.manual = Some(name.clone());
            }
            if *failed {
                hint(
                    ui,
                    l.t(
                        "입력 장치를 열 수 없습니다.",
                        "Could not open input.",
                        "入力を開けません。",
                        "无法打开输入设备。",
                    ),
                );
            } else {
                meter(ui, cap.progress().level, false);
            }
        }
        if finder.inputs.is_empty() {
            ui.label(l.t(
                "사용할 수 있는 마이크가 없습니다.",
                "No microphones found.",
                "マイクがありません。",
                "未找到麦克风。",
            ));
        }
        let mut done = false;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    selected.is_some(),
                    Button::new(l.t(
                        "이 마이크 사용",
                        "Use microphone",
                        "このマイクを使う",
                        "使用此麦克风",
                    )),
                )
                .clicked()
            {
                if let Some(name) = selected {
                    self.change_lane(0, "/outbound/mic_device", name.into(), ctx);
                }
                done = true;
            }
            if ui.button(l.t("돌아가기", "Back", "戻る", "返回")).clicked() {
                done = true;
            }
        });
        if done {
            self.checks.finder = None;
        } else {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    pub(super) fn install_driver(&mut self, ctx: &Context) {
        if self.installer.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.installer = Some(rx);
        let ctx = ctx.clone();
        self.rt.spawn(async move {
            let _ = tx.send(crate::setup::install_cable().await);
            ctx.request_repaint();
        });
    }
}
