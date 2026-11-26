use super::*;
use wrist::Screen;

#[test]
#[ignore = "requires the SteamVR application registry"]
fn startup_registration_enables_a_new_entry_and_respects_opt_out() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let key = CString::new(format!("vrclt.qa.startup.{}", std::process::id()))?;
    let exe = std::env::current_exe()?;
    let manifest = CString::new(
        dir.path()
            .join("vrclt.vrmanifest")
            .to_string_lossy()
            .as_bytes(),
    )?;
    unsafe {
        let mut err = 0;
        vr::VR_InitInternal(&mut err, vr::EVRApplicationType_VRApplication_Utility);
        ensure!(err == 0, "SteamVR utility initialization error {err}");
        let session = Session;
        let apps = table::<vr::VR_IVRApplications_FnTable>(&session, vr::IVRApplications_Version)?;
        let id = key.as_ptr().cast_mut();
        let check_registration = || -> Result<()> {
            register_manifest(apps, &key, dir.path(), &exe)?;
            ensure!(apps.IsApplicationInstalled.unwrap()(id));
            ensure!(apps.GetApplicationAutoLaunch.unwrap()(id));
            ensure!(apps.SetApplicationAutoLaunch.unwrap()(id, false) == 0);
            register_manifest(apps, &key, dir.path(), &exe)?;
            ensure!(!apps.GetApplicationAutoLaunch.unwrap()(id));
            Ok(())
        };
        let result = check_registration();
        let cleanup = apps.RemoveApplicationManifest.unwrap()(manifest.as_ptr().cast_mut());
        result?;
        ensure!(cleanup == 0);
    }
    Ok(())
}

#[test]
fn headset_caption_blends_edges_and_keeps_the_transparent_background() {
    use crate::captions::{Row, TextRenderer};
    use eframe::egui::Color32;
    let row = Row::new(
        &Caption {
            text: "지금 말씀하세요。你好".into(),
            ..Default::default()
        },
        Lang::Ko,
        false,
        true,
        false,
    );
    let height = 160 * CAPTION_DENSITY;
    let mut reference = vec![Color32::TRANSPARENT; W * height];
    let mut smooth = reference.clone();
    let size = 27. * CAPTION_DENSITY as f32;
    TextRenderer::default().draw(std::slice::from_ref(&row), W, height, size, &mut reference);
    TextRenderer::for_headset().draw(&[row], W, height, size, &mut smooth);
    assert_ne!(reference, smooth);
    assert!(smooth[..W].iter().all(|px| *px == Color32::TRANSPARENT));
    assert!(smooth.iter().any(|px| (1..255).contains(&px.a())));
    let ink = |image: &[Color32]| image.iter().map(|p| f64::from(p.a())).sum::<f64>();
    assert!((ink(&smooth) / ink(&reference) - 1.).abs() < 0.02);
}

#[test]
fn restarting_translation_does_not_change_overlay_creation_settings() {
    let c = Config::default();
    let mut next = c.clone();
    next.data["outbound"]["target_language"] = "ja".into();
    next.data["dashboard"]["translation_on"] = false.into();
    next.data["overlay"]["font_size"] = 32.into();
    assert!(!creation_settings_changed(&c, &next));
    next.data["wrist_ui"]["enabled"] = (!c.b("/wrist_ui/enabled")).into();
    assert!(creation_settings_changed(&c, &next));
}

#[test]
fn wrist_is_hidden_until_looked_at_and_only_holds_through_missing_poses() {
    let mut tracking = WristTracking::default();
    let start = Instant::now();
    let t = |ms| start + Duration::from_millis(ms);
    assert_eq!(tracking.display(2, Some(false), t(0)), None);
    assert_eq!(tracking.display(2, Some(true), t(1)), Some(2));
    assert_eq!(tracking.display(u32::MAX, None, t(99)), Some(2));
    assert_eq!(tracking.display(2, Some(false), t(100)), None);
    assert_eq!(tracking.display(2, None, t(101)), None);
    assert_eq!(tracking.display(2, Some(true), t(133)), Some(2));
    assert_eq!(tracking.display(2, None, t(334)), None);
    assert_eq!(tracking.display(2, Some(true), t(350)), Some(2));
    assert_eq!(tracking.display(3, None, t(355)), None);
}

