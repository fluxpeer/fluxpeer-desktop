// SPDX-License-Identifier: AGPL-3.0-or-later
//! Rasterize an SVG into a macOS `.iconset` directory of PNGs (and arbitrary
//! sizes for other platforms). Reuses the desktop's resvg/tiny-skia deps so the
//! packaging step needs no extra tooling.
//!
//! Usage: cargo run -q -p fluxpeer-desktop --example icongen -- <in.svg> <out-dir> [sizes]
//! - no `sizes` → emit the standard macOS iconset PNG names (16…512 @1x/@2x)
//! - `sizes=44,150,310` → emit `icon_<n>.png` for each (e.g. Windows tiles)

use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("usage: icongen <in.svg> <out-dir> [sizes]");
    let outdir = args.next().expect("usage: icongen <in.svg> <out-dir> [sizes]");
    let sizes_arg = args.next();

    let svg = std::fs::read(&input).expect("read svg");
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default()).expect("parse svg");
    let vb = tree.size();
    let side = vb.width().max(vb.height());
    std::fs::create_dir_all(&outdir).expect("mkdir out");

    let jobs: Vec<(u32, String)> = match sizes_arg {
        None => vec![
            (16, "icon_16x16.png"),
            (32, "icon_16x16@2x.png"),
            (32, "icon_32x32.png"),
            (64, "icon_32x32@2x.png"),
            (128, "icon_128x128.png"),
            (256, "icon_128x128@2x.png"),
            (256, "icon_256x256.png"),
            (512, "icon_256x256@2x.png"),
            (512, "icon_512x512.png"),
            (1024, "icon_512x512@2x.png"),
        ]
        .into_iter()
        .map(|(n, name)| (n, name.to_string()))
        .collect(),
        Some(list) => list
            .split(',')
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .map(|n| (n, format!("icon_{n}.png")))
            .collect(),
    };

    for (size, name) in jobs {
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).expect("pixmap");
        let scale = size as f32 / side;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        let png = pixmap.encode_png().expect("encode png");
        let path = Path::new(&outdir).join(&name);
        std::fs::write(&path, png).expect("write png");
        println!("{}", path.display());
    }
}
