use crate::{config::Config, osc::Action, platform};
use eframe::egui::Context;
use std::{
    ptr,
    sync::mpsc::{self, Sender},
    thread,
};
use windows_sys::Win32::{
    System::Threading::GetCurrentThreadId,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

fn chord(s: &str) -> Option<(u32, u32)> {
    let mut mods = MOD_NOREPEAT;
    let mut key = None;
    for token in s.split('+').map(|s| s.trim().to_ascii_lowercase()) {
        match token.as_str() {
            "alt" | "option" => mods |= MOD_ALT,
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "shift" => mods |= MOD_SHIFT,
            "win" | "windows" | "meta" | "cmd" => mods |= MOD_WIN,
            t => {
                if key.is_some() {
                    return None;
                }
                key = Some(match t {
                    "space" => 32,
                    "tab" => 9,
                    "enter" | "return" => 13,
                    "esc" | "escape" => 27,
                    "backspace" => 8,
                    "delete" | "del" => 46,
                    "home" => 36,
                    "end" => 35,
                    "up" => 38,
                    "down" => 40,
                    "left" => 37,
                    "right" => 39,
                    _ if t.len() == 1 && t.as_bytes()[0].is_ascii_alphanumeric() => {
                        t.as_bytes()[0].to_ascii_uppercase() as u32
                    }
                    _ if t.starts_with('f') => {
                        111 + t[1..]
                            .parse::<u32>()
                            .ok()
                            .filter(|v| (1..=24).contains(v))?
                    }
                    _ => return None,
                });
            }
        }
    }
    Some((mods, key?))
}
pub struct Hotkeys {
    thread_id: u32,
    worker: Option<thread::JoinHandle<()>>,
}
impl Hotkeys {
    pub fn start(c: &Config, tx: Sender<Action>, ctx: Context) -> Option<Self> {
        if !c.b("/hotkeys/enabled") {
            return None;
        }
        let bindings = [
            "/hotkeys/translation_toggle",
            "/hotkeys/subtitles_toggle",
            "/hotkeys/translation_hold",
        ]
        .map(|p| chord(c.s(p)));
        let in_vr = c.b("/hotkeys/enabled_in_vr");
        let (id_tx, id_rx) = mpsc::channel();
        let worker = thread::spawn(move || unsafe {
            let mut msg = std::mem::zeroed();
            PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_NOREMOVE);
            for (i, b) in bindings.iter().enumerate() {
                if let Some((m, k)) = b {
                    if RegisterHotKey(ptr::null_mut(), i as i32 + 1, *m, *k) == 0 {
                        let _ =
                            tx.send(Action::Warning(format!("Hotkey unavailable ({}).", i + 1)));
                        ctx.request_repaint();
                    }
                }
            }
            let _ = id_tx.send(GetCurrentThreadId());
            loop {
                if GetMessageW(&mut msg, ptr::null_mut(), 0, 0) <= 0 {
                    break;
                }
                let a = if msg.message == WM_HOTKEY {
                    if !in_vr && platform::process_id("vrserver").is_some() {
                        continue;
                    }
                    match msg.wParam {
                        1 => Action::ToggleTranslation,
                        2 => Action::ToggleSubtitles,
                        3 => {
                            SetTimer(ptr::null_mut(), 3, 30, None);
                            Action::Hold(true)
                        }
                        _ => continue,
                    }
                } else if msg.message == WM_TIMER
                    && bindings[2].is_some_and(|(_, key)| GetAsyncKeyState(key as i32) >= 0)
                {
                    KillTimer(ptr::null_mut(), msg.wParam);
                    Action::Hold(false)
                } else {
                    continue;
                };
                let _ = tx.send(a);
                ctx.request_repaint();
            }
            for id in 1..=3 {
                UnregisterHotKey(ptr::null_mut(), id);
            }
        });
        Some(Self {
            thread_id: id_rx.recv().ok()?,
            worker: Some(worker),
        })
    }
}
impl Drop for Hotkeys {
    fn drop(&mut self) {
        unsafe {
            PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
