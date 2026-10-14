//! Ctrl+Shift+middle-click: dump a window's Win32 + UIA structure so dialog
//! detection can be tuned for apps we can't test here (e.g. Revit versions).

use crate::platform::*;
use std::fmt::Write;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

fn win32_tree(out: &mut String, h: HWND, depth: usize) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    let style = unsafe { GetWindowLongW(h, GWL_STYLE) } as u32;
    let _ = writeln!(
        out,
        "{:indent$}{:?} class={:?} id=0x{:X} text={:?} vis={} style=0x{style:08X} rect=({},{},{},{})",
        "",
        h.0,
        class_name(h),
        unsafe { GetDlgCtrlID(h) },
        crate::target::window_text_fast(h),
        unsafe { IsWindowVisible(h).as_bool() },
        r.left,
        r.top,
        r.right,
        r.bottom,
        indent = depth * 2
    );
    // Direct children only, then recurse.
    let mut c = unsafe { GetWindow(h, GW_CHILD) }.ok();
    while let Some(k) = c.filter(|k| !k.is_invalid()) {
        win32_tree(out, k, depth + 1);
        c = unsafe { GetWindow(k, GW_HWNDNEXT) }.ok();
    }
}

pub fn dump(root: HWND) -> std::path::PathBuf {
    let mut s = String::new();
    let pid = window_pid(root);
    let _ = writeln!(s, "Tachyon inspect — process {} (pid {pid})", process_name(pid));
    let _ = writeln!(s, "classified as: {:?}\n", crate::target::classify(root).map(|t| t.kind));
    let _ = writeln!(s, "== Win32 ==");
    win32_tree(&mut s, root, 0);
    let _ = writeln!(s, "\n== Detection ==");
    let _ = writeln!(s, "filename edit: {:?}", super::dialog::find_filename_edit(root).map(|h| h.0));
    let _ = writeln!(s, "dialog dir:    {:?}", super::dialog::current_dir(root));
    let _ = writeln!(s, "uia dir:       {:?}", super::uia::current_dir(root));
    let _ = writeln!(s, "uia filename:  {:?}", super::uia::filename_value(root));
    let _ = writeln!(s, "\n== UI Automation ==");
    for e in super::uia::elements(root, 3000) {
        let _ = writeln!(
            s,
            "type={} aid={:?} class={:?} name={:?} value={:?}",
            e.ctype.0, e.aid, e.class, e.name, e.value
        );
    }
    let dir = crate::model::Config::dir();
    let _ = std::fs::create_dir_all(&dir);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let p = dir.join(format!("inspect-{}-{stamp}.txt", process_name(pid).trim_end_matches(".exe")));
    let _ = std::fs::write(&p, s);
    p
}
