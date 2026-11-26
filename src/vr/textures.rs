use super::check;
#[cfg(test)]
mod tests;
use anyhow::{Context, Result, ensure};
use openvr_sys as vr;
use std::collections::HashMap;
use windows::{
    Win32::Graphics::{
        Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN},
        Direct3D11::*,
        Dxgi::{
            Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC},
            CreateDXGIFactory1, IDXGIFactory1,
        },
    },
    core::Interface,
};

struct Surface {
    texture: ID3D11Texture2D,
    size: [u32; 2],
}

/// Owned by the VR render thread for the lifetime of one OpenVR session.
pub(super) struct Textures<'a> {
    overlays: &'a vr::VR_IVROverlay_FnTable,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    surfaces: HashMap<u64, Surface>,
}
impl<'a> Textures<'a> {
    pub fn new(
        system: &vr::VR_IVRSystem_FnTable,
        overlays: &'a vr::VR_IVROverlay_FnTable,
    ) -> Result<Self> {
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
            let mut index = -1;
            system.GetDXGIOutputInfo.unwrap()(&mut index);
            let adapter = if index >= 0 {
                Some(factory.EnumAdapters(index as u32)?)
            } else {
                None
            };
            let driver = if adapter.is_some() {
                D3D_DRIVER_TYPE_UNKNOWN
            } else {
                D3D_DRIVER_TYPE_HARDWARE
            };
            let (mut device, mut context) = (None, None);
            D3D11CreateDevice(
                adapter.as_ref(),
                driver,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .context("Create SteamVR graphics device")?;
            Ok(Self {
                overlays,
                device: device.context("Missing D3D11 device")?,
                context: context.context("Missing D3D11 immediate context")?,
                surfaces: HashMap::new(),
            })
        }
    }
    pub fn upload(&mut self, handle: u64, rgba: &[u8], width: u32, height: u32) -> Result<()> {
        let pitch = width.checked_mul(4).context("Overlay row too wide")?;
        ensure!(
            width > 0
                && height > 0
                && (pitch as usize).checked_mul(height as usize) == Some(rgba.len()),
            "Invalid RGBA overlay dimensions"
        );
        let size = [width, height];
        if let Some(surface) = self.surfaces.get(&handle).filter(|s| s.size == size) {
            unsafe {
                self.context.UpdateSubresource(
                    &surface.texture,
                    0,
                    None,
                    rgba.as_ptr().cast(),
                    pitch,
                    0,
                );
            }
            return self.submit(handle, &surface.texture);
        }
        // Populate the replacement before swapping it in when the panel is resized.
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET).0 as u32,
            MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
            ..Default::default()
        };
        let initial = D3D11_SUBRESOURCE_DATA {
            pSysMem: rgba.as_ptr().cast(),
            SysMemPitch: pitch,
            SysMemSlicePitch: 0,
        };
        let mut texture = None;
        unsafe {
            self.device
                .CreateTexture2D(&desc, Some(&initial), Some(&mut texture))?;
        }
        let texture = texture.context("Missing SteamVR texture")?;
        self.submit(handle, &texture)?;
        self.surfaces.insert(handle, Surface { texture, size });
        Ok(())
    }
    fn submit(&self, handle: u64, texture: &ID3D11Texture2D) -> Result<()> {
        let mut frame = vr::Texture_t {
            handle: texture.as_raw(),
            eType: vr::ETextureType_TextureType_DirectX,
            eColorSpace: vr::EColorSpace_ColorSpace_Gamma,
        };
        unsafe {
            self.context.Flush();
            check(self.overlays.SetOverlayTexture.unwrap()(handle, &mut frame))
        }
        .context("Submit SteamVR texture")
    }
}
