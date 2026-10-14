//! Host process: hidden window, global mouse hook, hotkey, tray icon, and the
//! effect runner that performs cross-process work outside the `App` borrow.

use crate::gfx::Gfx;
use crate::menu::MENU_CLASS;
use crate::model::{Config, Id, Modifier};
use crate::platform::*;
use crate::target::{self, Target};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Mutex;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

const WM_APP_SHOW: u32 = WM_APP + 1;
const WM_APP_RESOLVE: u32 = WM_APP + 2;
const WM_APP_CLOSE: u32 = WM_APP + 3;
const WM_APP_TRAY: u32 = WM_APP + 4;
const WM_APP_PATHS: u32 = WM_APP + 5;
const WM_APP_DEACTIVATED: u32 = WM_APP + 6;
const HOTKEY_ID: i32 = 1;

const TRAY_SETTINGS: usize = 1;
const TRAY_PAUSE: usize = 2;
const TRAY_EXIT: usize = 3;

pub enum Effect {
    Close,
    Save,
    Resolve,
    Settings(Vec<String>),
    CheckPaths(Vec<String>),
    AddHere { folder: Id, shift: bool },
    Go { target: Option<Target>, path: String, is_file: bool, reveal: bool },
}

pub struct App {
    pub cfg: Config,
    pub gfx: Gfx,
    pub menu: Option<crate::menu::Menu>,
    pub exists: HashMap<String, bool>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    // Hook state lives outside `App` so the hook never contends for its borrow.
    static PAUSED: Cell<bool> = const { Cell::new(false) };
    static MODIFIER: Cell<Modifier> = const { Cell::new(Modifier::None) };
    static SWALLOW_UP: Cell<bool> = const { Cell::new(false) };
    static PENDING: Cell<Option<(Target, POINT, bool)>> = const { Cell::new(None) };
    static MENU_RECTS: RefCell<Vec<RECT>> = const { RefCell::new(Vec::new()) };
    static HOST: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    static WARNED_ELEVATED: Cell<bool> = const { Cell::new(false) };
}

static PATH_RESULTS: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());

