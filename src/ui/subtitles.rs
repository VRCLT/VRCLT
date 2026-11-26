mod window;
use crate::{
    captions::{self, Row},
    config::Config,
    labels::Lang,
    platform,
    runtime::{Caption, Lane},
    theme::*,
};
use eframe::egui::*;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub(super) fn history(
    ui: &mut Ui,
    lines: &[(Instant, Row)],
    l: Lang,
    running: bool,
    size: f32,
) -> scroll_area::ScrollAreaOutput<()> {
    let space_below_title =
        ui.clip_rect().height() - (ui.next_widget_position().y - ui.max_rect().top());
    let frame = card_frame().inner_margin(Margin::symmetric(14, 8));
    let height = (space_below_title - frame.total_margin().sum().y).clamp(98., 210.);
    frame
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ScrollArea::vertical()
                .id_salt("conversation-history")
                .auto_shrink([false, false])
                .max_height(height)
                .min_scrolled_height(height)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if lines.is_empty() {
                        ui.label(bold(
                            l.t(
                                "표시할 대화가 없습니다",
                                "No conversation to display",
                                "表示する会話はありません",
                                "暂无对话",
                            ),
                            18.,
                        ));
                        hint(
                            ui,
                            if running {
                                l.t(
                                    "대화가 시작되면 원문과 번역이 여기에 표시됩니다.",
                                    "Original speech and its translation will appear here.",
                                    "会話が始まると原文と訳文が表示されます。",
                                    "对话开始后，原文和译文将显示在这里。",
                                )
                            } else {
                                l.t(
                                "통역을 시작하면 원문과 번역이 여기에 표시됩니다.",
                                "Start translation to see original speech and translations here.",
                                "通訳を開始すると原文と訳文が表示されます。",
                                "开始翻译后，这里将显示原文和译文。",
                            )
                            },
                        );
                    }
                    ui.spacing_mut().item_spacing.y = 4.;
                    for (_, row) in lines {
                        ui.add(Label::new(row.inline(size)).wrap());
                    }
                })
        })
        .inner
}

pub(super) fn font_picker(
    ui: &mut Ui,
    id: &str,
    label: &str,
    value: &mut u32,
    range: std::ops::RangeInclusive<u32>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.;
        hint(ui, label);
        let was_open = ComboBox::is_open(ui.ctx(), ui.make_persistent_id(Id::new(id)));
        ComboBox::from_id_salt(id)
            .width(70.)
            .selected_text(format!("{value} px"))
            .show_ui(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.;
                for size in range {
                    let selected = *value == size;
                    let item = ui.selectable_value(value, size, format!("{size} px"));
                    if selected && !was_open {
                        item.scroll_to_me(Some(Align::Center));
                    }
                }
            });
    });
}

pub(super) struct DesktopOverlay {
    pub editing: bool,
    path: PathBuf,
    last: Option<Preferences>,
    worker: Option<(mpsc::Sender<Preferences>, thread::JoinHandle<()>)>,
    edit_finished: Arc<AtomicBool>,
    geometry: Arc<parking_lot::Mutex<Geometry>>,
}

struct Geometry {
    current: Rect,
    requested: Option<Rect>,
}
fn fit_bounds(area: Rect, pos: Pos2, size: Vec2) -> Rect {
    let size = size.max(vec2(420., 84.).min(area.size())).min(area.size());
    Rect::from_min_size(pos.clamp(area.min, area.max - size), size)
}
fn default_bounds(area: Rect) -> Rect {
    let size = vec2((area.width() - 96.).min(820.), 156.);
    fit_bounds(
        area,
        pos2(area.center().x - size.x / 2., area.bottom() - size.y - 96.),
        size,
    )
}

