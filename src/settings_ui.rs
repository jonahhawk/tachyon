//! Settings window (separate `--settings` process, so the resident host
//! never loads egui/GL). Every change saves immediately and pings the host.

use crate::icons;
use crate::model::*;
use crate::platform::{self, SETTINGS_TITLE, WM_TACHYON_PREVIEW, WM_TACHYON_RELOAD};
use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions, Vec2};
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Appearance,
    Behavior,
    Shortcuts,
    About,
}

struct Picker {
    node: Id,
    set: usize,
    query: String,
    close_after: bool,
}

enum TreeOp {
    Up(Id),
    Down(Id),
    Remove(Id),
    AddFolder(Id),
    AddPath(Id, bool),
    Icon(Id),
}

struct SettingsApp {
    cfg: Config,
    page: Page,
    picker: Option<Picker>,
    tex: HashMap<String, TextureHandle>,
    mtime: Option<SystemTime>,
    last_poll: Instant,
    dirty: bool,
}

fn mtime() -> Option<SystemTime> {
    std::fs::metadata(Config::file()).and_then(|m| m.modified()).ok()
}

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

impl SettingsApp {
    fn save(&mut self) {
        let _ = self.cfg.save();
        self.mtime = mtime();
        platform::notify_host(WM_TACHYON_RELOAD);
    }

    fn apply_visuals(&self, ctx: &egui::Context) {
        let p = self.cfg.settings.palette();
        let lum = ((p.bg >> 16) & 0xFF) + ((p.bg >> 8) & 0xFF) + (p.bg & 0xFF);
        let mut v = if lum > 384 { egui::Visuals::light() } else { egui::Visuals::dark() };
        v.panel_fill = rgb(p.bg);
        v.window_fill = rgb(p.bg);
        v.extreme_bg_color = rgb(p.hover);
        v.selection.bg_fill = rgb(p.accent).gamma_multiply(0.45);
        v.hyperlink_color = rgb(p.accent);
        v.override_text_color = Some(rgb(p.fg));
        v.widgets.noninteractive.bg_stroke.color = rgb(p.border);
        v.widgets.hovered.weak_bg_fill = rgb(p.hover);
        v.window_corner_radius = egui::CornerRadius::same(10);
        ctx.set_visuals(v);
    }

    fn texture(&mut self, ctx: &egui::Context, r: &str, px: u32, budget: &mut u32) -> Option<TextureHandle> {
        if let Some(t) = self.tex.get(r) {
            return Some(t.clone());
        }
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        let tint = self.cfg.settings.palette().fg;
        let img = if r == "shell" {
            crate::gfx::shell_icon("", px, true).map(|mut b| {
                for c in b.chunks_exact_mut(4) {
                    c.swap(0, 2);
                }
                b
            })
        } else {
            icons::render(r, px, tint)
        }?;
        let t = ctx.load_texture(r, ColorImage::from_rgba_premultiplied([px as usize, px as usize], &img), TextureOptions::LINEAR);
        self.tex.insert(r.to_string(), t.clone());
        Some(t)
    }

    fn effective_icon(&self, n: &Node) -> Option<String> {
        n.icon.clone().or_else(|| {
            let role = match n.kind {
                Kind::Folder { .. } => icons::Role::Group,
                Kind::Dir { .. } => icons::Role::Dir,
                Kind::File { .. } => icons::Role::File,
            };
            let set = match self.cfg.settings.icon_set {
                IconSet::Shell | IconSet::None => IconSet::Lucide,
                s => s,
            };
            icons::default_icon(set, role)
        })
    }

    // ------------------------------------------------------------ pages

