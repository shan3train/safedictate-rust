fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_icon();
    }
}

fn embed_icon() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let ico_path = std::path::PathBuf::from(&out_dir).join("safedictate.ico");

    let (rgba32, _, _) = feather_rgba(32);

    // Downscale 32→16 by averaging 2×2 blocks
    let mut rgba16 = vec![0u8; 16 * 16 * 4];
    for y in 0..16usize {
        for x in 0..16usize {
            let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
            for dy in 0..2usize {
                for dx in 0..2usize {
                    let i = ((y * 2 + dy) * 32 + (x * 2 + dx)) * 4;
                    r += rgba32[i] as u32;
                    g += rgba32[i + 1] as u32;
                    b += rgba32[i + 2] as u32;
                    a += rgba32[i + 3] as u32;
                }
            }
            let i = (y * 16 + x) * 4;
            rgba16[i] = (r / 4) as u8;
            rgba16[i + 1] = (g / 4) as u8;
            rgba16[i + 2] = (b / 4) as u8;
            rgba16[i + 3] = (a / 4) as u8;
        }
    }

    // Write ICO (two sizes)
    use image::codecs::ico::{IcoEncoder, IcoFrame};
    let mut buf = Vec::new();
    let encoder = IcoEncoder::new(&mut buf);
    encoder
        .encode_images(&[
            IcoFrame::as_png(&rgba16, 16, 16, image::ExtendedColorType::Rgba8).unwrap(),
            IcoFrame::as_png(&rgba32, 32, 32, image::ExtendedColorType::Rgba8).unwrap(),
        ])
        .unwrap();
    std::fs::write(&ico_path, &buf).unwrap();

    // Embed into the .exe as a Windows resource
    let mut res = winres::WindowsResource::new();
    res.set_icon(ico_path.to_str().unwrap());
    res.compile().unwrap();
}

/// Same feather design as src/icon.rs, parameterised by SIZE.
fn feather_rgba(size: u32) -> (Vec<u8>, u32, u32) {
    let mut px = vec![0u8; (size * size * 4) as usize];

    let plot = |px: &mut Vec<u8>, x: i32, y: i32, r: u8, g: u8, b: u8, a: u8| {
        if x < 0 || y < 0 || x >= size as i32 || y >= size as i32 {
            return;
        }
        let i = ((y as u32 * size + x as u32) * 4) as usize;
        px[i] = r;
        px[i + 1] = g;
        px[i + 2] = b;
        px[i + 3] = a;
    };

    let s = size as f32;

    // Dark background circle
    let bg_cx = s / 2.0;
    let bg_cy = s / 2.0;
    let r_bg = s / 2.0 - 0.5;
    for y in 0..size as i32 {
        for x in 0..size as i32 {
            let dx = x as f32 + 0.5 - bg_cx;
            let dy = y as f32 + 0.5 - bg_cy;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= r_bg {
                let aa = ((r_bg - dist) * 4.0).clamp(0.0, 1.0);
                plot(&mut px, x, y, 30, 30, 46, (aa * 255.0) as u8);
            }
        }
    }

    // Feather vane: filled rotated ellipse
    let cx = s * 0.516;
    let cy = s * 0.484;
    let angle: f32 = std::f32::consts::PI * 0.25;
    let (cos_a, sin_a) = (angle.cos(), angle.sin());
    let major = s * 0.281;
    let minor = s * 0.141;

    for y in 0..size as i32 {
        for x in 0..size as i32 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let u = dx * cos_a + dy * sin_a;
            let v = -dx * sin_a + dy * cos_a;
            let e = (u / major) * (u / major) + (v / minor) * (v / minor);
            if e < 1.0 {
                let edge_aa = ((1.0 - e) * 5.0).clamp(0.0, 1.0);
                let barb = (u * 2.5).sin() * 0.08 + 1.0;
                let lum = barb * edge_aa;
                plot(
                    &mut px,
                    x,
                    y,
                    (205.0 * lum) as u8,
                    (214.0 * lum) as u8,
                    (244.0 * lum) as u8,
                    (255.0 * edge_aa) as u8,
                );
            }
        }
    }

    // Spine
    let steps = (major * 2.0) as i32 + 2;
    for i in 0..=steps {
        let t = (i as f32 / steps as f32) * 2.0 - 1.0;
        let u = t * major;
        let sx = (cx + u * cos_a) as i32;
        let sy = (cy + u * sin_a) as i32;
        plot(&mut px, sx, sy, 255, 255, 255, 220);
    }

    // Quill tail
    for i in 1..=5i32 {
        let u = major + i as f32 * 1.4;
        let sx = (cx + u * cos_a) as i32;
        let sy = (cy + u * sin_a) as i32;
        let a = (190 - i * 32).max(0) as u8;
        plot(&mut px, sx, sy, 205, 214, 244, a);
    }

    (px, size, size)
}
