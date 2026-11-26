use super::assets;
use anyhow::{Context, Result, anyhow, ensure};
use libloading::os::windows::{
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, Library,
};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    ptr,
};

type Handle = *mut c_void;
type Free = unsafe extern "C" fn(Handle);
struct Owned(Handle, Free);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { (self.1)(self.0) };
        }
    }
}
#[repr(C)]
struct Backend {
    name: *const c_char,
    device: i32,
    threads: i32,
}

// Signatures from audio.cpp v0.9.6 include/audiocpp.h. Handles are destroyed in
// reverse dependency order; the DLL remains loaded until the last handle is freed.
pub struct Model {
    request: Owned,
    session: Owned,
    _model: Owned,
    _registry: Owned,
    set_audio: unsafe extern "C" fn(Handle, *const f32, usize, i32, i32) -> i32,
    run: unsafe extern "C" fn(Handle, Handle, *mut Handle) -> i32,
    result_audio:
        unsafe extern "C" fn(Handle, *mut *const f32, *mut usize, *mut i32, *mut i32) -> i32,
    result_free: Free,
    error: unsafe extern "C" fn() -> *const c_char,
    _library: Library,
}
impl Model {
    pub fn load() -> Result<Self> {
        let dir = assets::prepare()?;
        let weights = CString::new(
            dir.join(assets::MODEL)
                .to_str()
                .context("Invalid model path")?,
        )?;
        unsafe {
            let library = Library::load_with_flags(
                dir.join("audiocpp.dll"),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )?;
            macro_rules! sym {
                ($name:literal, $ty:ty) => {
                    *library.get::<$ty>(concat!("audiocpp_", $name, "\0").as_bytes())?
                };
            }
            let error = sym!("last_error", unsafe extern "C" fn() -> *const c_char);
            let mut registry = Owned(ptr::null_mut(), sym!("registry_free", Free));
            check(
                sym!(
                    "registry_create",
                    unsafe extern "C" fn(*const c_char, *mut Handle) -> i32
                )(ptr::null_mut(), &mut registry.0),
                error,
            )?;
            let mut model = Owned(ptr::null_mut(), sym!("model_free", Free));
            check(
                sym!(
                    "model_load",
                    unsafe extern "C" fn(Handle, *const c_char, Handle, Handle, *mut Handle) -> i32
                )(
                    registry.0,
                    weights.as_ptr(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    &mut model.0,
                ),
                error,
            )?;
            let backend = Backend {
                name: c"vulkan".as_ptr(),
                device: 0,
                threads: 2,
            };
            let mut session = Owned(ptr::null_mut(), sym!("session_free", Free));
            check(
                sym!(
                    "session_create",
                    unsafe extern "C" fn(
                        Handle,
                        *const c_char,
                        *const c_char,
                        *const Backend,
                        Handle,
                        *mut Handle,
                    ) -> i32
                )(
                    model.0,
                    c"s2s".as_ptr(),
                    c"offline".as_ptr(),
                    &backend,
                    ptr::null_mut(),
                    &mut session.0,
                ),
                error,
            )?;
            let request = Owned(
                sym!("request_create", unsafe extern "C" fn() -> Handle)(),
                sym!("request_free", Free),
            );
            ensure!(
                !request.0.is_null(),
                "Out of memory creating RE-USE request"
            );
            Ok(Self {
                request,
                session,
                _model: model,
                _registry: registry,
                set_audio: sym!("request_set_audio", _),
                run: sym!("session_run", _),
                result_audio: sym!("result_audio", _),
                result_free: sym!("result_free", Free),
                error,
                _library: library,
            })
        }
    }
    pub fn enhance(&mut self, samples: &[f32], output: &mut [i16]) -> Result<()> {
        unsafe {
            check(
                (self.set_audio)(self.request.0, samples.as_ptr(), samples.len(), 16000, 1),
                self.error,
            )?;
            let mut result = Owned(ptr::null_mut(), self.result_free);
            check(
                (self.run)(self.session.0, self.request.0, &mut result.0),
                self.error,
            )?;
            let (mut raw, mut count, mut rate, mut channels) = (ptr::null(), 0, 0, 0);
            check(
                (self.result_audio)(result.0, &mut raw, &mut count, &mut rate, &mut channels),
                self.error,
            )?;
            ensure!(
                rate == 16000 && channels == 1 && count == output.len() && !raw.is_null(),
                "Invalid RE-USE output"
            );
            for (out, &v) in output
                .iter_mut()
                .zip(std::slice::from_raw_parts(raw, count))
            {
                ensure!(v.is_finite(), "Non-finite RE-USE output");
                *out = (v * 32768.).round() as i16;
            }
        }
        Ok(())
    }
}
fn check(status: i32, error: unsafe extern "C" fn() -> *const c_char) -> Result<()> {
    if status == 0 {
        return Ok(());
    }
    let message = unsafe { CStr::from_ptr(error()).to_string_lossy() };
    Err(anyhow!("RE-USE GPU ({status}): {message}"))
}
