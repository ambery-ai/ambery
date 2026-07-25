// 透明置顶窗口 + 跨虚拟桌面 pin + 任务栏 fight-back
use tauri::WebviewWindow;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
};
use std::thread;
use std::time::Duration;

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
