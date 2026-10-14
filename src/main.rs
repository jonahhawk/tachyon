#![windows_subsystem = "windows"]

mod app;
mod gfx;
mod icons;
mod menu;
mod model;
mod platform;
mod settings_ui;
mod target;

use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::w;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--settings") {
        settings_ui::run(&args[1..]);
        return;
    }
    unsafe {
        let _mutex = CreateMutexW(None, true, w!("Local\\TachyonHost"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // Already running: launching again opens Settings.
            platform::launch_settings(&[]);
            return;
        }
    }
    if let Err(e) = app::main_loop() {
        let msg = platform::wide(&format!("Tachyon failed to start:\n{e}"));
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                None,
                windows::core::PCWSTR(msg.as_ptr()),
                w!("Tachyon"),
                windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
    }
}
