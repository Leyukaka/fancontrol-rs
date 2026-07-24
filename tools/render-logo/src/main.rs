//! One-shot: render assets/logo.svg → PNG + multi-size ICO for the app icon.
use std::fs;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root");
    let svg_path = root.join("assets/logo.svg");
    let svg = fs::read(&svg_path).expect("read logo.svg");
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(&svg, &opt).expect("parse svg");
    let size = tree.size().to_int_size();
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height()).expect("pixmap");
    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());

    // Also produce a padded square 256 for Windows
    let png_256 = render_scaled(&tree, 256);
    let png_path = root.join("assets/icon.png");
    fs::write(&png_path, &png_256).expect("write icon.png");
    println!("wrote {}", png_path.display());

    let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);
    for dim in [16u32, 32, 48, 64, 128, 256] {
        let png = render_scaled(&tree, dim);
        let image = ico::IconImage::read_png(std::io::Cursor::new(&png)).expect("png decode");
        icon_dir.add_entry(ico::IconDirEntry::encode(&image).expect("encode ico entry"));
    }
    let ico_path = root.join("assets/icon.ico");
    let mut out = fs::File::create(&ico_path).expect("create ico");
    icon_dir.write(&mut out).expect("write ico");
    println!("wrote {}", ico_path.display());
}

fn render_scaled(tree: &usvg::Tree, dim: u32) -> Vec<u8> {
    let mut pixmap = tiny_skia::Pixmap::new(dim, dim).expect("pixmap");
    // Fit SVG into square
    let sz = tree.size();
    let scale = (dim as f32 / sz.width()).min(dim as f32 / sz.height());
    let tx = (dim as f32 - sz.width() * scale) / 2.0;
    let ty = (dim as f32 - sz.height() * scale) / 2.0;
    let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, tx, ty);
    // Dark transparent background is fine for icon
    resvg::render(tree, transform, &mut pixmap.as_mut());
    pixmap.encode_png().expect("encode png")
}
