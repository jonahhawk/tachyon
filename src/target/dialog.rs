//! Win32 file dialogs: Vista-style IFileDialog, classic common dialogs, and
//! custom dialogs (Autodesk/Revit style) that label their filename box.

use crate::platform::*;
use std::cell::RefCell;
use std::collections::HashMap;
use windows::Win32::Foundation::*;
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Controls::Dialogs::CDM_GETFOLDERPATH;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
use windows::Win32::UI::WindowsAndMessaging::*;

const ID_FILENAME_COMBO: i32 = 0x47C; // cmb13
const ID_FILENAME_EDIT: i32 = 0x480; // edt1
const ID_LOOKIN: i32 = 0x471; // cmb2 (classic dialogs only)

/// Localized "File name" labels (lower-case, '&' stripped).
const FILENAME_LABELS: &[&str] = &[
    "file name", "filename", "file:", "name:", "dateiname", "nom du fichier", "nom de fichier", "nombre de archivo",
    "nombre:", "nome file", "nome do arquivo", "nome:", "bestandsnaam", "nazwa pliku", "název souboru", "имя файла",
    "ファイル名", "文件名", "檔案名稱", "파일 이름",
];

thread_local! {
    static LAST_NAV: RefCell<HashMap<isize, String>> = RefCell::new(HashMap::new());
}

pub fn remember_nav(root: HWND, path: &str) {
    LAST_NAV.with(|m| {
        m.borrow_mut().insert(root.0 as isize, path.to_string());
    });
}

pub fn last_nav(root: HWND) -> Option<String> {
    LAST_NAV.with(|m| m.borrow().get(&(root.0 as isize)).cloned())
}

fn ctrl_id(h: HWND) -> i32 {
    unsafe { GetDlgCtrlID(h) }
}

fn visible(h: HWND) -> bool {
    unsafe { IsWindowVisible(h).as_bool() }
}

fn parent(h: HWND) -> HWND {
    unsafe { GetParent(h).unwrap_or_default() }
}

/// Resolve a control (Edit, ComboBox, ComboBoxEx32) to its editable Edit.
fn edit_of(h: HWND) -> Option<HWND> {
    let c = class_name(h);
    if c == "Edit" || c.starts_with("RichEdit") {
        return Some(h);
    }
    if c.contains("ComboBox") {
        return children(h).into_iter().find(|&k| class_name(k) == "Edit");
    }
    None
}

fn is_filename_label(text: &str) -> bool {
    let t = text.replace('&', "").to_lowercase();
    let t = t.trim();
    !t.is_empty() && t.len() < 40 && FILENAME_LABELS.iter().any(|l| t.starts_with(l))
}

pub fn find_filename_edit(root: HWND) -> Option<HWND> {
    let kids = children(root);
    // 1. Standard control IDs (classic + Vista Open dialogs).
    for id in [ID_FILENAME_COMBO, ID_FILENAME_EDIT] {
        if let Some(e) = kids.iter().filter(|&&h| ctrl_id(h) == id && visible(h)).find_map(|&h| edit_of(h)) {
            return Some(e);
        }
    }
    // 2. Vista Save dialog: editable ComboBox hosted in a FloatNotifySink.
    for &h in &kids {
        if class_name(h) == "ComboBox" && visible(h) && class_name(parent(h)) == "FloatNotifySink" {
            if let Some(e) = edit_of(h) {
                return Some(e);
            }
        }
    }
    // 3. Custom dialogs: a "File name:" label followed by its input in Z-order.
    for &h in &kids {
        if class_name(h) != "Static" || !visible(h) {
            continue;
        }
        if !is_filename_label(&crate::target::window_text_fast(h)) {
            continue;
        }
        let mut n = h;
        for _ in 0..3 {
            n = match unsafe { GetWindow(n, GW_HWNDNEXT) } {
                Ok(x) if !x.is_invalid() => x,
                _ => break,
            };
            if let Some(e) = edit_of(n).filter(|&e| visible(e)) {
                return Some(e);
            }
        }
    }
    None
}

pub fn current_dir(root: HWND) -> Option<String> {
    let kids = children(root);
    // Vista-style: breadcrumb toolbar caption is "Address: C:\path".
    for &h in &kids {
        if class_name(h) == "ToolbarWindow32" && class_name(parent(h)) == "Breadcrumb Parent" {
            if let Some(d) = find_dir_in_text(&crate::target::window_text_fast(h)) {
                return Some(d);
            }
        }
    }
    // Classic common dialog: ask it directly (only when it really is one —
    // CDM_* values collide with arbitrary WM_USER messages elsewhere).
    if class_name(root) == "#32770" && kids.iter().any(|&h| ctrl_id(h) == ID_LOOKIN) {
        if let Some(d) = cdm_folder_path(root) {
            return Some(d);
        }
    }
    None
}