#[test]
fn wrist_fades_from_zero_to_full_and_reverses_without_a_jump() {
    let mut fade = WristFade::default();
    let start = Instant::now();
    let mut sample = |target, ms| fade.sample(target, start + Duration::from_millis(ms));
    assert_eq!(sample(None, 0), (None, 0.));
    assert_eq!(sample(Some(2), 1000), (Some(2), 0.));
    assert_eq!(sample(Some(2), 1050), (Some(2), 0.15625));
    assert_eq!(sample(None, 1100), (Some(2), 0.5));
    assert_eq!(sample(Some(2), 1140), (Some(2), 0.15625));
    assert_eq!(sample(Some(2), 1300), (Some(2), 1.));
    assert_eq!(sample(None, 1400), (Some(2), 1.));
    assert_eq!(sample(None, 1480), (Some(2), 0.5));
    assert_eq!(sample(None, 1560), (None, 0.));
    assert_eq!(sample(Some(2), 1600), (Some(2), 0.));
    assert_eq!(sample(Some(2), 1820), (Some(2), 1.));
    assert_eq!(sample(Some(3), 1821), (Some(3), 0.));
}

fn view() -> View {
    View {
        cable: crate::setup::Cable::Ready,
        cfg: Config::default(),
        running: true,
        routes: [None, None],
        outputs: vec![],
    }
}
fn approx(a: [f32; 3], b: [f32; 3]) {
    assert!(
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5),
        "{a:?} != {b:?}"
    );
}
fn frame() -> Frame {
    Frame {
        head: Pose::IDENTITY,
        wrist: Some(Pose::IDENTITY.at([0., 0., -0.5])),
        pointer: Pose::IDENTITY,
        trigger: false,
        grip: false,
    }
}
fn hands() -> Hands {
    Hands::new(
        Pose::IDENTITY,
        Pose::IDENTITY.at([0., 0., -1.]),
        Size::new(0.9, 0.225),
    )
}

#[test]
fn wrist_gaze_controls_beam_and_trigger_requires_release_on_same_button() {
    let rows = wrist::Layout::build(&Menu::default(), &view(), Lang::Ko);
    let mut hands = hands();
    let mut f = frame();
    f.pointer = Pose::IDENTITY.at([-0.035, 0.006, 0.]);
    let aimed = hands.tick(Some(f), None, &rows, 0.14);
    assert!(aimed.beam.is_some() && aimed.cursor.is_some());
    f.trigger = true;
    assert!(hands.tick(Some(f), None, &rows, 0.14).click.is_none());
    f.pointer = f.pointer.at([0.035, 0.006, 0.]);
    f.trigger = false;
    assert!(hands.tick(Some(f), None, &rows, 0.14).click.is_none());
    f.trigger = true;
    hands.tick(Some(f), None, &rows, 0.14);
    f.trigger = false;
    assert_eq!(
        hands.tick(Some(f), None, &rows, 0.14).click,
        Some(Command::Lane(1))
    );
    f.head = Pose::pitch(30.);
    assert!(hands.tick(Some(f), None, &rows, 0.14).beam.is_some());
    f.head = Pose::pitch(40.);
    assert!(hands.tick(Some(f), None, &rows, 0.14).beam.is_none());
    f.head = Pose::pitch(30.);
    assert!(hands.tick(Some(f), None, &rows, 0.14).beam.is_none());
    f.head = Pose::IDENTITY;
    f.wrist = Some(Pose::IDENTITY.at([0., 0., -1.4]));
    assert!(hands.tick(Some(f), None, &rows, 0.14).beam.is_none());
}

#[test]
fn panel_drag_is_relative_to_its_anchor_and_commits_only_on_release() {
    let rows = wrist::Layout::default();
    for panel in [Panel::Wrist, Panel::Captions] {
        let mut h = hands();
        let mut f = frame();
        if panel == Panel::Captions {
            f.wrist = None;
        }
        let tick = |h: &mut Hands, f| h.tick(Some(f), Some(panel), &rows, 0.14);
        tick(&mut h, f);
        f.grip = true;
        tick(&mut h, f);
        f.pointer = Pose::pitch(20.).at([0.12, 0.05, 0.]);
        if panel == Panel::Wrist {
            f.wrist = Some(Pose::pitch(-14.).at([0.09, 0.015, -0.55]));
        } else {
            f.head = Pose::pitch(7.).at([0.08, 0., 0.]);
        }
        assert!(tick(&mut h, f).placed.is_none());
        let (anchor, relative, original) = if panel == Panel::Wrist {
            (f.wrist.unwrap(), h.watch, [0., 0., -0.5])
        } else {
            (f.head, h.captions, [0., 0., -1.])
        };
        approx(
            anchor.compose(relative).pos(),
            f.pointer.offset(original).pos(),
        );
        f.grip = false;
        let saved = tick(&mut h, f).placed.unwrap();
        assert_eq!(saved.panel, panel);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.yaml");
        let mut cfg = Config::default();
        saved.save_to(&mut cfg);
        cfg.save(&path).unwrap();
        assert_eq!(panel.configured(&Config::load(&path).unwrap()), saved.pose);
        assert!(tick(&mut h, f).placed.is_none());
    }
}

