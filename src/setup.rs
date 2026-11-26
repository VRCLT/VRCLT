use crate::platform;
use anyhow::{Result, ensure};
use std::io::ErrorKind;
use winreg::{
    RegKey, RegValue,
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_BINARY, REG_DWORD},
};

const VRCHAT: &str = r"Software\VRChat\VRChat";
fn name(key: &RegKey, field: &str) -> String {
    let prefix = format!("{field}_h");
    key.enum_values()
        .flatten()
        .map(|(n, _)| n)
        .find(|n| n.starts_with(&prefix))
        .unwrap_or_else(|| {
            let hash = field
                .bytes()
                .fold(5381u32, |a, b| a.wrapping_mul(33) ^ u32::from(b));
            format!("{prefix}{hash}")
        })
}

// This action opens VB-Audio's own signed installer after the user chooses it.
pub async fn install_cable() -> Result<()> {
    use std::{
        env, fs,
        io::{Cursor, Read},
        os::windows::process::CommandExt,
        path::PathBuf,
        process::Command,
        time::Duration,
    };
    let mut response = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()?
        .get("https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip")
        .send()
        .await?
        .error_for_status()?;
    let mut pack = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            pack.len() + chunk.len() <= 20 * 1024 * 1024,
            "VB-CABLE download too large"
        );
        pack.extend(chunk);
    }
    tokio::task::spawn_blocking(move||->Result<()>{
        let dir=tempfile::tempdir()?;let mut archive=zip::ZipArchive::new(Cursor::new(pack))?;let mut left=80u64*1024*1024;
        for i in 0..archive.len(){let member=archive.by_index(i)?;let relative=member.enclosed_name().ok_or_else(||anyhow::anyhow!("Invalid driver archive path"))?;let path=dir.path().join(relative);
            if member.is_dir(){fs::create_dir_all(&path)?;continue;}
            ensure!(member.size()<=left,"Driver archive too large");if let Some(parent)=path.parent(){fs::create_dir_all(parent)?;}
            let mut f=fs::File::create(&path)?;let n=std::io::copy(&mut member.take(left+1),&mut f)?;ensure!(n<=left,"Driver archive too large");left-=n;
        }
        let exe=dir.path().join("VBCABLE_Setup_x64.exe");ensure!(exe.is_file(),"VB-CABLE installer missing");
        let ps=PathBuf::from(env::var_os("SystemRoot").unwrap_or_else(||r"C:\Windows".into())).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let status=Command::new(ps).creation_flags(0x08000000).env("VRCLT_CABLE_SETUP",&exe)
            .args(["-NoProfile","-NonInteractive","-Command",r#"$ErrorActionPreference='Stop'
$s = Get-AuthenticodeSignature -LiteralPath $env:VRCLT_CABLE_SETUP
if ($s.Status -ne 'Valid' -or $s.SignerCertificate.Subject -notmatch 'BUREL VINCENT') { exit 91 }
$p = Start-Process -FilePath $env:VRCLT_CABLE_SETUP -WorkingDirectory (Split-Path -LiteralPath $env:VRCLT_CABLE_SETUP) -Verb RunAs -PassThru -Wait
exit $p.ExitCode"#]).status()?;
        ensure!(status.code()!=Some(91),"VB-Audio signature verification failed");
        ensure!(status.success()||status.code()==Some(3010),"Installer closed with status {status}");Ok(())
    }).await?
}
pub struct Changes(Vec<(String, Option<RegValue>, RegValue)>);
impl Changes {
    pub fn apply(mic: Option<&str>, osc: bool) -> Result<Self> {
        ensure!(
            platform::process_id("VRChat.exe").is_none(),
            "Close VRChat first"
        );
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(VRCHAT, KEY_READ | KEY_SET_VALUE)?;
        let mut plan = vec![];
        if let Some(mic) = mic {
            let text = mic.replacen('(', " (", 1).replace("  (", " (");
            for field in [
                "VRC_INPUT_MIC_DEVICE_NAME_VR",
                "VRC_INPUT_MIC_DEVICE_NAME_Desktop",
            ] {
                plan.push((
                    field,
                    RegValue {
                        bytes: format!("{text}\0").into_bytes(),
                        vtype: REG_BINARY,
                    },
                ));
            }
        }
        if osc {
            plan.push((
                "VRC_INPUT_OSC",
                RegValue {
                    bytes: 1u32.to_le_bytes().to_vec(),
                    vtype: REG_DWORD,
                },
            ));
        }
        let mut pending = vec![];
        for (field, value) in plan {
            let id = name(&key, field);
            let old = match key.get_raw_value(&id) {
                Ok(v) => Some(v),
                Err(e) if e.kind() == ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            pending.push((id, old, value));
        }
        let mut changed = Self(vec![]);
        for item in pending {
            if let Err(e) = key.set_raw_value(&item.0, &item.2) {
                changed.restore_key(&key)?;
                return Err(e.into());
            }
            changed.0.push(item);
        }
        Ok(changed)
    }
    fn restore_key(&self, key: &RegKey) -> Result<()> {
        for (id, previous, ours) in self.0.iter().rev() {
            if key
                .get_raw_value(id)
                .is_ok_and(|v| v.bytes == ours.bytes && v.vtype == ours.vtype)
            {
                if let Some(value) = previous {
                    key.set_raw_value(id, value)?;
                } else {
                    key.delete_value(id)?;
                }
            }
        }
        Ok(())
    }
    pub fn undo(&self) -> Result<()> {
        ensure!(
            platform::process_id("VRChat.exe").is_none(),
            "Close VRChat first"
        );
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(VRCHAT, KEY_READ | KEY_SET_VALUE)?;
        self.restore_key(&key)
    }
}
#[derive(Default)]
pub struct Environment {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub cable_mics: Vec<String>,
    pub cable_speakers: Vec<String>,
    pub default_input: String,
    pub default_output: String,
    pub game: bool,
    pub game_open: bool,
    pub game_mics: [Option<String>; 2],
    pub osc: Option<bool>,
    pub driver: bool,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cable {
    Unknown,
    Missing,
    Inactive,
    Ready,
}
impl Cable {
    pub fn text_only(self, cfg: &crate::config::Config) -> bool {
        self != Self::Ready || cfg.b("/outbound/text_only")
    }
    pub fn voice_label(self, l: crate::labels::Lang) -> &'static str {
        if self == Self::Ready {
            l.t("번역 음성", "Translated voice", "翻訳音声", "翻译语音")
        } else {
            l.t(
                "번역 음성 안내",
                "Voice setup",
                "翻訳音声の設定",
                "语音模式设置",
            )
        }
    }
    pub fn guidance(self, l: crate::labels::Lang) -> &'static str {
        match self {
            Self::Missing => l.t(
                "음성 모드를 사용하려면 VB-Audio Cable을 설치하세요.",
                "Install VB-Audio Cable to use voice mode.",
                "音声モードを使うには VB-Audio Cable をインストールしてください。",
                "请安装 VB-Audio Cable 以使用语音模式。",
            ),
            Self::Inactive => l.t(
                "소리 설정에서 CABLE Input과 CABLE Output을 모두 활성화하세요.",
                "Enable both CABLE Input and CABLE Output in Sound settings.",
                "サウンド設定で CABLE Input と CABLE Output を有効にしてください。",
                "请在声音设置中启用 CABLE Input 和 CABLE Output。",
            ),
            Self::Unknown => l.t(
                "음성 모드를 사용하려면 먼저 오디오 장치를 확인하세요.",
                "Check audio devices to use voice mode.",
                "音声モードを使うにはオーディオ機器を確認してください。",
                "请先检查音频设备以使用语音模式。",
            ),
            Self::Ready => l.t(
                "음성 모드를 사용할 수 있습니다.",
                "Voice mode is available.",
                "音声モードを使用できます。",
                "语音模式已可用。",
            ),
        }
    }
}
pub fn virtual_device(name: &str) -> bool {
    let name = name.to_lowercase();
    [
        "cable",
        "vb-audio",
        "voicemeeter",
        "blackhole",
        "steam streaming",
        "stereo mix",
        "스테레오 믹스",
        "what u hear",
    ]
    .iter()
    .any(|x| name.contains(x))
}
pub fn vb_cable_name(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    [
        "vb-audio virtual cable",
        "vb-audio cable",
        "vb-audio hi-fi cable",
        "vb-audio hifi cable",
    ]
    .iter()
    .any(|s| n.contains(s))
        || [
            "cable input",
            "cable output",
            "cable in 16ch",
            "hi-fi cable input",
            "hi-fi cable output",
        ]
        .iter()
        .any(|s| n.starts_with(s))
        || ["cable-a ", "cable-b ", "cable-c ", "cable-d "]
            .iter()
            .any(|prefix| {
                n.strip_prefix(prefix)
                    .is_some_and(|s| s.starts_with("input") || s.starts_with("output"))
            })
}
pub fn selected_output<'a>(outputs: &'a [String], saved: &'a str) -> &'a str {
    outputs
        .iter()
        .find(|n| n.as_str() == saved)
        .or_else(|| outputs.iter().find(|n| matches_device(n, saved)))
        .map_or(saved, String::as_str)
}
pub fn matches_device(actual: &str, requested: &str) -> bool {
    let norm = |s: &str| s.trim().to_lowercase().replace(" (", "(");
    !actual.is_empty() && !requested.is_empty() && norm(actual).contains(&norm(requested))
}
impl Environment {
    pub fn cable(&self) -> Cable {
        match (self.cable_in(), self.cable_out()) {
            (Some(_), Some(_)) => Cable::Ready,
            (None, None) if !self.driver => Cable::Missing,
            _ => Cable::Inactive,
        }
    }
    pub fn read(
        inputs: Vec<String>,
        outputs: Vec<String>,
        cable_mics: Vec<String>,
        cable_speakers: Vec<String>,
        default_input: String,
        default_output: String,
    ) -> Self {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(VRCHAT).ok();
        let game_mic = |field| {
            let key = key.as_ref()?;
            let id = name(key, field);
            if let Ok(value) = key.get_value::<String, _>(&id) {
                return Some(value.trim_end_matches('\0').to_owned());
            }
            Some(
                String::from_utf8_lossy(&key.get_raw_value(&id).ok()?.bytes)
                    .trim_end_matches('\0')
                    .to_owned(),
            )
        };
        Self {
            inputs,
            outputs,
            cable_mics,
            cable_speakers,
            default_input,
            default_output,
            game: key.is_some(),
            game_open: platform::process_id("VRChat.exe").is_some(),
            game_mics: [
                game_mic("VRC_INPUT_MIC_DEVICE_NAME_VR"),
                game_mic("VRC_INPUT_MIC_DEVICE_NAME_Desktop"),
            ],
            osc: key
                .as_ref()
                .and_then(|k| k.get_value::<u32, _>(name(k, "VRC_INPUT_OSC")).ok())
                .map(|v| v != 0),
            driver: RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
                .open_subkey(r"SYSTEM\CurrentControlSet\Services\VBAudioVACMME")
                .is_ok(),
        }
    }
    pub fn cable_in(&self) -> Option<&str> {
        self.cable_speakers
            .iter()
            .find(|n| matches_device(n, "CABLE Input"))
            .or_else(|| self.cable_speakers.first())
            .map(String::as_str)
    }
    pub fn passthrough_output(&self, saved: &str) -> Option<&str> {
        if self.cable() != Cable::Ready {
            return None;
        }
        self.cable_speakers
            .iter()
            .find(|name| name.as_str() == saved)
            .or_else(|| {
                self.cable_speakers
                    .iter()
                    .find(|name| matches_device(name, saved))
            })
            .map(String::as_str)
            .or_else(|| self.cable_in())
    }
    pub fn cable_out(&self) -> Option<&str> {
        self.cable_mics
            .iter()
            .find(|n| matches_device(n, "CABLE Output"))
            .or_else(|| self.cable_mics.first())
            .map(String::as_str)
    }
    pub fn was_cable_mic(&self, saved: &str) -> bool {
        vb_cable_name(saved)
            || (self
                .cable_mics
                .iter()
                .any(|name| matches_device(name, saved))
                && !self.inputs.iter().any(|name| matches_device(name, saved)))
    }
    pub fn cable_output_selected(&self, name: &str) -> bool {
        self.cable_speakers
            .iter()
            .any(|device| matches_device(device, name))
    }
    pub fn default_mic(&self) -> &str {
        if self.default_input.is_empty() || self.was_cable_mic(&self.default_input) {
            self.inputs.first().map_or("", String::as_str)
        } else {
            &self.default_input
        }
    }
    pub fn current<'a>(&'a self, dir_in: bool, want: &str) -> &'a str {
        let (devices, default) = if dir_in {
            (&self.inputs, self.default_mic())
        } else {
            (&self.outputs, self.default_output.as_str())
        };
        if want.is_empty() {
            default
        } else {
            devices
                .iter()
                .find(|n| matches_device(n, want))
                .map(String::as_str)
                .unwrap_or("")
        }
    }
    pub fn game_mics_ready(&self) -> bool {
        self.game_mics.iter().any(Option::is_some)
            && self.game_mics.iter().flatten().all(|name| {
                self.was_cable_mic(if name.is_empty() {
                    &self.default_input
                } else {
                    name
                })
            })
    }
    pub fn needs_osc(&self, c: &crate::config::Config) -> bool {
        c.s("/app/mode") == "vrchat"
            && (c.b("/outbound/chatbox")
                || (c.b("/outbound/enabled") && self.cable().text_only(c))
                || c.b("/control/enabled"))
    }
    pub fn plan(&self, c: &crate::config::Config) -> Plan {
        let mut plan = Plan::default();
        if c.b("/outbound/enabled") || self.cable() == Cable::Ready {
            let mic = self.current(true, c.s("/outbound/mic_device"));
            let real: Vec<_> = self.inputs.iter().filter(|n| !virtual_device(n)).collect();
            let preferred = self
                .game_mics
                .iter()
                .flatten()
                .filter(|n| !virtual_device(n))
                .find_map(|v| real.iter().find(|n| matches_device(n, v)).copied());
            if mic.is_empty()
                || virtual_device(mic)
                || (c.s("/outbound/mic_device").is_empty()
                    && self.was_cable_mic(&self.default_input))
                || (c.s("/outbound/mic_device").is_empty() && preferred.is_some_and(|n| n != mic))
            {
                plan.mic = preferred
                    .or_else(|| {
                        real.iter()
                            .find(|n| n.as_str() == self.default_input)
                            .copied()
                    })
                    .or_else(|| real.first().copied())
                    .cloned();
            }
            if self.cable() == Cable::Ready
                && c.translation_active(false)
                && !c.b("/outbound/text_only")
                && !self.cable_output_selected(self.current(false, c.s("/outbound/tts_device")))
            {
                plan.out = self.cable_in().map(String::from);
            }
            if self.game
                && self.cable() == Cable::Ready
                && !self.game_open
                && c.s("/app/mode") == "vrchat"
                && !self.game_mics_ready()
            {
                plan.game_mic = self.cable_out().map(String::from);
            }
        }
        plan.osc = self.game && !self.game_open && self.needs_osc(c) && self.osc != Some(true);
        plan
    }
}
#[derive(Default, Clone, Eq, PartialEq)]
pub struct Plan {
    pub mic: Option<String>,
    pub out: Option<String>,
    pub game_mic: Option<String>,
    pub osc: bool,
}
impl Plan {
    pub fn empty(&self) -> bool {
        self.mic.is_none() && self.out.is_none() && self.game_mic.is_none() && !self.osc
    }
}
pub struct Undo {
    pub registry: Option<Changes>,
    pub local: Vec<(&'static str, String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cable_mics_are_excluded_without_hiding_unrelated_microphones() {
        for name in [
            "CABLE Output (VB-Audio Virtual Cable)",
            "CABLE-A Output",
            "마이크 (VB-Audio Virtual Cable)",
            "Hi-Fi CABLE Output",
        ] {
            assert!(vb_cable_name(name), "{name}");
        }
        for name in [
            "Headset Microphone",
            "Microphone (NVIDIA Broadcast)",
            "Voicemeeter Output (VB-Audio VoiceMeeter VAIO)",
            "USB Cable Headset",
            "USB Cable Input Mic",
        ] {
            assert!(!vb_cable_name(name), "{name}");
        }
        let env = Environment {
            inputs: vec!["My USB mic".into()],
            default_input: "Renamed cable".into(),
            cable_mics: vec!["Renamed cable".into()],
            ..Default::default()
        };
        assert!(env.was_cable_mic("Renamed cable"));
        assert_eq!(env.current(true, ""), "My USB mic");
    }
    #[test]
    fn text_routing_needs_no_cable_and_preserves_the_saved_choice() {
        let mut c = crate::config::Config::default();
        c.data["outbound"]["text_only"] = false.into();
        c.data["outbound"]["chatbox"] = false.into();
        c.data["control"]["enabled"] = false.into();
        let mut env = Environment {
            game: true,
            ..Default::default()
        };
        assert_eq!(env.cable(), Cable::Missing);
        assert!(env.cable().text_only(&c) && env.plan(&c).osc);
        env.cable_speakers
            .push("CABLE Input (VB-Audio Virtual Cable)".into());
        assert_eq!(env.cable(), Cable::Inactive);
        assert!(env.plan(&c).out.is_none() && env.plan(&c).game_mic.is_none());
        env.cable_mics
            .push("CABLE Output (VB-Audio Virtual Cable)".into());
        assert_eq!(env.cable(), Cable::Ready);
        assert!(!env.cable().text_only(&c));
        c.data["outbound"]["text_only"] = true.into();
        assert!(env.cable().text_only(&c));
    }
    #[test]
    fn auto_setup_respects_existing_mic_and_running_game() {
        let mut env = Environment {
            inputs: vec!["Headset".into()],
            cable_mics: vec!["CABLE Output".into()],
            cable_speakers: vec!["CABLE Input".into()],
            outputs: vec!["CABLE Input".into(), "Speakers".into()],
            default_input: "CABLE Output".into(),
            default_output: "Speakers".into(),
            game: true,
            ..Default::default()
        };
        let mut cfg = crate::config::Config::default();
        cfg.data["outbound"]["tts_device"] = "Speakers".into();
        let plan = env.plan(&cfg);
        assert_eq!(plan.mic.as_deref(), Some("Headset"));
        assert_eq!(plan.out.as_deref(), Some("CABLE Input"));
        assert_eq!(plan.game_mic.as_deref(), Some("CABLE Output"));
        assert!(plan.osc);
        env.game_open = true;
        cfg.data["outbound"]["mic_device"] = "Headset".into();
        let plan = env.plan(&cfg);
        assert!(plan.mic.is_none() && plan.game_mic.is_none() && !plan.osc && plan.out.is_some());
        cfg.profile("discord");
        assert!(!env.plan(&cfg).osc);
    }
}