fn cdm_folder_path(root: HWND) -> Option<String> {
    const CAP: usize = 1024;
    unsafe {
        let proc = OpenProcess(PROCESS_VM_OPERATION | PROCESS_VM_READ | PROCESS_VM_WRITE, false, window_pid(root)).ok()?;
        let mem = VirtualAllocEx(proc, None, CAP * 2, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        let mut out = None;
        if !mem.is_null() {
            let mut r = 0usize;
            SendMessageTimeoutW(
                root,
                CDM_GETFOLDERPATH,
                WPARAM(CAP),
                LPARAM(mem as isize),
                SMTO_ABORTIFHUNG,
                500,
                Some(&mut r),
            );
            if (r as isize) > 1 {
                let mut buf = vec![0u16; CAP];
                if ReadProcessMemory(proc, mem, buf.as_mut_ptr() as *mut _, CAP * 2, None).is_ok() {
                    out = existing_dir(&from_wide(&buf));
                }
            }
            let _ = VirtualFreeEx(proc, mem, 0, MEM_RELEASE);
        }
        let _ = CloseHandle(proc);
        out
    }
}

fn press_ok(root: HWND, edit: HWND) {
    unsafe {
        if let Ok(btn) = GetDlgItem(Some(root), IDOK.0) {
            if visible(btn) {
                let _ = PostMessageW(
                    Some(root),
                    WM_COMMAND,
                    WPARAM(((BN_CLICKED as usize) << 16) | IDOK.0 as usize),
                    LPARAM(btn.0 as isize),
                );
                return;
            }
        }
        let _ = PostMessageW(Some(edit), WM_KEYDOWN, WPARAM(VK_RETURN.0 as usize), LPARAM(0x001C_0001));
        let _ = PostMessageW(Some(edit), WM_KEYUP, WPARAM(VK_RETURN.0 as usize), LPARAM(0xC01C_0001u32 as isize));
    }
}

/// Type `path` into the filename box and confirm. For folders the dialog
/// navigates; afterwards the user's original filename is put back.
pub fn navigate(root: HWND, path: &str, is_file: bool) -> bool {
    let Some(edit) = find_filename_edit(root) else { return false };
    let old = window_text(edit);
    set_window_text(edit, path);
    press_ok(root, edit);
    let keep = !is_file && !old.trim().is_empty() && find_dir_in_text(&old).is_none();
    if keep {
        let e = edit.0 as isize;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            set_window_text(HWND(e as *mut _), &old);
        });
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows::Win32::UI::Controls::Dialogs::*;

    static DLG: AtomicIsize = AtomicIsize::new(0);

    unsafe extern "system" fn hook(h: HWND, msg: u32, _: WPARAM, _: LPARAM) -> usize {
        if msg == WM_INITDIALOG {
            unsafe {
                let p = GetParent(h).unwrap();
                // Park the dialog off-screen so the test never covers the desktop.
                let _ = SetWindowPos(p, None, -32000, -32000, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                DLG.store(p.0 as isize, Ordering::SeqCst);
            }
        }
        0
    }

    /// Classic (hooked) common dialog end-to-end: `cargo test classic_dialog_live -- --ignored`
    #[test]
    #[ignore]
    fn classic_dialog_live() {
        std::thread::spawn(|| unsafe {
            let mut file = [0u16; 512];
            let init = wide(r"C:\Windows\Web");
            let mut ofn = OPENFILENAMEW {
                lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
                lpstrFile: windows::core::PWSTR(file.as_mut_ptr()),
                nMaxFile: file.len() as u32,
                lpstrInitialDir: windows::core::PCWSTR(init.as_ptr()),
                Flags: OFN_EXPLORER | OFN_ENABLEHOOK,
                lpfnHook: Some(hook),
                ..Default::default()
            };
            let _ = GetOpenFileNameW(&mut ofn);
        });
        let mut root = HWND::default();
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let h = DLG.load(Ordering::SeqCst);
            if h != 0 {
                root = HWND(h as *mut _);
                break;
            }
        }
        assert!(!root.is_invalid(), "dialog did not open");
        std::thread::sleep(std::time::Duration::from_millis(300));
        let t = crate::target::classify(root).expect("classic dialog is a target");
        assert_eq!(t.kind, crate::target::Kind::Dialog);
        let edit = find_filename_edit(root).expect("filename edit");
        set_window_text(edit, "keep-me.txt");
        assert_eq!(current_dir(root).as_deref(), Some(r"C:\Windows\Web"));
        assert!(navigate(root, r"C:\Windows\Media", false));
        let mut d = None;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            d = current_dir(root);
            if d.as_deref() == Some(r"C:\Windows\Media") {
                break;
            }
        }
        assert_eq!(d.as_deref(), Some(r"C:\Windows\Media"));
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(window_text(edit), "keep-me.txt", "typed filename restored");
        unsafe {
            let _ = PostMessageW(Some(root), WM_COMMAND, WPARAM(IDCANCEL.0 as usize), LPARAM(0));
        }
    }

    static VISTA: AtomicIsize = AtomicIsize::new(0);

    unsafe extern "system" fn cbt(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            let h = HWND(w.0 as *mut _);
            match code as u32 {
                HCBT_CREATEWND => {
                    let cw = &*(l.0 as *const CBT_CREATEWNDW);
                    let cs = &mut *cw.lpcs;
                    if cs.hwndParent.is_invalid() && class_name(h) == "#32770" {
                        cs.x = -32000;
                        cs.y = -32000;
                        VISTA.store(h.0 as isize, Ordering::SeqCst);
                    }
                }
                // Keep it parked and inactive.
                HCBT_MOVESIZE | HCBT_ACTIVATE if VISTA.load(Ordering::SeqCst) == h.0 as isize => return LRESULT(1),
                _ => {}
            }
            CallNextHookEx(None, code, w, l)
        }
    }

    /// Vista-style IFileDialog (via GetOpenFileName without hook):
    /// `cargo test vista_dialog_live -- --ignored`
    #[test]
    #[ignore]
    fn vista_dialog_live() {
        std::thread::spawn(|| unsafe {
            let tid = windows::Win32::System::Threading::GetCurrentThreadId();
            let _hk = SetWindowsHookExW(WH_CBT, Some(cbt), None, tid).unwrap();
            let mut file = [0u16; 512];
            let init = wide(r"C:\Windows\Web");
            let mut ofn = OPENFILENAMEW {
                lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
                lpstrFile: windows::core::PWSTR(file.as_mut_ptr()),
                nMaxFile: file.len() as u32,
                lpstrInitialDir: windows::core::PCWSTR(init.as_ptr()),
                Flags: OFN_EXPLORER,
                ..Default::default()
            };
            let _ = GetOpenFileNameW(&mut ofn);
        });
        let mut root = HWND::default();
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let h = VISTA.load(Ordering::SeqCst);
            if h != 0 {
                root = HWND(h as *mut _);
                break;
            }
        }
        assert!(!root.is_invalid(), "dialog did not open");
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let t = crate::target::classify(root).expect("vista dialog is a target");
        assert_eq!(t.kind, crate::target::Kind::Dialog);
        assert!(children(root).iter().any(|&h| class_name(h) == "DUIViewWndClassName"), "is vista style");
        let edit = find_filename_edit(root).expect("filename edit");
        set_window_text(edit, "keep-me.txt");
        assert_eq!(current_dir(root).as_deref(), Some(r"C:\Windows\Web"));
        assert!(navigate(root, r"C:\Windows\Media", false));
        let mut d = None;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            d = current_dir(root);
            if d.as_deref() == Some(r"C:\Windows\Media") {
                break;
            }
        }
        assert_eq!(d.as_deref(), Some(r"C:\Windows\Media"));
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert_eq!(window_text(edit), "keep-me.txt", "typed filename restored");
        // UIA path must also find the folder on this dialog.
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        assert_eq!(crate::target::uia::current_dir(root).as_deref(), Some(r"C:\Windows\Media"));
        // UIA navigation (the Revit/custom-dialog strategy) on the same dialog.
        assert!(crate::target::uia::navigate(root, r"C:\Windows\Web", false));
        let mut d = None;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            d = current_dir(root);
            if d.as_deref() == Some(r"C:\Windows\Web") {
                break;
            }
        }
        assert_eq!(d.as_deref(), Some(r"C:\Windows\Web"), "uia navigate");
        unsafe {
            let _ = PostMessageW(Some(root), WM_COMMAND, WPARAM(IDCANCEL.0 as usize), LPARAM(0));
        }
    }

    #[test]
    fn labels() {
        assert!(is_filename_label("File &name:"));
        assert!(is_filename_label("Dateiname:"));
        assert!(!is_filename_label("Files of type:"));
        assert!(!is_filename_label("Look in:"));
    }
}
