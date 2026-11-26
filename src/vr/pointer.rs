use super::{Frame, Pose, check, textures::Textures};
use anyhow::Result;
use openvr_sys as vr;

#[derive(Default)]
pub struct Tracking {
    pub head_and_wrist: Option<(Pose, Pose)>,
    pub input: Option<Frame>,
}
pub fn input(sys: &vr::VR_IVRSystem_FnTable, hand: u32, other: u32, tilt: Pose) -> Tracking {
    let mut poses = [vr::TrackedDevicePose_t::default(); vr::k_unMaxTrackedDeviceCount as usize];
    let mut buttons = vr::VRControllerState_t::default();
    unsafe {
        sys.GetDeviceToAbsoluteTrackingPose.unwrap()(
            vr::ETrackingUniverseOrigin_TrackingUniverseStanding,
            0.,
            poses.as_mut_ptr(),
            poses.len() as u32,
        );
    }
    let get = |index: u32| {
        poses
            .get(index as usize)
            .filter(|p| p.bDeviceIsConnected && p.bPoseIsValid)
            .map(|p| Pose(p.mDeviceToAbsoluteTracking.m))
    };
    let head = get(0);
    let wrist = get(hand);
    let input = head.zip(get(other)).and_then(|(head, controller)| {
        let tracked = unsafe {
            sys.GetControllerState.unwrap()(other, &mut buttons, size_of_val(&buttons) as u32)
        };
        tracked.then(|| Frame {
            head,
            wrist,
            pointer: controller.compose(tilt),
            trigger: buttons.ulButtonPressed & (1 << vr::EVRButtonId_k_EButton_SteamVR_Trigger)
                != 0,
            grip: buttons.ulButtonPressed & (1 << vr::EVRButtonId_k_EButton_Grip) != 0,
        })
    });
    Tracking {
        head_and_wrist: head.zip(wrist),
        input,
    }
}
pub fn relative(
    ov: &vr::VR_IVROverlay_FnTable,
    handle: u64,
    device: u32,
    pose: Pose,
) -> Result<()> {
    check(unsafe {
        ov.SetOverlayTransformTrackedDeviceRelative.unwrap()(
            handle,
            device,
            &mut vr::HmdMatrix34_t { m: pose.0 },
        )
    })
}
pub struct Mark {
    handle: u64,
    visible: bool,
    width: f32,
}
impl Mark {
    pub fn new(
        ov: &vr::VR_IVROverlay_FnTable,
        textures: &mut Textures<'_>,
        handle: u64,
        beam: bool,
    ) -> Result<Self> {
        let (w, h) = if beam { (4, 512) } else { (32, 32) };
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let color = if beam {
                    [
                        123,
                        195,
                        255,
                        (235. * (0.3 + 0.7 * y as f32 / h as f32)) as u8,
                    ]
                } else {
                    let radius = ((x as f32 - 15.5).powi(2) + (y as f32 - 15.5).powi(2)).sqrt();
                    if radius < 4. {
                        [130, 206, 255, 255]
                    } else if (11.0..=14.0).contains(&radius) {
                        [255, 255, 255, 245]
                    } else {
                        [0, 0, 0, 0]
                    }
                };
                rgba.extend(color);
            }
        }
        textures.upload(handle, &rgba, w as u32, h as u32)?;
        check(unsafe { ov.SetOverlaySortOrder.unwrap()(handle, if beam { 200 } else { 201 }) })?;
        Ok(Self {
            handle,
            visible: false,
            width: 0.,
        })
    }
    pub fn show(
        &mut self,
        ov: &vr::VR_IVROverlay_FnTable,
        pose: Option<Pose>,
        width: f32,
    ) -> Result<()> {
        unsafe {
            if let Some(pose) = pose {
                if (width - self.width).abs() > 0.00002 {
                    check(ov.SetOverlayWidthInMeters.unwrap()(self.handle, width))?;
                    self.width = width;
                }
                check(ov.SetOverlayTransformAbsolute.unwrap()(
                    self.handle,
                    vr::ETrackingUniverseOrigin_TrackingUniverseStanding,
                    &mut vr::HmdMatrix34_t { m: pose.0 },
                ))?;
                if !self.visible {
                    check(ov.ShowOverlay.unwrap()(self.handle))?;
                    self.visible = true;
                }
            } else if self.visible {
                check(ov.HideOverlay.unwrap()(self.handle))?;
                self.visible = false;
            }
        }
        Ok(())
    }
}
