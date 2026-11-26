use super::*;
use crate::{
    captions::{Row, TextRenderer},
    labels::Lang,
    runtime::Caption,
    vr::{Command, Menu, Session, View, dashboard, panel_ui::Renderer, table, wrist},
};
use eframe::egui::Color32;
use std::{ffi::CString, thread, time::Duration};

// Run only against an existing SteamVR session, with private overlays never shown in the HMD.
#[test]
#[ignore = "requires a running SteamVR compositor and a Direct3D adapter"]
fn steamvr_repeated_wrist_and_caption_frames() -> Result<()> {
    ensure!(
        crate::platform::process_id("vrserver").is_some(),
        "Start SteamVR before this test"
    );
    let mut err = 0;
    unsafe { vr::VR_InitInternal(&mut err, vr::EVRApplicationType_VRApplication_Background) };
    ensure!(err == 0, "OpenVR initialization error {err}");
    let session = Session;
    let ov = unsafe { table::<vr::VR_IVROverlay_FnTable>(&session, vr::IVROverlay_Version)? };
    let sys = unsafe { table::<vr::VR_IVRSystem_FnTable>(&session, vr::IVRSystem_Version)? };
    let mut textures = Textures::new(sys, ov)?;
    let create = |suffix| -> Result<u64> {
        let name = CString::new(format!("vrclt.qa.{}.{suffix}", std::process::id()))?;
        let mut h = 0;
        unsafe {
            check(ov.CreateOverlay.unwrap()(
                name.as_ptr().cast_mut(),
                name.as_ptr().cast_mut(),
                &mut h,
            ))?;
        }
        Ok(h)
    };
    let watch = create("wrist")?;
    let sub = create("captions")?;
    let view = View {
        cable: crate::setup::Cable::Ready,
        cfg: Default::default(),
        running: true,
        routes: [None, None],
        outputs: vec![],
    };
    let layout = wrist::Layout::build(&Menu::default(), &view, Lang::Ko);
    let painter = Renderer::default();
    let wrists = [
        painter.render(&layout, None, None),
        painter.render(&layout, Some(&Command::Lane(0)), None),
    ];
    let ws = [wrist::WIDTH as u32, wrist::HEIGHT as u32];
    use crate::vr::{CAPTION_DENSITY, W};
    let cs = [W as u32, (160 * CAPTION_DENSITY) as u32];
    let renderer = TextRenderer::for_headset();
    let caption = |text: &str, partial, height| {
        let height = height * CAPTION_DENSITY;
        let row = Row::new(
            &Caption {
                text: text.into(),
                speaker: "1".into(),
                ..Default::default()
            },
            Lang::Ko,
            true,
            partial,
            false,
        );
        let mut img = vec![Color32::from_rgba_unmultiplied(10, 10, 12, 185); W * height];
        renderer.draw(&[row], W, height, 27. * CAPTION_DENSITY as f32, &mut img);
        img.into_iter()
            .flat_map(|p| p.to_srgba_unmultiplied())
            .collect::<Vec<_>>()
    };
    let captions = [
        caption("안녕", true, 160),
        caption("안녕하세요.", false, 160),
    ];
    ensure!(
        wrists[0] != wrists[1] && captions[0] != captions[1],
        "Missing highlight or caption change"
    );
    let menu = Menu::default();
    let dash_layout = dashboard::layout(&menu, &view, Lang::Ko);
    let dashboards = [
        painter.render(&dash_layout, None, None),
        painter.render(
            &dash_layout,
            Some(&Command::Lane(1)),
            Some(&Command::Lane(1)),
        ),
    ];
    let ds = [dashboard::WIDTH as u32, dashboard::HEIGHT as u32];
    let name = CString::new(format!("vrclt.qa.{}.dash", std::process::id()))?;
    let (mut dash, mut thumb) = (0, 0);
    unsafe {
        check(ov.CreateDashboardOverlay.unwrap()(
            name.as_ptr().cast_mut(),
            name.as_ptr().cast_mut(),
            &mut dash,
            &mut thumb,
        ))?;
        check(ov.SetOverlayFlag.unwrap()(
            dash,
            vr::VROverlayFlags_NoDashboardTab,
            true,
        ))?;
    }
    dashboard::configure(ov, dash)?;
    textures.upload(dash, &dashboards[0], ds[0], ds[1])?;
    let point = |x, y| -> Result<[f32; 3]> {
        let mut transform = vr::HmdMatrix34_t::default();
        check(unsafe {
            ov.GetTransformForOverlayCoordinates.unwrap()(
                dash,
                vr::ETrackingUniverseOrigin_TrackingUniverseStanding,
                vr::HmdVector2_t { v: [x, y] },
                &mut transform,
            )
        })?;
        Ok(transform.m.map(|row| row[3]))
    };
    let origin = point(0., 0.)?;
    let distance = |p: [f32; 3]| {
        p.into_iter()
            .zip(origin)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            .sqrt()
    };
    let x = distance(point(ds[0] as f32, 0.)?);
    let y = distance(point(0., ds[1] as f32)?);
    ensure!(
        (x / y - ds[0] as f32 / ds[1] as f32).abs() < 0.001
            && (x - dashboard::WIDTH_METERS).abs() < 0.001,
        "Dashboard input surface {x} x {y} does not match the submitted texture"
    );
    ensure!(
        dashboards[0][((ds[1] as usize - 1) * ds[0] as usize + ds[0] as usize / 2) * 4 + 3] > 0,
        "Transparent margin below dashboard"
    );
    if let Some(path) = std::env::var_os("VRCLT_DASHBOARD_PREVIEW") {
        image::save_buffer(path, &dashboards[0], ds[0], ds[1], image::ColorType::Rgba8)?;
    }
    let mut original = Vec::new();
    for n in 0..360 {
        for (panel, h, size, frames) in [
            (0, watch, ws, &wrists),
            (1, sub, cs, &captions),
            (2, dash, ds, &dashboards),
        ] {
            let pixels = &frames[n % 2];
            textures.upload(h, pixels, size[0], size[1])?;
            let texture = textures.surfaces[&h].texture.as_raw();
            if n == 0 {
                original.push(texture);
            }
            ensure!(
                texture == original[panel],
                "Texture was reallocated at frame {n}"
            );
            let previous = (n > 0).then(|| frames[1 - n % 2].as_slice());
            verify_frame(&textures, h, size, pixels, previous)
                .with_context(|| format!("Panel {panel} frame {n}"))?;
        }
        thread::sleep(Duration::from_millis(30));
    }
    let resized = caption("크기를 바꾼 자막", false, 240);
    let size = [W as u32, (240 * CAPTION_DENSITY) as u32];
    textures.upload(sub, &resized, size[0], size[1])?;
    verify_frame(&textures, sub, size, &resized, None)?;
    textures.upload(sub, &captions[0], cs[0], cs[1])?;
    verify_frame(&textures, sub, cs, &captions[0], None)?;
    unsafe {
        check(ov.DestroyOverlay.unwrap()(dash))?;
        check(ov.DestroyOverlay.unwrap()(watch))?;
        check(ov.DestroyOverlay.unwrap()(sub))?;
    }
    Ok(())
}