#[test]
fn bottom_corners_resize_with_opposite_corner_fixed_and_save_both_axes() {
    let rows = wrist::Layout::default();
    for side in [-1., 1.] {
        let mut h = hands();
        let start = h.captions;
        let size = h.size;
        let mut f = frame();
        f.wrist = None;
        f.pointer = f.pointer.at([side * 0.44, -0.1, 0.]);
        let edit = Some(Panel::Captions);
        h.tick(Some(f), edit, &rows, 0.14);
        f.trigger = true;
        h.tick(Some(f), edit, &rows, 0.14);
        assert_eq!(h.size, size);
        f.pointer = f.pointer.at([side * 0.68, -0.24, 0.]);
        h.tick(Some(f), edit, &rows, 0.14);
        approx([h.size.width, h.size.height, 0.], [1.14, 0.365, 0.]);
        approx(
            start
                .offset([-side * size.width / 2., size.height / 2., 0.])
                .pos(),
            h.captions
                .offset([-side * h.size.width / 2., h.size.height / 2., 0.])
                .pos(),
        );
        f.trigger = false;
        let p = h.tick(Some(f), edit, &rows, 0.14).placed.unwrap();
        let mut cfg = Config::default();
        p.save_to(&mut cfg);
        assert_eq!(Size::configured(&cfg), h.size);
    }
}

#[test]
fn interrupted_drag_reverts_and_held_grip_does_not_regrab() {
    let mut h = hands();
    let original = h.captions;
    let mut f = frame();
    f.wrist = None;
    let edit = Some(Panel::Captions);
    h.tick(Some(f), edit, &wrist::Layout::default(), 0.14);
    f.grip = true;
    h.tick(Some(f), edit, &wrist::Layout::default(), 0.14);
    f.pointer = f.pointer.at([0.22, 0., 0.]);
    h.tick(Some(f), edit, &wrist::Layout::default(), 0.14);
    assert_ne!(h.captions, original);
    let lost = h.tick(None, edit, &wrist::Layout::default(), 0.14);
    assert!(lost.placed.is_none() && lost.cursor.is_none() && lost.beam.is_none());
    assert_eq!(h.captions, original);
    h.tick(Some(f), edit, &wrist::Layout::default(), 0.14);
    f.pointer = f.pointer.at([0.31, 0., 0.]);
    h.tick(Some(f), edit, &wrist::Layout::default(), 0.14);
    f.grip = false;
    assert!(
        h.tick(Some(f), edit, &wrist::Layout::default(), 0.14)
            .placed
            .is_none()
    );
    assert_eq!(h.captions, original);
    f.grip = true;
    h.tick(Some(f), None, &wrist::Layout::default(), 0.14);
    f.pointer = f.pointer.at([0.33, 0., 0.]);
    h.tick(Some(f), None, &wrist::Layout::default(), 0.14);
    assert_eq!(h.captions, original);
}