pub fn set_menu_rects(v: Vec<RECT>) {
    MENU_RECTS.with(|r| *r.borrow_mut() = v);
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| match a.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

fn post_host(msg: u32) {
    let h = HOST.with(|h| h.get());
    unsafe {
        let _ = PostMessageW(Some(h), msg, WPARAM(0), LPARAM(0));
    }
}

pub fn clipboard_text() -> Option<String> {
    unsafe {
        OpenClipboard(None).ok()?;
        let mut out = None;
        if let Ok(h) = GetClipboardData(13 /* CF_UNICODETEXT */) {
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if !p.is_null() {
                let mut n = 0;
                while *p.add(n) != 0 {
                    n += 1;
                }
                out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
                let _ = GlobalUnlock(g);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

// ------------------------------------------------------------------- hook

fn modifier_ok(m: Modifier) -> bool {
    let down = |vk: VIRTUAL_KEY| unsafe { GetAsyncKeyState(vk.0 as i32) } < 0;
    match m {
        Modifier::None => !down(VK_CONTROL) && !down(VK_MENU),
        Modifier::Ctrl => down(VK_CONTROL),
        Modifier::Alt => down(VK_MENU),
        Modifier::Shift => down(VK_SHIFT),
    }
}

fn in_menu(pt: POINT) -> bool {
    MENU_RECTS.with(|r| {
        r.borrow().iter().any(|rc| pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom)
    })
}

unsafe extern "system" fn mouse_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let ms = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
        match wp.0 as u32 {
            WM_MBUTTONDOWN if !PAUSED.with(|p| p.get()) && !in_menu(ms.pt) => {
                let down = |vk: VIRTUAL_KEY| unsafe { GetAsyncKeyState(vk.0 as i32) } < 0;
                let inspect = down(VK_CONTROL) && down(VK_SHIFT);
                if inspect || modifier_ok(MODIFIER.with(|m| m.get())) {
                    let hit = unsafe { WindowFromPoint(ms.pt) };
                    let t = if inspect {
                        // Inspect any window, supported or not.
                        Some(target::classify(hit).unwrap_or(Target { root: target::root_of(hit), kind: target::Kind::Uia }))
                    } else {
                        target::classify(hit)
                    };
                    if let Some(t) = t {
                        SWALLOW_UP.with(|s| s.set(true));
                        PENDING.with(|p| p.set(Some((t, ms.pt, inspect))));
                        post_host(WM_APP_SHOW);
                        return LRESULT(1);
                    }
                }
            }
            WM_MBUTTONUP if SWALLOW_UP.with(|s| s.replace(false)) => return LRESULT(1),
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
                if !MENU_RECTS.with(|r| r.borrow().is_empty()) && !in_menu(ms.pt) {
                    post_host(WM_APP_CLOSE);
                }
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

// ------------------------------------------------------------------- effects

pub fn run(effects: Vec<Effect>) {
    let mut queue: std::collections::VecDeque<Effect> = effects.into();
    while let Some(e) = queue.pop_front() {
        match e {
            Effect::Close => {
                with_app(|a| a.close_menu());
            }
            Effect::Save => {
                with_app(|a| {
                    let _ = a.cfg.save();
                });
            }
            Effect::Resolve => post_host(WM_APP_RESOLVE),
            Effect::Settings(args) => launch_settings(&args),
            Effect::CheckPaths(paths) => {
                std::thread::spawn(move || {
                    let res: Vec<(String, bool)> =
                        paths.into_iter().map(|p| (p.clone(), std::path::Path::new(&p).exists())).collect();
                    PATH_RESULTS.lock().unwrap().extend(res);
                    if let Some(h) = host_window() {
                        unsafe {
                            let _ = PostMessageW(Some(h), WM_APP_PATHS, WPARAM(0), LPARAM(0));
                        }
                    }
                });
            }
            Effect::AddHere { folder, shift } => {
                let Some((t, dir)) = with_app(|a| a.menu.as_ref().map(|m| (m.target, m.cur_dir.clone()))).flatten() else {
                    continue;
                };
                let paths = match (t, shift) {
                    (Some(t), true) => t.selection(),
                    (Some(t), false) => dir.or_else(|| t.current_dir()).into_iter().collect(),
                    (None, _) => vec![],
                };
                with_app(|a| {
                    if paths.is_empty() {
                        // Location unknown: ask for a path inline, prefilled from the clipboard.
                        let clip = clipboard_text()
                            .map(|c| c.trim().trim_matches('"').to_string())
                            .filter(|c| std::path::Path::new(c).exists())
                            .unwrap_or_default();
                        a.start_editor(crate::menu::EditKind::Path(folder), clip);
                    } else {
                        a.add_paths(folder, paths);
                    }
                });
            }
            Effect::Go { target, path, is_file, reveal } => {
                let done = match target {
                    Some(t) if !reveal => unsafe {
                        let _ = SetForegroundWindow(t.root);
                        t.go(&path, is_file)
                    },
                    _ => false,
                };
                if !done {
                    if is_file && !reveal { shell_open(&path) } else { crate::platform::reveal(&path) }
                }
            }
        }
    }
}

// ------------------------------------------------------------------- windows

unsafe extern "system" fn menu_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_ACTIVATEAPP if wp.0 == 0 => {
            post_host(WM_APP_DEACTIVATED);
            return LRESULT(0);
        }
        WM_SETCURSOR => {
            unsafe {
                SetCursor(LoadCursorW(None, IDC_ARROW).ok());
            }
            return LRESULT(1);
        }
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP | WM_MOUSEWHEEL | WM_MOUSELEAVE | WM_TIMER
        | WM_KEYDOWN | WM_SYSKEYDOWN | WM_CHAR | WM_CAPTURECHANGED => {
            // Wheel/keys arrive at the focused root; menu_event routes by cursor/active level.
            if let Some(Some(fx)) = with_app(|a| a.menu_event(h, msg, wp, lp)) {
                run(fx);
                return LRESULT(0);
            }
        }
        _ => {}
    }
    unsafe { DefWindowProcW(h, msg, wp, lp) }
}

fn tray_add(host: HWND, add: bool) {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: host,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_APP_TRAY,
            hIcon: LoadIconW(Some(hinst.into()), PCWSTR(1 as _)).or_else(|_| LoadIconW(None, IDI_APPLICATION)).unwrap_or_default(),
            ..Default::default()
        };
        let tip = wide("Tachyon — middle-click in Explorer or a file dialog");
        nid.szTip[..tip.len().min(127)].copy_from_slice(&tip[..tip.len().min(127)]);
        let _ = Shell_NotifyIconW(if add { NIM_ADD } else { NIM_DELETE }, &nid);
    }
}

fn balloon(host: HWND, title: &str, text: &str) {
    unsafe {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: host,
            uID: 1,
            uFlags: NIF_INFO,
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        let t = wide(title);
        let b = wide(text);
        nid.szInfoTitle[..t.len().min(63)].copy_from_slice(&t[..t.len().min(63)]);
        nid.szInfo[..b.len().min(255)].copy_from_slice(&b[..b.len().min(255)]);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

fn tray_menu(host: HWND) {
    unsafe {
        let Ok(m) = CreatePopupMenu() else { return };
        let paused = PAUSED.with(|p| p.get());
        let _ = AppendMenuW(m, MF_STRING, TRAY_SETTINGS, w!("Settings"));
        let _ = AppendMenuW(m, if paused { MF_STRING | MF_CHECKED } else { MF_STRING }, TRAY_PAUSE, w!("Pause"));
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(m, MF_STRING, TRAY_EXIT, w!("Exit"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(host);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, None, host, None);
        let _ = DestroyMenu(m);
        match cmd.0 as usize {
            TRAY_SETTINGS => launch_settings(&[]),
            TRAY_PAUSE => PAUSED.with(|p| p.set(!paused)),
            TRAY_EXIT => {
                let _ = DestroyWindow(host);
            }
            _ => {}
        }
    }
}

fn apply_settings(host: HWND) {
    let (modifier, hotkey) = with_app(|a| (a.cfg.settings.trigger_modifier, a.cfg.settings.hotkey)).unwrap_or((Modifier::None, true));
    MODIFIER.with(|m| m.set(modifier));
    unsafe {
        let _ = UnregisterHotKey(Some(host), HOTKEY_ID);
        if hotkey {
            let _ = RegisterHotKey(Some(host), HOTKEY_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_SPACE.0 as u32);
        }
    }
}

fn show_for(t: Option<Target>, pt: POINT) {
    if let Some(t) = t {
        let pid = window_pid(t.root);
        if is_elevated(pid) && !self_elevated() && !WARNED_ELEVATED.with(|w| w.replace(true)) {
            balloon(
                HOST.with(|h| h.get()),
                "Tachyon",
                "That window runs as administrator. Run Tachyon as administrator to jump inside it.",
            );
        }
    }
    let t0 = std::time::Instant::now();
    if let Some(fx) = with_app(|a| a.open_menu(t, pt)) {
        if cfg!(debug_assertions) {
            eprintln!("popup shown in {:?}", t0.elapsed());
        }
        run(fx);
    }
}

unsafe extern "system" fn host_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_APP_SHOW => {
            if let Some((t, pt, inspect)) = PENDING.with(|p| p.take()) {
                if inspect {
                    let p = target::inspect::dump(t.root);
                    shell_open(&p.to_string_lossy());
                } else {
                    show_for(Some(t), pt);
                }
            }
        }
        WM_HOTKEY => {
            let fg = unsafe { GetForegroundWindow() };
            let root = target::root_of(fg);
            let t = match class_name(root).as_str() {
                "CabinetWClass" | "ExploreWClass" => Some(Target { root, kind: target::Kind::Explorer }),
                _ => target::classify(root),
            };
            let mut pt = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut pt);
            }
            show_for(t, pt);
        }
        WM_TACHYON_PREVIEW => {
            let mut pt = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut pt);
            }
            show_for(None, pt);
        }
        WM_APP_RESOLVE => {
            if let Some(Some(t)) = with_app(|a| a.menu.as_ref().and_then(|m| m.target)) {
                let d = t.current_dir();
                with_app(|a| a.set_cur_dir(d));
            }
        }
        WM_APP_CLOSE => {
            with_app(|a| a.close_menu());
        }
        WM_APP_DEACTIVATED => {
            let fg = unsafe { GetForegroundWindow() };
            with_app(|a| {
                if !a.menu_hwnds().contains(&fg) && a.menu.as_ref().map(|m| m.drag.is_none()).unwrap_or(false) {
                    a.close_menu();
                }
            });
        }
        WM_APP_PATHS => {
            let res: Vec<(String, bool)> = std::mem::take(&mut *PATH_RESULTS.lock().unwrap());
            with_app(|a| {
                let any_missing = res.iter().any(|(_, e)| !e);
                a.exists.extend(res);
                if any_missing {
                    a.repaint_all();
                }
            });
        }
        WM_TACHYON_RELOAD => {
            with_app(|a| {
                if let Some(c) = Config::load_from(&Config::file()) {
                    a.cfg = c;
                    a.gfx.clear_icons();
                    a.exists.clear();
                    a.rebuild();
                }
            });
            apply_settings(h);
        }
        WM_APP_TRAY => match (lp.0 as u32) & 0xFFFF {
            WM_RBUTTONUP | WM_CONTEXTMENU => tray_menu(h),
            WM_LBUTTONUP => launch_settings(&[]),
            _ => {}
        },
        WM_SETTINGCHANGE => {
            // Theme flips (System theme) apply on next open.
            with_app(|a| a.gfx.clear_icons());
        }
        WM_DESTROY => {
            tray_add(h, false);
            unsafe { PostQuitMessage(0) };
        }
        m if m == TASKBAR_CREATED.with(|t| t.get()) && m != 0 => tray_add(h, true),
        _ => return unsafe { DefWindowProcW(h, msg, wp, lp) },
    }
    LRESULT(0)
}