fn verify_frame(
    textures: &Textures,
    handle: u64,
    size: [u32; 2],
    expected: &[u8],
    previous: Option<&[u8]>,
) -> Result<()> {
    let ov = textures.overlays;
    let src = &textures.surfaces[&handle].texture;
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe {
        src.GetDesc(&mut desc);
    }
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.MiscFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    let mut staging = None;
    unsafe {
        textures
            .device
            .CreateTexture2D(&desc, None, Some(&mut staging))?;
    }
    let staging = staging.context("Missing readback texture")?;
    for _ in 0..15 {
        unsafe {
            let mut raw = std::ptr::null_mut();
            let (mut w, mut h, mut format, mut api, mut color) = (0, 0, 0, 0, 0);
            let mut bounds = std::mem::zeroed();
            check(ov.GetOverlayTexture.unwrap()(
                handle,
                &mut raw,
                src.as_raw(),
                &mut w,
                &mut h,
                &mut format,
                &mut api,
                &mut color,
                &mut bounds,
            ))?;
            // SteamVR owns this SRV. Release via OpenVR after retaining its resource.
            let srv = ID3D11ShaderResourceView::from_raw_borrowed(&raw)
                .context("Missing compositor texture view")?;
            let resource = srv.GetResource();
            check(ov.ReleaseNativeOverlayHandle.unwrap()(handle, raw))?;
            let resource = resource?;
            if [w, h] != size {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            textures.context.CopyResource(&staging, &resource);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            textures
                .context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let row_bytes = w as usize * 4;
            let matches = |frame: &[u8]| {
                frame
                    .chunks_exact(row_bytes)
                    .enumerate()
                    .all(|(row, bytes)| {
                        let start = mapped
                            .pData
                            .cast::<u8>()
                            .add(row * mapped.RowPitch as usize);
                        std::slice::from_raw_parts(start, row_bytes) == bytes
                    })
            };
            let current = matches(expected);
            let intact = current || previous.is_none_or(matches);
            textures.context.Unmap(&staging, 0);
            ensure!(
                intact,
                "Compositor returned a blank or incomplete frame between updates"
            );
            if current {
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    anyhow::bail!(
        "SteamVR did not return the expected RGBA frame at {}x{}",
        size[0],
        size[1]
    )
}
