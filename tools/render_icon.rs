//! Render assets/icon/AppIcon.svg into an .iconset (all macOS sizes).
//! Run: cargo run --example render_icon -- <svg> <out.iconset>
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (svg, out) = (Path::new(&args[1]), Path::new(&args[2]));
    let data = std::fs::read(svg).expect("read svg");
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).expect("parse svg");
    std::fs::create_dir_all(out).unwrap();
    for (base, scale) in [(16, 1), (16, 2), (32, 1), (32, 2), (128, 1), (128, 2), (256, 1), (256, 2), (512, 1), (512, 2)] {
        let px = base * scale;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px).unwrap();
        let s = px as f32 / tree.size().width();
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(s, s), &mut pixmap.as_mut());
        let name = if scale == 1 { format!("icon_{base}x{base}.png") } else { format!("icon_{base}x{base}@2x.png") };
        pixmap.save_png(out.join(name)).unwrap();
    }
}