pub fn main_loop() -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let cursor = LoadCursorW(None, IDC_ARROW)?;
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(host_proc),
            hInstance: hinst,
            lpszClassName: HOST_CLASS,
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(menu_proc),
            hInstance: hinst,
            lpszClassName: MENU_CLASS,
            hCursor: cursor,
            hbrBackground: HBRUSH::default(),
            style: CS_DBLCLKS,
            ..Default::default()
        });
        let host = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            HOST_CLASS,
            w!("Tachyon"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst),
            None,
        )?;
        HOST.with(|h| h.set(host));
        TASKBAR_CREATED.with(|t| t.set(RegisterWindowMessageW(w!("TaskbarCreated"))));

        let cfg = Config::load();
        if !Config::file().exists() {
            let _ = cfg.save();
        }
        if cfg.settings.autostart {
            // Migrates older Run-key installs and follows the exe if it moved.
            crate::platform::set_autostart(true);
        }
        let gfx = Gfx::new()?;
        APP.with(|a| *a.borrow_mut() = Some(App { cfg, gfx, menu: None, exists: HashMap::new() }));
        // Warm caches so the very first popup is as fast as the rest.
        let _ = crate::icons::exists("lucide/folder");
        with_app(|a| {
            a.gfx.ensure_fonts(&a.cfg.settings.font_family, a.cfg.settings.font_size);
            let _ = a.gfx.measure("Tachyon", &a.gfx.text);
        });

        apply_settings(host);
        tray_add(host, true);
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(hinst), 0)?;

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
        CoUninitialize();
    }
    Ok(())
}