    fn appearance(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let s = &mut self.cfg.settings;
        section(ui, "Theme");
        ui.horizontal_wrapped(|ui| {
            for (t, l) in [
                (ThemeChoice::System, "System"),
                (ThemeChoice::Dark, "Dark"),
                (ThemeChoice::Light, "Light"),
                (ThemeChoice::Nebula, "Nebula"),
                (ThemeChoice::Custom, "Custom"),
            ] {
                changed |= ui.selectable_value(&mut s.theme, t, l).changed();
            }
        });
        if s.theme == ThemeChoice::Custom {
            ui.add_space(4.0);
            egui::Grid::new("colors").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                for (label, c) in [
                    ("Background", &mut s.custom.bg),
                    ("Text", &mut s.custom.fg),
                    ("Muted text", &mut s.custom.dim),
                    ("Accent", &mut s.custom.accent),
                    ("Hover", &mut s.custom.hover),
                    ("Border", &mut s.custom.border),
                ] {
                    ui.label(label);
                    let mut a = [(*c >> 16) as u8, (*c >> 8) as u8, *c as u8];
                    if ui.color_edit_button_srgb(&mut a).changed() {
                        *c = (a[0] as u32) << 16 | (a[1] as u32) << 8 | a[2] as u32;
                        changed = true;
                    }
                    ui.end_row();
                }
            });
            if ui.small_button("Start from Dark").clicked() {
                s.custom = Palette::DARK;
                changed = true;
            }
        }
        section(ui, "Popup");
        egui::Grid::new("popup").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.label("Opacity");
            changed |= ui.add(egui::Slider::new(&mut s.opacity, 0.3..=1.0).fixed_decimals(2)).changed();
            ui.end_row();
            ui.label("Text size");
            changed |= ui.add(egui::Slider::new(&mut s.font_size, 9.0..=22.0).step_by(0.5).suffix(" px")).changed();
            ui.end_row();
            ui.label("Font");
            changed |= ui.text_edit_singleline(&mut s.font_family).lost_focus();
            ui.end_row();
            ui.label("Icon set");
            egui::ComboBox::from_id_salt("iconset").selected_text(s.icon_set.label()).show_ui(ui, |ui| {
                for set in IconSet::ALL {
                    changed |= ui.selectable_value(&mut s.icon_set, set, set.label()).changed();
                }
            });
            ui.end_row();
            ui.label("Location header");
            changed |= ui.checkbox(&mut s.show_header, "Show current folder at the top").changed();
            ui.end_row();
        });
        ui.add_space(10.0);
        if ui.button("Preview popup").on_hover_text("Opens the popup at the mouse cursor").clicked() {
            platform::notify_host(WM_TACHYON_PREVIEW);
        }
        if changed {
            self.tex.clear();
            self.dirty = true;
        }
    }

    fn behavior(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let s = &mut self.cfg.settings;
        section(ui, "Trigger");
        egui::Grid::new("trigger").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.label("Middle-click with");
            ui.horizontal(|ui| {
                for (m, l) in [(Modifier::None, "Nothing"), (Modifier::Ctrl, "Ctrl"), (Modifier::Alt, "Alt"), (Modifier::Shift, "Shift")] {
                    changed |= ui.selectable_value(&mut s.trigger_modifier, m, l).changed();
                }
            });
            ui.end_row();
            ui.label("Hotkey");
            changed |= ui.checkbox(&mut s.hotkey, "Ctrl + Alt + Space opens the popup").changed();
            ui.end_row();
            ui.label("Submenu delay");
            changed |= ui.add(egui::Slider::new(&mut s.hover_delay_ms, 0..=600).suffix(" ms")).changed();
            ui.end_row();
        });
        section(ui, "System");
        if ui.checkbox(&mut s.autostart, "Start with Windows").changed() {
            platform::set_autostart(s.autostart);
            changed = true;
        }
        if changed {
            self.dirty = true;
        }
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Edit names, paths and icons. Drag-reordering is available in the popup itself.").weak());
        ui.add_space(6.0);
        let mut ops = Vec::new();
        let mut changed = false;
        let mut budget = 64;
        let root = self.cfg.root.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.tree(ui, &root, &mut ops, &mut changed, &mut budget);
        });
        if budget == 0 {
            ui.ctx().request_repaint();
        }
        for op in ops {
            match op {
                TreeOp::Up(id) => changed |= self.cfg.shift(id, -1),
                TreeOp::Down(id) => changed |= self.cfg.shift(id, 1),
                TreeOp::Remove(id) => changed |= self.cfg.remove(id).is_some(),
                TreeOp::AddFolder(p) => changed |= self.cfg.add_folder(p, "New Folder").is_some(),
                TreeOp::AddPath(p, folder) => {
                    if let Some(path) = pick_path(folder) {
                        changed |= self.cfg.add_shortcut(p, &path).is_some();
                    }
                }
                TreeOp::Icon(id) => {
                    self.picker = Some(Picker { node: id, set: 0, query: String::new(), close_after: false });
                }
            }
        }
        if changed {
            self.dirty = true;
        }
    }

    fn tree(&mut self, ui: &mut egui::Ui, folder: &Node, ops: &mut Vec<TreeOp>, changed: &mut bool, budget: &mut u32) {
        let n = folder.children().len();
        for (i, child) in folder.children().iter().enumerate() {
            let id = child.id;
            let draw_row = |this: &mut SettingsApp, ui: &mut egui::Ui, ops: &mut Vec<TreeOp>, changed: &mut bool, budget: &mut u32| {
                let icon = this.effective_icon(child);
                let tex = icon.as_deref().and_then(|r| this.texture(ui.ctx(), r, 32, budget));
                let btn = match tex {
                    Some(t) => ui.add(egui::Button::image(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(16.0)))),
                    None => ui.add(egui::Button::new("·").min_size(Vec2::splat(22.0))),
                };
                if btn.on_hover_text("Change icon").clicked() {
                    ops.push(TreeOp::Icon(id));
                }
                let Some(node) = this.cfg.find_mut(id) else { return };
                *changed |= ui.add(egui::TextEdit::singleline(&mut node.name).desired_width(170.0)).changed();
                if let Kind::Dir { path } | Kind::File { path } = &mut node.kind {
                    let missing = !std::path::Path::new(path.as_str()).exists();
                    let te = egui::TextEdit::singleline(path).desired_width(ui.available_width() - 90.0);
                    let te = if missing { te.text_color(Color32::from_rgb(0xE5, 0x48, 0x4D)) } else { te };
                    *changed |= ui.add(te).on_hover_text(if missing { "Path not found" } else { "" }).changed();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").on_hover_text("Remove").clicked() {
                        ops.push(TreeOp::Remove(id));
                    }
                    if ui.add_enabled(i + 1 < n, egui::Button::new("↓").small()).clicked() {
                        ops.push(TreeOp::Down(id));
                    }
                    if ui.add_enabled(i > 0, egui::Button::new("↑").small()).clicked() {
                        ops.push(TreeOp::Up(id));
                    }
                });
            };
            if child.is_folder() {
                let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id(id), false);
                state
                    .show_header(ui, |ui| draw_row(self, ui, ops, changed, budget))
                    .body(|ui| self.tree(ui, child, ops, changed, budget));
            } else {
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    draw_row(self, ui, ops, changed, budget);
                });
            }
        }
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            if ui.small_button("+ Folder").clicked() {
                ops.push(TreeOp::AddFolder(folder.id));
            }
            if ui.small_button("+ Directory…").clicked() {
                ops.push(TreeOp::AddPath(folder.id, true));
            }
            if ui.small_button("+ File…").clicked() {
                ops.push(TreeOp::AddPath(folder.id, false));
            }
        });
    }

    fn about(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tachyon");
        ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
        ui.label(egui::RichText::new("Faster-than-light jumps between directories.").weak());
        section(ui, "Using it");
        for (k, v) in [
            ("Middle-click", "Open the popup in Explorer or any Open/Save dialog"),
            ("Ctrl+Alt+Space", "Open the popup for the active window"),
            ("Add Here", "Add the window's current folder to this menu"),
            ("Shift + Add Here", "Add the selected file(s) instead"),
            ("Ctrl + click", "Open the shortcut in a new Explorer window"),
            ("Right-click / F2 / Del", "Edit, rename, remove — Ctrl+Z undoes a removal"),
            ("Drag", "Reorder, or drop onto a folder to move into it"),
            ("Ctrl+Shift+middle-click", "Write a diagnostic dump of the window (for unsupported dialogs)"),
        ] {
            ui.horizontal(|ui| {
                ui.add_sized([170.0, 18.0], egui::Label::new(egui::RichText::new(k).strong()));
                ui.label(v);
            });
        }
        section(ui, "Licenses");
        ui.label("Lucide icons — ISC License, © Lucide Contributors");
        ui.label("Fluent UI System Icons — MIT License, © Microsoft Corporation");
        ui.add_space(6.0);
        if ui.button("Open config folder").clicked() {
            platform::reveal(&Config::dir().to_string_lossy());
        }
    }

    fn picker_ui(&mut self, ctx: &egui::Context) {
        let Some(mut p) = self.picker.take() else { return };
        let mut done = false;
        let mut chosen: Option<Option<String>> = None;
        let name = self.cfg.find(p.node).map(|n| n.name.clone()).unwrap_or_default();
        egui::Modal::new(egui::Id::new("picker")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                ui.heading(format!("Icon for “{name}”"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        done = true;
                    }
                    if ui.button("Default").on_hover_text("Use the icon set's default").clicked() {
                        chosen = Some(None);
                    }
                    if ui.button("Windows").on_hover_text("Use the native Windows icon for this path").clicked() {
                        chosen = Some(Some("shell".into()));
                    }
                });
            });
            ui.horizontal(|ui| {
                for (i, (_, label)) in icons::SETS.iter().enumerate() {
                    ui.selectable_value(&mut p.set, i, *label);
                }
            });
            let search = ui.add(egui::TextEdit::singleline(&mut p.query).hint_text("Search icons…").desired_width(f32::INFINITY));
            if search.changed() || !search.has_focus() && p.query.is_empty() {
                search.request_focus();
            }
            let set = icons::SETS[p.set].0;
            let q = p.query.to_lowercase().replace(' ', "-");
            let q2 = q.replace('-', "_");
            let names: Vec<&str> = icons::list(set).into_iter().filter(|n| q.is_empty() || n.contains(&q) || n.contains(&q2)).collect();
            ui.label(egui::RichText::new(format!("{} icons", names.len())).weak().small());
            let cell = 40.0;
            let cols = ((ui.available_width() / cell).floor() as usize).max(1);
            let rows = names.len().div_ceil(cols);
            let mut budget = 120;
            egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, false]).show_rows(ui, cell, rows, |ui, range| {
                for r in range {
                    ui.horizontal(|ui| {
                        for c in 0..cols {
                            let Some(n) = names.get(r * cols + c) else { break };
                            let full = format!("{set}/{n}");
                            let tex = self.texture(ctx, &full, 48, &mut budget);
                            let resp = match tex {
                                Some(t) => ui.add_sized(
                                    [cell - 6.0, cell - 6.0],
                                    egui::Button::image(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(22.0))),
                                ),
                                None => ui.add_sized([cell - 6.0, cell - 6.0], egui::Button::new("")),
                            };
                            if resp.on_hover_text(*n).clicked() {
                                chosen = Some(Some(full));
                            }
                        }
                    });
                }
            });
            if budget == 0 {
                ctx.request_repaint();
            }
        });
        if let Some(icon) = chosen {
            if let Some(n) = self.cfg.find_mut(p.node) {
                n.icon = icon;
            }
            self.save();
            done = true;
            if p.close_after {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else if done && p.close_after {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if !done {
            self.picker = Some(p);
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(12.0);
    ui.label(egui::RichText::new(title.to_uppercase()).small().strong().weak());
    ui.add_space(4.0);
}

/// Native folder/file picker.
fn pick_path(folder: bool) -> Option<String> {
    use windows::Win32::System::Com::*;
    use windows::Win32::UI::Shell::*;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        if folder {
            let o = d.GetOptions().ok()?;
            d.SetOptions(o | FOS_PICKFOLDERS).ok()?;
        }
        d.Show(None).ok()?;
        let item = d.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Pick up changes the popup made (Add Here, drag, rename…).
        if self.last_poll.elapsed() > Duration::from_millis(700) {
            self.last_poll = Instant::now();
            let m = mtime();
            if m != self.mtime && ctx.memory(|mem| mem.focused().is_none()) {
                if let Some(c) = Config::load_from(&Config::file()) {
                    self.cfg = c;
                    self.tex.clear();
                }
                self.mtime = m;
            }
        }
        ctx.request_repaint_after(Duration::from_millis(800));
        self.apply_visuals(&ctx);

        egui::Panel::left("nav").exact_size(150.0).resizable(false).show(ui, |ui| {
            ui.add_space(14.0);
            ui.label(egui::RichText::new("tachyon").size(20.0).color(rgb(self.cfg.settings.palette().accent)));
            ui.add_space(14.0);
            for (p, l) in [
                (Page::Appearance, "Appearance"),
                (Page::Behavior, "Behavior"),
                (Page::Shortcuts, "Shortcuts"),
                (Page::About, "About"),
            ] {
                ui.add_sized([130.0, 28.0], egui::Button::selectable(self.page == p, l)).clicked().then(|| self.page = p);
            }
        });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("page").show(ui, |ui| {
                ui.add_space(6.0);
                match self.page {
                    Page::Appearance => self.appearance(ui),
                    Page::Behavior => self.behavior(ui),
                    Page::Shortcuts => self.shortcuts(ui),
                    Page::About => self.about(ui),
                }
            });
        });
        self.picker_ui(&ctx);
        if self.dirty {
            self.dirty = false;
            self.save();
        }
    }
}

pub fn run(args: &[String]) {
    let cfg = Config::load();
    let picker = match args {
        [flag, id, ..] if flag == "--icon" => {
            id.parse().ok().map(|node| Picker { node, set: 0, query: String::new(), close_after: true })
        }
        _ => None,
    };
    let page = if picker.is_some() { Page::Shortcuts } else { Page::Appearance };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(SETTINGS_TITLE)
        .with_inner_size([780.0, 560.0])
        .with_min_inner_size([620.0, 420.0]);
    if let Some(px) = icons::render_svg(include_str!("../assets/logo.svg"), 64) {
        viewport = viewport.with_icon(egui::IconData { rgba: px, width: 64, height: 64 });
    }
    let opts = eframe::NativeOptions { viewport, centered: true, ..Default::default() };
    let app = SettingsApp {
        cfg,
        page,
        picker,
        tex: HashMap::new(),
        mtime: mtime(),
        last_poll: Instant::now(),
        dirty: false,
    };
    let _ = eframe::run_native(SETTINGS_TITLE, opts, Box::new(|_cc| Ok(Box::new(app))));
}
