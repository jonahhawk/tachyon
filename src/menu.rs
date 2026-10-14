//! The popup: cascading custom-drawn layered windows.
//!
//! All state lives in `App` (main thread). Handlers never call into COM/UIA
//! while `App` is borrowed; they return `Effect`s that `app::run` executes
//! after the borrow is released.

use crate::app::{App, Effect};
use crate::gfx::{IconKey, Surface, rectf};
use crate::icons::{self, Role};
use crate::model::{Id, IconSet, Kind, Node, Palette};
use crate::target::Target;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

pub const MENU_CLASS: PCWSTR = w!("TachyonMenu");
const TIMER_HOVER: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Rename,
    Icon,
    Up,
    Down,
    Reveal,
    Remove,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Action::Rename => "Rename",
            Action::Icon => "Change Icon…",
            Action::Up => "Move Up",
            Action::Down => "Move Down",
            Action::Reveal => "Open Location",
            Action::Remove => "Remove",
        }
    }
    fn role(self) -> Role {
        match self {
            Action::Rename => Role::Rename,
            Action::Icon => Role::Icon,
            Action::Up => Role::Up,
            Action::Down => Role::Down,
            Action::Reveal => Role::Reveal,
            Action::Remove => Role::Remove,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Row {
    Node(Id),
    Sep,
    AddHere,
    AddFolder,
    Editor,
    Action(Action),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LevelKind {
    Folder(Id),
    Context(Id),
}

pub struct Level {
    pub hwnd: HWND,
    pub kind: LevelKind,
    pub rows: Vec<Row>,
    pub hover: Option<usize>,
    pub gear_hover: bool,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub scroll: f32,
    pub content_h: f32,
    pub view_h: f32,
    pub surface: Option<Surface>,
}

impl Level {
    pub fn panel_contains(&self, pt: POINT, m: f32) -> bool {
        let m = m as i32;
        pt.x >= self.x + m && pt.x < self.x + self.w - m && pt.y >= self.y + m && pt.y < self.y + self.h - m
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditKind {
    NewFolder(Id),
    Path(Id),
    Rename(Id),
}

pub struct Editor {
    pub kind: EditKind,
    pub text: String,
    pub caret: usize,
    pub all: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DropAt {
    Into(usize, usize, Id),
    /// level, parent folder, insert index, indicator y (client px)
    Before(usize, Id, usize, f32),
}

pub struct Drag {
    pub id: Id,
    pub at: Option<DropAt>,
}

#[derive(Clone, Copy)]
pub struct Metrics {
    pub s: f32,
    pub m: f32,
    pub pad: f32,
    pub row: f32,
    pub sep: f32,
    pub icon: f32,
    pub font: f32,
    pub radius: f32,
    pub header: f32,
}

impl Metrics {
    fn new(dpi: u32, font_size: f32) -> Metrics {
        let s = dpi as f32 / 96.0;
        let font = (font_size * s).round();
        Metrics {
            s,
            m: (10.0 * s).round(),
            pad: (5.0 * s).round(),
            row: (font * 2.0).round(),
            sep: (9.0 * s).round(),
            icon: (font * 1.25).round(),
            font,
            radius: (9.0 * s).round(),
            header: (font * 1.9).round(),
        }
    }
    fn row_h(&self, r: &Row) -> f32 {
        if matches!(r, Row::Sep) { self.sep } else { self.row }
    }
}

pub struct Menu {
    pub target: Option<Target>,
    pub cur_dir: Option<String>,
    pub resolved: bool,
    pub levels: Vec<Level>,
    pub active: usize,
    pub mt: Metrics,
    pub work: RECT,
    pub press: Option<(usize, usize, POINT)>,
    pub drag: Option<Drag>,
    pub editor: Option<Editor>,
    pub pending_hover: Option<(usize, usize)>,
    pub undo: Option<(Node, Id, usize)>,
}

fn lo(l: LPARAM) -> i32 {
    (l.0 & 0xFFFF) as i16 as i32
}
fn hi(l: LPARAM) -> i32 {
    ((l.0 >> 16) & 0xFFFF) as i16 as i32
}
fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk.0 as i32) < 0 }
}

pub fn monitor_at(pt: POINT) -> (RECT, u32) {
    unsafe {
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        (mi.rcWork, dx)
    }
}

fn create_window(x: i32, y: i32, w: i32, h: i32, activatable: bool) -> HWND {
    unsafe {
        let hinst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | if activatable { WINDOW_EX_STYLE(0) } else { WS_EX_NOACTIVATE },
            MENU_CLASS,
            w!("Tachyon"),
            WS_POPUP,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .unwrap_or_default()
    }
}

fn short_path(p: &str) -> String {
    let parts: Vec<&str> = p.trim_end_matches('\\').split('\\').collect();
    if p.chars().count() <= 42 || parts.len() <= 3 {
        return p.to_string();
    }
    format!("{}\\…\\{}\\{}", parts[0], parts[parts.len() - 2], parts[parts.len() - 1])
}

impl App {
    pub fn palette(&self) -> Palette {
        self.cfg.settings.palette()
    }

    // ---------------------------------------------------------------- open/close

    /// Show the root popup at `pt` for `target`. Returns effects to run.
    pub fn open_menu(&mut self, target: Option<Target>, pt: POINT) -> Vec<Effect> {
        self.close_menu();
        let (work, dpi) = monitor_at(pt);
        let mt = Metrics::new(dpi, self.cfg.settings.font_size);
        self.gfx.ensure_fonts(&self.cfg.settings.font_family, mt.font);
        self.menu = Some(Menu {
            target,
            cur_dir: None,
            resolved: target.is_none(),
            levels: Vec::new(),
            active: 0,
            mt,
            work,
            press: None,
            drag: None,
            editor: None,
            pending_hover: None,
            undo: None,
        });
        let mut fx = Vec::new();
        self.push_level(LevelKind::Folder(0), pt.x - mt.m as i32, pt.y - mt.m as i32, None, &mut fx);
        let root = self.menu.as_ref().unwrap().levels[0].hwnd;
        unsafe {
            // Take the foreground so keyboard navigation works. A process that only
            // observed the click through a hook may not, so borrow the input queue.
            let fg = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(fg, None);
            let me = windows::Win32::System::Threading::GetCurrentThreadId();
            let attached = fg_thread != me && windows::Win32::System::Threading::AttachThreadInput(me, fg_thread, true).as_bool();
            if !cfg!(test) {
                let _ = SetForegroundWindow(root);
                let _ = SetFocus(Some(root));
            }
            if attached {
                let _ = windows::Win32::System::Threading::AttachThreadInput(me, fg_thread, false);
            }
        }
        if target.is_some() {
            fx.push(Effect::Resolve);
        }
        fx
    }

    pub fn close_menu(&mut self) {
        if self.menu.as_ref().map(|m| m.editor.is_some()).unwrap_or(false) {
            self.commit_editor();
        }
        if let Some(m) = self.menu.take() {
            for l in m.levels.iter().rev() {
                unsafe {
                    let _ = DestroyWindow(l.hwnd);
                }
            }
        }
        crate::app::set_menu_rects(Vec::new());
    }

    fn sync_rects(&self) {
        let v = self
            .menu
            .as_ref()
            .map(|m| m.levels.iter().map(|l| RECT { left: l.x, top: l.y, right: l.x + l.w, bottom: l.y + l.h }).collect())
            .unwrap_or_default();
        crate::app::set_menu_rects(v);
    }

    fn rows_for(&self, kind: LevelKind) -> Vec<Row> {
        let m = self.menu.as_ref().unwrap();
        match kind {
            LevelKind::Folder(id) => {
                let mut rows: Vec<Row> = self
                    .cfg
                    .find(id)
                    .map(|n| n.children().iter().map(|c| Row::Node(c.id)).collect())
                    .unwrap_or_default();
                if let Some(e) = &m.editor {
                    if matches!(e.kind, EditKind::NewFolder(p) | EditKind::Path(p) if p == id) {
                        rows.push(Row::Editor);
                    }
                }
                if !rows.is_empty() {
                    rows.push(Row::Sep);
                }
                rows.push(Row::AddHere);
                rows.push(Row::AddFolder);
                rows
            }
            LevelKind::Context(id) => {
                let n = self.cfg.find(id);
                let mut rows = vec![Row::Action(Action::Rename), Row::Action(Action::Icon)];
                if let Some((p, i)) = self.cfg.locate(id) {
                    if i > 0 {
                        rows.push(Row::Action(Action::Up));
                    }
                    if i + 1 < self.cfg.find(p).map(|p| p.children().len()).unwrap_or(0) {
                        rows.push(Row::Action(Action::Down));
                    }
                }
                if n.and_then(|n| n.path()).is_some() {
                    rows.push(Row::Action(Action::Reveal));
                }
                rows.push(Row::Sep);
                rows.push(Row::Action(Action::Remove));
                rows
            }
        }
    }

    fn label(&self, r: &Row) -> String {
        match r {
            Row::Node(id) => self.cfg.find(*id).map(|n| n.name.clone()).unwrap_or_default(),
            Row::AddHere => "Add Here".into(),
            Row::AddFolder => "Add Folder".into(),
            Row::Action(a) => a.label().into(),
            Row::Editor => {
                let e = self.menu.as_ref().and_then(|m| m.editor.as_ref());
                match e {
                    Some(e) if !e.text.is_empty() => e.text.clone(),
                    Some(Editor { kind: EditKind::Path(_), .. }) => "Paste a folder or file path".into(),
                    _ => "Folder name".into(),
                }
            }
            Row::Sep => String::new(),
        }
    }

    /// Compute size for a level's rows; returns (w, h, content_h, view_h).
    fn measure_level(&self, kind: LevelKind, rows: &[Row]) -> (i32, i32, f32, f32) {
        let m = self.menu.as_ref().unwrap();
        let mt = m.mt;
        let mut maxw: f32 = 0.0;
        for r in rows {
            maxw = maxw.max(self.gfx.measure(&self.label(r), &self.gfx.text));
        }
        let header = if kind == LevelKind::Folder(0) { mt.header } else { 0.0 };
        if header > 0.0 {
            if let Some(d) = &m.cur_dir {
                maxw = maxw.max(self.gfx.measure(&short_path(d), &self.gfx.small) - mt.icon * 1.5);
            }
        }
        let inner = (maxw + mt.icon + mt.pad * 6.0 + mt.icon).clamp(190.0 * mt.s, 460.0 * mt.s);
        let content: f32 = rows.iter().map(|r| mt.row_h(r)).sum();
        let max_h = (m.work.bottom - m.work.top) as f32 - header - 2.0 * (mt.m + mt.pad);
        let view = content.min(max_h.max(mt.row * 3.0));
        let w = (inner + 2.0 * (mt.m + mt.pad)).round() as i32;
        let h = (view + header + 2.0 * (mt.m + mt.pad)).round() as i32;
        (w, h, content, view)
    }

    fn clamp_pos(&self, x: i32, y: i32, w: i32, h: i32) -> (i32, i32) {
        let m = self.menu.as_ref().unwrap();
        let mm = m.mt.m as i32;
        let x = x.min(m.work.right - w + mm).max(m.work.left - mm);
        let y = y.min(m.work.bottom - h + mm).max(m.work.top - mm);
        (x, y)
    }

    fn push_level(&mut self, kind: LevelKind, x: i32, y: i32, anchor: Option<(i32, i32)>, fx: &mut Vec<Effect>) {
        let rows = self.rows_for(kind);
        let (w, h, content_h, view_h) = self.measure_level(kind, &rows);
        let (mut x, y) = (x, y);
        if let Some((parent_x, parent_w)) = anchor {
            let m = self.menu.as_ref().unwrap();
            let mm = m.mt.m as i32;
            let gap = (2.0 * m.mt.s) as i32;
            x = parent_x + parent_w - 2 * mm + gap;
            if x + w - mm > m.work.right {
                x = parent_x - w + 2 * mm - gap;
            }
        }
        let (x, y) = self.clamp_pos(x, y, w, h);
        let first = self.menu.as_ref().unwrap().levels.is_empty();
        let hwnd = create_window(x, y, w, h, first);
        self.collect_paths(&rows, fx);
        let m = self.menu.as_mut().unwrap();
        m.levels.push(Level {
            hwnd,
            kind,
            rows,
            hover: None,
            gear_hover: false,
            x,
            y,
            w,
            h,
            scroll: 0.0,
            content_h,
            view_h,
            surface: None,
        });
        let i = m.levels.len() - 1;
        self.paint(i);
        unsafe {
            if !cfg!(test) {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
        }
        self.sync_rects();
    }

    fn collect_paths(&self, rows: &[Row], fx: &mut Vec<Effect>) {
        let paths: Vec<String> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Node(id) => self.cfg.find(*id).and_then(|n| n.path()).map(String::from),
                _ => None,
            })
            .filter(|p| !self.exists.contains_key(p))
            .collect();
        if !paths.is_empty() {
            fx.push(Effect::CheckPaths(paths));
        }
    }

    fn truncate(&mut self, keep: usize) {
        let Some(m) = self.menu.as_mut() else { return };
        while m.levels.len() > keep {
            let l = m.levels.pop().unwrap();
            unsafe {
                let _ = DestroyWindow(l.hwnd);
            }
        }
        m.active = m.active.min(keep.saturating_sub(1));
        self.sync_rects();
    }

    /// Row top (client y, unscrolled for header) of row `r` in level `i`.
    fn row_top(&self, i: usize, r: usize) -> f32 {
        let m = self.menu.as_ref().unwrap();
        let l = &m.levels[i];
        let mt = m.mt;
        let header = if i == 0 { mt.header } else { 0.0 };
        let mut y = mt.m + mt.pad + header - l.scroll;
        for row in &l.rows[..r] {
            y += mt.row_h(row);
        }
        y
    }

    fn open_sub(&mut self, i: usize, r: usize, fx: &mut Vec<Effect>) {
        let Some(m) = self.menu.as_ref() else { return };
        let Some(Row::Node(id)) = m.levels.get(i).and_then(|l| l.rows.get(r)).copied() else { return };
        if !self.cfg.find(id).map(|n| n.is_folder()).unwrap_or(false) {
            return;
        }
        if m.levels.get(i + 1).map(|l| l.kind) == Some(LevelKind::Folder(id)) {
            return;
        }
        self.truncate(i + 1);
        let m = self.menu.as_ref().unwrap();
        let l = &m.levels[i];
        let (lx, lw, ly) = (l.x, l.w, l.y);
        let mt = m.mt;
        let y = ly + self.row_top(i, r) as i32 - (mt.m + mt.pad) as i32;
        self.push_level(LevelKind::Folder(id), lx, y, Some((lx, lw)), fx);
    }

    fn open_context(&mut self, i: usize, r: usize, pt: POINT, fx: &mut Vec<Effect>) {
        let Some(Row::Node(id)) = self.menu.as_ref().and_then(|m| m.levels[i].rows.get(r)).copied() else { return };
        self.truncate(i + 1);
        let m = self.menu.as_mut().unwrap();
        m.levels[i].hover = Some(r);
        let mm = m.mt.m as i32;
        self.push_level(LevelKind::Context(id), pt.x - mm, pt.y - mm, None, fx);
        let m = self.menu.as_mut().unwrap();
        m.active = m.levels.len() - 1;
    }

    /// Recompute rows/sizes for every level after the model changed.
    pub fn rebuild(&mut self) {
        let Some(m) = self.menu.as_ref() else { return };
        let n = m.levels.len();
        let mut keep = n;
        for i in 0..n {
            let kind = self.menu.as_ref().unwrap().levels[i].kind;
            let alive = match kind {
                LevelKind::Folder(id) | LevelKind::Context(id) => self.cfg.find(id).is_some(),
            };
            if !alive {
                keep = i;
                break;
            }
        }
        self.truncate(keep.max(1));
        let n = self.menu.as_ref().unwrap().levels.len();
        for i in 0..n {
            let kind = self.menu.as_ref().unwrap().levels[i].kind;
            let rows = self.rows_for(kind);
            let (w, h, content_h, view_h) = self.measure_level(kind, &rows);
            let (x0, y0) = {
                let l = &self.menu.as_ref().unwrap().levels[i];
                (l.x, l.y)
            };
            let (x, y) = self.clamp_pos(x0, y0, w, h);
            let m = self.menu.as_mut().unwrap();
            let l = &mut m.levels[i];
            l.hover = l.hover.filter(|&r| r < rows.len());
            l.rows = rows;
            l.w = w;
            l.h = h;
            l.x = x;
            l.y = y;
            l.content_h = content_h;
            l.view_h = view_h;
            l.scroll = l.scroll.min((content_h - view_h).max(0.0));
            self.paint(i);
        }
        self.sync_rects();
    }

    pub fn repaint_all(&mut self) {
        let n = self.menu.as_ref().map(|m| m.levels.len()).unwrap_or(0);
        for i in 0..n {
            self.paint(i);
        }
    }

    // -------------------------------------------------------------------- paint

    fn node_icon(&self, n: &Node, px: u32, tint: u32) -> Option<IconKey> {
        let set = self.cfg.settings.icon_set;
        if let Some(r) = &n.icon {
            if r == "shell" {
                return Some(match &n.kind {
                    Kind::Dir { path } => IconKey::Shell(path.clone(), px, true),
                    Kind::File { path } => IconKey::Shell(path.clone(), px, false),
                    Kind::Folder { .. } => IconKey::Shell(String::new(), px, true),
                });
            }
            return Some(IconKey::Svg(r.clone(), px, tint));
        }
        if set == IconSet::Shell {
            return Some(match &n.kind {
                Kind::Dir { path } => IconKey::Shell(path.clone(), px, true),
                Kind::File { path } => IconKey::Shell(path.clone(), px, false),
                Kind::Folder { .. } => IconKey::Svg(icons::default_icon(IconSet::Lucide, Role::Group)?, px, tint),
            });
        }
        let role = match n.kind {
            Kind::Folder { .. } => Role::Group,
            Kind::Dir { .. } => Role::Dir,
            Kind::File { .. } => Role::File,
        };
        icons::default_icon(set, role).map(|r| IconKey::Svg(r, px, tint))
    }

    fn role_icon(&self, role: Role, px: u32, tint: u32) -> Option<IconKey> {
        let set = match self.cfg.settings.icon_set {
            IconSet::None => return None,
            IconSet::Shell => IconSet::Lucide,
            s => s,
        };
        icons::default_icon(set, role).map(|r| IconKey::Svg(r, px, tint))
    }

    pub fn paint(&mut self, i: usize) {
        for attempt in 0..2 {
            match self.paint_inner(i) {
                Err(e) if e.code() == D2DERR_RECREATE_TARGET && attempt == 0 => self.gfx.reset(),
                _ => break,
            }
        }
    }

    fn paint_inner(&mut self, i: usize) -> Result<()> {
        let pal = self.palette();
        let opacity = self.cfg.settings.opacity.clamp(0.3, 1.0);
        let Some(m) = self.menu.as_mut() else { return Ok(()) };
        let Some(l) = m.levels.get_mut(i) else { return Ok(()) };
        if l.surface.as_ref().map(|s| (s.w, s.h)) != Some((l.w, l.h)) {
            l.surface = Surface::new(l.w, l.h);
        }
        let Some(dc) = l.surface.as_ref().map(|s| s.dc) else { return Ok(()) };
        let (w, h) = (l.w as f32, l.h as f32);
        let m = self.menu.as_ref().unwrap();
        let l = &m.levels[i];
        let mt = m.mt;
        let g = &self.gfx;
        unsafe {
            g.rt.BindDC(dc, &RECT { left: 0, top: 0, right: l.w, bottom: l.h })?;
            g.rt.BeginDraw();
            g.rt.Clear(Some(&crate::gfx::color(0, 0.0)));
        }
        // Soft shadow: stacked translucent rounded rects.
        let steps = mt.m as i32;
        for k in (1..=steps).rev() {
            let k = k as f32;
            let a = 0.018 * (1.0 - k / (steps as f32 + 1.0));
            g.fill(rectf(mt.m - k, mt.m - k + mt.s, w - mt.m + k, h - mt.m + k + mt.s), 0x000000, a, mt.radius + k);
        }
        let panel = rectf(mt.m, mt.m, w - mt.m, h - mt.m);
        g.fill(panel, pal.bg, opacity, mt.radius);
        g.stroke(
            rectf(mt.m + 0.5, mt.m + 0.5, w - mt.m - 0.5, h - mt.m - 0.5),
            pal.border,
            0.9,
            mt.radius,
            1.0,
        );

        let x0 = mt.m + mt.pad;
        let x1 = w - mt.m - mt.pad;
        let icon_px = mt.icon as u32;
        let mut deferred_icons: Vec<(IconKey, f32, f32, f32)> = Vec::new();

        // Header (root only): current location + gear.
        let mut top = mt.m + mt.pad;
        if i == 0 {
            let hy = top;
            let gear_x = x1 - mt.icon - mt.pad;
            if l.gear_hover {
                g.fill(rectf(gear_x - mt.pad, hy + 2.0, x1, hy + mt.header - 2.0), pal.hover, 1.0, mt.radius * 0.6);
            }
            let txt = match (&m.cur_dir, m.target.is_some(), m.resolved) {
                (Some(d), _, _) => short_path(d),
                (None, true, false) => "…".into(),
                (None, true, true) => "Location unavailable".into(),
                (None, false, _) => "No window targeted".into(),
            };
            g.text(&txt, rectf(x0 + mt.pad * 1.6, hy, gear_x - mt.pad * 2.0, hy + mt.header), pal.dim, 1.0, true);
            if let Some(k) = self.role_icon(Role::Gear, icon_px, if l.gear_hover { pal.fg } else { pal.dim }) {
                deferred_icons.push((k, gear_x, hy + (mt.header - mt.icon) / 2.0, 1.0));
            }
            top += mt.header;
        }

        unsafe {
            g.rt.PushAxisAlignedClip(&rectf(mt.m, top, w - mt.m, h - mt.m - mt.pad * 0.5), D2D1_ANTIALIAS_MODE_ALIASED);
        }
        let mut y = top - l.scroll;
        let dragging = m.drag.as_ref().map(|d| d.id);
        for (ri, row) in l.rows.iter().enumerate() {
            let rh = mt.row_h(row);
            if y + rh < top || y > h {
                y += rh;
                continue;
            }
            let hovered = l.hover == Some(ri);
            let rr = rectf(x0, y + 1.0, x1, y + rh - 1.0);
            let text_x = x0 + mt.pad * 1.6 + mt.icon + mt.pad * 1.8;
            let iy = y + (rh - mt.icon) / 2.0;
            let ix = x0 + mt.pad * 1.6;
            match row {
                Row::Sep => {
                    let cy = (y + rh / 2.0).floor();
                    g.fill(rectf(x0 + mt.pad, cy, x1 - mt.pad, cy + 1.0), pal.border, 1.0, 0.0);
                }
                Row::Node(id) => {
                    let Some(n) = self.cfg.find(*id) else { y += rh; continue };
                    let missing = n.path().map(|p| self.exists.get(p) == Some(&false)).unwrap_or(false);
                    let is_drag = dragging == Some(*id);
                    let into = matches!(m.drag.as_ref().and_then(|d| d.at), Some(DropAt::Into(li, r, _)) if li == i && r == ri);
                    if hovered || into {
                        g.fill(rr, pal.hover, 1.0, mt.radius * 0.6);
                    }
                    if into {
                        g.stroke(rr, pal.accent, 1.0, mt.radius * 0.6, 1.5 * mt.s);
                    }
                    let fade = if is_drag { 0.4 } else if missing { 0.45 } else { 1.0 };
                    let renaming = matches!(m.editor.as_ref().map(|e| e.kind), Some(EditKind::Rename(rid)) if rid == *id);
                    if let Some(k) = self.node_icon(n, icon_px, pal.fg) {
                        deferred_icons.push((k, ix, iy, fade));
                    }
                    if renaming {
                        self.paint_editor(rectf(text_x - mt.pad, y + 2.0, x1 - mt.pad, y + rh - 2.0), &pal);
                    } else {
                        let right = if n.is_folder() { x1 - mt.icon - mt.pad } else { x1 - mt.pad };
                        g.text(&n.name, rectf(text_x, y, right, y + rh), if missing { pal.dim } else { pal.fg }, fade, false);
                    }
                    if n.is_folder() {
                        let cs = (mt.icon * 0.8).round();
                        deferred_icons.push((
                            IconKey::Svg("lucide/chevron-right".into(), cs as u32, pal.dim),
                            x1 - cs - mt.pad * 0.6,
                            y + (rh - cs) / 2.0,
                            1.0,
                        ));
                    }
                }
                Row::AddHere | Row::AddFolder => {
                    if hovered {
                        g.fill(rr, pal.hover, 1.0, mt.radius * 0.6);
                    }
                    let role = if *row == Row::AddHere { Role::AddHere } else { Role::AddFolder };
                    if let Some(k) = self.role_icon(role, icon_px, pal.accent) {
                        deferred_icons.push((k, ix, iy, 1.0));
                    }
                    let label = if *row == Row::AddHere { "Add Here" } else { "Add Folder" };
                    g.text(label, rectf(text_x, y, x1 - mt.pad, y + rh), if hovered { pal.fg } else { pal.dim }, 1.0, false);
                }
                Row::Action(a) => {
                    if hovered {
                        g.fill(rr, pal.hover, 1.0, mt.radius * 0.6);
                    }
                    let tint = if *a == Action::Remove { 0xE5484D } else { pal.fg };
                    if let Some(k) = self.role_icon(a.role(), icon_px, tint) {
                        deferred_icons.push((k, ix, iy, 1.0));
                    }
                    g.text(a.label(), rectf(text_x, y, x1 - mt.pad, y + rh), tint, 1.0, false);
                }
                Row::Editor => {
                    let role = match m.editor.as_ref().map(|e| e.kind) {
                        Some(EditKind::Path(_)) => Role::AddHere,
                        _ => Role::AddFolder,
                    };
                    if let Some(k) = self.role_icon(role, icon_px, pal.accent) {
                        deferred_icons.push((k, ix, iy, 1.0));
                    }
                    self.paint_editor(rectf(text_x - mt.pad, y + 2.0, x1 - mt.pad, y + rh - 2.0), &pal);
                }
            }
            y += rh;
        }
        // Drop indicator.
        if let Some(DropAt::Before(li, _, _, iy)) = m.drag.as_ref().and_then(|d| d.at) {
            if li == i {
                g.fill(rectf(x0 + mt.pad, iy - mt.s, x1 - mt.pad, iy + mt.s), pal.accent, 1.0, mt.s);
            }
        }
        unsafe {
            g.rt.PopAxisAlignedClip();
        }
        // Scroll thumb.
        if l.content_h > l.view_h + 0.5 {
            let track = l.view_h;
            let th = (track * l.view_h / l.content_h).max(mt.row);
            let ty = top + (track - th) * (l.scroll / (l.content_h - l.view_h));
            g.fill(rectf(w - mt.m - 4.0 * mt.s, ty, w - mt.m - 2.0 * mt.s, ty + th), pal.dim, 0.5, mt.s);
        }
        let (lw, lh, lx, ly, hwnd) = (l.w, l.h, l.x, l.y, l.hwnd);
        for (k, x, y, a) in deferred_icons {
            self.gfx.icon(&k, x.round(), y.round(), a);
        }
        unsafe {
            self.gfx.rt.EndDraw(None, None)?;
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(
                hwnd,
                None,
                Some(&POINT { x: lx, y: ly }),
                Some(&SIZE { cx: lw, cy: lh }),
                Some(dc),
                Some(&POINT { x: 0, y: 0 }),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
        }
        Ok(())
    }

    fn paint_editor(&self, r: D2D_RECT_F, pal: &Palette) {
        let g = &self.gfx;
        let Some(m) = self.menu.as_ref() else { return };
        let Some(e) = m.editor.as_ref() else { return };
        let mt = m.mt;
        g.fill(r, pal.hover, 1.0, mt.radius * 0.5);
        g.stroke(r, pal.accent, 0.9, mt.radius * 0.5, mt.s);
        let tx = r.left + mt.pad;
        if e.text.is_empty() {
            let ph = if matches!(e.kind, EditKind::Path(_)) { "Paste a folder or file path" } else { "Folder name" };
            g.text(ph, rectf(tx, r.top, r.right - mt.pad, r.bottom), pal.dim, 0.8, false);
        } else {
            if e.all {
                let tw = g.measure(&e.text, &g.text);
                g.fill(rectf(tx, r.top + 3.0 * mt.s, (tx + tw).min(r.right), r.bottom - 3.0 * mt.s), pal.accent, 0.35, 2.0);
            }
            g.text(&e.text, rectf(tx, r.top, r.right - mt.pad, r.bottom), pal.fg, 1.0, false);
        }
        let cx = tx + g.caret_x(&e.text, e.caret, &g.text);
        if !e.all {
            g.fill(rectf(cx, r.top + 4.0 * mt.s, cx + mt.s.max(1.0), r.bottom - 4.0 * mt.s), pal.fg, 1.0, 0.0);
        }
    }

    // ------------------------------------------------------------------ events

    fn level_of(&self, hwnd: HWND) -> Option<usize> {
        self.menu.as_ref()?.levels.iter().position(|l| l.hwnd == hwnd)
    }

    fn level_at(&self, pt: POINT) -> Option<usize> {
        let m = self.menu.as_ref()?;
        // Deepest first: submenus overlap their parents slightly.
        (0..m.levels.len()).rev().find(|&i| m.levels[i].panel_contains(pt, m.mt.m))
    }

    /// Hit-test a client point: Ok(row) / Err(true) = gear / Err(false) = nothing.
    fn hit(&self, i: usize, x: f32, y: f32) -> std::result::Result<usize, bool> {
        let m = self.menu.as_ref().unwrap();
        let l = &m.levels[i];
        let mt = m.mt;
        let mut top = mt.m + mt.pad;
        if i == 0 {
            if y >= top && y < top + mt.header {
                let gear_x = l.w as f32 - mt.m - mt.pad * 2.0 - mt.icon * 1.6;
                return Err(x >= gear_x);
            }
            top += mt.header;
        }
        if x < mt.m || x > l.w as f32 - mt.m || y < top || y > l.h as f32 - mt.m - mt.pad * 0.5 {
            return Err(false);
        }
        let mut cy = top - l.scroll;
        for (ri, r) in l.rows.iter().enumerate() {
            let rh = mt.row_h(r);
            if y >= cy && y < cy + rh {
                return if *r == Row::Sep { Err(false) } else { Ok(ri) };
            }
            cy += rh;
        }
        Err(false)
    }

    pub fn menu_event(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<Vec<Effect>> {
        let i = self.level_of(hwnd)?;
        let mut fx = Vec::new();
        match msg {
            WM_MOUSEMOVE => {
                let (x, y) = (lo(lp), hi(lp));
                let l = &self.menu.as_ref()?.levels[i];
                let pt = POINT { x: l.x + x, y: l.y + y };
                self.on_move(i, pt, &mut fx);
            }
            WM_LBUTTONDOWN => {
                let (x, y) = (lo(lp), hi(lp));
                let l = &self.menu.as_ref()?.levels[i];
                let pt = POINT { x: l.x + x, y: l.y + y };
                match self.hit(i, x as f32, y as f32) {
                    Ok(r) => {
                        let m = self.menu.as_mut()?;
                        m.press = Some((i, r, pt));
                        unsafe {
                            SetCapture(hwnd);
                        }
                    }
                    Err(true) => {
                        fx.push(Effect::Close);
                        fx.push(Effect::Settings(vec![]));
                    }
                    Err(false) => {}
                }
            }
            WM_LBUTTONUP => {
                let m = self.menu.as_mut()?;
                let press = m.press.take();
                let drag = m.drag.take();
                unsafe {
                    let _ = ReleaseCapture();
                }
                if let Some(d) = drag {
                    self.finish_drag(d, &mut fx);
                } else if let Some((pi, pr, _)) = press {
                    let (x, y) = (lo(lp), hi(lp));
                    if pi == i && self.hit(i, x as f32, y as f32) == Ok(pr) {
                        self.activate(i, pr, &mut fx);
                    }
                }
            }
            WM_RBUTTONUP => {
                let (x, y) = (lo(lp), hi(lp));
                if let Ok(r) = self.hit(i, x as f32, y as f32) {
                    let l = &self.menu.as_ref()?.levels[i];
                    if matches!(l.rows[r], Row::Node(_)) {
                        let pt = POINT { x: l.x + x, y: l.y + y };
                        self.commit_editor();
                        self.open_context(i, r, pt, &mut fx);
                    }
                }
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xFFFF) as i16 as f32 / 120.0;
                let pt = POINT { x: lo(lp), y: hi(lp) };
                if let Some(li) = self.level_at(pt) {
                    let m = self.menu.as_mut()?;
                    let step = m.mt.row * 3.0;
                    let l = &mut m.levels[li];
                    l.scroll = (l.scroll - delta * step).clamp(0.0, (l.content_h - l.view_h).max(0.0));
                    self.paint(li);
                }
            }
            WM_MOUSELEAVE => {
                let m = self.menu.as_mut()?;
                let has_child = m.levels.len() > i + 1;
                let l = &mut m.levels[i];
                let changed = l.gear_hover || (!has_child && l.hover.is_some());
                l.gear_hover = false;
                if !has_child && m.drag.is_none() {
                    l.hover = None;
                }
                if changed {
                    self.paint(i);
                }
            }
            WM_TIMER if wp.0 == TIMER_HOVER => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER);
                }
                let m = self.menu.as_mut()?;
                if let Some((li, r)) = m.pending_hover.take() {
                    if m.levels.get(li).and_then(|l| l.hover) == Some(r) {
                        let is_folder = matches!(m.levels[li].rows[r], Row::Node(id) if self.cfg.find(id).map(|n| n.is_folder()).unwrap_or(false));
                        if is_folder {
                            self.open_sub(li, r, &mut fx);
                        } else if !matches!(m.levels.get(li + 1).map(|l| l.kind), Some(LevelKind::Context(_))) {
                            self.truncate(li + 1);
                        }
                    }
                }
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => self.on_key(VIRTUAL_KEY(wp.0 as u16), &mut fx),
            WM_CHAR => self.on_char(char::from_u32(wp.0 as u32).unwrap_or('\0'), &mut fx),
            WM_CAPTURECHANGED => {
                if let Some(m) = self.menu.as_mut() {
                    if m.drag.is_some() && unsafe { GetCapture() } != hwnd {
                        m.drag = None;
                        m.press = None;
                        self.repaint_all();
                    }
                }
            }
            _ => return None,
        }
        Some(fx)
    }

    fn track_leave(hwnd: HWND) {
        let mut t = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        unsafe {
            let _ = TrackMouseEvent(&mut t);
        }
    }

    fn on_move(&mut self, i: usize, pt: POINT, fx: &mut Vec<Effect>) {
        let Some(m) = self.menu.as_mut() else { return };
        Self::track_leave(m.levels[i].hwnd);
        // Drag start / update.
        if let Some((pi, pr, p0)) = m.press {
            let thresh = (4.0 * m.mt.s) as i32;
            if m.drag.is_none() && ((pt.x - p0.x).abs() > thresh || (pt.y - p0.y).abs() > thresh) {
                if let Row::Node(id) = m.levels[pi].rows[pr] {
                    if m.editor.is_none() && !matches!(m.levels[pi].kind, LevelKind::Context(_)) {
                        m.drag = Some(Drag { id, at: None });
                    }
                }
            }
        }
        let li = if self.menu.as_ref().unwrap().drag.is_some() { self.level_at(pt) } else { Some(i) };
        let Some(li) = li else {
            if let Some(d) = self.menu.as_mut().unwrap().drag.as_mut() {
                d.at = None;
            }
            self.repaint_all();
            return;
        };
        let m = self.menu.as_ref().unwrap();
        let l = &m.levels[li];
        let (cx, cy) = ((pt.x - l.x) as f32, (pt.y - l.y) as f32);
        let hit = self.hit(li, cx, cy);
        if m.drag.is_some() {
            let at = self.drop_at(li, cx, cy);
            let m = self.menu.as_mut().unwrap();
            m.drag.as_mut().unwrap().at = at;
        }
        let m = self.menu.as_mut().unwrap();
        m.active = li;
        let has_child = m.levels.len() > li + 1;
        let l = &mut m.levels[li];
        let new_hover = hit.ok();
        let gear = hit == Err(true);
        let changed = l.hover != new_hover || l.gear_hover != gear;
        if changed {
            l.hover = new_hover.or(if has_child { l.hover } else { None });
            l.gear_hover = gear;
            if let Some(r) = new_hover {
                m.pending_hover = Some((li, r));
                let delay = self.cfg.settings.hover_delay_ms.clamp(0, 1000);
                unsafe {
                    SetTimer(Some(m.levels[li].hwnd), TIMER_HOVER, delay.max(1), None);
                }
            }
        }
        if m.drag.is_some() {
            self.repaint_all();
        } else if changed {
            self.paint(li);
        }
        let _ = fx;
    }

    fn drop_at(&self, li: usize, cx: f32, cy: f32) -> Option<DropAt> {
        let m = self.menu.as_ref()?;
        let LevelKind::Folder(folder) = m.levels[li].kind else { return None };
        let drag = m.drag.as_ref()?.id;
        let l = &m.levels[li];
        let mt = m.mt;
        let mut top = mt.m + mt.pad + if li == 0 { mt.header } else { 0.0 };
        top -= l.scroll;
        let mut y = top;
        let mut idx = 0usize;
        for (ri, r) in l.rows.iter().enumerate() {
            let rh = mt.row_h(r);
            if let Row::Node(id) = r {
                let is_folder = self.cfg.find(*id).map(|n| n.is_folder()).unwrap_or(false);
                if cy >= y && cy < y + rh {
                    let f = (cy - y) / rh;
                    if is_folder && *id != drag && (0.28..0.72).contains(&f) && !self.cfg.is_within(*id, drag) {
                        return Some(DropAt::Into(li, ri, *id));
                    }
                    return Some(if f < 0.5 {
                        DropAt::Before(li, folder, idx, y)
                    } else {
                        DropAt::Before(li, folder, idx + 1, y + rh)
                    });
                }
                idx += 1;
            } else if cy >= y {
                // Below the last node: append.
                return Some(DropAt::Before(li, folder, idx, y));
            }
            y += rh;
        }
        let _ = cx;
        Some(DropAt::Before(li, folder, idx, y))
    }

    fn finish_drag(&mut self, d: Drag, fx: &mut Vec<Effect>) {
        let ok = match d.at {
            Some(DropAt::Into(_, _, folder)) => {
                let n = self.cfg.find(folder).map(|f| f.children().len()).unwrap_or(0);
                self.cfg.move_to(d.id, folder, n)
            }
            Some(DropAt::Before(_, folder, idx, _)) => self.cfg.move_to(d.id, folder, idx),
            None => false,
        };
        if ok {
            fx.push(Effect::Save);
        }
        self.rebuild();
    }

    fn activate(&mut self, i: usize, r: usize, fx: &mut Vec<Effect>) {
        let Some(m) = self.menu.as_ref() else { return };
        let row = m.levels[i].rows[r];
        let kind = m.levels[i].kind;
        if !matches!(row, Row::Editor) && !matches!(row, Row::Node(id) if matches!(m.editor.as_ref().map(|e| e.kind), Some(EditKind::Rename(x)) if x == id))
        {
            self.commit_editor();
        }
        match (row, kind) {
            (Row::Node(id), _) => {
                let Some(n) = self.cfg.find(id) else { return };
                match &n.kind {
                    Kind::Folder { .. } => self.open_sub(i, r, fx),
                    Kind::Dir { path } | Kind::File { path } => {
                        let is_file = matches!(n.kind, Kind::File { .. });
                        let reveal = key_down(VK_CONTROL);
                        let target = self.menu.as_ref().unwrap().target;
                        fx.push(Effect::Close);
                        fx.push(Effect::Go { target, path: path.clone(), is_file, reveal });
                    }
                }
            }
            (Row::AddHere, LevelKind::Folder(folder)) => {
                fx.push(Effect::AddHere { folder, shift: key_down(VK_SHIFT) });
            }
            (Row::AddFolder, LevelKind::Folder(folder)) => self.start_editor(EditKind::NewFolder(folder), String::new()),
            (Row::Action(a), LevelKind::Context(id)) => self.context_action(a, id, fx),
            _ => {}
        }
    }

    pub fn start_editor(&mut self, kind: EditKind, text: String) {
        let Some(m) = self.menu.as_mut() else { return };
        let caret = text.chars().count();
        let all = !text.is_empty();
        m.editor = Some(Editor { kind, text, caret, all });
        // Close context menus; keep keyboard on the level hosting the editor.
        let host = match kind {
            EditKind::NewFolder(p) | EditKind::Path(p) => m.levels.iter().position(|l| l.kind == LevelKind::Folder(p)),
            EditKind::Rename(id) => self.cfg.locate(id).and_then(|(p, _)| m.levels.iter().position(|l| l.kind == LevelKind::Folder(p))),
        };
        if let Some(h) = host {
            self.truncate(h + 1);
            self.menu.as_mut().unwrap().active = h;
        }
        self.rebuild();
    }

    /// Apply the inline editor (Enter, or clicking elsewhere).
    pub fn commit_editor(&mut self) -> bool {
        let Some(e) = self.menu.as_mut().and_then(|m| m.editor.take()) else { return false };
        let text = e.text.trim().to_string();
        let changed = match e.kind {
            EditKind::NewFolder(p) => !text.is_empty() && self.cfg.add_folder(p, &text).is_some(),
            EditKind::Rename(id) => match self.cfg.find_mut(id) {
                Some(n) if !text.is_empty() && n.name != text => {
                    n.name = text;
                    true
                }
                _ => false,
            },
            EditKind::Path(p) => {
                let path = text.trim_matches('"').to_string();
                if !path.is_empty() && std::path::Path::new(&path).exists() {
                    self.cfg.add_shortcut(p, &path).is_some()
                } else {
                    false
                }
            }
        };
        if changed {
            let _ = self.cfg.save();
        }
        self.rebuild();
        changed
    }

    fn cancel_editor(&mut self) {
        if let Some(m) = self.menu.as_mut() {
            m.editor = None;
        }
        self.rebuild();
    }

    fn context_action(&mut self, a: Action, id: Id, fx: &mut Vec<Effect>) {
        match a {
            Action::Rename => {
                let name = self.cfg.find(id).map(|n| n.name.clone()).unwrap_or_default();
                self.start_editor(EditKind::Rename(id), name);
            }
            Action::Icon => {
                fx.push(Effect::Close);
                fx.push(Effect::Settings(vec!["--icon".into(), id.to_string()]));
            }
            Action::Up | Action::Down => {
                if self.cfg.shift(id, if a == Action::Up { -1 } else { 1 }) {
                    fx.push(Effect::Save);
                }
                self.rebuild();
            }
            Action::Reveal => {
                if let Some(p) = self.cfg.find(id).and_then(|n| n.path()).map(String::from) {
                    fx.push(Effect::Close);
                    fx.push(Effect::Go { target: None, path: p, is_file: false, reveal: true });
                }
            }
            Action::Remove => {
                if let Some((p, i)) = self.cfg.locate(id) {
                    if let Some(n) = self.cfg.remove(id) {
                        if let Some(m) = self.menu.as_mut() {
                            m.undo = Some((n, p, i));
                        }
                        fx.push(Effect::Save);
                    }
                }
                self.rebuild();
                let m = self.menu.as_mut().unwrap();
                m.active = m.levels.len() - 1;
            }
        }
    }

    fn on_char(&mut self, c: char, fx: &mut Vec<Effect>) {
        let Some(m) = self.menu.as_mut() else { return };
        if let Some(e) = m.editor.as_mut() {
            if c.is_control() {
                return;
            }
            if e.all {
                e.text.clear();
                e.caret = 0;
                e.all = false;
            }
            let bi = e.text.char_indices().nth(e.caret).map(|(b, _)| b).unwrap_or(e.text.len());
            e.text.insert(bi, c);
            e.caret += 1;
            self.rebuild();
            return;
        }
        // Type-ahead: jump to the next row starting with `c`.
        if c.is_control() || c == ' ' {
            return;
        }
        let ai = m.active.min(m.levels.len() - 1);
        let lc = c.to_lowercase().next().unwrap_or(c);
        let rows = m.levels[ai].rows.clone();
        let start = m.levels[ai].hover.map(|h| h + 1).unwrap_or(0);
        let n = rows.len();
        for k in 0..n {
            let r = (start + k) % n;
            let label = self.label(&rows[r]);
            if label.to_lowercase().starts_with(lc) {
                let m = self.menu.as_mut().unwrap();
                m.levels[ai].hover = Some(r);
                self.ensure_visible(ai, r);
                self.paint(ai);
                break;
            }
        }
        let _ = fx;
    }

    fn ensure_visible(&mut self, i: usize, r: usize) {
        let top_unscrolled = self.row_top(i, r) + self.menu.as_ref().unwrap().levels[i].scroll;
        let m = self.menu.as_mut().unwrap();
        let mt = m.mt;
        let l = &mut m.levels[i];
        let base = mt.m + mt.pad + if i == 0 { mt.header } else { 0.0 };
        let rel = top_unscrolled - base;
        let rh = mt.row_h(&l.rows[r]);
        if rel < l.scroll {
            l.scroll = rel;
        } else if rel + rh > l.scroll + l.view_h {
            l.scroll = rel + rh - l.view_h;
        }
        l.scroll = l.scroll.clamp(0.0, (l.content_h - l.view_h).max(0.0));
    }

    fn on_key(&mut self, vk: VIRTUAL_KEY, fx: &mut Vec<Effect>) {
        let Some(m) = self.menu.as_mut() else { return };
        if let Some(e) = m.editor.as_mut() {
            let len = e.text.chars().count();
            let byte = |t: &str, ci: usize| t.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(t.len());
            match vk {
                VK_RETURN => {
                    self.commit_editor();
                    return;
                }
                VK_ESCAPE => {
                    self.cancel_editor();
                    return;
                }
                VK_LEFT => {
                    e.caret = if e.all { 0 } else { e.caret.saturating_sub(1) };
                    e.all = false;
                }
                VK_RIGHT => {
                    e.caret = if e.all { len } else { (e.caret + 1).min(len) };
                    e.all = false;
                }
                VK_HOME => {
                    e.caret = 0;
                    e.all = false;
                }
                VK_END => {
                    e.caret = len;
                    e.all = false;
                }
                VK_BACK | VK_DELETE => {
                    if e.all {
                        e.text.clear();
                        e.caret = 0;
                        e.all = false;
                    } else if vk == VK_BACK && e.caret > 0 {
                        let b = byte(&e.text, e.caret - 1);
                        e.text.remove(b);
                        e.caret -= 1;
                    } else if vk == VK_DELETE && e.caret < len {
                        let b = byte(&e.text, e.caret);
                        e.text.remove(b);
                    }
                }
                _ if key_down(VK_CONTROL) && vk.0 == b'A' as u16 => e.all = true,
                _ if key_down(VK_CONTROL) && vk.0 == b'V' as u16 => {
                    if let Some(clip) = crate::app::clipboard_text() {
                        let clip: String = clip.lines().next().unwrap_or("").to_string();
                        if e.all {
                            e.text.clear();
                            e.caret = 0;
                            e.all = false;
                        }
                        let b = byte(&e.text, e.caret);
                        e.text.insert_str(b, &clip);
                        e.caret += clip.chars().count();
                    }
                }
                _ => return,
            }
            self.rebuild();
            return;
        }
        let ai = m.active.min(m.levels.len() - 1);
        let rows = m.levels[ai].rows.clone();
        let hover = m.levels[ai].hover;
        let selectable = |r: usize| rows.get(r).map(|x| *x != Row::Sep).unwrap_or(false);
        let n = rows.len();
        let step = |from: Option<usize>, dir: isize| -> Option<usize> {
            let mut r = match from {
                Some(h) => h as isize,
                None => if dir > 0 { -1 } else { n as isize },
            };
            for _ in 0..n {
                r = (r + dir).rem_euclid(n as isize);
                if selectable(r as usize) {
                    return Some(r as usize);
                }
            }
            None
        };
        match vk {
            VK_DOWN | VK_UP | VK_HOME | VK_END => {
                let r = match vk {
                    VK_DOWN => step(hover, 1),
                    VK_UP => step(hover, -1),
                    VK_HOME => step(None, 1),
                    _ => step(None, -1),
                };
                if let Some(r) = r {
                    self.menu.as_mut().unwrap().levels[ai].hover = Some(r);
                    self.ensure_visible(ai, r);
                    self.truncate(ai + 1);
                    self.paint(ai);
                }
            }
            VK_RIGHT => {
                if let Some(r) = hover {
                    self.open_sub(ai, r, fx);
                    let m = self.menu.as_mut().unwrap();
                    if m.levels.len() > ai + 1 {
                        m.active = ai + 1;
                        let first = m.levels[ai + 1].rows.iter().position(|x| *x != Row::Sep);
                        m.levels[ai + 1].hover = first;
                        self.paint(ai + 1);
                    }
                }
            }
            VK_RETURN => {
                if let Some(r) = hover {
                    self.activate(ai, r, fx);
                    let m = self.menu.as_mut().unwrap();
                    if m.levels.len() > ai + 1 && matches!(rows[r], Row::Node(_)) {
                        m.active = ai + 1;
                        let first = m.levels[ai + 1].rows.iter().position(|x| *x != Row::Sep);
                        m.levels[ai + 1].hover = first;
                        self.paint(ai + 1);
                    }
                }
            }
            VK_LEFT | VK_ESCAPE => {
                if ai == 0 {
                    if vk == VK_ESCAPE {
                        fx.push(Effect::Close);
                    }
                } else {
                    self.truncate(ai);
                    let m = self.menu.as_mut().unwrap();
                    m.active = ai - 1;
                }
            }
            VK_F2 => {
                if let Some(Row::Node(id)) = hover.map(|r| rows[r]) {
                    let name = self.cfg.find(id).map(|n| n.name.clone()).unwrap_or_default();
                    self.start_editor(EditKind::Rename(id), name);
                }
            }
            VK_DELETE => {
                if let Some(Row::Node(id)) = hover.map(|r| rows[r]) {
                    self.context_action(Action::Remove, id, fx);
                }
            }
            VK_APPS => {
                if let Some(r) = hover.filter(|&r| matches!(rows[r], Row::Node(_))) {
                    let top = self.row_top(ai, r);
                    let m = self.menu.as_ref().unwrap();
                    let l = &m.levels[ai];
                    let pt = POINT { x: l.x + l.w / 2, y: l.y + top as i32 };
                    self.open_context(ai, r, pt, fx);
                }
            }
            _ if key_down(VK_CONTROL) && vk.0 == b'Z' as u16 => {
                let undo = self.menu.as_mut().unwrap().undo.take();
                if let Some((node, p, i)) = undo {
                    if self.cfg.insert(p, Some(i), node) {
                        fx.push(Effect::Save);
                    }
                    self.rebuild();
                }
            }
            _ => {}
        }
    }

    /// Add paths to `folder` (after the effect resolved them outside the borrow).
    pub fn add_paths(&mut self, folder: Id, paths: Vec<String>) {
        let mut last = None;
        for p in paths {
            last = self.cfg.add_shortcut(folder, &p).or(last);
        }
        if last.is_some() {
            let _ = self.cfg.save();
        }
        self.rebuild();
        // Highlight the new entry.
        if let (Some(id), Some(m)) = (last, self.menu.as_mut()) {
            if let Some(li) = m.levels.iter().position(|l| l.kind == LevelKind::Folder(folder)) {
                if let Some(r) = m.levels[li].rows.iter().position(|r| *r == Row::Node(id)) {
                    m.levels[li].hover = Some(r);
                    m.active = li;
                    self.ensure_visible(li, r);
                    self.paint(li);
                }
            }
        }
    }

    pub fn set_cur_dir(&mut self, d: Option<String>) {
        if let Some(m) = self.menu.as_mut() {
            m.cur_dir = d;
            m.resolved = true;
        }
        self.rebuild();
    }

    pub fn menu_hwnds(&self) -> Vec<HWND> {
        self.menu.as_ref().map(|m| m.levels.iter().map(|l| l.hwnd).collect()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::model::*;

    unsafe extern "system" fn def_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe { DefWindowProcW(h, m, w, l) }
    }

    fn sample() -> Config {
        let mut c = Config { settings: Settings::default(), root: Config::default().root, next_id: 1 };
        c.root.children_mut().unwrap().clear();
        let p = c.add_folder(0, "Projects").unwrap();
        c.add_shortcut(0, r"C:\Windows").unwrap();
        c.add_shortcut(0, r"C:\Users").unwrap();
        c.add_shortcut(0, r"C:\Windows\notepad.exe").unwrap();
        c.add_shortcut(0, r"Z:\gone\offline").unwrap();
        let a = c.add_folder(p, "2026 — Riverside Library").unwrap();
        c.add_shortcut(p, r"C:\Program Files").unwrap();
        c.add_folder(a, "Families").unwrap();
        c
    }

    fn save_bmp(s: &Surface, path: &std::path::Path) {
        let (w, h) = (s.w as u32, s.h as u32);
        let px = unsafe { std::slice::from_raw_parts(s.bits, (w * h * 4) as usize) };
        let mut f = Vec::new();
        let size = 54 + w * h * 4;
        f.extend_from_slice(b"BM");
        f.extend_from_slice(&size.to_le_bytes());
        f.extend_from_slice(&[0; 4]);
        f.extend_from_slice(&54u32.to_le_bytes());
        f.extend_from_slice(&40u32.to_le_bytes());
        f.extend_from_slice(&(w as i32).to_le_bytes());
        f.extend_from_slice(&(-(h as i32)).to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&32u16.to_le_bytes());
        f.extend_from_slice(&[0; 24]);
        // Composite premultiplied pixels over a mid-gray "desktop".
        for c in px.chunks_exact(4) {
            let a = c[3] as u32;
            let bg = 0x80u32;
            let mix = |v: u8| (v as u32 + bg * (255 - a) / 255).min(255) as u8;
            f.extend_from_slice(&[mix(c[0]), mix(c[1]), mix(c[2]), 255]);
        }
        std::fs::write(path, f).unwrap();
    }

    /// Headless render of the popup to BMPs: `cargo test render_preview -- --ignored`
    /// (TACHYON_PREVIEW_DIR = output folder)
    #[test]
    #[ignore]
    fn render_preview() {
        unsafe {
            let hinst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap();
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(def_proc),
                hInstance: hinst.into(),
                lpszClassName: MENU_CLASS,
                ..Default::default()
            });
        }
        let out = std::path::PathBuf::from(std::env::var("TACHYON_PREVIEW_DIR").unwrap());
        for (name, theme, set) in [
            ("dark-lucide", ThemeChoice::Dark, IconSet::Lucide),
            ("light-fluent", ThemeChoice::Light, IconSet::FluentRegular),
            ("nebula-color", ThemeChoice::Nebula, IconSet::FluentColor),
            ("dark-shell", ThemeChoice::Dark, IconSet::Shell),
        ] {
            let mut cfg = sample();
            cfg.settings.theme = theme;
            cfg.settings.icon_set = set;
            let mut app = App { cfg, gfx: crate::gfx::Gfx::new().unwrap(), menu: None, exists: Default::default() };
            app.exists.insert(r"Z:\gone\offline".into(), false);
            app.open_menu(None, POINT { x: 200, y: 200 });
            app.set_cur_dir(Some(r"C:\Users\jonah\Documents\Revit\Projects\Riverside".into()));
            let mut fx = Vec::new();
            app.menu.as_mut().unwrap().levels[0].hover = Some(0);
            app.open_sub(0, 0, &mut fx);
            app.menu.as_mut().unwrap().levels[1].hover = Some(1);
            app.paint(0);
            app.paint(1);
            for (i, l) in app.menu.as_ref().unwrap().levels.iter().enumerate() {
                save_bmp(l.surface.as_ref().unwrap(), &out.join(format!("{name}-{i}.bmp")));
            }
            // Editor + context menu states.
            if name == "dark-lucide" {
                app.start_editor(EditKind::NewFolder(0), String::new());
                if let Some(e) = app.menu.as_mut().unwrap().editor.as_mut() {
                    e.text = "Client Archive".into();
                    e.caret = 6;
                }
                app.rebuild();
                save_bmp(app.menu.as_ref().unwrap().levels[0].surface.as_ref().unwrap(), &out.join("editor.bmp"));
                app.menu.as_mut().unwrap().editor = None;
                app.rebuild();
                app.open_context(0, 1, POINT { x: 300, y: 260 }, &mut fx);
                let m = app.menu.as_ref().unwrap();
                save_bmp(m.levels.last().unwrap().surface.as_ref().unwrap(), &out.join("context.bmp"));
            }
            app.close_menu();
        }
    }
}
