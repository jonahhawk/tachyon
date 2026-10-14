//! Explorer windows via IShellWindows (Windows 11 tab aware).

use crate::platform::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Variant::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

/// The IWebBrowser2/IShellBrowser pair for the *active tab* of `root`.
fn browser(root: HWND) -> Option<(IWebBrowser2, Option<IShellBrowser>)> {
    unsafe {
        let sw: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
        // The active tab's ShellTabWindowClass is the first one in Z-order.
        let tab = FindWindowExW(Some(root), None, w!("ShellTabWindowClass"), PCWSTR::null()).ok();
        let n = sw.Count().ok()?;
        let mut fallback = None;
        for i in 0..n {
            let Ok(disp) = sw.Item(&VARIANT::from(i)) else { continue };
            let Ok(wb) = disp.cast::<IWebBrowser2>() else { continue };
            let Ok(h) = wb.HWND() else { continue };
            if h.0 as isize != root.0 as isize {
                continue;
            }
            let sb = wb
                .cast::<IServiceProvider>()
                .ok()
                .and_then(|sp| sp.QueryService::<IShellBrowser>(&SID_STopLevelBrowser).ok());
            match (tab, &sb) {
                (Some(t), Some(b)) => {
                    if b.GetWindow().ok() == Some(t) {
                        return Some((wb, sb));
                    }
                    fallback.get_or_insert((wb, sb));
                }
                _ => return Some((wb, sb)),
            }
        }
        fallback
    }
}

fn folder_view(sb: &IShellBrowser) -> Option<IFolderView2> {
    unsafe { sb.QueryActiveShellView().ok()?.cast::<IFolderView2>().ok() }
}

pub fn current_dir(root: HWND) -> Option<String> {
    let (wb, sb) = browser(root)?;
    unsafe {
        if let Some(fv) = sb.as_ref().and_then(folder_view) {
            if let Ok(pf) = fv.GetFolder::<IPersistFolder2>() {
                if let Ok(pidl) = pf.GetCurFolder() {
                    let mut buf = [0u16; 1024];
                    let ok = SHGetPathFromIDListEx(pidl, &mut buf, GPFIDL_DEFAULT).as_bool();
                    CoTaskMemFree(Some(pidl as *const _));
                    if ok {
                        return existing_dir(&from_wide(&buf));
                    }
                }
            }
        }
        // Fallback: file:/// URL of the location.
        let url = wb.LocationURL().ok()?.to_string();
        if url.is_empty() {
            return None;
        }
        let u = wide(&url);
        let mut out = [0u16; 1024];
        let mut len = out.len() as u32;
        PathCreateFromUrlW(PCWSTR(u.as_ptr()), PWSTR(out.as_mut_ptr()), &mut len, 0).ok()?;
        existing_dir(&from_wide(&out))
    }
}

pub fn selection(root: HWND) -> Vec<String> {
    let mut v = Vec::new();
    let Some((_, Some(sb))) = browser(root) else { return v };
    unsafe {
        let Some(fv) = folder_view(&sb) else { return v };
        let Ok(arr) = fv.GetSelection(false) else { return v };
        let n = arr.GetCount().unwrap_or(0);
        for i in 0..n {
            if let Ok(item) = arr.GetItemAt(i) {
                if let Ok(p) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                    v.push(p.to_string().unwrap_or_default());
                    CoTaskMemFree(Some(p.0 as *const _));
                }
            }
        }
    }
    v.retain(|s| !s.is_empty());
    v
}

pub fn navigate(root: HWND, path: &str) -> bool {
    let Some((wb, sb)) = browser(root) else { return false };
    unsafe {
        let target = VARIANT::from(BSTR::from(path));
        let empty = VARIANT::default();
        let r = wb.Navigate2(&target, Some(&empty), Some(&empty), Some(&empty), Some(&empty));
        if r.is_ok() {
            return true;
        }
        // Fallback: browse the PIDL directly in the active tab.
        let Some(sb) = sb else { return false };
        let p = wide(path);
        let mut pidl = std::ptr::null_mut();
        if SHParseDisplayName(PCWSTR(p.as_ptr()), None, &mut pidl, 0, None).is_err() {
            return false;
        }
        let r = sb.BrowseObject(pidl, SBSP_SAMEBROWSER | SBSP_ABSOLUTE);
        CoTaskMemFree(Some(pidl as *const _));
        r.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs TACHYON_TEST_HWND = an Explorer window showing C:\Windows\Web (explorer C:\Windows\Web):
    /// `cargo test explorer_live -- --ignored`
    #[test]
    #[ignore]
    fn explorer_live() {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            let h: isize = std::env::var("TACHYON_TEST_HWND").expect("set TACHYON_TEST_HWND").parse().unwrap();
            let root = HWND(h as *mut _);
            let t = crate::target::classify(root);
            assert!(t.is_none(), "title bar itself must not be intercepted");
            let tab = FindWindowExW(Some(root), None, w!("ShellTabWindowClass"), PCWSTR::null()).unwrap();
            let t = crate::target::classify(tab).expect("content area is a target");
            assert_eq!(t.kind, crate::target::Kind::Explorer);
            assert_eq!(current_dir(root).as_deref(), Some(r"C:\Windows\Web"));
            assert!(navigate(root, r"C:\Windows\Media"));
            let mut d = None;
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(50));
                d = current_dir(root);
                if d.as_deref() == Some(r"C:\Windows\Media") {
                    break;
                }
            }
            assert_eq!(d.as_deref(), Some(r"C:\Windows\Media"));
            let _ = PostMessageW(Some(root), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}
