use super::{
    View,
    menu::{Command, Menu},
    panel_ui::{Icon, Layout, r},
    placement::Panel,
    wrist::{Screen, available},
};
use crate::{
    labels::{Lang, engine_name, lang_name},
    theme::{CANVAS, INK, LINE, MINT, MUTE, PAPER, SKY, SOFT, TEXT},
};

pub const WIDTH: i32 = 960;
pub const HEIGHT: i32 = 484;
pub const WIDTH_METERS: f32 = 2.4; // Same dashboard width as the original VRCLT panel.

pub fn configure(ov: &openvr_sys::VR_IVROverlay_FnTable, handle: u64) -> anyhow::Result<()> {
    // SteamVR uses the mouse scale's aspect ratio for its overlay coordinates.
    // Register the same pixel grid that we upload to the compositor.
    unsafe {
        super::check(ov.SetOverlayWidthInMeters.unwrap()(handle, WIDTH_METERS))?;
        super::check(ov.SetOverlayInputMethod.unwrap()(
            handle,
            openvr_sys::VROverlayInputMethod_Mouse,
        ))?;
        super::check(ov.SetOverlayMouseScale.unwrap()(
            handle,
            &mut openvr_sys::HmdVector2_t {
                v: [WIDTH as f32, HEIGHT as f32],
            },
        ))
    }
}

pub fn filter_status(screen: Screen, v: &View, l: Lang) -> Option<&'static str> {
    let inbound = match screen {
        Screen::Microphones => false,
        Screen::Captions => true,
        _ => return None,
    };
    let lane = v.routes[usize::from(inbound)].as_ref()?;
    use crate::audio::reuse::Status;
    match &*lane.gpu_filter.lock() {
        Status::Preparing => Some(l.t("GPU 준비 중", "Preparing GPU", "GPU 準備中", "GPU 准备中")),
        Status::Assisted => Some(l.t(
            "GPU 혼잡 / CPU 보조",
            "GPU busy / CPU assist",
            "GPU 混雑 / CPU 補助",
            "GPU 繁忙 / CPU 辅助",
        )),
        Status::Fallback(_) => Some(l.t(
            "CPU 필터로 전환",
            "Using CPU fallback",
            "CPU フィルターに切替",
            "已切换至 CPU",
        )),
        _ => None,
    }
}

#[derive(Default)]
pub struct Pointer {
    pixel: Option<(f32, f32)>,
    pressed: Option<(usize, Command)>,
}
impl Pointer {
    pub fn clear(&mut self) {
        self.pixel = None;
        self.pressed = None;
    }
    pub fn moved(&mut self, x: f32, y: f32) {
        self.pixel = ((0.0..=WIDTH as f32).contains(&x) && (0.0..=HEIGHT as f32).contains(&y))
            .then_some((x, HEIGHT as f32 - y));
    }
    fn target<'a>(&self, layout: &'a Layout) -> Option<(usize, &'a Command)> {
        let (x, y) = self.pixel?;
        layout.target_pixel(x, y)
    }
    pub fn focus(&self, layout: &Layout) -> (Option<usize>, Option<usize>) {
        let hovered = self.target(layout);
        let pressed = self
            .pressed
            .as_ref()
            .and_then(|(id, cmd)| (Some((*id, cmd)) == hovered).then_some(*id));
        (hovered.map(|(id, _)| id), pressed)
    }
    pub fn down(&mut self, layout: &Layout, x: f32, y: f32) {
        self.moved(x, y);
        self.pressed = self.target(layout).map(|(i, cmd)| (i, cmd.clone()));
    }
    pub fn up(&mut self, layout: &Layout, x: f32, y: f32) -> Option<Command> {
        self.moved(x, y);
        let released = self.target(layout);
        let (id, cmd) = self.pressed.take()?;
        (released == Some((id, &cmd))).then_some(cmd)
    }
}

fn title(screen: Screen, l: Lang) -> &'static str {
    match screen {
        Screen::Live => l.t("통역 콘솔", "Console", "通訳コンソール", "翻译控制台"),
        Screen::Captions => l.t("자막 설정", "Captions", "字幕設定", "字幕设置"),
        Screen::Placement => l.t("패널 배치", "Panel placement", "パネル配置", "面板位置"),
        Screen::Microphones => l.t("마이크", "Microphone", "マイク", "麦克风"),
        Screen::Interface => l.t("화면 언어", "Display language", "表示言語", "界面语言"),
        Screen::Languages { inbound: false, .. } => l.t(
            "내 말 번역 언어",
            "My target language",
            "自分の翻訳先",
            "我的翻译语言",
        ),
        Screen::Languages { inbound: true, .. } => l.t(
            "상대방 자막 언어",
            "Caption language",
            "相手の字幕言語",
            "对方字幕语言",
        ),
        Screen::Settings => l.t("설정", "Settings", "設定", "设置"),
        Screen::Voice { .. } => l.voice_output(),
    }
}

