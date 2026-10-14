//! Small Win32 helpers shared by every module.

use std::path::Path;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

pub const HOST_CLASS: PCWSTR = w!("TachyonHost");
pub const SETTINGS_TITLE: &str = "Tachyon Settings";
/// Posted to the host window: reload config from disk.
pub const WM_TACHYON_RELOAD: u32 = WM_APP + 40;
/// Posted to the host window: show a preview popup at the cursor.
pub const WM_TACHYON_PREVIEW: u32 = WM_APP + 41;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Window text via WM_GETTEXT with a timeout so a hung target can't stall us.
pub fn window_text(h: HWND) -> String {
    unsafe {
        let mut len = 0usize;
        let r = SendMessageTimeoutW(h, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG, 200, Some(&mut len));
        if r.0 == 0 || len == 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len + 1];
        let mut got = 0usize;
        SendMessageTimeoutW(
            h,
            WM_GETTEXT,
            WPARAM(buf.len()),
            LPARAM(buf.as_mut_ptr() as isize),
            SMTO_ABORTIFHUNG,
            200,
            Some(&mut got),
        );
        String::from_utf16_lossy(&buf[..got.min(len)])
    }
}

pub fn set_window_text(h: HWND, s: &str) {
    let w = wide(s);
    unsafe {
        SendMessageTimeoutW(h, WM_SETTEXT, WPARAM(0), LPARAM(w.as_ptr() as isize), SMTO_ABORTIFHUNG, 500, None);
    }
}

pub fn children(parent: HWND) -> Vec<HWND> {
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        unsafe { (*(l.0 as *mut Vec<HWND>)).push(h) };
        TRUE
    }
    let mut v: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumChildWindows(Some(parent), Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

pub fn window_pid(h: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

/// Lower-case exe file name of a process, e.g. "revit.exe".
pub fn process_name(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit('\\').next().unwrap_or("").to_lowercase()
    }
}

pub fn system_uses_light_theme() -> bool {
    let mut v = 0u32;
    let mut sz = 4u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut _),
            Some(&mut sz),
        )
    };
    r.is_ok() && v == 1
}

/// `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\Tachyon.lnk`
fn startup_link() -> Option<std::path::PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Startup, KF_FLAG_DEFAULT, None).ok()?;
        let dir = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        Some(Path::new(&dir?).join("Tachyon.lnk"))
    }
}

/// Start with Windows via a Startup-folder shortcut (a Run-key entry from an
/// unsigned exe is a classic EDR trigger; a visible shortcut is not).
pub fn set_autostart(on: bool) {
    unsafe {
        // Clean up the Run value written by earlier versions.
        let _ = RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            w!("Tachyon"),
        );
        let Some(lnk) = startup_link() else { return };
        if !on {
            let _ = std::fs::remove_file(&lnk);
            return;
        }
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER) else { return };
        let exe = std::env::current_exe().unwrap_or_default();
        let exe_w = wide(&exe.to_string_lossy());
        let _ = link.SetPath(PCWSTR(exe_w.as_ptr()));
        if let Some(dir) = exe.parent() {
            let d = wide(&dir.to_string_lossy());
            let _ = link.SetWorkingDirectory(PCWSTR(d.as_ptr()));
        }
        let _ = link.SetDescription(w!("Tachyon"));
        if let Ok(pf) = link.cast::<IPersistFile>() {
            let l = wide(&lnk.to_string_lossy());
            let _ = pf.Save(PCWSTR(l.as_ptr()), true);
        }
    }
}

pub fn host_window() -> Option<HWND> {
    unsafe { FindWindowW(HOST_CLASS, PCWSTR::null()).ok() }
}

pub fn notify_host(msg: u32) {
    if let Some(h) = host_window() {
        unsafe {
            let _ = PostMessageW(Some(h), msg, WPARAM(0), LPARAM(0));
        }
    }
}

/// Launch (or focus) the settings process. `args` e.g. ["--icon", "42"].
pub fn launch_settings(args: &[String]) {
    unsafe {
        let title = wide(SETTINGS_TITLE);
        if args.is_empty() {
            if let Ok(h) = FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) {
                let _ = ShowWindow(h, SW_RESTORE);
                let _ = SetForegroundWindow(h);
                return;
            }
        }
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--settings").args(args);
    let _ = cmd.spawn();
}

pub fn shell_open(path: &str) {
    let p = wide(path);
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(p.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

/// Open an Explorer window showing `path` (a dir) or selecting it (a file).
pub fn reveal(path: &str) {
    let p = Path::new(path);
    if p.is_dir() {
        let arg = wide(&format!("\"{path}\""));
        unsafe {
            ShellExecuteW(None, w!("open"), w!("explorer.exe"), PCWSTR(arg.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL);
        }
    } else {
        let arg = wide(&format!("/select,\"{path}\""));
        unsafe {
            ShellExecuteW(None, w!("open"), w!("explorer.exe"), PCWSTR(arg.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL);
        }
    }
}

pub fn is_elevated(pid: u32) -> bool {
    use windows::Win32::Security::*;
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            // Can't even query it: almost certainly higher integrity than us.
            return true;
        };
        let mut tok = HANDLE::default();
        let mut elevated = false;
        if OpenProcessToken(h, TOKEN_QUERY, &mut tok).is_ok() {
            let mut e = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            if GetTokenInformation(
                tok,
                TokenElevation,
                Some(&mut e as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
            .is_ok()
            {
                elevated = e.TokenIsElevated != 0;
            }
            let _ = CloseHandle(tok);
        }
        let _ = CloseHandle(h);
        elevated
    }
}

pub fn self_elevated() -> bool {
    is_elevated(std::process::id())
}

/// Does `s` look like an absolute filesystem path that exists as a directory?
pub fn existing_dir(s: &str) -> Option<String> {
    let t = s.trim().trim_matches('"');
    let abs = (t.len() >= 2 && t.as_bytes()[1] == b':') || t.starts_with("\\\\");
    if abs && Path::new(t).is_dir() { Some(t.to_string()) } else { None }
}

/// Extract the first absolute path inside arbitrary text ("Address: C:\x" etc.)
/// that exists as a directory.
pub fn find_dir_in_text(s: &str) -> Option<String> {
    if let Some(d) = existing_dir(s) {
        return Some(d);
    }
    let b = s.as_bytes();
    for i in 0..b.len() {
        let drive = i + 2 < b.len() && b[i].is_ascii_alphabetic() && b[i + 1] == b':' && b[i + 2] == b'\\';
        let unc = i + 1 < b.len() && b[i] == b'\\' && b[i + 1] == b'\\';
        if drive || unc {
            if let Some(d) = existing_dir(&s[i..]) {
                return Some(d);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_dir_after_label() {
        let win = std::env::var("WINDIR").unwrap();
        assert_eq!(find_dir_in_text(&format!("Address: {win}")), Some(win.clone()));
        assert_eq!(find_dir_in_text("Adresse : nowhere"), None);
    }
}
