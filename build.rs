//! Packs every SVG under `assets/icons/<set>/` into one indexed archive with
//! per-icon deflate, so the exe can decompress one icon at a time.
//! Also embeds the manifest and app icon.

use std::{env, fs, path::Path};

fn minify(svg: &str) -> String {
    let mut s = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(i) = rest.find("<!--") {
        s.push_str(&rest[..i]);
        rest = match rest[i..].find("-->") {
            Some(j) => &rest[i + j + 3..],
            None => "",
        };
    }
    s.push_str(rest);
    let mut out = String::with_capacity(s.len());
    let mut ws = false;
    for c in s.chars() {
        if c.is_whitespace() {
            ws = true;
        } else {
            if ws && !out.is_empty() && !out.ends_with('>') && c != '>' && c != '/' {
                out.push(' ');
            }
            ws = false;
            out.push(c);
        }
    }
    out
}

fn main() {
    println!("cargo:rerun-if-changed=assets/icons");
    println!("cargo:rerun-if-changed=assets/tachyon.ico");
    println!("cargo:rerun-if-changed=assets/tachyon.manifest");

    let root = Path::new("assets/icons");
    let mut entries: Vec<(String, Vec<u8>, u32)> = Vec::new();
    if let Ok(sets) = fs::read_dir(root) {
        for set in sets.flatten() {
            if !set.path().is_dir() {
                continue;
            }
            let set_name = set.file_name().to_string_lossy().into_owned();
            for f in fs::read_dir(set.path()).unwrap().flatten() {
                let p = f.path();
                if p.extension().and_then(|e| e.to_str()) != Some("svg") {
                    continue;
                }
                let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
                let raw = minify(&fs::read_to_string(&p).unwrap());
                let comp = miniz_oxide::deflate::compress_to_vec(raw.as_bytes(), 9);
                entries.push((format!("{set_name}/{stem}"), comp, raw.len() as u32));
            }
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    // Layout: "TPK1" | count u32 | index entries | blob
    // entry: name_len u16 | name | offset u32 | comp_len u32 | raw_len u32
    let mut index = Vec::new();
    let mut blob = Vec::new();
    for (name, comp, raw_len) in &entries {
        index.extend_from_slice(&(name.len() as u16).to_le_bytes());
        index.extend_from_slice(name.as_bytes());
        index.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        index.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        index.extend_from_slice(&raw_len.to_le_bytes());
        blob.extend_from_slice(comp);
    }
    let mut out = b"TPK1".to_vec();
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    out.extend_from_slice(&(index.len() as u32).to_le_bytes());
    out.extend_from_slice(&index);
    out.extend_from_slice(&blob);
    let out_dir = env::var("OUT_DIR").unwrap();
    fs::write(Path::new(&out_dir).join("icons.pak"), out).unwrap();

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Generated .rc: icon + manifest + VERSIONINFO (SignPath requires
        // product name/version metadata on signed binaries).
        let assets = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets");
        let esc = |p: &Path| p.display().to_string().replace('\\', "\\\\");
        let mut rc = String::new();
        if assets.join("tachyon.ico").exists() {
            rc += &format!("1 ICON \"{}\"\n", esc(&assets.join("tachyon.ico")));
        }
        rc += &format!("1 24 \"{}\"\n", esc(&assets.join("tachyon.manifest")));
        let v = |k| env::var(k).unwrap_or_else(|_| "0".into());
        let (maj, min, pat, ver) = (
            v("CARGO_PKG_VERSION_MAJOR"),
            v("CARGO_PKG_VERSION_MINOR"),
            v("CARGO_PKG_VERSION_PATCH"),
            v("CARGO_PKG_VERSION"),
        );
        rc += &format!(
            r#"1 VERSIONINFO
FILEVERSION {maj},{min},{pat},0
PRODUCTVERSION {maj},{min},{pat},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Tachyon"
      VALUE "FileDescription", "Tachyon"
      VALUE "FileVersion", "{ver}"
      VALUE "InternalName", "tachyon"
      VALUE "LegalCopyright", "Copyright (c) 2026 Jonah Hawk. MIT License."
      VALUE "OriginalFilename", "tachyon.exe"
      VALUE "ProductName", "Tachyon"
      VALUE "ProductVersion", "{ver}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
        );
        let rc_path = Path::new(&out_dir).join("tachyon.rc");
        fs::write(&rc_path, rc).unwrap();
        embed_resource::compile(&rc_path, embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
