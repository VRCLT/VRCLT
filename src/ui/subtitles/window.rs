use crate::captions::{Row, TextRenderer};
use crate::platform::{overlay_work_area, wide};
use eframe::egui::Color32;
use eframe::egui::{Rect, pos2, vec2};
use std::{mem, ptr};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

// A layered window composites the background and the glyphs with independent alpha values.
// It also keeps rendering independent of the OpenGL driver's window transparency support.
pub(super) struct CaptionWindow {
    hwnd: HWND,
    last: Option<(Vec<Row>, i32, i32, i32, bool)>,
    renderer: TextRenderer,
}
pub(super) fn pump_messages() {
    unsafe {
        let mut msg: MSG = mem::zeroed();
        while PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
pub(super) fn end_move(worker: &std::thread::JoinHandle<()>) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Threading::GetThreadId;
    unsafe extern "system" fn cancel(hwnd: HWND, _: LPARAM) -> i32 {
        unsafe {
            PostMessageW(hwnd, WM_CANCELMODE, 0, 0);
        }
        1
    }
    unsafe {
        EnumThreadWindows(GetThreadId(worker.as_raw_handle()), Some(cancel), 0);
    }
}
impl CaptionWindow {
    pub fn new(bounds: Rect) -> Option<Self> {
        let scale = overlay_work_area(bounds.center()).1;
        unsafe {
            let name = wide("VRCLT.DesktopSubtitles");
            let class = WNDCLASSW {
                style: CS_DBLCLKS,
                lpfnWndProc: Some(messages),
                lpszClassName: name.as_ptr(),
                hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                ..mem::zeroed()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOPMOST
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT,
                name.as_ptr(),
                wide("VRCLT Captions").as_ptr(),
                WS_POPUP,
                (bounds.left() * scale).round() as i32,
                (bounds.top() * scale).round() as i32,
                (bounds.width() * scale).round() as i32,
                (bounds.height() * scale).round() as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            (!hwnd.is_null()).then_some(Self {
                hwnd,
                last: None,
                renderer: TextRenderer::default(),
            })
        }
    }
    pub fn bounds(&self) -> Option<Rect> {
        unsafe {
            let mut r: RECT = mem::zeroed();
            if GetWindowRect(self.hwnd, &mut r) == 0 {
                return None;
            }
            let scale = GetDpiForWindow(self.hwnd).max(1) as f32 / 96.;
            Some(Rect::from_min_size(
                pos2(r.left as f32 / scale, r.top as f32 / scale),
                vec2(
                    (r.right - r.left) as f32 / scale,
                    (r.bottom - r.top) as f32 / scale,
                ),
            ))
        }
    }
    pub fn set_bounds(&self, bounds: Rect) {
        let scale = overlay_work_area(bounds.center()).1;
        unsafe {
            SetWindowPos(
                self.hwnd,
                ptr::null_mut(),
                (bounds.left() * scale).round() as i32,
                (bounds.top() * scale).round() as i32,
                (bounds.width() * scale).round() as i32,
                (bounds.height() * scale).round() as i32,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
    }
    pub fn finished_edit(&self) -> bool {
        unsafe { SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0) != 0 }
    }
    pub fn hide(&mut self) {
        if self.last.take().is_some() {
            unsafe {
                ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }
    pub fn display(&mut self, rows: &[Row], font: f32, editing: bool) {
        unsafe {
            let mut r: RECT = mem::zeroed();
            if GetWindowRect(self.hwnd, &mut r) == 0 {
                return;
            }
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let scale = GetDpiForWindow(self.hwnd).max(1) as f32 / 96.;
            let font = (font * scale).round() as i32;
            if self
                .last
                .as_ref()
                .is_some_and(|(s, old_w, old_h, old_font, old_edit)| {
                    s == rows && (*old_w, *old_h, *old_font, *old_edit) == (w, h, font, editing)
                })
            {
                return;
            }
            let flags = GetWindowLongW(self.hwnd, GWL_EXSTYLE) as u32;
            let next = if editing {
                flags & !WS_EX_TRANSPARENT
            } else {
                flags | WS_EX_TRANSPARENT
            };
            if flags != next {
                SetWindowLongW(self.hwnd, GWL_EXSTYLE, next as i32);
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
            if paint(&self.renderer, self.hwnd, w, h, rows, font, scale, editing) {
                if self.last.is_none() {
                    ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                }
                self.last = Some((rows.to_vec(), w, h, font, editing));
            }
        }
    }
}
impl Drop for CaptionWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn messages(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => MA_NOACTIVATE as _,
            WM_SETCURSOR => {
                let cursor = match l as u16 as u32 {
                    HTCAPTION => IDC_SIZEALL,
                    HTLEFT | HTRIGHT => IDC_SIZEWE,
                    HTTOP | HTBOTTOM => IDC_SIZENS,
                    HTTOPLEFT | HTBOTTOMRIGHT => IDC_SIZENWSE,
                    HTTOPRIGHT | HTBOTTOMLEFT => IDC_SIZENESW,
                    _ => return DefWindowProcW(hwnd, msg, w, l),
                };
                SetCursor(LoadCursorW(ptr::null_mut(), cursor));
                1
            }
            WM_NCHITTEST => {
                if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TRANSPARENT != 0 {
                    return HTTRANSPARENT as i32 as _;
                }
                let mut r: RECT = mem::zeroed();
                GetWindowRect(hwnd, &mut r);
                let (x, y) = (l as i16 as i32, (l >> 16) as i16 as i32);
                let edge = (12 * GetDpiForWindow(hwnd) / 96).max(6) as i32;
                match (
                    x < r.left + edge,
                    x >= r.right - edge,
                    y < r.top + edge,
                    y >= r.bottom - edge,
                ) {
                    (true, _, true, _) => HTTOPLEFT as _,
                    (_, true, true, _) => HTTOPRIGHT as _,
                    (true, _, _, true) => HTBOTTOMLEFT as _,
                    (_, true, _, true) => HTBOTTOMRIGHT as _,
                    (true, _, _, _) => HTLEFT as _,
                    (_, true, _, _) => HTRIGHT as _,
                    (_, _, true, _) => HTTOP as _,
                    (_, _, _, true) => HTBOTTOM as _,
                    _ => HTCAPTION as _,
                }
            }
            WM_GETMINMAXINFO => {
                let dpi = GetDpiForWindow(hwnd).max(96) as i32;
                let info = &mut *(l as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT {
                    x: 420 * dpi / 96,
                    y: 84 * dpi / 96,
                };
                0
            }
            WM_NCLBUTTONDBLCLK | WM_CLOSE => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 1);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

fn paint(
    renderer: &TextRenderer,
    hwnd: HWND,
    w: i32,
    h: i32,
    rows: &[Row],
    font_size: i32,
    scale: f32,
    editing: bool,
) -> bool {
    if w <= 0 || h <= 0 {
        return false;
    }
    unsafe {
        let dc = CreateCompatibleDC(ptr::null_mut());
        if dc.is_null() {
            return false;
        }
        let mut info: BITMAPINFO = mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..mem::zeroed()
        };
        let mut bits = ptr::null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if bitmap.is_null() {
            DeleteDC(dc);
            return false;
        }
        let previous = SelectObject(dc, bitmap);
        let background =
            Color32::from_rgba_unmultiplied(12, 14, 18, if editing { 190 } else { 160 });
        let mut image = vec![background; w as usize * h as usize];
        renderer.draw(rows, w as usize, h as usize, font_size as f32, &mut image);
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), image.len() * 4);
        let radius = (8. * scale).max(2.);
        for (i, (px, color)) in pixels.chunks_exact_mut(4).zip(image).enumerate() {
            let x = (i % w as usize) as i32;
            let y = (i / w as usize) as i32;
            let dx = x.min(w - 1 - x) as f32 + 0.5;
            let dy = y.min(h - 1 - y) as f32 + 0.5;
            let corner = (radius - dx).max(0.).hypot((radius - dy).max(0.));
            let from_corner = ((w - 1 - x) + (h - 1 - y)) as f32 / scale;
            if corner > radius {
                px.fill(0);
            } else if editing
                && (dx < 2.
                    || dy < 2.
                    || corner > radius - 2.
                    || (10.0..=12.).contains(&from_corner)
                    || (16.0..=18.).contains(&from_corner))
            {
                px.copy_from_slice(&[248, 204, 152, 255]);
            } else {
                px.copy_from_slice(&[color.b(), color.g(), color.r(), color.a()]);
            }
        }
        let size = SIZE { cx: w, cy: h };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let ok = UpdateLayeredWindow(
            hwnd,
            ptr::null_mut(),
            ptr::null(),
            &size,
            dc,
            &POINT { x: 0, y: 0 },
            0,
            &blend,
            ULW_ALPHA,
        ) != 0;
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        ok
    }
}
