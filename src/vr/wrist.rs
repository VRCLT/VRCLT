pub(super) use super::panel_ui::Layout;
#[cfg(test)]
use super::panel_ui::Part;
use super::panel_ui::{Icon, r};
use super::{
    Panel, View,
    menu::{Command, Menu, MicStatus},
};
use crate::{
    labels::{LANGUAGES, Lang, lang_name},
    theme::{CANVAS, INK, LINE, MINT, MUTE, PAPER, SKY, SOFT, TEXT},
};
pub const WIDTH: i32 = 720;
pub const HEIGHT: i32 = 480;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Live,
    Settings,
    Captions,
    Placement,
    Microphones,
    Voice { page: usize },
    Languages { inbound: bool, page: usize },
    Interface,
}
impl Layout {
    fn header(&mut self, menu: &Menu, v: &View, l: Lang) {
        self.surface(r(0, 0, WIDTH, HEIGHT), 23, CANVAS, Some(LINE), None);
        self.icon(23, 20, 43, Icon::Brand, SKY);
        self.label(r(78, 12, 115, 53), "VRCLT", 38, true, TEXT);
        self.fill(r(201, 24, 1, 30), 0, LINE);
        let section = match menu.screen {
            Screen::Live => match v.cfg.engine(false) {
                "soniox" => "Soniox",
                "openai" => "GPT",
                "qwen" => "Qwen",
                _ => "Gemini",
            },
            Screen::Settings => l.t("설정", "Settings", "設定", "设置"),
            Screen::Captions => l.t("자막 설정", "Caption settings", "字幕設定", "字幕设置"),
            Screen::Placement => l.t("패널 배치", "Panel placement", "パネル配置", "面板位置"),
            Screen::Microphones => l.t("마이크", "Microphone", "マイク", "麦克风"),
            Screen::Voice { .. } => l.voice_output(),
            Screen::Interface => l.t("화면 언어", "Interface language", "表示言語", "界面语言"),
            Screen::Languages { inbound, .. } => {
                if inbound {
                    l.incoming()
                } else {
                    l.out()
                }
            }
        };
        self.label(
            r(
                218,
                13,
                if menu.screen == Screen::Live {
                    139
                } else {
                    450
                },
                54,
            ),
            section,
            if menu.screen == Screen::Live { 27 } else { 31 },
            true,
            if menu.screen == Screen::Live {
                MUTE
            } else {
                TEXT
            },
        );
        if menu.screen == Screen::Live {
            self.fill(r(376, 34, 10, 10), 5, if v.running { MINT } else { MUTE });
            self.label(
                r(396, 16, 116, 46),
                if v.running {
                    l.t("통역 중", "Running", "通訳中", "翻译中")
                } else {
                    l.t("대기 중", "Standby", "待機中", "待机中")
                },
                24,
                false,
                if v.running { MINT } else { MUTE },
            );
            let (label, id) = if v.running {
                (l.t("재시작", "Restart", "再起動", "重启"), Icon::Restart)
            } else {
                (l.start(), Icon::Play)
            };
            self.button(
                r(521, 13, 175, 54),
                if v.running { PAPER } else { SKY },
                Some(if v.running { LINE } else { SKY }),
                Command::Session,
            );
            let fg = if v.running { TEXT } else { INK };
            self.icon(539, 28, 24, id, fg);
            self.center(r(568, 16, 114, 47), label, 27, true, fg);
        }
        self.fill(r(1, 79, WIDTH - 2, 1), 0, LINE);
    }
    fn footer(&mut self, screen: Screen, l: Lang) {
        self.fill(r(24, 394, 672, 1), 0, LINE);
        let live = matches!(screen, Screen::Live | Screen::Languages { .. });
        for (i, id, label, dest, active) in [
            (
                0,
                Icon::Talk,
                l.t("통역 콘솔", "Console", "通訳コンソール", "翻译控制台"),
                Screen::Live,
                live,
            ),
            (
                1,
                Icon::Settings,
                l.t("설정", "Settings", "設定", "设置"),
                Screen::Settings,
                !live,
            ),
        ] {
            let x = 24 + i * 342;
            self.button(
                r(x, 407, 330, 54),
                if active { SOFT } else { INK },
                if active { Some(SKY) } else { Some(LINE) },
                Command::Open(dest),
            );
            self.icon(x + 44, 421, 26, id, if active { SKY } else { MUTE });
            self.center(
                r(x + 73, 409, 194, 50),
                label,
                28,
                true,
                if active { TEXT } else { MUTE },
            );
        }
    }
    fn live(&mut self, v: &View, l: Lang) {
        let c = &v.cfg;
        let vrchat = c.s("/app/mode") == "vrchat";
        if vrchat {
            self.label(
                r(24, 99, 151, 51),
                l.t("보내는 방식", "Send as", "送信方法", "发送方式"),
                25,
                false,
                MUTE,
            );
            let text = v.cable.text_only(c);
            for (x, on, id, title) in [
                (180, false, Icon::Voice, v.cable.voice_label(l)),
                (
                    443,
                    true,
                    Icon::Text,
                    l.t("텍스트 모드", "Text only", "テキストモード", "文本模式"),
                ),
            ] {
                let selected = on == text;
                self.button(
                    r(x, 96, 253, 57),
                    if selected { SOFT } else { INK },
                    Some(if selected { SKY } else { LINE }),
                    Command::TextOnly(on),
                );
                self.icon(x + 17, 112, 25, id, if selected { SKY } else { MUTE });
                self.center(
                    r(x + 48, 99, 190, 51),
                    title,
                    27,
                    true,
                    if selected { TEXT } else { MUTE },
                );
            }
        }
        let y = if vrchat { 170 } else { 103 };
        for (i, accent, title) in [
            (
                0,
                SKY,
                l.t("내 말 번역", "My voice", "自分の音声", "我的语音"),
            ),
            (
                1,
                MINT,
                l.t("상대방 자막", "Their captions", "相手の字幕", "对方字幕"),
            ),
        ] {
            let inbound = i == 1;
            let x = 24 + i * 344;
            let width = 328;
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
            self.surface(r(x, y, width, 374 - y), 15, PAPER, Some(LINE), None);
            self.button(
                r(x + 6, y + 6, width - 12, 66),
                PAPER,
                None,
                Command::Lane(i as usize),
            );
            self.fill(r(x + 15, y + 24, 5, 28), 2, accent);
            self.label(r(x + 32, y + 8, 174, 59), title, 31, true, TEXT);
            self.center(
                r(x + 206, y + 10, 49, 56),
                if on {
                    l.t("켬", "On", "オン", "开")
                } else {
                    l.t("끔", "Off", "オフ", "关")
                },
                22,
                true,
                if on { accent } else { MUTE },
            );
            self.toggle(x + width - 71, y + 23, on, accent);
            self.fill(r(x + 16, y + 76, width - 32, 1), 0, LINE);
            let languages = available(v, inbound);
            let page = languages.iter().position(|s| *s == code).unwrap_or(0) / 9;
            let top = y + 85;
            let height = 374 - top - 12;
            self.button(
                r(x + 12, top, width - 24, height),
                INK,
                Some(LINE),
                Command::Open(Screen::Languages { inbound, page }),
            );
            self.label(
                r(x + 28, top + 7, width - 60, 32),
                l.t("번역할 언어", "Translate into", "翻訳先", "目标语言"),
                22,
                false,
                MUTE,
            );
            self.label(
                r(x + 28, top + 35, width - 85, height - 39),
                lang_name(code),
                43,
                true,
                TEXT,
            );
            self.icon(
                x + width - 52,
                top + height / 2 - 2,
                24,
                Icon::Chevron,
                accent,
            );
        }
    }
    fn settings(&mut self, l: Lang) {
        for (i, (id, label, screen, color)) in [
            (
                Icon::Captions,
                l.t("자막 설정", "Captions", "字幕設定", "字幕设置"),
                Screen::Captions,
                SKY,
            ),
            (
                Icon::Placement,
                l.t("패널 배치", "Panel placement", "パネル配置", "面板位置"),
                Screen::Placement,
                MINT,
            ),
            (
                Icon::Mic,
                l.t("마이크", "Microphone", "マイク", "麦克风"),
                Screen::Microphones,
                MINT,
            ),
            (
                Icon::Globe,
                l.t("화면 언어", "Interface language", "表示言語", "界面语言"),
                Screen::Interface,
                SKY,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let x = 24 + (i % 2) as i32 * 344;
            let y = 96 + (i / 2) as i32 * 96;
            self.button(r(x, y, 328, 82), PAPER, Some(LINE), Command::Open(screen));
            self.icon(x + 17, y + 25, 31, id, color);
            self.label(r(x + 62, y + 8, 252, 66), label, 31, true, TEXT);
        }
        self.button(
            r(24, 288, 672, 85),
            PAPER,
            Some(LINE),
            Command::Open(Screen::Voice { page: 0 }),
        );
        self.icon(42, 314, 31, Icon::Voice, SKY);
        self.label(r(85, 296, 540, 68), l.voice_output(), 31, true, TEXT);
        self.icon(645, 318, 23, Icon::Chevron, MUTE);
    }
    fn voice(&mut self, v: &View, l: Lang, page: usize) {
        if v.cable != crate::setup::Cable::Ready {
            self.label(r(27, 112, 663, 59), "VB-Audio Cable", 35, true, TEXT);
            self.label(r(27, 182, 663, 59), v.cable.guidance(l), 23, false, TEXT);
            return;
        }
        let volume = v.cfg.voice_volume();
        if v.cfg.b("/outbound/text_only") {
            self.label(r(27, 88, 355, 36), l.voice_volume(), 30, true, TEXT);
            self.label(
                r(27, 124, 370, 28),
                l.text_only_audio_hint(),
                19,
                false,
                MUTE,
            );
        } else {
            self.label(r(27, 93, 355, 57), l.voice_volume(), 31, true, TEXT);
        }
        self.icon_button(
            r(405, 94, 64, 55),
            Icon::Minus,
            (volume > 0).then_some(Command::Volume(-10)),
        );
        self.center(r(474, 94, 148, 55), &format!("{volume}%"), 35, true, SKY);
        self.icon_button(
            r(630, 94, 66, 55),
            Icon::Plus,
            (volume < 200).then_some(Command::Volume(10)),
        );
        let current =
            crate::setup::selected_output(&v.outputs, v.cfg.s("/outbound/monitor_device"));
        let mut names: Vec<_> = v.outputs.iter().map(String::as_str).collect();
        if !current.is_empty() && !names.contains(&current) {
            names.push(current);
        }
        let last = names.len().saturating_sub(1) / 3;
        let page = page.min(last);
        self.label(r(27, 156, 420, 50), l.monitor(), 27, true, TEXT);
        self.icon_button(
            r(515, 158, 51, 45),
            Icon::Back,
            (page > 0).then(|| Command::Open(Screen::Voice { page: page - 1 })),
        );
        self.center(
            r(567, 156, 75, 49),
            &format!("{}/{}", page + 1, last + 1),
            23,
            false,
            MUTE,
        );
        self.icon_button(
            r(645, 158, 51, 45),
            Icon::Next,
            (page < last).then_some(Command::Open(Screen::Voice { page: page + 1 })),
        );
        for (i, name) in std::iter::once("")
            .chain(names.into_iter().skip(page * 3).take(3))
            .enumerate()
        {
            let y = 211 + i as i32 * 45;
            let selected = current == name;
            self.button(
                r(24, y, 672, 41),
                if selected { SOFT } else { PAPER },
                Some(if selected { MINT } else { LINE }),
                Command::Monitor(name.into()),
            );
            if selected {
                self.icon(40, y + 10, 21, Icon::Check, MINT);
            }
            self.label(
                r(73, y, 607, 41),
                if name.is_empty() {
                    l.monitor_off()
                } else {
                    name
                },
                25,
                selected,
                TEXT,
            );
        }
    }
    fn stepper(
        &mut self,
        x: i32,
        label: &str,
        val: String,
        minus: Command,
        plus: Command,
        min: bool,
        max: bool,
    ) {
        self.surface(r(x, 99, 328, 130), 12, PAPER, Some(LINE), None);
        self.label(r(x + 16, 108, 295, 39), label, 24, false, MUTE);
        self.icon_button(r(x + 13, 156, 65, 57), Icon::Minus, (!min).then_some(minus));
        self.center(r(x + 81, 152, 166, 64), &val, 37, true, TEXT);
        self.icon_button(r(x + 250, 156, 65, 57), Icon::Plus, (!max).then_some(plus));
    }
    fn switch_line(&mut self, y: i32, label: &str, on: bool, cmd: Command) {
        self.button(r(24, y, 672, 60), PAPER, Some(LINE), cmd);
        self.label(r(44, y + 2, 545, 56), label, 28, true, TEXT);
        self.toggle(621, y + 15, on, SKY);
    }
    fn captions(&mut self, v: &View, l: Lang) {
        let c = &v.cfg;
        let font = c.n("/overlay/font_size", 27.);
        let width = c.n("/overlay/width_m", 0.9);
        self.stepper(
            24,
            l.t("오버레이 글자", "Overlay text", "字幕の文字", "字幕字号"),
            format!("{font:.0} px"),
            Command::Font(-1),
            Command::Font(1),
            font <= 18.,
            font >= 72.,
        );
        self.stepper(
            368,
            l.t("자막판 너비", "Panel width", "字幕枠の幅", "字幕板宽度"),
            format!("{width:.2} m"),
            Command::Scale(-1),
            Command::Scale(1),
            width <= 0.45,
            width >= 1.6,
        );
        self.switch_line(
            243,
            l.t("원문 표시", "Show original text", "原文を表示", "显示原文"),
            c.b("/overlay/show_source"),
            Command::Original,
        );
        if c.engine(false) == "soniox" || c.engine(true) == "soniox" {
            self.switch_line(
                314,
                l.t(
                    "잠깐 멈춰도 화자 번호 유지",
                    "Keep speaker IDs during pauses",
                    "短い無音でも話者番号を維持",
                    "短暂停顿时保留说话人编号",
                ),
                c.b("/soniox/keep_speaker_context"),
                Command::SpeakerContext,
            );
        }
    }
    fn placement(&mut self, m: &Menu, v: &View, l: Lang) {
        for (i, (panel, id, label, subtitle, enabled)) in [
            (
                Panel::Wrist,
                Icon::Wrist,
                l.t("손목 메뉴", "Wrist menu", "手首メニュー", "腕部菜单"),
                l.t(
                    "그립으로 이동",
                    "Grip to move",
                    "グリップで移動",
                    "握把移动",
                ),
                v.cfg.b("/wrist_ui/enabled"),
            ),
            (
                Panel::Captions,
                Icon::Captions,
                l.t("자막판", "Caption panel", "字幕パネル", "字幕板"),
                l.t(
                    "그립으로 이동 / 모서리로 크기 조정",
                    "Grip to move / drag a corner to resize",
                    "グリップで移動／角でサイズ変更",
                    "握把移动 / 拖动下角缩放",
                ),
                v.cfg.b("/overlay/enabled"),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let y = 99 + i as i32 * 145;
            let editing = m.edit == Some(panel);
            self.surface(
                r(24, y, 505, 131),
                12,
                if editing { SOFT } else { PAPER },
                Some(if editing { SKY } else { LINE }),
                enabled.then_some(Command::Edit(panel)),
            );
            self.icon(44, y + 19, 29, id, if enabled { SKY } else { MUTE });
            self.label(r(85, y + 9, 333, 49), label, 32, true, TEXT);
            self.toggle(454, y + 19, editing, SKY);
            self.label(r(44, y + 69, 462, 46), subtitle, 24, false, MUTE);
            self.surface(
                r(541, y, 155, 131),
                12,
                INK,
                Some(LINE),
                enabled.then_some(Command::Reset(panel)),
            );
            self.icon(605, y + 23, 29, Icon::Reset, MUTE);
            self.center(
                r(545, y + 67, 147, 39),
                l.t("초기화", "Reset", "リセット", "重置"),
                25,
                true,
                TEXT,
            );
        }
    }
    fn languages(&mut self, v: &View, l: Lang, inbound: bool, page: usize) {
        let list = available(v, inbound);
        let current = v.cfg.s(if inbound {
            "/inbound/target_language"
        } else {
            "/outbound/target_language"
        });
        let last = list.len().saturating_sub(1) / 9;
        let page = page.min(last);
        self.label(
            r(25, 93, 431, 51),
            l.t("번역할 언어", "Translate into", "翻訳先", "目标语言"),
            29,
            true,
            TEXT,
        );
        self.icon_button(
            r(469, 93, 59, 49),
            Icon::Back,
            page.checked_sub(1)
                .map(|p| Command::Open(Screen::Languages { inbound, page: p })),
        );
        self.center(
            r(531, 94, 100, 46),
            &format!("{} / {}", page + 1, last + 1),
            24,
            true,
            MUTE,
        );
        self.icon_button(
            r(637, 93, 59, 49),
            Icon::Next,
            (page < last).then_some(Command::Open(Screen::Languages {
                inbound,
                page: page + 1,
            })),
        );
        for (i, code) in list.iter().skip(page * 9).take(9).enumerate() {
            let x = 24 + (i % 3) as i32 * 228;
            let y = 156 + (i / 3) as i32 * 77;
            let sel = *code == current;
            self.button(
                r(x, y, 216, 65),
                if sel { SOFT } else { PAPER },
                Some(if sel { SKY } else { LINE }),
                Command::Target(inbound, (*code).into()),
            );
            self.center(
                r(x + 9, y + 2, 198, 61),
                lang_name(code),
                29,
                true,
                if sel { SKY } else { TEXT },
            );
        }
    }
    fn interface(&mut self, l: Lang) {
        for (i, (code, lang)) in [
            ("ko", Lang::Ko),
            ("en", Lang::En),
            ("ja", Lang::Ja),
            ("zh", Lang::Zh),
        ]
        .into_iter()
        .enumerate()
        {
            let x = 24 + (i % 2) as i32 * 344;
            let y = 100 + (i / 2) as i32 * 144;
            let sel = lang == l;
            self.button(
                r(x, y, 328, 129),
                if sel { SOFT } else { PAPER },
                Some(if sel { SKY } else { LINE }),
                Command::Interface(code),
            );
            self.center(
                r(x + 18, y + 19, 292, 91),
                lang_name(code),
                38,
                true,
                if sel { SKY } else { TEXT },
            );
        }
    }
    fn microphones(&mut self, m: &Menu, v: &View, l: Lang) {
        let status = match m.mic {
            MicStatus::Ready => l.t(
                "말해서 내 마이크 찾기",
                "Find my mic by speaking",
                "話してマイクを探す",
                "说话查找麦克风",
            ),
            MicStatus::Listening => l.t(
                "말해 보세요 / 찾기 취소",
                "Speak now / cancel search",
                "話してください／検索中止",
                "请说话 / 取消搜索",
            ),
            MicStatus::Loading => l.t(
                "장치 목록 읽는 중",
                "Reading devices",
                "一覧を取得中",
                "正在获取设备",
            ),
            MicStatus::NoSpeech => l.t(
                "음성 없음 / 다시 찾기",
                "No speech / try again",
                "音声なし／再検索",
                "未检测到声音 / 重试",
            ),
            MicStatus::Unavailable => l.t(
                "마이크 열기 실패 / 다시 찾기",
                "Mic unavailable / retry",
                "マイクを開けません／再検索",
                "麦克风不可用 / 重试",
            ),
        };
        self.surface(
            r(24, 94, 430, 55),
            10,
            SOFT,
            Some(SKY),
            (m.mic != MicStatus::Loading).then_some(Command::Listen),
        );
        self.icon(40, 108, 26, Icon::Mic, SKY);
        let found = m
            .detected
            .as_ref()
            .filter(|_| m.mic == MicStatus::Ready)
            .map(|name| format!("{}  {name}", l.t("찾음", "Found", "検出", "已找到")));
        self.label(
            r(77, 95, 365, 53),
            found.as_deref().unwrap_or(status),
            26,
            true,
            TEXT,
        );
        self.icon_button(r(465, 94, 55, 55), Icon::Refresh, Some(Command::List));
        let last = m.names.len().saturating_sub(1) / 4;
        self.icon_button(
            r(532, 94, 51, 55),
            Icon::Back,
            (m.mic_page > 0).then_some(Command::More(-1)),
        );
        self.center(
            r(584, 96, 61, 51),
            &format!("{}/{}", m.mic_page + 1, last + 1),
            20,
            false,
            MUTE,
        );
        self.icon_button(
            r(646, 94, 50, 55),
            Icon::Next,
            (m.mic_page < last).then_some(Command::More(1)),
        );
        let default = l.auto_mic();
        for (i, (id, name)) in std::iter::once(("", default))
            .chain(
                m.names
                    .iter()
                    .skip(m.mic_page * 4)
                    .take(4)
                    .map(|n| (n.as_str(), n.as_str())),
            )
            .enumerate()
        {
            let y = 158 + i as i32 * 46;
            let selected = v.cfg.s("/outbound/mic_device") == id;
            self.button(
                r(24, y, 672, 42),
                if selected { SOFT } else { PAPER },
                Some(if selected { MINT } else { LINE }),
                Command::Pick(id.into()),
            );
            if selected {
                self.icon(40, y + 10, 22, Icon::Check, MINT);
            }
            self.label(r(74, y + 1, 604, 40), name, 26, selected, TEXT);
        }
    }
    pub(super) fn body(&mut self, screen: Screen, m: &Menu, v: &View, l: Lang) {
        match screen {
            Screen::Live => self.live(v, l),
            Screen::Settings => self.settings(l),
            Screen::Placement => self.placement(m, v, l),
            Screen::Captions => self.captions(v, l),
            Screen::Microphones => self.microphones(m, v, l),
            Screen::Voice { page } => self.voice(v, l, page),
            Screen::Languages { inbound, page } => self.languages(v, l, inbound, page),
            Screen::Interface => self.interface(l),
        }
    }
    pub fn build(m: &Menu, v: &View, l: Lang) -> Self {
        let mut layout = Self::default();
        layout.header(m, v, l);
        layout.body(m.screen, m, v, l);
        layout.footer(m.screen, l);
        layout
    }
}
pub(super) fn available(view: &View, inbound: bool) -> Vec<&str> {
    let c = &view.cfg;
    let engine = c.engine(inbound);
    let favorites = c
        .data
        .pointer(if inbound {
            "/inbound/languages"
        } else {
            "/control/languages"
        })
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str());
    let current = c.s(if inbound {
        "/inbound/target_language"
    } else {
        "/outbound/target_language"
    });
    let mut result = Vec::new();
    for code in favorites
        .chain(["ko", "en", "ja", "zh-Hans", "zh-Hant", current])
        .chain(LANGUAGES.iter().map(|l| l.0))
    {
        if !code.is_empty()
            && !result.contains(&code)
            && crate::provider::supports_target(engine, code)
        {
            result.push(code);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, osc::Action};
    fn view() -> View {
        View {
            cable: crate::setup::Cable::Ready,
            cfg: Config::default(),
            routes: [None, None],
            outputs: vec![],
            running: true,
        }
    }
    #[test]
    fn watch_targets_match_their_drawn_bounds_on_every_page() {
        let mut menu = Menu::default();
        menu.names = (1..=7).map(|i| format!("Mic {i}")).collect();
        for lang in [Lang::Ko, Lang::En, Lang::Ja, Lang::Zh] {
            for screen in [
                Screen::Live,
                Screen::Settings,
                Screen::Placement,
                Screen::Captions,
                Screen::Microphones,
                Screen::Interface,
                Screen::Voice { page: 0 },
                Screen::Languages {
                    inbound: true,
                    page: 0,
                },
            ] {
                menu.screen = screen;
                let layout = Layout::build(&menu, &view(), lang);
                for part in &layout.parts {
                    if let Part::Surface {
                        rect: b,
                        command: Some(cmd),
                        ..
                    } = part
                    {
                        let hit =
                            |x: f32, y: f32| layout.hit(x / WIDTH as f32, 1. - y / HEIGHT as f32);
                        assert_eq!(
                            hit((b.x + b.w / 2) as f32, (b.y + b.h / 2) as f32),
                            Some(cmd)
                        );
                        assert_ne!(hit(b.x as f32 + 0.1, b.y as f32 + 0.1), Some(cmd));
                        assert!(b.x >= 0 && b.y >= 0 && b.x + b.w <= WIDTH && b.y + b.h <= HEIGHT);
                    }
                }
                assert!(layout.hit(0.5, 0.5 / HEIGHT as f32).is_none());
            }
        }
    }
    #[test]
    fn both_panels_route_volume_and_paged_monitor_choices() {
        let mut v = view();
        v.outputs = (1..=6).map(|i| format!("Headset {i}")).collect();
        for dashboard in [false, true] {
            let mut menu = Menu::default();
            menu.press(Command::Open(Screen::Voice { page: 0 }), &v, dashboard);
            let target = |m: &Menu, x: f32, y: f32| {
                if dashboard {
                    super::super::dashboard::layout(m, &v, Lang::Ko)
                        .target_pixel(x + 232., y + 29.)
                        .map(|(_, cmd)| cmd.clone())
                } else {
                    Layout::build(m, &v, Lang::Ko)
                        .target_pixel(x, y)
                        .map(|(_, cmd)| cmd.clone())
                }
            };
            for (x, step) in [(430., -10), (660., 10)] {
                let cmd = target(&menu, x, 120.).unwrap();
                assert!(
                    matches!(menu.press(cmd, &v, dashboard), Some(Action::VoiceVolume(n)) if n == step)
                );
            }
            let cmd = target(&menu, 665., 180.).unwrap();
            menu.press(cmd, &v, dashboard);
            let cmd = target(&menu, 150., 276.).unwrap();
            assert!(
                matches!(menu.press(cmd, &v, dashboard), Some(Action::VoiceMonitor(n)) if n == "Headset 4")
            );
            let cmd = target(&menu, 150., 232.).unwrap();
            assert!(
                matches!(menu.press(cmd, &v, dashboard), Some(Action::VoiceMonitor(n)) if n.is_empty())
            );
        }
    }
    #[test]
    fn console_mode_target_picker_and_settings_dispatch() {
        let mut menu = Menu::default();
        let mut v = view();
        v.cfg.data["provider"] = "openai".into();
        let layout = Layout::build(&menu, &v, Lang::Ko);
        let at = |x: f32, y: f32| {
            layout
                .hit(x / WIDTH as f32, 1. - y / HEIGHT as f32)
                .cloned()
        };
        assert!(at(360., 206.).is_none());
        assert_eq!(at(120., 206.), Some(Command::Lane(0)));
        assert_eq!(at(480., 206.), Some(Command::Lane(1)));
        assert!(matches!(
            menu.press(at(568., 125.).unwrap(), &v, false),
            Some(Action::TextOnly(true))
        ));
        assert!(matches!(
            menu.press(at(604., 47.).unwrap(), &v, false),
            Some(Action::StartSession)
        ));
        menu.press(at(550., 292.).unwrap(), &v, false);
        assert!(matches!(
            menu.screen,
            Screen::Languages { inbound: true, .. }
        ));
        let list = available(&v, true);
        assert!(
            list.iter()
                .all(|code| crate::provider::supports_target("openai", code))
        );
        let picker = Layout::build(&menu, &v, Lang::Ko);
        let cmd = picker
            .hit(100. / WIDTH as f32, 1. - 185. / HEIGHT as f32)
            .unwrap()
            .clone();
        assert!(
            matches!(menu.press(cmd,&v,false),Some(Action::SubtitleLanguage(code)) if code==list[0])
        );
        assert_eq!(menu.screen, Screen::Live);
        menu.press(Command::Open(Screen::Placement), &v, false);
        menu.press(Command::Edit(Panel::Captions), &v, false);
        assert_eq!(menu.edit, Some(Panel::Captions));
        menu.press(Command::Open(Screen::Settings), &v, false);
        assert!(menu.edit.is_none());
        assert!(matches!(
            menu.press(Command::Interface("ja"), &v, false),
            Some(Action::Interface("ja"))
        ));
        assert!(matches!(
            menu.press(Command::SpeakerContext, &v, false),
            Some(Action::SpeakerContext)
        ));
    }
}
