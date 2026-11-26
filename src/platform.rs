use std::{collections::HashSet, iter, mem, ptr};
use windows_sys::Win32::{
    Foundation::*,
    System::{Diagnostics::ToolHelp::*, Threading::CreateMutexW},
    UI::WindowsAndMessaging::*,
};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(iter::once(0)).collect()
}
fn console_window_id() -> eframe::egui::Id {
    eframe::egui::Id::new("native_console_hwnd")
}
pub fn register_console(cc: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if let Ok(RawWindowHandle::Win32(handle)) = cc.window_handle().map(|w| w.as_raw()) {
        cc.egui_ctx
            .data_mut(|d| d.insert_temp(console_window_id(), handle.hwnd.get()));
    }
}
pub fn wake_console(ctx: &eframe::egui::Context) {
    ctx.request_repaint();
    // Winit defers normal redraw requests for a hidden window. Pump the queued VR
    // action on the console's thread without making the window visible or active.
    if let Some(raw) = ctx.data(|d| d.get_temp::<isize>(console_window_id())) {
        let hwnd = raw as HWND;
        unsafe {
            if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
                PostMessageW(hwnd, WM_PAINT, 0, 0);
            }
        }
    }
}
pub fn console_size() -> [f32; 2] {
    let area = primary_work_area();
    [
        (area.width() - 20.).clamp(780., 980.),
        (area.height() - 24.).clamp(540., 828.),
    ]
}
fn monitor_area(
    monitor: windows_sys::Win32::Graphics::Gdi::HMONITOR,
) -> Option<(eframe::egui::Rect, f32)> {
    use windows_sys::Win32::{Graphics::Gdi::*, UI::HiDpi::*};
    unsafe {
        let mut info: MONITORINFO = mem::zeroed();
        info.cbSize = mem::size_of::<MONITORINFO>() as u32;
        let (mut dx, mut dy) = (96, 96);
        if GetMonitorInfoW(monitor, &mut info) == 0
            || GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) != 0
        {
            return None;
        }
        let work = info.rcWork;
        let point = |x, y| {
            eframe::egui::pos2(
                x as f32 * 96. / dx.max(1) as f32,
                y as f32 * 96. / dy.max(1) as f32,
            )
        };
        Some((
            eframe::egui::Rect::from_min_max(
                point(work.left, work.top),
                point(work.right, work.bottom),
            ),
            dx.max(1) as f32 / 96.,
        ))
    }
}
pub fn primary_work_area() -> eframe::egui::Rect {
    use windows_sys::Win32::Graphics::Gdi::*;
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    monitor_area(monitor)
        .map(|(area, _)| area)
        .unwrap_or_else(|| {
            eframe::egui::Rect::from_min_size(
                eframe::egui::Pos2::ZERO,
                eframe::egui::vec2(1280., 736.),
            )
        })
}
pub fn overlay_work_area(center: eframe::egui::Pos2) -> (eframe::egui::Rect, f32) {
    use windows_sys::Win32::Graphics::Gdi::*;
    unsafe extern "system" fn visit(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        let (center, found) =
            unsafe { &mut *(data as *mut (eframe::egui::Pos2, Option<(eframe::egui::Rect, f32)>)) };
        if let Some(area) = monitor_area(monitor).filter(|(r, _)| r.contains(*center)) {
            *found = Some(area);
            return 0;
        }
        1
    }
    let mut search = (center, None::<(eframe::egui::Rect, f32)>);
    unsafe {
        EnumDisplayMonitors(
            ptr::null_mut(),
            ptr::null(),
            Some(visit),
            &mut search as *mut _ as LPARAM,
        )
    };
    search.1.unwrap_or_else(|| (primary_work_area(), 1.))
}
pub struct Handle(pub HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub fn single_instance(name: &str, focus_existing: bool) -> std::io::Result<Option<Handle>> {
    unsafe {
        let h = CreateMutexW(ptr::null(), 0, wide(name).as_ptr());
        if h.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let duplicate = GetLastError() == ERROR_ALREADY_EXISTS;
        let handle = Handle(h);
        if duplicate {
            let hwnd = FindWindowW(ptr::null(), wide("VRCLT").as_ptr());
            if focus_existing && !hwnd.is_null() {
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            }
            return Ok(None);
        }
        Ok(Some(handle))
    }
}
pub fn process_id(target: &str) -> Option<u32> {
    let target = target.trim().to_ascii_lowercase();
    if target.is_empty() {
        return None;
    }
    let name = if target.ends_with(".exe") {
        target
    } else {
        format!("{target}.exe")
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let h = Handle(snapshot);
        let mut entry: PROCESSENTRY32W = mem::zeroed();
        entry.dwSize = mem::size_of_val(&entry) as u32;
        let mut ok = Process32FirstW(h.0, &mut entry);
        let mut matches = Vec::new();
        while ok != 0 {
            let n = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..n]).eq_ignore_ascii_case(&name) {
                matches.push((entry.th32ProcessID, entry.th32ParentProcessID));
            }
            ok = Process32NextW(h.0, &mut entry);
        }
        let ids: HashSet<_> = matches.iter().map(|x| x.0).collect();
        matches
            .iter()
            .filter(|x| !ids.contains(&x.1))
            .max_by_key(|x| matches.iter().filter(|p| p.1 == x.0).count())
            .map(|x| x.0)
    }
}
pub fn locale() -> String {
    let mut buf = [0u16; 85];
    unsafe {
        windows_sys::Win32::Globalization::GetUserDefaultLocaleName(buf.as_mut_ptr(), 85);
    }
    String::from_utf16_lossy(&buf)
        .split(['-', '\0'])
        .next()
        .unwrap_or("en")
        .into()
}
