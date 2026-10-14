# Tachyon

Faster-than-light jumps between directories. Middle-click in any Explorer window or Open/Save dialog to get a minimal popup of your shortcut folders. Pick one, and the window you clicked goes there.

## Use

| | |
|---|---|
| **Middle-click** | Opens the popup over Explorer's file list or nav pane, or over any file dialog (Win32, Vista, or custom such as Revit) |
| **Ctrl+Alt+Space** | Opens the popup for the active window |
| **Add Here** | Adds the window's current folder to this (sub)menu |
| **Shift + Add Here** | Adds the selected file(s) instead |
| **Add Folder** | Creates a submenu. Type its name, then press Enter |
| **Click a shortcut** | Navigates the window, or opens the file. **Ctrl+click** opens it in a new Explorer window |
| **Right-click / F2 / Del** | Rename, change icon, move, remove. **Ctrl+Z** undoes a removal |
| **Drag** | Reorder, or drop onto a folder to move the item into it |
| **Keyboard** | ↑↓←→, Enter, Esc, and type-ahead letters |
| **⚙ (popup header)** | Opens Settings: themes, custom colors, opacity, text size, icon sets, per-item icons, trigger modifier, start with Windows |
| **Ctrl+Shift+middle-click** | Writes a diagnostic dump of the window under the cursor to `%APPDATA%\Tachyon\inspect-*.txt` (for dialogs Tachyon doesn't recognize yet) |

Shortcuts are stored in `%APPDATA%\Tachyon\tachyon.json`.

## How dialogs are driven

`src/target/` tries each of these in order:

1. **Explorer**: uses `IShellWindows` → the active tab's `IShellBrowser`, which works with Windows 11 tabs.
2. **Win32 file dialogs**: finds the filename box by control ID or by its "File name:" label, types the path, presses OK, then puts your typed filename back. It reads the current folder from the breadcrumb bar or `CDM_GETFOLDERPATH`.
3. **UI Automation fallback**: for dialogs without usable Win32 controls (Revit, WPF, Qt), it uses `ValuePattern` + `InvokePattern`.

It never intercepts middle-clicks in an application's main window, so Revit's middle-drag pan is untouched.

## Build

```
cargo build --release                      # target\release\tachyon.exe
cargo test                                 # unit tests
cargo test -- --ignored classic_dialog_live vista_dialog_live   # off-screen dialog tests
cargo test gen_icon -- --ignored           # regenerate assets\tachyon.ico from assets\logo.svg
```

## Icons

- [Lucide](https://lucide.dev): ISC License
- [Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons): MIT License

`build.rs` packs them into the executable, compressing each icon separately. See `LICENSES/`.