pub fn layout(menu: &Menu, v: &View, l: Lang) -> Layout {
    let mut ui = Layout::new(WIDTH, HEIGHT);
    let screen = menu.dashboard;
    ui.surface(r(0, 0, WIDTH, HEIGHT), 18, CANVAS, Some(LINE), None);
    ui.fill(r(1, 1, 226, HEIGHT - 2), 17, INK);
    ui.fill(r(226, 1, 1, HEIGHT - 2), 0, LINE);
    ui.icon(26, 29, 39, Icon::Brand, SKY);
    ui.label(r(77, 18, 133, 62), "VRCLT", 40, true, TEXT);
    for (i, (dest, icon)) in [
        (Screen::Live, Icon::Talk),
        (Screen::Captions, Icon::Captions),
        (Screen::Microphones, Icon::Mic),
        (Screen::Placement, Icon::Placement),
        (Screen::Interface, Icon::Globe),
        (Screen::Voice { page: 0 }, Icon::Voice),
    ]
    .into_iter()
    .enumerate()
    {
        let top = 101 + i as i32 * 49;
        let selected = screen == dest
            || matches!(
                (screen, dest),
                (Screen::Voice { .. }, Screen::Voice { .. })
                    | (Screen::Languages { .. }, Screen::Live)
            );
        ui.surface(
            r(14, top, 198, 45),
            10,
            if selected { SOFT } else { INK },
            None,
            Some(Command::Open(dest)),
        );
        if selected {
            ui.fill(r(14, top + 12, 4, 25), 2, SKY);
        }
        ui.icon(30, top + 12, 24, icon, if selected { SKY } else { MUTE });
        ui.label(
            r(67, top, 136, 45),
            title(dest, l),
            24,
            selected,
            if selected { TEXT } else { MUTE },
        );
    }
    ui.fill(r(25, 398, 174, 1), 0, LINE);
    ui.label(
        r(27, 404, 171, 30),
        l.t(
            "내 말 번역 엔진",
            "Voice engine",
            "自分の翻訳エンジン",
            "语音翻译引擎",
        ),
        19,
        false,
        MUTE,
    );
    ui.label(
        r(27, 432, 178, 39),
        engine_name(v.cfg.engine(false)),
        25,
        true,
        TEXT,
    );

    ui.label(r(256, 19, 341, 67), title(screen, l), 47, true, TEXT);
    let state = if v.running {
        l.t("통역 중", "Running", "通訳中", "翻译中")
    } else {
        l.t("대기 중", "Standby", "待機中", "待机中")
    };
    ui.fill(r(656, 45, 10, 10), 5, if v.running { MINT } else { MUTE });
    ui.label(
        r(675, 27, 82, 46),
        state,
        22,
        false,
        if v.running { MINT } else { MUTE },
    );
    let (icon, action) = if v.running {
        (Icon::Restart, l.t("재시작", "Restart", "再起動", "重启"))
    } else {
        (Icon::Play, l.start())
    };
    ui.button(
        r(771, 23, 157, 54),
        if v.running { PAPER } else { SOFT },
        Some(if v.running { LINE } else { SKY }),
        Command::Session,
    );
    ui.icon(785, 38, 24, icon, SKY);
    ui.center(r(812, 23, 107, 54), action, 26, true, TEXT);
    ui.fill(r(255, 95, 673, 1), 0, LINE);
    if screen == Screen::Live {
        live(&mut ui, v, l);
    } else {
        let mut content = Layout::default();
        content.body(screen, menu, v, l);
        ui.append_at(content, 232, 29);
        if matches!(screen, Screen::Microphones | Screen::Captions) {
            let inbound = screen == Screen::Captions;
            let enabled = v.cfg.b(crate::config::Config::speech_filter_path(inbound));
            let gpu = v.cfg.b(crate::config::Config::nemotron_path(inbound));
            ui.button(
                r(256, 420, 320, 52),
                PAPER,
                Some(LINE),
                Command::SpeechFilter(inbound),
            );
            ui.label(r(272, 420, 220, 52), l.speech_filter(), 25, true, TEXT);
            ui.toggle(506, 431, enabled, if inbound { MINT } else { SKY });
            ui.surface(
                r(586, 420, 342, 52),
                10,
                PAPER,
                Some(LINE),
                enabled.then_some(Command::Nemotron(inbound)),
            );
            ui.center(
                r(594, 420, 326, 52),
                if !enabled {
                    l.original_audio()
                } else if gpu {
                    filter_status(screen, v, l).unwrap_or("Nemotron RE-USE (GPU)")
                } else {
                    "DeepFilterNet3 (CPU)"
                },
                25,
                true,
                if enabled { TEXT } else { MUTE },
            );
        }
        if screen == Screen::Placement && menu.edit.is_some() {
            let hint = if menu.edit == Some(Panel::Captions) {
                l.t(
                    "메뉴를 닫고 그립으로 이동하거나 아래 모서리를 트리거로 끌어주세요.",
                    "Close the dashboard. Hold grip to move or drag a lower corner to resize.",
                    "メニューを閉じ、グリップで移動、下の角をトリガーでサイズ変更。",
                    "关闭菜单后，握住握把移动，或扣住扳机拖动下角调整大小。",
                )
            } else {
                l.t(
                    "메뉴를 닫고 손목판을 그립으로 잡아 옮기세요.",
                    "Close the dashboard and hold grip to move the wrist menu.",
                    "メニューを閉じ、グリップで手首メニューを移動。",
                    "关闭菜单后，按住握把移动腕部菜单。",
                )
            };
            ui.label(r(257, 418, 669, 47), hint, 23, false, MUTE);
        }
    }
    ui
}

