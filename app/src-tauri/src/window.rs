// 透明置顶窗口 + 跨虚拟桌面 pin + 任务栏 fight-back
// Windows 专属实现（docs/tauri-shell.md §跨平台与 UIA 边界）：
// 非 Windows 构建不编译本模块的 Win32 体（winvd/windows 依赖也仅 cfg(windows) 目标拉入）。
use tauri::WebviewWindow;

#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
};

#[cfg(windows)]
pub fn init_window(window: &WebviewWindow) {
    let raw = window.hwnd().expect("hwnd").0 as *mut core::ffi::c_void;

    // ① 跨虚拟桌面 pin
    if let Err(err) = winvd::pin_window(HWND(raw)) {
        eprintln!("winvd pin_window: {err:?}");
    }

    // ② 任务栏 fight-back：每 500ms 安静地重申 TOPMOST
    let hwnd_val = raw as isize;
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(500));
        unsafe {
            let _ = SetWindowPos(
                HWND(hwnd_val as *mut _),
                HWND_TOPMOST,
                0, 0, 0, 0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            );
        }
    });
}

/// 非 Windows：alwaysOnTop 由 tauri.conf.json 声明（Tauri 跨平台处理）；
/// 无跨虚拟桌面 pin / TOPMOST 重申（Hook 驱动核心体验不依赖这些 Windows 增强）
#[cfg(not(windows))]
pub fn init_window(_window: &WebviewWindow) {}