#[test]
fn dashboard_controls_hit_the_drawn_target_and_dispatch_on_release() {
    let mut v = view();
    let mut menu = Menu::default();
    let mut pointer = dashboard::Pointer::default();
    let pos = |x: f32, y: f32| {
        (
            x / dashboard::WIDTH as f32,
            1. - y / dashboard::HEIGHT as f32,
        )
    };
    let live = dashboard::layout(&menu, &v, Lang::Ko);
    let (x, right, y) = (550., 880., dashboard::HEIGHT as f32 - 154.);
    pointer.down(&live, x, y);
    assert!(pointer.focus(&live).1.is_some());
    assert!(pointer.up(&live, right, y).is_none());
    pointer.down(&live, right, y);
    assert!(matches!(
        menu.press(pointer.up(&live, right, y).unwrap(), &v, true),
        Some(Action::ToggleSubtitles)
    ));
    assert!(live.target_pixel(592., 154.).is_none());
    let (nav_x, nav_y) = (85., dashboard::HEIGHT as f32 - 238.);
    let (mic_x, mic_y) = (419., dashboard::HEIGHT as f32 - 324.);
    pointer.down(&live, nav_x, nav_y);
    assert!(pointer.up(&live, mic_x, mic_y).is_none()); // Two separate buttons open the mic page.
    assert!(live.hit(f32::NAN, y).is_none());
    let (x, y) = pos(85., 179.);
    let nav = live.hit(x, y).unwrap().clone();
    menu.press(nav, &v, true);
    assert_eq!(menu.dashboard, Screen::Captions);
    assert_eq!(menu.screen, Screen::Live);
    for (screen, direction, inbound) in [
        (Screen::Microphones, "outbound", false),
        (Screen::Captions, "inbound", true),
    ] {
        menu.dashboard = screen;
        let (x, y) = pos(750., 446.);
        v.cfg.data[direction]["speech_filter"] = false.into();
        let off = dashboard::layout(&menu, &v, Lang::Ko);
        assert!(off.hit(x, y).is_none());
        v.cfg.data[direction]["speech_filter"] = true.into();
        let on = dashboard::layout(&menu, &v, Lang::Ko);
        assert_eq!(on.hit(x, y), Some(&Command::Nemotron(inbound)));
        assert_eq!(
            on.hit(pos(430., 446.).0, y),
            Some(&Command::SpeechFilter(inbound))
        );
    }
    let panel = dashboard::layout(&menu, &v, Lang::Ko);
    for (x, y, cmd) in [
        (300., 298., Command::Original),
        (301., 211., Command::Font(-1)),
        (524., 211., Command::Font(1)),
        (646., 211., Command::Scale(-1)),
        (870., 211., Command::Scale(1)),
    ] {
        let (u, vv) = pos(x, y);
        assert_eq!(panel.hit(u, vv), Some(&cmd));
        let Some(Action::CaptionDisplay(change)) = menu.press(cmd.clone(), &v, true) else {
            panic!()
        };
        change.apply(&mut v.cfg);
        match cmd {
            Command::Original => assert!(v.cfg.b("/overlay/show_source")),
            Command::Font(-1) => assert_eq!(v.cfg.n("/overlay/font_size", 0.), 26.),
            Command::Font(1) => assert_eq!(v.cfg.n("/overlay/font_size", 0.), 27.),
            Command::Scale(-1) => assert_eq!(v.cfg.n("/overlay/width_m", 0.), 0.85),
            _ => assert!((v.cfg.n("/overlay/height_m", 0.) - 0.225).abs() < 1e-6),
        }
    }
    menu.press(Command::Open(Screen::Placement), &v, true);
    menu.press(Command::Edit(Panel::Captions), &v, true);
    assert_eq!(menu.edit, Some(Panel::Captions));
    menu.press(Command::Open(Screen::Live), &v, true);
    assert!(menu.edit.is_none());
}

#[test]
fn dashboard_pointer_uses_native_pixels_without_a_vertical_offset() {
    let v = view();
    let layout = dashboard::layout(&Menu::default(), &v, Lang::Ko);
    let mut pointer = dashboard::Pointer::default();
    let x = 384.;
    for (top_y, expected) in [(182., Some(Command::Lane(0))), (190., None)] {
        let y = dashboard::HEIGHT as f32 - top_y;
        pointer.down(&layout, x, y);
        assert_eq!(pointer.up(&layout, x, y), expected);
    }
    pointer.moved(x, f32::NAN);
    assert_eq!(pointer.focus(&layout), (None, None));
    pointer.clear();
    assert_eq!(pointer.focus(&layout), (None, None));
}

