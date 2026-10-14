//! UI Automation fallback for dialogs that expose no usable Win32 controls
//! (Revit and other WPF/Qt/custom-drawn file choosers).

use crate::platform::*;
use std::cell::RefCell;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

thread_local! {
    static UIA: RefCell<Option<IUIAutomation>> = const { RefCell::new(None) };
}

pub fn automation() -> Option<IUIAutomation> {
    UIA.with(|u| {
        let mut u = u.borrow_mut();
        if u.is_none() {
            *u = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() };
        }
        u.clone()
    })
}

pub struct El {
    pub el: IUIAutomationElement,
    pub ctype: UIA_CONTROLTYPE_ID,
    pub name: String,
    pub aid: String,
    pub class: String,
    pub value: String,
}

fn variant_str(v: &VARIANT) -> String {
    unsafe {
        let inner = &v.Anonymous.Anonymous;
        if inner.vt == windows::Win32::System::Variant::VT_BSTR {
            inner.Anonymous.bstrVal.to_string()
        } else {
            String::new()
        }
    }
}

fn bstr(v: Result<BSTR>) -> String {
    v.map(|b| b.to_string()).unwrap_or_default()
}

/// All descendants with key properties fetched in one cross-process round trip.
pub fn elements(root: HWND, limit: usize) -> Vec<El> {
    let mut out = Vec::new();
    let Some(ua) = automation() else { return out };
    unsafe {
        let Ok(el) = ua.ElementFromHandle(root) else { return out };
        let Ok(cr) = ua.CreateCacheRequest() else { return out };
        for p in [
            UIA_NamePropertyId,
            UIA_ControlTypePropertyId,
            UIA_AutomationIdPropertyId,
            UIA_ClassNamePropertyId,
            UIA_ValueValuePropertyId,
        ] {
            let _ = cr.AddProperty(p);
        }
        let Ok(cond) = ua.CreateTrueCondition() else { return out };
        let Ok(arr) = el.FindAllBuildCache(TreeScope_Descendants, &cond, &cr) else { return out };
        let n = arr.Length().unwrap_or(0).min(limit as i32);
        for i in 0..n {
            let Ok(e) = arr.GetElement(i) else { continue };
            let value = e
                .GetCachedPropertyValue(UIA_ValueValuePropertyId)
                .ok()
                .map(|v: VARIANT| variant_str(&v))
                .unwrap_or_default();
            out.push(El {
                ctype: e.CachedControlType().unwrap_or_default(),
                name: bstr(e.CachedName()),
                aid: bstr(e.CachedAutomationId()),
                class: bstr(e.CachedClassName()),
                value,
                el: e,
            });
        }
    }
    out
}

fn norm(s: &str) -> String {
    s.replace('&', "").trim().trim_end_matches(':').trim().to_lowercase()
}

const NAME_WORDS: &[&str] = &[
    "file name", "filename", "dateiname", "nom du fichier", "nom de fichier", "nombre de archivo", "nome file",
    "nome do arquivo", "bestandsnaam", "nazwa pliku", "název souboru", "имя файла", "ファイル名", "文件名", "檔案名稱", "파일 이름",
];

fn find_filename(els: &[El]) -> Option<&El> {
    let editable = |e: &&El| e.ctype == UIA_EditControlTypeId || e.ctype == UIA_ComboBoxControlTypeId;
    els.iter()
        .filter(editable)
        .find(|e| e.ctype == UIA_EditControlTypeId && (e.aid == "1148" || e.aid == "1001"))
        .or_else(|| {
            els.iter().filter(editable).find(|e| {
                let n = norm(&e.name);
                NAME_WORDS.iter().any(|w| n.starts_with(w))
            })
        })
        .or_else(|| els.iter().find(|e| e.ctype == UIA_EditControlTypeId && norm(&e.name).contains("name")))
}

const OK_WORDS: &[&str] = &[
    "open", "save", "ok", "select", "link", "import", "öffnen", "speichern", "ouvrir", "enregistrer", "abrir", "guardar",
    "apri", "salva", "openen", "opslaan", "otwórz", "zapisz", "открыть", "сохранить", "開く", "保存", "打开", "열기", "저장",
];

fn find_ok(els: &[El]) -> Option<&El> {
    let buttons = || els.iter().filter(|e| e.ctype == UIA_ButtonControlTypeId || e.ctype == UIA_SplitButtonControlTypeId);
    buttons()
        .find(|e| e.aid == "1")
        .or_else(|| buttons().find(|e| OK_WORDS.contains(&norm(&e.name).as_str())))
}

pub fn filename_value(root: HWND) -> Option<String> {
    let els = elements(root, 4000);
    let f = find_filename(&els)?;
    unsafe {
        let vp: IUIAutomationValuePattern = f.el.GetCurrentPatternAs(UIA_ValuePatternId).ok()?;
        Some(vp.CurrentValue().ok()?.to_string())
    }
}

fn set_filename(root: HWND, text: &str) -> Option<String> {
    let els = elements(root, 4000);
    let f = find_filename(&els)?;
    unsafe {
        let vp: IUIAutomationValuePattern = f.el.GetCurrentPatternAs(UIA_ValuePatternId).ok()?;
        let old = vp.CurrentValue().map(|b| b.to_string()).unwrap_or_default();
        vp.SetValue(&BSTR::from(text)).ok()?;
        Some(old)
    }
}

fn send_enter() {
    let key = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_RETURN,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                ..Default::default()
            },
        },
    };
    unsafe {
        SendInput(&[key(false), key(true)], std::mem::size_of::<INPUT>() as i32);
    }
}

pub fn navigate(root: HWND, path: &str, is_file: bool) -> bool {
    let els = elements(root, 4000);
    let Some(f) = find_filename(&els) else { return false };
    let old = unsafe {
        let Ok(vp) = f.el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) else { return false };
        let old = vp.CurrentValue().map(|b| b.to_string()).unwrap_or_default();
        if vp.SetValue(&BSTR::from(path)).is_err() {
            return false;
        }
        old
    };
    let invoked = find_ok(&els)
        .and_then(|b| unsafe { b.el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId).ok() })
        .map(|ip| unsafe { ip.Invoke().is_ok() })
        .unwrap_or(false);
    if !invoked {
        unsafe {
            let _ = SetForegroundWindow(root);
            let _ = f.el.SetFocus();
        }
        send_enter();
    }
    if !is_file && !old.trim().is_empty() && find_dir_in_text(&old).is_none() {
        let r = root.0 as isize;
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            std::thread::sleep(std::time::Duration::from_millis(350));
            set_filename(HWND(r as *mut _), &old);
        });
    }
    true
}

/// Best-effort current folder: address bars first, then combo/edit values,
/// then any element text that names an existing absolute directory.
pub fn current_dir(root: HWND) -> Option<String> {
    let els = elements(root, 4000);
    let fname = find_filename(&els).map(|e| e.value.clone());
    let addr = els.iter().filter(|e| {
        let n = norm(&e.name);
        n.starts_with("address") || n.contains("look in") || n.contains("save in") || e.class.contains("Breadcrumb")
    });
    for e in addr {
        if let Some(d) = find_dir_in_text(&e.name).or_else(|| find_dir_in_text(&e.value)) {
            return Some(d);
        }
    }
    for e in &els {
        if (e.ctype == UIA_ComboBoxControlTypeId || e.ctype == UIA_EditControlTypeId)
            && Some(&e.value) != fname.as_ref()
        {
            if let Some(d) = find_dir_in_text(&e.value) {
                return Some(d);
            }
        }
    }
    None
}
