# Third-party notices

Tachyon itself is MIT licensed (see [LICENSE](LICENSE)). `tachyon.exe` also contains the open-source components below, all under permissive licenses. It contains no proprietary components.

## Icons

| Component | License | Full text |
|---|---|---|
| [Lucide](https://github.com/lucide-icons/lucide) | ISC | [LICENSES/lucide-ISC.txt](LICENSES/lucide-ISC.txt) |
| [Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons) (Microsoft) | MIT | [LICENSES/fluentui-system-icons-MIT.txt](LICENSES/fluentui-system-icons-MIT.txt) |

The SVGs are in `assets/icons/`. `build.rs` packs them into the executable unmodified.

## Fonts

The Settings window uses egui's default fonts, embedded through the `epaint_default_fonts` crate:

| Font | License |
|---|---|
| Ubuntu Light | Ubuntu Font Licence 1.0 |
| Hack | MIT / Bitstream Vera |
| Noto Emoji, Emoji Icon Font | SIL Open Font License 1.1 |

## Rust crates

The direct dependencies are:

| Crate | License |
|---|---|
| [windows](https://github.com/microsoft/windows-rs) (Microsoft) | MIT OR Apache-2.0 |
| [eframe / egui](https://github.com/emilk/egui) | MIT OR Apache-2.0 |
| [resvg / usvg / tiny-skia](https://github.com/linebender/resvg) | Apache-2.0 OR MIT |
| [serde / serde_json](https://github.com/serde-rs) | MIT OR Apache-2.0 |
| [miniz_oxide](https://github.com/Frommi/miniz_oxide) | MIT OR Zlib OR Apache-2.0 |
| [embed-resource](https://github.com/nabijaczleweli/rust-embed-resource) (build only) | MIT |

The full dependency tree, about 160 crates for Windows, uses only permissive licenses: MIT, Apache-2.0, BSD-2/3-Clause, Zlib, ISC, BSL-1.0, Unicode-3.0, 0BSD and Unlicense. Where a crate offers a choice, Tachyon uses it under MIT or Apache-2.0. `cargo metadata --format-version 1` lists every crate with its license.
