//! Embedded icon archive (built by build.rs) and SVG rasterization.
//!
//! Icon references are `"<set>/<name>"`, e.g. `"lucide/folder"`,
//! `"fluent-color/rocket"`, or the special `"shell"` (native Windows icon of
//! the target path).

use crate::model::IconSet;
use std::sync::OnceLock;

static PAK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icons.pak"));

pub const SETS: [(&str, &str); 4] = [
    ("lucide", "Lucide"),
    ("fluent-regular", "Fluent"),
    ("fluent-filled", "Fluent Filled"),
    ("fluent-color", "Fluent Color"),
];

struct Entry {
    name: &'static str,
    off: u32,
    len: u32,
    raw: u32,
}

struct Index {
    entries: Vec<Entry>,
    blob: &'static [u8],
}

fn index() -> &'static Index {
    static IDX: OnceLock<Index> = OnceLock::new();
    IDX.get_or_init(|| {
        let rd = |o: usize| u32::from_le_bytes(PAK[o..o + 4].try_into().unwrap());
        assert_eq!(&PAK[..4], b"TPK1");
        let count = rd(4) as usize;
        let ilen = rd(8) as usize;
        let mut entries = Vec::with_capacity(count);
        let mut o = 12;
        for _ in 0..count {
            let nl = u16::from_le_bytes([PAK[o], PAK[o + 1]]) as usize;
            o += 2;
            let name = std::str::from_utf8(&PAK[o..o + nl]).unwrap();
            o += nl;
            entries.push(Entry { name, off: rd(o), len: rd(o + 4), raw: rd(o + 8) });
            o += 12;
        }
        Index { entries, blob: &PAK[12 + ilen..] }
    })
}

/// All icon names in a set (without the set prefix), sorted.
pub fn list(set: &str) -> Vec<&'static str> {
    let prefix_len = set.len() + 1;
    index()
        .entries
        .iter()
        .filter(|e| e.name.len() > prefix_len && e.name.starts_with(set) && e.name.as_bytes()[set.len()] == b'/')
        .map(|e| &e.name[prefix_len..])
        .collect()
}

pub fn exists(r: &str) -> bool {
    index().entries.binary_search_by(|e| e.name.cmp(r)).is_ok()
}

fn svg_source(r: &str) -> Option<String> {
    let idx = index();
    let i = idx.entries.binary_search_by(|e| e.name.cmp(r)).ok()?;
    let e = &idx.entries[i];
    let comp = &idx.blob[e.off as usize..(e.off + e.len) as usize];
    let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(comp, e.raw as usize).ok()?;
    String::from_utf8(raw).ok()
}

/// Rasterize to premultiplied RGBA, `size`×`size`. `tint` = 0xRRGGBB for mono icons.
pub fn render(r: &str, size: u32, tint: u32) -> Option<Vec<u8>> {
    let mut svg = svg_source(r)?;
    let hex = format!("#{tint:06X}");
    if r.starts_with("lucide/") {
        svg = svg.replace("currentColor", &hex);
    } else if r.starts_with("fluent-regular/") || r.starts_with("fluent-filled/") {
        svg = svg.replacen("<svg ", &format!("<svg fill=\"{hex}\" "), 1);
    }
    render_svg(&svg, size)
}

/// Rasterize arbitrary SVG source to premultiplied RGBA.
pub fn render_svg(svg: &str, size: u32) -> Option<Vec<u8>> {
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_str(svg, &opt).ok()?;
    let mut pm = resvg::tiny_skia::Pixmap::new(size, size)?;
    let ts = tree.size();
    let s = (size as f32 / ts.width()).min(size as f32 / ts.height());
    let dx = (size as f32 - ts.width() * s) / 2.0;
    let dy = (size as f32 - ts.height() * s) / 2.0;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_row(s, 0.0, 0.0, s, dx, dy), &mut pm.as_mut());
    Some(pm.take())
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Role {
    Group,
    Dir,
    File,
    AddHere,
    AddFolder,
    Gear,
    Rename,
    Icon,
    Up,
    Down,
    Remove,
    Reveal,
}

