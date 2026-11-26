#[derive(Clone, Copy, PartialEq)]
pub enum Lang {
    Ko,
    En,
    Ja,
    Zh,
}
impl Lang {
    pub fn voice_output(self) -> &'static str {
        self.t(
            "번역 음성 출력",
            "Voice output",
            "翻訳音声出力",
            "翻译语音输出",
        )
    }
    pub fn voice_volume(self) -> &'static str {
        self.t("번역 음량", "Voice volume", "翻訳音量", "翻译音量")
    }
    pub fn monitor(self) -> &'static str {
        self.t(
            "내 번역 듣기",
            "Hear my translation",
            "自分の翻訳を聴く",
            "监听我的译音",
        )
    }
    pub fn monitor_off(self) -> &'static str {
        self.t("끄기", "Off", "オフ", "关闭")
    }
    pub fn text_only_audio_hint(self) -> &'static str {
        self.t(
            "번역 음성 모드에서 재생됩니다.",
            "Playback resumes in translated voice mode.",
            "翻訳音声モードで再生します。",
            "切换到翻译语音模式后播放。",
        )
    }
    pub fn new(code: &str) -> Self {
        match code {
            "ko" => Self::Ko,
            "ja" => Self::Ja,
            "zh" => Self::Zh,
            _ => Self::En,
        }
    }
    pub fn t<'a>(self, ko: &'a str, en: &'a str, ja: &'a str, zh: &'a str) -> &'a str {
        match self {
            Self::Ko => ko,
            Self::En => en,
            Self::Ja => ja,
            Self::Zh => zh,
        }
    }
    pub fn step(self, i: usize) -> &'static str {
        match i {
            0 => self.t("대상 앱", "Application", "対象アプリ", "目标应用"),
            1 => self.t(
                "번역 엔진",
                "Translation engine",
                "翻訳エンジン",
                "翻译引擎",
            ),
            _ => self.t("오디오 장치", "Audio devices", "音声デバイス", "音频设备"),
        }
    }
    pub fn speech_filter(self) -> &'static str {
        self.t(
            "배경음 줄이기",
            "Reduce background audio",
            "背景音を抑える",
            "抑制背景音",
        )
    }
    pub fn original_audio(self) -> &'static str {
        self.t(
            "원음 (기본)",
            "Original audio (default)",
            "原音（標準）",
            "原声（默认）",
        )
    }
    pub fn out(self) -> &'static str {
        self.t(
            "내 말 번역",
            "Translate my voice",
            "自分の声を翻訳",
            "翻译我的语音",
        )
    }
    pub fn incoming(self) -> &'static str {
        self.t(
            "상대방 자막",
            "Caption other speakers",
            "相手の字幕",
            "对方字幕",
        )
    }
    pub fn key_needed(self) -> &'static str {
        self.t(
            "선택한 엔진의 API 키를 입력하세요.",
            "Enter an API key for the selected engine.",
            "選択したエンジンの API キーを入力してください。",
            "请输入所选引擎的 API 密钥。",
        )
    }
    pub fn start(self) -> &'static str {
        self.t("통역 시작", "Start translation", "通訳を開始", "开始翻译")
    }
    pub fn input(self) -> &'static str {
        self.t("내 마이크", "My microphone", "自分のマイク", "我的麦克风")
    }
    pub fn output(self) -> &'static str {
        self.t(
            "번역 음성 출력",
            "Translated voice output",
            "翻訳音声の出力先",
            "翻译语音输出",
        )
    }
    pub fn auto_mic(self) -> &'static str {
        self.t(
            "자동 마이크 선택",
            "Automatic microphone",
            "マイクを自動選択",
            "自动选择麦克风",
        )
    }
    pub fn default_device(self) -> &'static str {
        self.t(
            "시스템 기본 장치",
            "System default",
            "システム既定",
            "系统默认",
        )
    }
}
pub fn engine_name(p: &str) -> &'static str {
    match p {
        "soniox" => "Soniox",
        "gemini" => "Gemini 3.5 Live Translate",
        "openai" => "GPT Realtime (OpenAI)",
        _ => "Qwen LiveTranslate",
    }
}
pub fn lang_name(s: &str) -> &str {
    match s {
        "ko" => "한국어",
        "en" => "English",
        "ja" => "日本語",
        "zh-Hans" => "简体中文",
        "zh-Hant" => "繁體中文",
        "zh" => "中文",
        "es" => "Español",
        "fr" => "Français",
        "de" => "Deutsch",
        "ru" => "Русский",
        "pt-BR" => "Português (BR)",
        "it" => "Italiano",
        "yue" => "粵語",
        _ => LANGUAGES.iter().find(|v| v.0 == s).map_or(s, |v| v.1),
    }
}
include!("languages.rs");