#[test]
fn dashboard_targets_fit_each_page_and_language() {
    use panel_ui::Part;
    let mut menu = Menu::default();
    menu.names = (1..=8).map(|n| format!("Microphone {n}")).collect();
    let v = view();
    for lang in [Lang::Ko, Lang::En, Lang::Ja, Lang::Zh] {
        for screen in [
            Screen::Live,
            Screen::Captions,
            Screen::Placement,
            Screen::Microphones,
            Screen::Interface,
            Screen::Voice { page: 0 },
            Screen::Languages {
                inbound: true,
                page: 0,
            },
            Screen::Languages {
                inbound: false,
                page: 1,
            },
        ] {
            menu.dashboard = screen;
            let layout = dashboard::layout(&menu, &v, lang);
            for part in &layout.parts {
                if let Part::Surface {
                    rect: b,
                    command: Some(cmd),
                    ..
                } = part
                {
                    assert!(
                        b.x >= 0
                            && b.y >= 0
                            && b.x + b.w <= dashboard::WIDTH
                            && b.y + b.h <= dashboard::HEIGHT
                    );
                    assert_eq!(
                        layout.hit(
                            (b.x + b.w / 2) as f32 / dashboard::WIDTH as f32,
                            1. - (b.y + b.h / 2) as f32 / dashboard::HEIGHT as f32
                        ),
                        Some(cmd)
                    );
                    let mut pointer = dashboard::Pointer::default();
                    let x = (b.x + b.w / 2) as f32;
                    let y = (dashboard::HEIGHT - b.y - b.h / 2) as f32;
                    pointer.down(&layout, x, y);
                    assert_eq!(pointer.up(&layout, x, y).as_ref(), Some(cmd));
                }
            }
        }
    }
}

#[test]
#[ignore = "writes localized release screenshots to VRCLT_PREVIEW_DIR"]
fn export_headset_previews() -> Result<()> {
    use crate::captions::{Row, TextRenderer};
    use eframe::egui::Color32;
    use image::{ColorType, save_buffer};

    let dir = std::path::PathBuf::from(
        std::env::var_os("VRCLT_PREVIEW_DIR").context("VRCLT_PREVIEW_DIR")?,
    );
    let renderer = panel_ui::Renderer::default();
    let text = TextRenderer::for_headset();
    let menu = Menu::default();
    for (locale, lang, words) in [
        (
            "en",
            Lang::En,
            ["Hello! Can you hear me?", "Yes, loud and clear."],
        ),
        (
            "ko",
            Lang::Ko,
            ["안녕하세요. 잘 들리세요?", "네, 잘 들려요."],
        ),
        (
            "ja",
            Lang::Ja,
            ["こんにちは。聞こえますか？", "はい、よく聞こえます。"],
        ),
        ("zh", Lang::Zh, ["你好，听得见吗？", "听得很清楚。"]),
    ] {
        let mut cfg = Config::default();
        cfg.data["provider"] = "soniox".into();
        cfg.data["inbound"]["target_language"] = locale.into();
        let view = View {
            cfg,
            running: true,
            routes: [None, None],
            outputs: vec![],
            cable: crate::setup::Cable::Ready,
        };
        let watch = wrist::Layout::build(&menu, &view, lang);
        let dash = dashboard::layout(&menu, &view, lang);
        for (panel, layout, width, height) in [
            ("wrist", &watch, wrist::WIDTH, wrist::HEIGHT),
            (
                "steamvr-dashboard",
                &dash,
                dashboard::WIDTH,
                dashboard::HEIGHT,
            ),
        ] {
            save_buffer(
                dir.join(format!("native-{panel}-{locale}.png")),
                &renderer.render(layout, None, None),
                width as u32,
                height as u32,
                ColorType::Rgba8,
            )?;
        }
        let rows: Vec<_> = words
            .iter()
            .enumerate()
            .map(|(i, s)| {
                Row::new(
                    &Caption {
                        text: (*s).into(),
                        speaker: (i + 1).to_string(),
                        ..Default::default()
                    },
                    lang,
                    false,
                    false,
                    false,
                )
            })
            .collect();
        let height = 160 * CAPTION_DENSITY;
        let bg = Color32::from_rgba_unmultiplied(10, 10, 12, 185);
        let mut image = vec![bg; W * height];
        text.draw(&rows, W, height, 27. * CAPTION_DENSITY as f32, &mut image);
        let pixels: Vec<_> = image
            .into_iter()
            .flat_map(|p| caption_rgba(p, bg, bg.to_srgba_unmultiplied()))
            .collect();
        save_buffer(
            dir.join(format!("native-subtitles-{locale}.png")),
            &pixels,
            W as u32,
            height as u32,
            ColorType::Rgba8,
        )?;
    }
    Ok(())
}