#[derive(Clone)]
struct Preferences {
    lane: Option<Arc<Lane>>,
    lang: Lang,
    editing: bool,
    seconds: f64,
    lines: usize,
    font: f32,
    source: bool,
}
impl PartialEq for Preferences {
    fn eq(&self, other: &Self) -> bool {
        self.lane.as_ref().map(Arc::as_ptr) == other.lane.as_ref().map(Arc::as_ptr)
            && (
                self.lang,
                self.editing,
                self.seconds,
                self.lines,
                self.font,
                self.source,
            ) == (
                other.lang,
                other.editing,
                other.seconds,
                other.lines,
                other.font,
                other.source,
            )
    }
}

impl DesktopOverlay {
    pub fn load(path: &Path) -> Self {
        let path = path.with_file_name("desktop_subtitle_overlay.json");
        let bounds = Surface::read_bounds(&path);
        Self {
            editing: false,
            path,
            geometry: Arc::new(parking_lot::Mutex::new(Geometry {
                current: bounds,
                requested: None,
            })),
            last: None,
            worker: None,
            edit_finished: Arc::default(),
        }
    }
    pub fn finish_edit(&mut self) {
        self.editing = false;
    }
    pub fn geometry_controls(&self, ui: &mut Ui, lang: Lang) {
        let bounds = {
            let g = self.geometry.lock();
            g.requested.unwrap_or(g.current)
        };
        let mut size = bounds.size();
        let mut reset = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(bold(
                lang.t(
                    "오버레이 창",
                    "Overlay window",
                    "字幕ウィンドウ",
                    "字幕窗口",
                ),
                15.,
            ));
            for (label, value) in [
                (lang.t("너비", "Width", "幅", "宽度"), &mut size.x),
                (lang.t("높이", "Height", "高さ", "高度"), &mut size.y),
            ] {
                ui.horizontal(|ui| {
                    hint(ui, label);
                    ui.add(
                        DragValue::new(value)
                            .speed(1.)
                            .fixed_decimals(0)
                            .suffix(" px"),
                    );
                });
            }
            reset = ui
                .button(lang.t(
                    "기본 위치와 크기",
                    "Reset position and size",
                    "位置とサイズを戻す",
                    "重置大小和位置",
                ))
                .clicked();
        });
        hint(
            ui,
            lang.t(
                "창 안쪽을 끌어 이동하고 가장자리를 끌어 크기를 조절하세요.",
                "Drag inside the window to move it or drag an edge to resize it.",
                "内側をドラッグして移動、端をドラッグしてサイズを変更できます。",
                "拖动窗口内部可移动，拖动边缘可调整大小。",
            ),
        );
        if reset || size != bounds.size() {
            let next = if reset {
                default_bounds(platform::primary_work_area())
            } else {
                fit_bounds(
                    platform::overlay_work_area(bounds.center()).0,
                    bounds.min,
                    size,
                )
            };
            self.geometry.lock().requested = Some(next);
            if let Some((_, worker)) = &self.worker {
                worker.thread().unpark();
            }
        }
    }
    pub fn show(&mut self, ctx: &Context, lang: Lang, cfg: &Config, lane: Option<&Arc<Lane>>) {
        let enabled = cfg.b("/ui/desktop_subtitles");
        if !enabled || self.edit_finished.swap(false, Ordering::AcqRel) {
            self.finish_edit();
        }
        let next = Preferences {
            lane: lane.filter(|_| enabled).cloned(),
            lang,
            editing: self.editing,
            seconds: cfg.n("/overlay/display_sec", 7.),
            lines: cfg.n("/overlay/lines", 3.).clamp(1., 80.) as usize,
            font: cfg.n("/overlay/font_size", 27.).clamp(18., 72.) as f32,
            source: cfg.b("/overlay/show_source"),
        };
        if self.last.as_ref() == Some(&next) {
            return;
        }
        if self.worker.is_none() {
            if next.lane.is_none() && !next.editing {
                return;
            }
            let (tx, rx) = mpsc::channel();
            let (path, finished, ctx, geometry) = (
                self.path.clone(),
                self.edit_finished.clone(),
                ctx.clone(),
                self.geometry.clone(),
            );
            self.worker = Some((
                tx,
                thread::spawn(move || {
                    let mut surface = Surface {
                        path,
                        geometry,
                        editing: false,
                        window: None,
                    };
                    let mut state: Preferences = match rx.recv() {
                        Ok(p) => p,
                        Err(_) => return,
                    };
                    loop {
                        match rx.try_recv() {
                            Ok(p) => {
                                state = p;
                                continue;
                            }
                            Err(mpsc::TryRecvError::Disconnected) => break,
                            Err(mpsc::TryRecvError::Empty) => {}
                        }
                        window::pump_messages();
                        if surface.sync_bounds() {
                            ctx.request_repaint();
                        }
                        if surface.editing && !state.editing {
                            surface.finish_edit();
                        }
                        surface.editing = state.editing;
                        if surface.editing
                            && surface.window.as_ref().is_some_and(|w| w.finished_edit())
                        {
                            surface.finish_edit();
                            state.editing = false;
                            finished.store(true, Ordering::Release);
                            ctx.request_repaint();
                        }
                        surface.show(&state);
                        if state.editing {
                            thread::park_timeout(Duration::from_millis(16));
                        } else if state.lane.is_some() {
                            thread::park_timeout(Duration::from_millis(100));
                        } else {
                            thread::park();
                        }
                    }
                    surface.finish_edit();
                }),
            ));
        }
        if let Some((tx, worker)) = &self.worker {
            let _ = tx.send(next.clone());
            worker.thread().unpark();
            self.last = Some(next);
        }
    }
}
impl Drop for DesktopOverlay {
    fn drop(&mut self) {
        if let Some((tx, worker)) = self.worker.take() {
            drop(tx);
            window::end_move(&worker);
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

struct Surface {
    editing: bool,
    path: PathBuf,
    geometry: Arc<parking_lot::Mutex<Geometry>>,
    window: Option<window::CaptionWindow>,
}

impl Surface {
    fn read_bounds(path: &Path) -> Rect {
        let saved = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|v| {
                let number =
                    |key: &str| v[key].as_f64().map(|n| n as f32).filter(|n| n.is_finite());
                let rect = Rect::from_min_size(
                    pos2(number("x")?, number("y")?),
                    vec2(number("w")?, number("h")?),
                );
                rect.is_positive().then_some(rect)
            });
        saved.map_or_else(
            || default_bounds(platform::primary_work_area()),
            |r| fit_bounds(platform::overlay_work_area(r.center()).0, r.min, r.size()),
        )
    }
    fn sync_bounds(&mut self) -> bool {
        let request = self.geometry.lock().requested.take();
        if let (Some(r), Some(window)) = (request, &self.window) {
            window.set_bounds(r);
        }
        if !self.editing && request.is_none() {
            return false;
        }
        let Some(actual) = self.window.as_ref().and_then(|w| w.bounds()).or(request) else {
            return false;
        };
        let mut g = self.geometry.lock();
        let changed = g.current != actual;
        g.current = actual;
        changed
    }
    pub fn finish_edit(&mut self) {
        if !self.editing {
            return;
        }
        self.sync_bounds();
        self.editing = false;
        let rect = self.geometry.lock().current;
        let data = serde_json::json!({
            "x": rect.left().round() as i32, "y": rect.top().round() as i32,
            "w": rect.width().round() as i32, "h": rect.height().round() as i32
        });
        if let Some(dir) = self.path.parent()
            && fs::create_dir_all(dir).is_ok()
            && let Ok(mut file) = tempfile::NamedTempFile::new_in(dir)
            && file.write_all(data.to_string().as_bytes()).is_ok()
        {
            let _ = file.persist(&self.path);
        }
    }

    fn show(&mut self, state: &Preferences) {
        let mut rows = state
            .lane
            .as_ref()
            .filter(|lane| lane.alive())
            .map(|lane| {
                captions::recent(lane, state.lang, state.lines, state.seconds, state.source)
            })
            .unwrap_or_default();
        if rows.is_empty() && !self.editing {
            if let Some(window) = &mut self.window {
                window.hide();
            }
            return;
        }
        if self.window.is_none() {
            self.window = window::CaptionWindow::new(self.geometry.lock().current);
        }
        let Some(window) = &mut self.window else {
            return;
        };
        if rows.is_empty() {
            rows.push(Row::new(
                &Caption {
                    source: state
                        .lang
                        .t(
                            "This is a sample caption.",
                            "자막 표시 예시입니다.",
                            "This is a sample caption.",
                            "This is a sample caption.",
                        )
                        .into(),
                    text: state
                        .lang
                        .t(
                            "자막 표시 예시입니다.",
                            "This is a sample caption.",
                            "字幕の表示例です。",
                            "这是字幕显示示例。",
                        )
                        .into(),
                    ..Default::default()
                },
                state.lang,
                state.source,
                false,
                false,
            ));
        }
        window.display(&rows, state.font, self.editing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_keeps_its_height_with_wrapped_captions() {
        let ctx = Context::default();
        crate::theme::setup(&ctx);
        let entries = (0..40)
            .map(|n| {
                (
                    Instant::now(),
                    Row::new(
                        &Caption {
                            speaker: n.to_string(),
                            source: "원문 ".repeat(40),
                            text: "Translation ".repeat(24),
                        },
                        Lang::Ko,
                        true,
                        false,
                        n % 2 == 0,
                    ),
                )
            })
            .collect::<Vec<_>>();
        for width in [780., 980.] {
            let mut heights = Vec::new();
            for count in [0, 1, 40] {
                let input = RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 540.))),
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    CentralPanel::default().show(ctx, |ui| {
                        ScrollArea::vertical().show(ui, |ui| {
                            ui.add_space(200.);
                            let top = ui.next_widget_position().y;
                            history(ui, &entries[..count], Lang::Ko, true, 17.);
                            heights.push(ui.min_rect().bottom() - top);
                        });
                    });
                });
            }
            assert!(heights[0] > 80. && heights[0] < 260.);
            assert!(
                heights.iter().all(|h| (h - heights[0]).abs() < 1.),
                "{heights:?}"
            );
        }
    }

    #[test]
    fn scrolling_up_pauses_following_new_captions() {
        let ctx = Context::default();
        crate::theme::setup(&ctx);
        let entries: Vec<_> = (0..40)
            .map(|n| {
                (
                    Instant::now(),
                    Row::new(
                        &Caption {
                            text: format!("Line {n}"),
                            ..Default::default()
                        },
                        Lang::En,
                        false,
                        false,
                        false,
                    ),
                )
            })
            .collect();
        let frame = |count: usize, events: Vec<Event>| {
            let mut offset = 0.;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800., 540.))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    ScrollArea::vertical().show(ui, |ui| {
                        offset = history(ui, &entries[..count], Lang::En, true, 17.)
                            .state
                            .offset
                            .y;
                    });
                });
            });
            offset
        };
        frame(1, vec![]);
        let bottom = frame(24, vec![]);
        assert!(bottom > 0.);
        let above = frame(
            24,
            vec![
                Event::PointerMoved(pos2(100., 100.)),
                Event::MouseWheel {
                    unit: MouseWheelUnit::Point,
                    delta: vec2(0., 120.),
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        assert!(above < bottom);
        assert!(frame(25, vec![]) <= above + 1.);
    }
    #[test]
    fn overlay_displays_source_only_and_expires_stale_fragments() {
        let lane = Lane::new(true, 16000, true, Context::default(), None);
        lane.fragment(true, "안녕하세요", "2");
        let recent = |sec| captions::recent(&lane, Lang::Ko, 3, sec, false);
        assert_eq!(recent(7.)[0].body, "안녕하세요");
        lane.captions.lock().pending[0].body_changed = Instant::now() - Duration::from_secs(8);
        assert!(recent(7.).is_empty());
        lane.finish();
        assert!(recent(7.).is_empty());
        assert_eq!(lane.captions.lock().history[0].caption.speaker, "2");
        assert!(recent(0.).is_empty());
    }
}