fn live(ui: &mut Layout, v: &View, l: Lang) {
    let c = &v.cfg;
    for (i, x, accent, icon, name) in [
        (0, 256, SKY, Icon::Mic, l.out()),
        (1, 600, MINT, Icon::Captions, l.incoming()),
    ] {
        let inbound = i == 1;
        let on = c.b(if inbound {
            "/dashboard/subtitles_on"
        } else {
            "/dashboard/translation_on"
        });
        let code = c.s(if inbound {
            "/inbound/target_language"
        } else {
            "/outbound/target_language"
        });
        let page = available(v, inbound)
            .iter()
            .position(|s| *s == code)
            .unwrap_or(0)
            / 9;
        ui.surface(r(x, 117, 328, 245), 15, PAPER, Some(LINE), None);
        ui.button(r(x + 6, 123, 316, 62), PAPER, None, Command::Lane(i));
        ui.icon(x + 17, 139, 29, icon, accent);
        ui.label(r(x + 56, 122, 189, 62), name, 30, true, TEXT);
        ui.toggle(x + 254, 139, on, accent);
        ui.button(
            r(x + 14, 193, 300, 101),
            INK,
            Some(LINE),
            Command::Open(Screen::Languages { inbound, page }),
        );
        ui.label(
            r(x + 31, 200, 246, 31),
            l.t("번역할 언어", "Translate into", "翻訳先", "目标语言"),
            21,
            false,
            MUTE,
        );
        ui.label(r(x + 31, 228, 229, 58), lang_name(code), 42, true, TEXT);
        ui.icon(x + 271, 235, 22, Icon::Chevron, accent);
        let detail = if inbound {
            c.s("/inbound/process")
        } else {
            c.s("/outbound/mic_device")
        };
        let detail = if detail.is_empty() {
            if inbound {
                l.t("앱 미선택", "No app selected", "アプリ未選択", "未选择应用")
            } else {
                l.auto_mic()
            }
        } else {
            detail
        };
        if !inbound {
            ui.button(
                r(x + 7, 302, 314, 51),
                PAPER,
                None,
                Command::Open(Screen::Microphones),
            );
            ui.icon(x + 285, 320, 16, Icon::Chevron, MUTE);
        }
        ui.label(
            r(x + 18, 306, 76, 42),
            if inbound {
                l.t("입력 앱", "App", "入力アプリ", "输入应用")
            } else {
                l.input()
            },
            20,
            false,
            MUTE,
        );
        ui.label(
            r(x + 103, 306, if inbound { 207 } else { 175 }, 42),
            detail,
            22,
            false,
            TEXT,
        );
    }
    if c.s("/app/mode") == "vrchat" {
        ui.fill(r(256, 382, 672, 1), 0, LINE);
        ui.label(
            r(256, 397, 158, 60),
            l.t("보내는 방식", "Send as", "送信方法", "发送方式"),
            26,
            true,
            TEXT,
        );
        for (x, text, icon, name) in [
            (420, false, Icon::Voice, v.cable.voice_label(l)),
            (
                677,
                true,
                Icon::Text,
                l.t("텍스트 모드", "Text only", "テキストモード", "文本模式"),
            ),
        ] {
            let selected = text == v.cable.text_only(c);
            ui.button(
                r(x, 397, 251, 60),
                if selected { SOFT } else { INK },
                Some(if selected { SKY } else { LINE }),
                Command::TextOnly(text),
            );
            ui.icon(x + 17, 415, 24, icon, if selected { SKY } else { MUTE });
            ui.center(
                r(x + 51, 397, 188, 60),
                name,
                27,
                true,
                if selected { TEXT } else { MUTE },
            );
        }
    }
}
