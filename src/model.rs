//! Shortcut tree, settings, and JSON persistence.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub type Id = u64;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase", tag = "type")]
pub enum Kind {
    Folder { children: Vec<Node> },
    Dir { path: String },
    File { path: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Node {
    pub id: Id,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(flatten)]
    pub kind: Kind,
}

impl Node {
    pub fn is_folder(&self) -> bool {
        matches!(self.kind, Kind::Folder { .. })
    }
    pub fn children(&self) -> &[Node] {
        match &self.kind {
            Kind::Folder { children } => children,
            _ => &[],
        }
    }
    pub fn children_mut(&mut self) -> Option<&mut Vec<Node>> {
        match &mut self.kind {
            Kind::Folder { children } => Some(children),
            _ => None,
        }
    }
    pub fn path(&self) -> Option<&str> {
        match &self.kind {
            Kind::Dir { path } | Kind::File { path } => Some(path),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThemeChoice {
    System,
    Dark,
    Light,
    Nebula,
    Custom,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum IconSet {
    Lucide,
    FluentRegular,
    FluentFilled,
    FluentColor,
    Shell,
    None,
}

impl IconSet {
    pub const ALL: [IconSet; 6] = [
        IconSet::Lucide,
        IconSet::FluentRegular,
        IconSet::FluentFilled,
        IconSet::FluentColor,
        IconSet::Shell,
        IconSet::None,
    ];
    pub fn label(self) -> &'static str {
        match self {
            IconSet::Lucide => "Lucide",
            IconSet::FluentRegular => "Fluent",
            IconSet::FluentFilled => "Fluent Filled",
            IconSet::FluentColor => "Fluent Color",
            IconSet::Shell => "Windows",
            IconSet::None => "None",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Modifier {
    None,
    Ctrl,
    Alt,
    Shift,
}

/// RGB colors stored as 0xRRGGBB.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Palette {
    pub bg: u32,
    pub fg: u32,
    pub dim: u32,
    pub accent: u32,
    pub hover: u32,
    pub border: u32,
}

impl Palette {
    pub const DARK: Palette = Palette {
        bg: 0x16181D,
        fg: 0xE6E8EC,
        dim: 0x8A909C,
        accent: 0x5EEAD4,
        hover: 0x2A2E37,
        border: 0x30343D,
    };
    pub const LIGHT: Palette = Palette {
        bg: 0xFAFAFB,
        fg: 0x1C1F24,
        dim: 0x6B7280,
        accent: 0x0D9488,
        hover: 0xE9EBEF,
        border: 0xD9DCE1,
    };
    pub const NEBULA: Palette = Palette {
        bg: 0x15122A,
        fg: 0xECE8FF,
        dim: 0x9A90C8,
        accent: 0xC4B5FD,
        hover: 0x2A2450,
        border: 0x3A3268,
    };
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub custom: Palette,
    /// 0.3 ..= 1.0
    pub opacity: f32,
    /// Text size in DIPs.
    pub font_size: f32,
    pub font_family: String,
    pub icon_set: IconSet,
    pub trigger_modifier: Modifier,
    pub hotkey: bool,
    pub autostart: bool,
    pub hover_delay_ms: u32,
    /// Show the current directory at the top of the root menu.
    pub show_header: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeChoice::System,
            custom: Palette::DARK,
            opacity: 0.97,
            font_size: 13.0,
            font_family: "Segoe UI Variable Text".into(),
            icon_set: IconSet::Lucide,
            trigger_modifier: Modifier::None,
            hotkey: true,
            autostart: false,
            hover_delay_ms: 120,
            show_header: true,
        }
    }
}

impl Settings {
    pub fn palette(&self) -> Palette {
        match self.theme {
            ThemeChoice::System => {
                if crate::platform::system_uses_light_theme() {
                    Palette::LIGHT
                } else {
                    Palette::DARK
                }
            }
            ThemeChoice::Dark => Palette::DARK,
            ThemeChoice::Light => Palette::LIGHT,
            ThemeChoice::Nebula => Palette::NEBULA,
            ThemeChoice::Custom => self.custom,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub settings: Settings,
    pub root: Node,
    pub next_id: Id,
}

impl Default for Config {
    fn default() -> Self {
        let mut c = Config {
            settings: Settings::default(),
            root: Node {
                id: 0,
                name: "Tachyon".into(),
                icon: None,
                kind: Kind::Folder { children: vec![] },
            },
            next_id: 1,
        };
        // Seed with a few useful jumps.
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        for (name, sub, icon) in [
            ("Desktop", "Desktop", "lucide/monitor"),
            ("Documents", "Documents", "lucide/file-text"),
            ("Downloads", "Downloads", "lucide/download"),
        ] {
            let p = Path::new(&home).join(sub);
            if p.is_dir() {
                let id = c.alloc_id();
                c.root.children_mut().unwrap().push(Node {
                    id,
                    name: name.into(),
                    icon: Some(icon.into()),
                    kind: Kind::Dir { path: p.to_string_lossy().into_owned() },
                });
            }
        }
        c
    }
}

impl Config {
    pub fn dir() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| ".".into());
        base.join("Tachyon")
    }
    pub fn file() -> PathBuf {
        Self::dir().join("tachyon.json")
    }

    pub fn load() -> Config {
        Self::load_from(&Self::file()).unwrap_or_default()
    }

    pub fn load_from(p: &Path) -> Option<Config> {
        let s = std::fs::read(p).ok()?;
        let mut c: Config = serde_json::from_slice(&s).ok()?;
        // Repair id counter if the file was hand-edited.
        let max = c.max_id();
        if c.next_id <= max {
            c.next_id = max + 1;
        }
        Some(c)
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::file())
    }

    pub fn save_to(&self, p: &Path) -> std::io::Result<()> {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap())?;
        std::fs::rename(&tmp, p)
    }

    pub fn alloc_id(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn max_id(&self) -> Id {
        fn walk(n: &Node, m: &mut Id) {
            *m = (*m).max(n.id);
            for c in n.children() {
                walk(c, m);
            }
        }
        let mut m = 0;
        walk(&self.root, &mut m);
        m
    }

    pub fn find(&self, id: Id) -> Option<&Node> {
        fn walk(n: &Node, id: Id) -> Option<&Node> {
            if n.id == id {
                return Some(n);
            }
            n.children().iter().find_map(|c| walk(c, id))
        }
        walk(&self.root, id)
    }

    pub fn find_mut(&mut self, id: Id) -> Option<&mut Node> {
        fn walk(n: &mut Node, id: Id) -> Option<&mut Node> {
            if n.id == id {
                return Some(n);
            }
            match &mut n.kind {
                Kind::Folder { children } => children.iter_mut().find_map(|c| walk(c, id)),
                _ => None,
            }
        }
        walk(&mut self.root, id)
    }

    /// Parent folder id and index of `id`.
    pub fn locate(&self, id: Id) -> Option<(Id, usize)> {
        fn walk(n: &Node, id: Id) -> Option<(Id, usize)> {
            for (i, c) in n.children().iter().enumerate() {
                if c.id == id {
                    return Some((n.id, i));
                }
                if let Some(r) = walk(c, id) {
                    return Some(r);
                }
            }
            None
        }
        walk(&self.root, id)
    }

    /// Is `id` equal to or inside `ancestor`?
    pub fn is_within(&self, id: Id, ancestor: Id) -> bool {
        self.find(ancestor)
            .map(|a| {
                fn walk(n: &Node, id: Id) -> bool {
                    n.id == id || n.children().iter().any(|c| walk(c, id))
                }
                walk(a, id)
            })
            .unwrap_or(false)
    }

    pub fn insert(&mut self, parent: Id, index: Option<usize>, node: Node) -> bool {
        match self.find_mut(parent).and_then(|p| p.children_mut()) {
            Some(ch) => {
                let i = index.unwrap_or(ch.len()).min(ch.len());
                ch.insert(i, node);
                true
            }
            None => false,
        }
    }

    pub fn add_shortcut(&mut self, parent: Id, path: &str) -> Option<Id> {
        let p = Path::new(path);
        let is_dir = p.is_dir();
        let name = display_name(path);
        let id = self.alloc_id();
        let kind = if is_dir {
            Kind::Dir { path: path.to_string() }
        } else {
            Kind::File { path: path.to_string() }
        };
        self.insert(parent, None, Node { id, name, icon: None, kind }).then_some(id)
    }

    pub fn add_folder(&mut self, parent: Id, name: &str) -> Option<Id> {
        let id = self.alloc_id();
        let name = if name.trim().is_empty() { "New Folder" } else { name.trim() };
        self.insert(
            parent,
            None,
            Node { id, name: name.into(), icon: None, kind: Kind::Folder { children: vec![] } },
        )
        .then_some(id)
    }

    pub fn remove(&mut self, id: Id) -> Option<Node> {
        let (parent, i) = self.locate(id)?;
        Some(self.find_mut(parent)?.children_mut()?.remove(i))
    }

    /// Move `id` into `parent` at `index` (index is in terms of the list *before* removal).
    pub fn move_to(&mut self, id: Id, parent: Id, index: usize) -> bool {
        if id == 0 || self.is_within(parent, id) {
            return false;
        }
        let Some((old_parent, old_i)) = self.locate(id) else { return false };
        let mut index = index;
        if old_parent == parent && old_i < index {
            index -= 1;
        }
        let Some(node) = self.remove(id) else { return false };
        self.insert(parent, Some(index), node)
    }

    pub fn shift(&mut self, id: Id, delta: isize) -> bool {
        let Some((parent, i)) = self.locate(id) else { return false };
        let Some(ch) = self.find_mut(parent).and_then(|p| p.children_mut()) else { return false };
        let j = i as isize + delta;
        if j < 0 || j as usize >= ch.len() {
            return false;
        }
        ch.swap(i, j as usize);
        true
    }
}

/// Leaf name for a path: "C:\\a\\b" -> "b", "C:\\" -> "C:".
pub fn display_name(path: &str) -> String {
    let t = path.trim_end_matches(['\\', '/']);
    match t.rsplit(['\\', '/']).next() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => t.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Config {
        Config {
            settings: Settings::default(),
            root: Node { id: 0, name: "r".into(), icon: None, kind: Kind::Folder { children: vec![] } },
            next_id: 1,
        }
    }

    #[test]
    fn add_move_remove_roundtrip() {
        let mut c = empty();
        let a = c.add_folder(0, "A").unwrap();
        let b = c.add_folder(a, "B").unwrap();
        let s = c.add_shortcut(b, "C:\\Windows").unwrap();
        assert_eq!(c.locate(s), Some((b, 0)));
        assert!(!c.move_to(a, b, 0), "cannot move into own descendant");
        assert!(c.move_to(s, 0, 0));
        assert_eq!(c.locate(s), Some((0, 0)));
        assert!(c.shift(s, 1));
        assert_eq!(c.locate(s), Some((0, 1)));
        let json = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
        assert!(c.remove(a).is_some());
        assert!(c.find(b).is_none());
    }

    #[test]
    fn move_within_same_parent_adjusts_index() {
        let mut c = empty();
        let x = c.add_folder(0, "x").unwrap();
        let y = c.add_folder(0, "y").unwrap();
        let z = c.add_folder(0, "z").unwrap();
        assert!(c.move_to(x, 0, 3));
        let ids: Vec<_> = c.root.children().iter().map(|n| n.id).collect();
        assert_eq!(ids, vec![y, z, x]);
    }

    #[test]
    fn names() {
        assert_eq!(display_name("C:\\Users\\me\\"), "me");
        assert_eq!(display_name("C:\\"), "C:");
        assert_eq!(display_name("\\\\srv\\share"), "share");
    }
}
