use super::{DisplayChange, Panel, View, wrist::Screen};
use crate::{
    audio::mic_search::{Found, Search},
    osc::Action,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Open(Screen),
    TextOnly(bool),
    Volume(i32),
    Monitor(String),
    Session,
    Target(bool, String),
    Interface(&'static str),
    SpeakerContext,
    SpeechFilter(bool),
    Nemotron(bool),
    Lane(usize),
    Original,
    Font(i32),
    Scale(i32),
    Edit(Panel),
    Reset(Panel),
    List,
    Listen,
    Pick(String),
    More(i32),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MicStatus {
    Ready,
    Loading,
    Listening,
    NoSpeech,
    Unavailable,
}
pub struct Menu {
    pub dashboard: Screen,
    pub screen: Screen,
    pub edit: Option<Panel>,
    pub dirty: bool,
    pub(super) names: Vec<String>,
    pub(super) mic_page: usize,
    pub(super) mic: MicStatus,
    pub(super) detected: Option<String>,
    search: Option<Search>,
}
impl Default for Menu {
    fn default() -> Self {
        Self {
            dashboard: Screen::Live,
            screen: Screen::Live,
            edit: None,
            dirty: true,
            names: vec![],
            mic_page: 0,
            mic: MicStatus::Ready,
            detected: None,
            search: None,
        }
    }
}
impl Menu {
    pub fn press(&mut self, cmd: Command, view: &View, dashboard: bool) -> Option<Action> {
        self.dirty = true;
        match cmd {
            Command::Open(screen) => {
                let current = if dashboard {
                    &mut self.dashboard
                } else {
                    &mut self.screen
                };
                if *current != screen {
                    *current = screen;
                    if self.screen != Screen::Microphones && self.dashboard != Screen::Microphones {
                        self.search = None;
                        self.mic = MicStatus::Ready;
                    }
                    if screen != Screen::Placement {
                        self.edit = None;
                    }
                    if screen == Screen::Microphones && self.search.is_none() {
                        self.load(false);
                    }
                }
            }
            Command::TextOnly(false) if view.cable != crate::setup::Cable::Ready => {
                return self.press(Command::Open(Screen::Voice { page: 0 }), view, dashboard);
            }
            Command::TextOnly(on) => return Some(Action::TextOnly(on)),
            Command::Volume(step) => return Some(Action::VoiceVolume(step)),
            Command::Monitor(name) => return Some(Action::VoiceMonitor(name)),
            Command::Session => return Some(Action::StartSession),
            Command::Interface(code) => return Some(Action::Interface(code)),
            Command::SpeakerContext => return Some(Action::SpeakerContext),
            Command::Nemotron(inbound) => return Some(Action::Nemotron(inbound)),
            Command::SpeechFilter(inbound) => return Some(Action::SpeechFilter(inbound)),
            Command::Target(inbound, code) => {
                if crate::provider::supports_target(view.cfg.engine(inbound), &code) {
                    self.press(Command::Open(Screen::Live), view, dashboard);
                    return Some(if inbound {
                        Action::SubtitleLanguage(code)
                    } else {
                        Action::Language(code)
                    });
                }
            }
            Command::Edit(panel) => {
                self.edit = (self.edit != Some(panel)).then_some(panel);
            }
            Command::Reset(_) => {}
            Command::List => self.load(false),
            Command::Listen => {
                if self.mic == MicStatus::Listening {
                    self.search = None;
                    self.mic = MicStatus::Ready;
                } else {
                    self.load(true);
                }
            }
            Command::Pick(id) => {
                self.search = None;
                self.mic = MicStatus::Ready;
                self.detected = None;
                return Some(Action::Microphone(id));
            }
            Command::More(step) => {
                self.mic_page = self
                    .mic_page
                    .saturating_add_signed(step as isize)
                    .min(self.names.len().saturating_sub(1) / 4);
            }
            Command::Lane(0) => return Some(Action::ToggleTranslation),
            Command::Lane(1) => return Some(Action::ToggleSubtitles),
            Command::Lane(_) => {}
            Command::Original => return Some(Action::CaptionDisplay(DisplayChange::Original)),
            Command::Font(step) => return Some(Action::CaptionDisplay(DisplayChange::Font(step))),
            Command::Scale(step) => {
                return Some(Action::CaptionDisplay(DisplayChange::Scale(step)));
            }
        }
        None
    }
    fn load(&mut self, listen: bool) {
        self.detected = None;
        self.search = Some(Search::start(listen));
        self.mic = if listen {
            MicStatus::Listening
        } else {
            MicStatus::Loading
        };
    }
    pub fn poll(&mut self) -> Option<Action> {
        use std::sync::mpsc::TryRecvError;
        let result = match self.search.as_ref()?.result.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("scan stopped")),
        };
        self.search = None;
        self.finish_search(result)
    }
    fn finish_search(&mut self, result: anyhow::Result<Found>) -> Option<Action> {
        self.dirty = true;
        self.mic = MicStatus::Ready;
        match result {
            Ok(Found::Devices(names)) => {
                self.names = names;
                self.mic_page = 0;
            }
            Ok(Found::Speaking(Some(name))) => {
                let i = self
                    .names
                    .iter()
                    .position(|n| n == &name)
                    .unwrap_or_else(|| {
                        self.names.push(name.clone());
                        self.names.len() - 1
                    });
                self.mic_page = i / 4;
                self.detected = Some(name.clone());
                return Some(Action::Microphone(name));
            }
            Ok(Found::Speaking(None)) => self.mic = MicStatus::NoSpeech,
            Err(_) => self.mic = MicStatus::Unavailable,
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_voice_mode_opens_help_inside_each_overlay() {
        use crate::setup::Cable;
        let mut view = View {
            cfg: Default::default(),
            outputs: vec![],
            routes: [None, None],
            running: true,
            cable: Cable::Missing,
        };
        let mut menu = Menu::default();
        for state in [Cable::Missing, Cable::Inactive, Cable::Unknown] {
            view.cable = state;
            for dashboard in [false, true] {
                menu.press(Command::Open(Screen::Live), &view, dashboard);
                assert!(
                    menu.press(Command::TextOnly(false), &view, dashboard)
                        .is_none()
                );
                assert_eq!(
                    if dashboard {
                        menu.dashboard
                    } else {
                        menu.screen
                    },
                    Screen::Voice { page: 0 }
                );
                assert!(matches!(
                    menu.press(Command::TextOnly(true), &view, dashboard),
                    Some(Action::TextOnly(true))
                ));
            }
        }
        view.cable = Cable::Ready;
        assert!(matches!(
            menu.press(Command::TextOnly(false), &view, false),
            Some(Action::TextOnly(false))
        ));
    }
    #[test]
    fn microphone_picker_routes_manual_default_and_detected_inputs() {
        let view = View {
            cable: crate::setup::Cable::Ready,
            cfg: Default::default(),
            running: true,
            routes: [None, None],
            outputs: vec![],
        };
        let mut menu = Menu {
            screen: Screen::Microphones,
            ..Default::default()
        };
        let names = (1..=6).map(|i| format!("Headset {i}")).collect();
        assert!(menu.finish_search(Ok(Found::Devices(names))).is_none());
        assert_eq!(menu.names.len(), 6);
        menu.press(Command::More(1), &view, false);
        assert_eq!(menu.mic_page, 1);
        let pick = Command::Pick(menu.names[5].clone());
        assert!(
            matches!(menu.press(pick,&view,false),Some(Action::Microphone(n)) if n=="Headset 6")
        );
        assert!(
            matches!(menu.finish_search(Ok(Found::Speaking(Some("New microphone".into())))),Some(Action::Microphone(n)) if n=="New microphone")
        );
        assert_eq!(menu.detected.as_deref(), Some("New microphone"));
        assert!(menu.finish_search(Ok(Found::Speaking(None))).is_none());
        assert!(
            matches!(menu.press(Command::Pick(String::new()),&view,false),Some(Action::Microphone(n)) if n.is_empty())
        );
    }
}