/// Default icon reference for a role in a given set.
pub fn default_icon(set: IconSet, role: Role) -> Option<String> {
    let (prefix, name) = match set {
        IconSet::None => return None,
        IconSet::Lucide | IconSet::Shell => (
            "lucide",
            match role {
                Role::Group => "folders",
                Role::Dir => "folder",
                Role::File => "file",
                Role::AddHere => "plus",
                Role::AddFolder => "folder-plus",
                Role::Gear => "settings",
                Role::Rename => "pencil",
                Role::Icon => "image",
                Role::Up => "arrow-up",
                Role::Down => "arrow-down",
                Role::Remove => "trash-2",
                Role::Reveal => "external-link",
            },
        ),
        IconSet::FluentRegular | IconSet::FluentFilled | IconSet::FluentColor => (
            match set {
                IconSet::FluentRegular => "fluent-regular",
                IconSet::FluentFilled => "fluent-filled",
                _ => "fluent-color",
            },
            match role {
                Role::Group => "folder_multiple",
                Role::Dir => "folder",
                Role::File => "document",
                Role::AddHere => "add",
                Role::AddFolder => "folder_add",
                Role::Gear => "settings",
                Role::Rename => "rename",
                Role::Icon => "image",
                Role::Up => "arrow_up",
                Role::Down => "arrow_down",
                Role::Remove => "delete",
                Role::Reveal => "open",
            },
        ),
    };
    let r = format!("{prefix}/{name}");
    if exists(&r) {
        return Some(r);
    }
    // Fluent Color has a small catalog; fall back to Filled for missing glyphs.
    let alt = format!("fluent-filled/{name}");
    exists(&alt).then_some(alt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pack_and_render() {
        assert!(list("lucide").len() > 1000);
        assert!(exists("lucide/folder"));
        let px = render("lucide/folder", 16, 0xFFFFFF).unwrap();
        assert_eq!(px.len(), 16 * 16 * 4);
        assert!(px.iter().any(|&b| b != 0));
        for set in IconSet::ALL {
            for role in [Role::Group, Role::Dir, Role::File, Role::AddHere, Role::AddFolder, Role::Gear] {
                if set != IconSet::None {
                    assert!(default_icon(set, role).is_some(), "{set:?} {role:?}");
                }
            }
        }
    }

    /// Regenerate assets/tachyon.ico from assets/logo.svg:
    /// `cargo test gen_icon -- --ignored`
    #[test]
    #[ignore]
    fn gen_icon() {
        let svg = std::fs::read_to_string("assets/logo.svg").unwrap();
        let sizes = [16u32, 20, 24, 32, 40, 48, 64, 256];
        let mut images = Vec::new();
        for &sz in &sizes {
            let px = render_svg(&svg, sz).unwrap();
            // BMP payload: BITMAPINFOHEADER, bottom-up BGRA (straight alpha), AND mask.
            let mut b = Vec::new();
            for v in [40u32, sz, sz * 2] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&1u16.to_le_bytes());
            b.extend_from_slice(&32u16.to_le_bytes());
            b.extend_from_slice(&[0u8; 24]);
            for y in (0..sz).rev() {
                for x in 0..sz {
                    let i = ((y * sz + x) * 4) as usize;
                    let a = px[i + 3] as u32;
                    let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a / 2) / a).min(255) as u8 };
                    b.extend_from_slice(&[un(px[i + 2]), un(px[i + 1]), un(px[i]), a as u8]);
                }
            }
            let row = sz.div_ceil(32) * 4;
            b.extend(std::iter::repeat_n(0u8, (row * sz) as usize));
            images.push((sz, b));
        }
        let mut ico = Vec::new();
        ico.extend_from_slice(&[0, 0, 1, 0]);
        ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
        let mut off = 6 + 16 * images.len() as u32;
        for (sz, b) in &images {
            let d = if *sz >= 256 { 0 } else { *sz as u8 };
            ico.extend_from_slice(&[d, d, 0, 0]);
            ico.extend_from_slice(&1u16.to_le_bytes());
            ico.extend_from_slice(&32u16.to_le_bytes());
            ico.extend_from_slice(&(b.len() as u32).to_le_bytes());
            ico.extend_from_slice(&off.to_le_bytes());
            off += b.len() as u32;
        }
        for (_, b) in &images {
            ico.extend_from_slice(b);
        }
        std::fs::write("assets/tachyon.ico", ico).unwrap();
    }
}
