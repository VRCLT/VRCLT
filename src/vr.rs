mod dashboard;
mod hands;
mod menu;
mod panel_ui;
mod placement;
mod pointer;
#[cfg(test)]
mod tests;
mod textures;
mod wrist;
use crate::{
    config::Config,
    labels::Lang,
    osc::Action,
    platform,
    runtime::{Caption, Lane},
};
use anyhow::{Context as _, Result, ensure};
use eframe::egui::Context;
use hands::{Frame, Hands};
use menu::{Command, Menu};
use openvr_sys as vr;
use parking_lot::Mutex;
pub use placement::Placement;
use placement::{Panel, Pose, Size};
use std::{
    ffi::CString,
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread,
    time::{Duration, Instant},
};

type Routes = [Option<Arc<Lane>>; 2];
struct View {
    cfg: Config,
    cable: crate::setup::Cable,
    outputs: Vec<String>,
    routes: Routes,
    running: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum DisplayChange {
    Font(i32),
    Scale(i32),
    Original,
}
impl DisplayChange {
    pub fn apply(self, cfg: &mut Config) {
        match self {
            Self::Original => {
                cfg.data["overlay"]["show_source"] = (!cfg.b("/overlay/show_source")).into()
            }
            Self::Font(step) => {
                cfg.data["overlay"]["font_size"] = (cfg.n("/overlay/font_size", 27.) as i32 + step)
                    .clamp(18, 72)
                    .into()
            }
            Self::Scale(step) => {
                let width = cfg.n("/overlay/width_m", 0.9).clamp(0.45, 1.6);
                let next = ((width * 100.).round() + f64::from(step) * 5.).clamp(45., 160.) / 100.;
                let height = (cfg.n("/overlay/height_m", 0.225) * next / width).clamp(0.10, 0.60);
                cfg.data["overlay"]["width_m"] = next.into();
                cfg.data["overlay"]["height_m"] = height.into();
            }
        }
    }
}

pub struct Vr {
    state: Arc<Mutex<Arc<View>>>,
    alive: Arc<AtomicBool>,
    worker: thread::Thread,
}
// The renderer owns the menu, placement, gaze fade and SteamVR handles.
// Only changes to settings consumed when creating those resources require a rebuild.
pub fn creation_settings_changed(old: &Config, new: &Config) -> bool {
    (old.s("/ui/mode") == "desktop") != (new.s("/ui/mode") == "desktop")
        || [
            "/steamvr/register",
            "/steamvr/dashboard_panel",
            "/wrist_ui/enabled",
            "/wrist_ui/hand",
            "/wrist_ui/width_m",
            "/wrist_ui/pointer_tilt_deg",
            "/wrist_ui/transform",
            "/wrist_ui/tilt_deg",
            "/wrist_ui/roll_deg",
            "/wrist_ui/offset",
            "/overlay/enabled",
            "/overlay/transform",
            "/overlay/tilt_deg",
            "/overlay/below_m",
            "/overlay/distance_m",
        ]
        .iter()
        .any(|path| old.data.pointer(path) != new.data.pointer(path))
}
impl Vr {
    pub fn start(
        cfg: Config,
        routes: Routes,
        running: bool,
        outputs: Vec<String>,
        cable: crate::setup::Cable,
        tx: Sender<Action>,
        ctx: Context,
    ) -> Option<Self> {
        let panels = cfg.s("/ui/mode") != "desktop"
            && [
                "/wrist_ui/enabled",
                "/steamvr/dashboard_panel",
                "/overlay/enabled",
            ]
            .iter()
            .any(|k| cfg.b(k));
        let startup = cfg.b("/steamvr/register");
        if !panels && !startup {
            return None;
        }
        let state = Arc::new(Mutex::new(Arc::new(View {
            cfg,
            cable,
            outputs,
            routes,
            running,
        })));
        let alive = Arc::new(AtomicBool::new(true));
        let (state2, alive2) = (state.clone(), alive.clone());
        let worker = thread::spawn(move || {
            // OpenVR initialization and shutdown are process-global, including during a settings restart.
            static OWNER: Mutex<()> = Mutex::new(());
            let _owner = OWNER.lock();
            // Utility mode can install the startup entry without a connected headset.
            // Release that OpenVR session before creating the overlay session.
            if startup && alive2.load(Ordering::Acquire) && unsafe { vr::VR_IsRuntimeInstalled() } {
                if let Err(e) = register_startup() {
                    let _ = tx.send(Action::Warning(format!("SteamVR: {e}")));
                    platform::wake_console(&ctx);
                }
            }
            if !panels {
                return;
            }
            while alive2.load(Ordering::Acquire) {
                if platform::process_id("vrcompositor").is_some() {
                    if let Err(e) = render(&state2, &alive2, &tx, &ctx) {
                        if alive2.load(Ordering::Acquire) {
                            let _ = tx.send(Action::Warning(format!("SteamVR: {e}")));
                            ctx.request_repaint();
                        }
                    }
                }
                if alive2.load(Ordering::Acquire) {
                    thread::park_timeout(Duration::from_secs(4));
                }
            }
        })
        .thread()
        .clone();
        Some(Self {
            state,
            alive,
            worker,
        })
    }
    pub fn update(
        &self,
        cfg: Config,
        routes: Routes,
        running: bool,
        outputs: Vec<String>,
        cable: crate::setup::Cable,
    ) {
        *self.state.lock() = Arc::new(View {
            cfg,
            cable,
            outputs,
            routes,
            running,
        });
        self.worker.unpark();
    }
}
impl Drop for Vr {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.worker.unpark();
    }
}
struct Session;
#[derive(Default)]
struct WristTracking {
    last_visible: Option<(u32, Instant)>,
}
impl WristTracking {
    // A known look-away starts the fade immediately. Hold the gaze target only
    // during brief tracking gaps on the same hand.
    fn display(&mut self, hand: u32, looking: Option<bool>, now: Instant) -> Option<u32> {
        self.last_visible = match looking {
            Some(true) if hand != u32::MAX => Some((hand, now)),
            Some(_) => None,
            None => self.last_visible.filter(|&(old, at)| {
                (hand == u32::MAX || hand == old) && now - at < Duration::from_millis(200)
            }),
        };
        self.last_visible.map(|(index, _)| index)
    }
}
#[derive(Default)]
struct WristFade {
    hand: Option<u32>,
    showing: bool,
    phase: f32,
    updated: Option<Instant>,
}
impl WristFade {
    fn sample(&mut self, target: Option<u32>, now: Instant) -> (Option<u32>, f32) {
        let elapsed = self
            .updated
            .replace(now)
            .map_or(0., |last| (now - last).as_secs_f32());
        // Advance the previous direction up to this sample, then accept the
        // new target. Reversing gaze retains the exact current opacity.
        self.phase = if self.showing {
            (self.phase + elapsed / 0.20).min(1.)
        } else {
            (self.phase - elapsed / 0.16).max(0.)
        };
        if let Some(hand) = target {
            if self.hand != Some(hand) {
                self.phase = 0.;
                self.hand = Some(hand);
            }
        } else if self.phase == 0. {
            self.hand = None;
        }
        self.showing = target.is_some();
        let t = self.phase;
        (self.hand, t * t * (3. - 2. * t))
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            vr::VR_ShutdownInternal();
        }
    }
}
unsafe fn table<'a, T>(_: &'a Session, version: &[u8]) -> Result<&'a T> {
    let name = [b"FnTable:".as_slice(), version].concat();
    let mut err = 0;
    let ptr = unsafe { vr::VR_GetGenericInterface(name.as_ptr().cast(), &mut err) };
    ensure!(err == 0 && ptr != 0, "OpenVR interface error {err}");
    Ok(unsafe { &*(ptr as *const T) })
}
fn check(n: i32) -> Result<()> {
    ensure!(n == 0, "overlay error {n}");
    Ok(())
}
const APP_KEY: &std::ffi::CStr = c"shgeum.vrclt";
fn register_startup() -> Result<()> {
    unsafe {
        let mut err = 0;
        vr::VR_InitInternal(&mut err, vr::EVRApplicationType_VRApplication_Utility);
        ensure!(err == 0, "startup registration error {err}");
        let session = Session;
        register(table(&session, vr::IVRApplications_Version)?)
    }
}
fn register(apps: &vr::VR_IVRApplications_FnTable) -> Result<()> {
    let config = Config::path();
    let dir = config.parent().context("config directory")?;
    register_manifest(apps, APP_KEY, dir, &std::env::current_exe()?)
}
fn register_manifest(
    apps: &vr::VR_IVRApplications_FnTable,
    app_key: &std::ffi::CStr,
    dir: &std::path::Path,
    exe: &std::path::Path,
) -> Result<()> {
    use std::io::Write;
    fs::create_dir_all(dir)?;
    let path = std::path::absolute(dir.join("vrclt.vrmanifest"))?;
    let manifest = serde_json::to_vec(&serde_json::json!({
        "source": "builtin", "applications": [{
            "app_key": app_key.to_str()?, "launch_type": "binary",
            "binary_path_windows": exe, "arguments": "run --steamvr-startup",
            "is_dashboard_overlay": true,
            "strings": {"en_us": {"name": "VRCLT", "description": "Live translation"}}
        }]
    }))?;
    if fs::read(&path).ok().as_deref() != Some(&manifest) {
        let mut f = tempfile::NamedTempFile::new_in(dir)?;
        f.write_all(&manifest)?;
        f.persist(&path)?;
    }
    let p = CString::new(path.to_string_lossy().as_bytes())?;
    let key = app_key.as_ptr().cast_mut();
    let known = unsafe { apps.IsApplicationInstalled.unwrap()(key) };
    let err = unsafe { apps.AddApplicationManifest.unwrap()(p.as_ptr().cast_mut(), false) };
    ensure!(err == 0, "SteamVR manifest error {err}");
    // SteamVR owns this preference. Only a fresh registration gets the default.
    if !known {
        let err = unsafe { apps.SetApplicationAutoLaunch.unwrap()(key, true) };
        ensure!(err == 0, "SteamVR auto-launch error {err}");
    }
    Ok(())
}
// Keep panel geometry in metres and font controls in the original 640-unit layout.
const CAPTION_DENSITY: usize = 2;
const W: usize = 640 * CAPTION_DENSITY;
fn caption_rgba(
    pixel: eframe::egui::Color32,
    background: eframe::egui::Color32,
    background_rgba: [u8; 4],
) -> [u8; 4] {
    if pixel == background {
        background_rgba
    } else {
        pixel.to_srgba_unmultiplied()
    }
}
fn draw_handles(rgba: &mut [u8], height: usize) {
    let reach = (height / 3).clamp(12 * CAPTION_DENSITY, 36 * CAPTION_DENSITY);
    for y in 0..height {
        for x in 0..W {
            let side = x.min(W - 1 - x);
            let bottom = height - 1 - y;
            let line = side + bottom;
            if side < 2 * CAPTION_DENSITY
                || y < 2 * CAPTION_DENSITY
                || bottom < 2 * CAPTION_DENSITY
                || (line < reach && line % (9 * CAPTION_DENSITY) < 3 * CAPTION_DENSITY)
            {
                let i = (y * W + x) * 4;
                rgba[i..i + 4].copy_from_slice(&[130, 206, 255, 245]);
            }
        }
    }
}
fn render(
    state: &Mutex<Arc<View>>,
    alive: &AtomicBool,
    tx: &Sender<Action>,
    ctx: &Context,
) -> Result<()> {
    // All handles, tables and pointers below live on this thread under a single initialized session.
    unsafe {
        let mut err = 0;
        vr::VR_InitInternal(&mut err, vr::EVRApplicationType_VRApplication_Overlay);
        ensure!(err == 0, "initialization error {err}");
        let session = Session;
        let ov = table::<vr::VR_IVROverlay_FnTable>(&session, vr::IVROverlay_Version)?;
        let sys = table::<vr::VR_IVRSystem_FnTable>(&session, vr::IVRSystem_Version)?;
        let mut textures = textures::Textures::new(sys, ov)?;
        let view = state.lock().clone();
        let c = &view.cfg;
        if c.b("/steamvr/register") {
            if let Err(e) = table(&session, vr::IVRApplications_Version).and_then(
                |apps: &vr::VR_IVRApplications_FnTable| {
                    register(apps)?;
                    let err = apps.IdentifyApplication.unwrap()(0, APP_KEY.as_ptr().cast_mut());
                    ensure!(err == 0, "SteamVR identification error {err}");
                    Ok(())
                },
            ) {
                let _ = tx.send(Action::Warning(format!("SteamVR: {e}")));
                platform::wake_console(ctx);
            }
        }
        let mut wrist = 0;
        let mut sub = 0;
        let mut dash = 0;
        let mut thumb = 0;
        let (mut laser, mut dot) = (0, 0);
        let create = |key: &std::ffi::CStr, h: &mut u64| {
            check(ov.CreateOverlay.unwrap()(
                key.as_ptr().cast_mut(),
                c"VRCLT".as_ptr().cast_mut(),
                h,
            ))
        };
        if c.b("/wrist_ui/enabled") {
            create(c"vrclt.rust.wrist", &mut wrist)?;
            check(ov.SetOverlayAlpha.unwrap()(wrist, 0.))?;
            check(ov.SetOverlayWidthInMeters.unwrap()(
                wrist,
                c.n("/wrist_ui/width_m", 0.14).clamp(0.05, 0.5) as f32,
            ))?;
        }
        if c.b("/overlay/enabled") {
            create(c"vrclt.rust.captions", &mut sub)?;
            check(ov.SetOverlayWidthInMeters.unwrap()(
                sub,
                c.n("/overlay/width_m", 0.9).clamp(0.45, 1.6) as f32,
            ))?;
        }
        if c.b("/steamvr/dashboard_panel") {
            check(ov.CreateDashboardOverlay.unwrap()(
                c"vrclt.rust.dashboard".as_ptr().cast_mut(),
                c"VRCLT".as_ptr().cast_mut(),
                &mut dash,
                &mut thumb,
            ))?;
            dashboard::configure(ov, dash)?;
            let img = image::load_from_memory(include_bytes!("../assets/icon.png"))?
                .thumbnail(128, 128)
                .to_rgba8();
            textures.upload(thumb, img.as_raw(), img.width(), img.height())?;
        }
        let left = c.s("/wrist_ui/hand") != "right";
        let role = if left { 1 } else { 2 };
        let marks = if wrist != 0 || sub != 0 {
            create(c"vrclt.rust.pointer", &mut laser)?;
            create(c"vrclt.rust.cursor", &mut dot)?;
            Some((
                pointer::Mark::new(ov, &mut textures, laser, true)?,
                pointer::Mark::new(ov, &mut textures, dot, false)?,
            ))
        } else {
            None
        };
        let mut marks = marks;
        let tilt = Pose::pitch(-c.n("/wrist_ui/pointer_tilt_deg", 50.) as f32);
        let wrist_width = c.n("/wrist_ui/width_m", 0.14).clamp(0.05, 0.5) as f32;
        let mut hands = Hands::new(
            Panel::Wrist.configured(c),
            Panel::Captions.configured(c),
            Size::configured(c),
        );
        let mut saved_size = hands.size;
        let mut menu = Menu::default();
        let painter = (wrist != 0 || dash != 0).then(panel_ui::Renderer::default);
        let mut layout = wrist::Layout::default();
        let mut last_focus = (None, None);
        let mut dash_layout = panel_ui::Layout::new(dashboard::WIDTH, dashboard::HEIGHT);
        let mut dash_pointer = dashboard::Pointer::default();
        let mut dash_focus = (None, None);
        let mut last_filter_status = None;
        let mut last_watch = None;
        let mut last_sub = None;
        let mut last_edit = false;
        let mut input_devices = (u32::MAX, u32::MAX);
        let send = |action| {
            if tx.send(action).is_ok() {
                platform::wake_console(ctx);
            }
        };
        let locale = platform::locale();
        let mut last_caption = Vec::new();
        let mut caption_visible = false;
        let mut last_font = 0.;
        let mut last_width = c.n("/overlay/width_m", 0.9).clamp(0.45, 1.6) as f32;
        let mut last_height = 0;
        let renderer = (sub != 0).then(crate::captions::TextRenderer::for_headset);
        let mut buffer = Vec::new();
        let mut img = Vec::new();
        let caption_thread = Arc::new(thread::current());
        let background = eframe::egui::Color32::from_rgba_unmultiplied(10, 10, 12, 185);
        let background_rgba = background.to_srgba_unmultiplied();
        let mut tracking = WristTracking::default();
        let mut fade = WristFade::default();
        let mut last_wrist_alpha = 0.;
        let mut attached = None;
        let mut previous_view: Option<Arc<View>> = None;
        let mut health = Instant::now();
        let hz = sys.GetFloatTrackedDeviceProperty.unwrap()(
            0,
            vr::ETrackedDeviceProperty_Prop_DisplayFrequency_Float,
            std::ptr::null_mut(),
        );
        let frame_time = Duration::from_secs_f32(
            1. / if hz.is_finite() && hz >= 30. {
                hz.min(240.)
            } else {
                90.
            },
        );
        let sync_timeout_ms = frame_time.as_micros().div_ceil(1000) as u32;
        while alive.load(Ordering::Acquire) {
            let tick_start = Instant::now();
            if health.elapsed() > Duration::from_secs(3) {
                if platform::process_id("vrserver").is_none() {
                    break;
                }
                health = Instant::now();
            }
            let view = state.lock().clone();
            let c = &view.cfg;
            let l = Lang::new(if c.s("/ui/lang").is_empty() {
                &locale
            } else {
                c.s("/ui/lang")
            });
            let mut changed = false;
            let mut dash_changed = false;
            if let Some(action) = menu.poll() {
                send(action);
            }
            let filter_status = dashboard::filter_status(menu.dashboard, &view, l);
            if filter_status != last_filter_status {
                last_filter_status = filter_status;
                menu.dirty = true;
            }
            let view_changed = !previous_view
                .as_ref()
                .is_some_and(|old| Arc::ptr_eq(old, &view));
            if view_changed {
                if sub != 0
                    && let Some(lane) = &view.routes[1]
                {
                    lane.wake_vr_on_caption(&caption_thread);
                }
                let config_size = Size::configured(c);
                if saved_size != config_size {
                    hands.size = config_size;
                    saved_size = config_size;
                }
            }
            let dashboard = ov.IsDashboardVisible.unwrap()();
            let mut command = None;
            if dash != 0 {
                let active = dashboard && ov.IsActiveDashboardOverlay.unwrap()(dash);
                let mut event = vr::VREvent_t::default();
                while ov.PollNextOverlayEvent.unwrap()(dash, &mut event, size_of_val(&event) as u32)
                {
                    match event.eventType as i32 {
                        vr::EVREventType_VREvent_Quit => return Ok(()),
                        vr::EVREventType_VREvent_OverlayHidden
                        | vr::EVREventType_VREvent_FocusLeave => {
                            dash_pointer.clear();
                        }
                        vr::EVREventType_VREvent_MouseMove if active => {
                            dash_pointer.moved(event.data.mouse.x, event.data.mouse.y);
                        }
                        vr::EVREventType_VREvent_MouseButtonDown
                            if active
                                && event.data.mouse.button
                                    == vr::EVRMouseButton_VRMouseButton_Left as u32 =>
                        {
                            dash_pointer.down(&dash_layout, event.data.mouse.x, event.data.mouse.y);
                        }
                        vr::EVREventType_VREvent_MouseButtonUp
                            if active
                                && event.data.mouse.button
                                    == vr::EVRMouseButton_VRMouseButton_Left as u32 =>
                        {
                            command = dash_pointer.up(
                                &dash_layout,
                                event.data.mouse.x,
                                event.data.mouse.y,
                            );
                            if command.is_some() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if !active {
                    dash_pointer.clear();
                }
            }
            let hand = if wrist != 0 {
                sys.GetTrackedDeviceIndexForControllerRole.unwrap()(role)
            } else {
                u32::MAX
            };
            let other = sys.GetTrackedDeviceIndexForControllerRole.unwrap()(3 - role);
            if input_devices != (hand, other) {
                hands.cancel();
                input_devices = (hand, other);
            }
            let sample = if !dashboard && marks.is_some() {
                pointer::input(sys, hand, other, tilt)
            } else {
                pointer::Tracking::default()
            };
            let feedback = hands.tick(sample.input, menu.edit, &layout, wrist_width);
            let clicked = feedback.click.is_some();
            if let Some(placement) = feedback.placed {
                send(Action::PlacePanel(placement));
            }
            if let Some(cmd) = command.or(feedback.click) {
                if let Command::Reset(panel) = cmd {
                    hands.cancel();
                    let defaults = Config::default();
                    let pose = match panel {
                        Panel::Wrist => panel.configured(&defaults),
                        Panel::Captions => panel.fallback(&defaults),
                    };
                    let p = Placement {
                        panel,
                        pose,
                        size: (panel == Panel::Captions).then(|| Size::configured(&defaults)),
                    };
                    hands.apply(p);
                    send(Action::PlacePanel(p));
                }
                if let Some(action) = menu.press(cmd, &view, dashboard) {
                    send(action);
                }
            }
            if let Some(renderer) = &renderer {
                if last_sub != Some(hands.captions) {
                    pointer::relative(ov, sub, 0, hands.captions)?;
                    last_sub = Some(hands.captions);
                }
                let editing = menu.edit == Some(Panel::Captions);
                let value = view.routes[1]
                    .as_ref()
                    .map(|lane| {
                        crate::captions::recent(
                            lane,
                            l,
                            c.n("/overlay/lines", 3.).clamp(1., 80.) as usize,
                            c.n("/overlay/display_sec", 7.),
                            c.b("/overlay/show_source"),
                        )
                    })
                    .unwrap_or_default();
                let font = c.n("/overlay/font_size", 27.).clamp(18., 72.) as f32;
                let width = hands.size.width;
                if width != last_width {
                    check(ov.SetOverlayWidthInMeters.unwrap()(sub, width))?;
                    last_width = width;
                }
                let height = (W as f32 * hands.size.height / width).round() as usize;
                if value != last_caption
                    || font != last_font
                    || height != last_height
                    || last_edit != editing
                {
                    let show = !value.is_empty() || editing;
                    if show {
                        let edit_hint;
                        let rows =
                            if value.is_empty() {
                                edit_hint = [crate::captions::Row::new(&Caption {
                                text: l.t("그립으로 이동    아래 모서리는 트리거로 크기 조정",
                                    "Grip to move    Drag a lower corner with the trigger",
                                    "グリップで移動    下の角はトリガーでサイズ調整",
                                    "握把移动    扳机拖动下角缩放").into(),
                                ..Default::default()
                            }, l, false, false, false)];
                                &edit_hint[..]
                            } else {
                                &value[..]
                            };
                        buffer.resize(W * height, background);
                        buffer.fill(background);
                        renderer.draw(rows, W, height, font * CAPTION_DENSITY as f32, &mut buffer);
                        img.resize(W * height * 4, 0);
                        for (dst, &pixel) in img.chunks_exact_mut(4).zip(&buffer) {
                            dst.copy_from_slice(&caption_rgba(pixel, background, background_rgba));
                        }
                        if editing {
                            draw_handles(&mut img, height);
                        }
                        textures.upload(sub, &img, W as u32, height as u32)?;
                    }
                    if show != caption_visible {
                        check(if show {
                            ov.ShowOverlay.unwrap()(sub)
                        } else {
                            ov.HideOverlay.unwrap()(sub)
                        })?;
                        caption_visible = show;
                    }
                    last_caption = value;
                    last_font = font;
                    last_height = height;
                    last_edit = editing;
                }
            }
            if menu.dirty || view_changed {
                if dash != 0 {
                    let next = dashboard::layout(&menu, &view, l);
                    if next != dash_layout {
                        dash_layout = next;
                        dash_changed = true;
                    }
                }
                if wrist != 0 {
                    let next = wrist::Layout::build(&menu, &view, l);
                    if next != layout {
                        layout = next;
                        changed = true;
                    }
                }
                menu.dirty = false;
                previous_view = Some(view.clone());
            }
            if let Some(painter) = painter.as_ref().filter(|_| wrist != 0) {
                let hover = feedback
                    .wrist_uv
                    .and_then(|(u, v)| layout.hit(u, v))
                    .cloned();
                let focus = (hover, hands.pressed().cloned());
                if changed || focus != last_focus {
                    let img = painter.render(&layout, focus.0.as_ref(), focus.1.as_ref());
                    textures.upload(wrist, &img, wrist::WIDTH as u32, wrist::HEIGHT as u32)?;
                    if other != u32::MAX && !dashboard {
                        let duration = if clicked {
                            1800
                        } else if focus.1.is_some() && focus.1 != last_focus.1 {
                            1100
                        } else if focus.0.is_some() && focus.0 != last_focus.0 {
                            500
                        } else {
                            0
                        };
                        if duration > 0 {
                            if let Some(pulse) = sys.TriggerHapticPulse {
                                pulse(other, 0, duration);
                            }
                        }
                    }
                    last_focus = focus;
                }
            }
            if dash != 0 {
                let focus = dash_pointer.focus(&dash_layout);
                if dash_changed || focus != dash_focus {
                    let img =
                        painter
                            .as_ref()
                            .unwrap()
                            .render_targets(&dash_layout, focus.0, focus.1);
                    textures.upload(
                        dash,
                        &img,
                        dashboard::WIDTH as u32,
                        dashboard::HEIGHT as u32,
                    )?;
                    dash_focus = focus;
                }
            }
            if let Some((beam, cursor)) = &mut marks {
                beam.show(
                    ov,
                    feedback.beam.map(|b| b.0),
                    feedback.beam.map_or(0., |b| b.1 / 128.),
                )?;
                cursor.show(
                    ov,
                    feedback.cursor,
                    if hands.engaged { 0.006 } else { 0.024 },
                )?;
            }
            if wrist != 0 {
                let looking = if dashboard {
                    Some(false)
                } else {
                    sample.head_and_wrist.map(|(head, w)| {
                        head.looking_at(
                            w.compose(hands.watch),
                            tracking
                                .last_visible
                                .is_some_and(|(index, _)| index == hand),
                        )
                    })
                };
                let now = Instant::now();
                let target = tracking.display(hand, looking, now);
                let (visible_hand, alpha) = fade.sample(target, now);
                if alpha != last_wrist_alpha {
                    check(ov.SetOverlayAlpha.unwrap()(wrist, alpha))?;
                    last_wrist_alpha = alpha;
                }
                if let Some(hand) = visible_hand {
                    if attached != visible_hand || last_watch != Some(hands.watch) {
                        pointer::relative(ov, wrist, hand, hands.watch)?;
                        last_watch = Some(hands.watch);
                    }
                    if attached.is_none() {
                        check(ov.ShowOverlay.unwrap()(wrist))?;
                    }
                } else if attached.is_some() {
                    check(ov.HideOverlay.unwrap()(wrist))?;
                }
                attached = visible_hand;
            }
            let tracking = attached.is_some() || menu.edit.is_some() && !dashboard;
            let live_captions = sub != 0 && view.routes[1].is_some();
            if tracking && !live_captions {
                // Sample the hands at the headset cadence while the wrist UI or placement tool is in use.
                if !ov
                    .WaitFrameSync
                    .is_some_and(|wait| wait(sync_timeout_ms) == 0)
                {
                    thread::park_timeout(frame_time.saturating_sub(tick_start.elapsed()));
                }
            } else {
                // The compositor's blocking wait cannot be interrupted by a
                // caption. Keep the same hand cadence and wake on new words.
                let cadence = if tracking {
                    frame_time
                } else {
                    Duration::from_millis(33)
                };
                thread::park_timeout(cadence.saturating_sub(tick_start.elapsed()));
            }
        }
        for h in [dot, laser, thumb, dash, wrist, sub]
            .into_iter()
            .filter(|&h| h != 0)
        {
            ov.DestroyOverlay.unwrap()(h);
        }
    }
    Ok(())
}
