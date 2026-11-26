use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    env, fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

pub const ENGINES: [&str; 4] = ["gemini", "qwen", "openai", "soniox"];

#[derive(Clone)]
pub struct Config {
    pub data: Value,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            data: serde_yaml_ng::from_str(include_str!("../config.example.yaml"))
                .expect("bundled defaults"),
        }
    }
}
fn merge(base: &mut Value, patch: Value) {
    if let (Some(a), Value::Object(b)) = (base.as_object_mut(), &patch) {
        for (key, value) in b {
            merge(a.entry(key).or_insert(Value::Null), value.clone());
        }
    } else {
        *base = patch;
    }
}
impl Config {
    pub fn voice_volume(&self) -> i32 {
        (self.n("/outbound/tts_gain", 1.).clamp(0., 2.) * 100.).round() as i32
    }
    pub fn nemotron_path(inbound: bool) -> &'static str {
        if inbound {
            "/inbound/nemotron_reuse"
        } else {
            "/outbound/nemotron_reuse"
        }
    }
    pub fn speech_filter_path(inbound: bool) -> &'static str {
        if inbound {
            "/inbound/speech_filter"
        } else {
            "/outbound/speech_filter"
        }
    }
    pub fn load(path: &Path) -> Result<Self> {
        let mut cfg = Self::default();
        let contents = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(cfg),
            Err(e) => return Err(e.into()),
        };
        let saved: Value = serde_yaml_ng::from_str(&contents).map_err(|e| {
            let line = e.location().map(|l| l.line()).unwrap_or(1);
            anyhow::anyhow!("Invalid YAML (line {line})")
        })?;
        if !saved.is_object() {
            bail!("Expected a YAML mapping");
        }
        for key in cfg.data.as_object().unwrap().keys() {
            if cfg.data[key].is_object() && saved.get(key).is_some_and(|v| !v.is_object()) {
                bail!("Invalid settings section: {key}");
            }
        }
        merge(&mut cfg.data, saved.clone());
        let mode = cfg.s("/app/mode").to_owned();
        if mode == "vrc_text" {
            cfg.data["app"]["mode"] = "vrchat".into();
            cfg.data["outbound"]["text_only"] = true.into();
        } else if mode != "vrchat" {
            let merged = cfg.clone();
            cfg.profile(&mode);
            merge(&mut cfg.data, saved);
            if !cfg.data["app"]["profiles"][&mode].is_object() {
                cfg = merged;
            }
        }
        if cfg.s("/soniox/model") == "stt-rt-v5" {
            cfg.data["soniox"]["model"] = "stt-rt-v6".into();
        }
        if let Some(lines) = cfg.data["outbound"]["glossary"].as_array() {
            let terms = lines
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            cfg.data["outbound"]["glossary"] = terms.into();
        }
        Ok(cfg)
    }
    pub fn path() -> PathBuf {
        if let Some(path) = env::var_os("VRCLT_CONFIG").filter(|s| !s.is_empty()) {
            return path.into();
        }
        PathBuf::from(env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into()))
            .join("vrclt/config.yaml")
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(dir)?;
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(serde_yaml_ng::to_string(&self.data)?.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path)
            .context("Could not replace configuration")?;
        Ok(())
    }
    pub fn profile(&mut self, mode: &str) {
        let p = self.data["app"]["profiles"][mode].clone();
        if !p.is_object() {
            return;
        }
        self.data["app"]["mode"] = mode.into();
        for (key, section, dest) in [
            ("process", "inbound", "process"),
            ("ui_mode", "ui", "mode"),
            ("voice_output", "outbound", "voice_output"),
            ("chatbox", "outbound", "chatbox"),
            (
                "passthrough_while_translating",
                "outbound",
                "passthrough_while_translating",
            ),
            ("osc_control", "control", "enabled"),
            ("vr_overlay", "overlay", "enabled"),
            ("wrist_ui", "wrist_ui", "enabled"),
        ] {
            if let Some(v) = p.get(key).filter(|v| v.as_str() != Some("")) {
                self.data[section][dest] = v.clone();
            }
        }
        if mode != "vrchat" {
            self.data["outbound"]["text_only"] = false.into();
        }
    }
    pub fn s(&self, path: &str) -> &str {
        self.data
            .pointer(path)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
    }
    pub fn b(&self, path: &str) -> bool {
        self.data
            .pointer(path)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
    pub fn n(&self, path: &str, default: f64) -> f64 {
        self.data
            .pointer(path)
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.trim().parse().ok()))
            .filter(|v| v.is_finite())
            .unwrap_or(default)
    }
    pub fn translation_active(&self, inbound: bool) -> bool {
        if inbound {
            self.b("/inbound/enabled") && self.b("/dashboard/subtitles_on")
        } else {
            self.b("/outbound/enabled") && self.b("/dashboard/translation_on")
        }
    }
    pub fn engine(&self, inbound: bool) -> &str {
        let p = self.s("/inbound/provider");
        if inbound && ENGINES.contains(&p) {
            return p;
        }
        let p = self.s("/provider");
        if ENGINES.contains(&p) { p } else { "gemini" }
    }
    pub fn speaker_ids(&self, engine: &str) -> bool {
        match engine {
            "soniox" => true,
            "qwen" => self
                .s("/qwen/model")
                .to_ascii_lowercase()
                .split_once("qwen")
                .and_then(|(_, m)| m.split_once("-livetranslate"))
                .and_then(|(version, _)| version.split_once('.'))
                .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
                .is_some_and(|version| version >= (3, 8)),
            _ => false,
        }
    }
    pub fn key_path(p: &str) -> String {
        if p == "gemini" {
            "/api_key".into()
        } else {
            format!("/{p}/api_key")
        }
    }
    pub fn key(&self, p: &str) -> String {
        let saved = self.s(&Self::key_path(p));
        if !saved.is_empty() {
            return saved.into();
        }
        env::var(match p {
            "openai" => "OPENAI_API_KEY",
            "soniox" => "SONIOX_API_KEY",
            "qwen" => "DASHSCOPE_API_KEY",
            _ => "GEMINI_API_KEY",
        })
        .unwrap_or_default()
        .trim()
        .into()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrates_and_preserves_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.yml");
        fs::write(
            &path,
            "app:\n  mode: discord\nsoniox:\n  model: stt-rt-v5\noutbound:\n  glossary: ['VR=브이알', ' 駅=역 ']\nfuture:\n  values: [1, 2]\n",
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.s("/inbound/process"), "Discord.exe");
        assert!(!cfg.b("/outbound/chatbox"));
        assert_eq!(cfg.s("/soniox/model"), "stt-rt-v6");
        assert_eq!(cfg.s("/outbound/glossary"), "VR=브이알\n駅=역");
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().data["future"]["values"][1], 2);
        fs::write(&path, "[").unwrap();
        assert!(Config::load(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "[");
    }
}
