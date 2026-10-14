//! Detecting supported windows under the cursor and driving them.
//!
//! `classify` runs inside the low-level mouse hook, so it must stay cheap:
//! only window-class / child-window inspection, no COM or UIA.

pub mod dialog;
pub mod explorer;
pub mod inspect;
pub mod uia;

use crate::platform::*;
use std::cell::RefCell;
use std::collections::HashMap;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Explorer,
    /// Win32 file dialog (common or custom) with a resolvable filename edit.
    Dialog,
    /// Child-less / custom-drawn dialog: driven entirely through UI Automation.
    Uia,
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub root: HWND,
    pub kind: Kind,
}

/// Processes whose owned dialogs are treated as file dialogs even when they
/// expose no Win32 controls (WPF/Qt/custom-drawn).
const DIALOG_APPS: &[&str] = &["revit.exe", "acad.exe", "navisworks.exe", "roamer.exe", "3dsmax.exe", "inventor.exe"];

/// Title keywords that mark an owned window as a file chooser.
const DIALOG_WORDS: &[&str] = &[
    "open", "save", "link", "import", "export", "browse", "select", "load", "insert", "attach", "öffnen",
    "speichern", "ouvrir", "enregistrer", "abrir", "guardar", "apri", "salva", "otwórz", "zapisz", "открыть",
    "сохранить", "開く", "保存", "打开", "열기", "저장",
];

thread_local! {
    static CACHE: RefCell<HashMap<isize, Option<Kind>>> = RefCell::new(HashMap::new());
}

pub fn root_of(h: HWND) -> HWND {
    unsafe { GetAncestor(h, GA_ROOT) }
}

/// Classify the window under the cursor. `None` = let the click through.
pub fn classify(hit: HWND) -> Option<Target> {
    if hit.is_invalid() {
        return None;
    }
    let root = root_of(hit);
    let root_cls = class_name(root);
    if root_cls.starts_with("Tachyon") {
        return None;
    }
    if root_cls == "CabinetWClass" || root_cls == "ExploreWClass" {
        // Only the content area (file list + nav pane). The XAML tab strip and
        // address bar keep their own middle-click behavior (close tab, etc).
        let mut h = hit;
        while !h.is_invalid() && h != root {
            let c = class_name(h);
            if c == "ShellTabWindowClass" || c == "SHELLDLL_DefView" || c == "SysTreeView32" {
                return Some(Target { root, kind: Kind::Explorer });
            }
            if c.contains("DesktopChildSiteBridge") || c.contains("Windows.UI") {
                return None;
            }
            h = unsafe { GetParent(h).unwrap_or_default() };
        }
        return None;
    }
    let key = root.0 as isize;
    if let Some(k) = CACHE.with(|c| c.borrow().get(&key).copied()) {
        // Window handles are recycled; re-validate cheaply.
        if k.is_none() || unsafe { IsWindow(Some(root)).as_bool() } {
            return k.map(|kind| Target { root, kind });
        }
    }
    let kind = classify_dialog(root, &root_cls);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 256 {
            c.clear();
        }
        c.insert(key, kind);
    });
    kind.map(|kind| Target { root, kind })
}

fn classify_dialog(root: HWND, cls: &str) -> Option<Kind> {
    let owned = unsafe { GetWindow(root, GW_OWNER) }.map(|h| !h.is_invalid()).unwrap_or(false);
    if cls != "#32770" && !owned {
        // Never hijack top-level app windows (Revit's main view uses middle-drag to pan).
        return None;
    }
    if dialog::find_filename_edit(root).is_some() {
        return Some(Kind::Dialog);
    }
    let app = process_name(window_pid(root));
    let known_app = DIALOG_APPS.contains(&app.as_str());
    let title = window_text_fast(root).to_lowercase();
    let titled = DIALOG_WORDS.iter().any(|w| title.contains(w));
    if titled && (known_app || cls == "#32770" || children(root).len() < 4) {
        return Some(Kind::Uia);
    }
    None
}

/// GetWindowTextW never blocks on a hung window for top-levels of other processes.
pub fn window_text_fast(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

impl Target {
    pub fn current_dir(&self) -> Option<String> {
        match self.kind {
            Kind::Explorer => explorer::current_dir(self.root),
            Kind::Dialog => dialog::current_dir(self.root).or_else(|| uia::current_dir(self.root)),
            Kind::Uia => uia::current_dir(self.root),
        }
        .or_else(|| dialog::last_nav(self.root))
    }

    /// Selected files (Explorer) or the typed/selected filename (dialogs), as full paths.
    pub fn selection(&self) -> Vec<String> {
        match self.kind {
            Kind::Explorer => explorer::selection(self.root),
            Kind::Dialog | Kind::Uia => {
                let name = match self.kind {
                    Kind::Dialog => dialog::find_filename_edit(self.root).map(window_text),
                    _ => uia::filename_value(self.root),
                };
                let Some(name) = name.filter(|s| !s.trim().is_empty()) else { return vec![] };
                let name = name.trim().trim_matches('"').to_string();
                if std::path::Path::new(&name).is_absolute() {
                    return vec![name];
                }
                self.current_dir()
                    .map(|d| vec![std::path::Path::new(&d).join(&name).to_string_lossy().into_owned()])
                    .unwrap_or_default()
            }
        }
    }

    /// Navigate to a directory, or open/choose a file. Returns false on failure.
    pub fn go(&self, path: &str, is_file: bool) -> bool {
        match self.kind {
            Kind::Explorer => {
                if is_file {
                    shell_open(path);
                    true
                } else {
                    explorer::navigate(self.root, path)
                }
            }
            Kind::Dialog => {
                let ok = dialog::navigate(self.root, path, is_file) || uia::navigate(self.root, path, is_file);
                if ok && !is_file {
                    dialog::remember_nav(self.root, path);
                }
                ok
            }
            Kind::Uia => {
                let ok = uia::navigate(self.root, path, is_file);
                if ok && !is_file {
                    dialog::remember_nav(self.root, path);
                }
                ok
            }
        }
    }
}
