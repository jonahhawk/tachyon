# Tachyon

Faster-than-light jumps between folders. Tachyon is a small, minimalist folder-shortcut menu for Windows, similar to Direct Folders or Quick Access Pro. Middle-click in any Explorer window or Open/Save dialog to get a popup of your shortcut folders. Pick one, and the window you clicked goes there.

It also works in custom file dialogs such as Autodesk Revit's, which many similar tools don't support.

- A single portable `.exe`, with no installer and no network access
- Folders and subfolders of shortcuts to directories and files
- **Add Here** and **Add Folder** at the bottom of every menu
- Themes, custom colors, transparency, text size, and per-item icons from [Lucide](https://lucide.dev) and [Fluent UI](https://github.com/microsoft/fluentui-system-icons)

## Download

Download `tachyon.exe` from the [Releases](https://github.com/jonahhawk/tachyon/releases) page and run it. Release builds are code signed; see [Code signing policy](#code-signing-policy). Each release includes `SHA256SUMS.txt` so you can verify the file.

Requires Windows 10 or 11 (x64).

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

Tachyon never intercepts middle-clicks in an application's main window, so Revit's middle-drag pan is untouched.

## Changes to your system

Tachyon changes only these:

| What | When |
|---|---|
| `%APPDATA%\Tachyon\tachyon.json` | Always. Stores your shortcuts and settings |
| `%APPDATA%\Tachyon\inspect-*.txt` | Only when you Ctrl+Shift+middle-click |
| `Tachyon.lnk` in your Startup folder (`shell:startup`) | Only while **Start with Windows** is on in Settings |

It does not write to the registry, install services or drivers, or need administrator rights.

### How it works

Security tools sometimes ask about these behaviors:

| Behavior | Why |
|---|---|
| Low-level mouse hook (`WH_MOUSE_LL`) | Detects a middle-click over Explorer or a file dialog. Every other click passes through untouched |
| Shell COM (`IShellWindows`, `IShellBrowser`) | Reads and sets the folder of the Explorer window you clicked |
| Window messages and UI Automation on file dialogs | Types the chosen path into the dialog's filename box and presses Open/Save, then puts back what you had typed |
| `SendInput` (Enter key) | Only for custom dialogs (e.g. Revit) whose Open button UI Automation can't press |

It does not read or write other processes' memory, inject DLLs, or run elevated.

## Uninstall

1. In Settings, turn off **Start with Windows**. You can also delete `Tachyon.lnk` from `shell:startup`.
2. Right-click the tray icon and choose **Exit**.
3. Delete `tachyon.exe` and the `%APPDATA%\Tachyon` folder.

## Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io), certificate by [SignPath Foundation](https://signpath.org).

- Committers and reviewers: [Jonah Hawk](https://github.com/jonahhawk)
- Approvers: [Jonah Hawk](https://github.com/jonahhawk)

Only `tachyon.exe`, built by the [Release workflow](.github/workflows/release.yml) from a tagged commit in this repository, is signed. Every signing request is approved manually. Changes from contributors outside the team are reviewed before they are merged.

### Privacy policy

This program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it.

Tachyon has no network code and collects no data. Your shortcuts and settings stay in `%APPDATA%\Tachyon`.

## Build

Requires a Rust toolchain (MSVC) on Windows.

```
cargo build --release                      # target\release\tachyon.exe
cargo test                                 # unit tests
cargo test -- --ignored --test-threads=1 classic_dialog_live vista_dialog_live   # off-screen dialog tests
cargo test gen_icon -- --ignored           # regenerate assets\tachyon.ico from assets\logo.svg
```

### How dialogs are driven

`src/target/` tries each of these in order:

1. **Explorer**: uses `IShellWindows` → the active tab's `IShellBrowser`, which works with Windows 11 tabs.
2. **Win32 file dialogs**: finds the filename box by control ID or by its "File name:" label, types the path, presses OK, then puts your typed filename back. It reads the current folder from the breadcrumb bar. Older dialogs without one fall back to the last folder you jumped to.
3. **UI Automation fallback**: for dialogs without usable Win32 controls (Revit, WPF, Qt), it uses `ValuePattern` + `InvokePattern`.

## License

Tachyon is released under the [MIT License](LICENSE). It bundles open-source icons and Rust libraries under their own permissive licenses; see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
