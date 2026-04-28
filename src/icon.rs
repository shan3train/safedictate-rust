/// Render a 32×32 RGBA feather icon. Returns (rgba_bytes, width, height).
///
/// Designed to stay legible at 16×16 (system tray). Uses a filled
/// rotated-ellipse vane + bright spine + quill tail, all on a dark circle.
pub fn feather_rgba() -> (Vec<u8>, u32, u32) {
    const SIZE: u32 = 32;
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];

    let plot = |px: &mut Vec<u8>, x: i32, y: i32, r: u8, g: u8, b: u8, a: u8| {
        if x < 0 || y < 0 || x >= SIZE as i32 || y >= SIZE as i32 { return; }
        let i = ((y as u32 * SIZE + x as u32) * 4) as usize;
        px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = a;
    };

    // --- Background: dark Catppuccin Mocha circle ---
    let bg_cx = SIZE as f32 / 2.0;
    let bg_cy = SIZE as f32 / 2.0;
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let dx = x as f32 + 0.5 - bg_cx;
            let dy = y as f32 + 0.5 - bg_cy;
            let dist = (dx*dx + dy*dy).sqrt();
            if dist <= 15.5 {
                let aa = ((15.5 - dist) * 4.0).clamp(0.0, 1.0);
                plot(&mut px, x, y, 30, 30, 46, (aa * 255.0) as u8);
            }
        }
    }

    // --- Feather vane: filled rotated ellipse ---
    // Center slightly upper-right, angled 45° (feather tip at NE, quill at SW)
    let cx = 16.5f32;
    let cy = 15.5f32;
    let angle = std::f32::consts::PI * 0.25;
    let (cos_a, sin_a) = (angle.cos(), angle.sin());
    let major = 9.0f32;  // half-length along spine
    let minor = 4.5f32;  // half-width

    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let u =  dx * cos_a + dy * sin_a; // along spine
            let v = -dx * sin_a + dy * cos_a; // perpendicular
            let e = (u/major)*(u/major) + (v/minor)*(v/minor);
            if e < 1.0 {
                // Soft edge AA + feather texture (faint barb pattern)
                let edge_aa = ((1.0 - e) * 5.0).clamp(0.0, 1.0);
                // Subtle barb shading: bands perpendicular to spine
                let barb = (u * 2.5).sin() * 0.08 + 1.0;
                let lum = (barb * edge_aa).clamp(0.0, 1.0);
                let r = (205.0 * lum) as u8;
                let g = (214.0 * lum) as u8;
                let b = (244.0 * lum) as u8;
                let a = (255.0 * edge_aa) as u8;
                plot(&mut px, x, y, r, g, b, a);
            }
        }
    }

    // --- Spine: bright white line through feather center ---
    let steps = 20;
    for i in 0..=steps {
        let t = (i as f32 / steps as f32) * 2.0 - 1.0; // -1..1
        let u = t * major;
        let sx = (cx + u * cos_a) as i32;
        let sy = (cy + u * sin_a) as i32;
        plot(&mut px, sx, sy, 255, 255, 255, 230);
    }

    // --- Quill tail: fading line below the vane ---
    for i in 1..=6i32 {
        let u = major + i as f32 * 1.4;
        let sx = (cx + u * cos_a) as i32;
        let sy = (cy + u * sin_a) as i32;
        let a = (200 - i * 28).max(0) as u8;
        plot(&mut px, sx, sy, 205, 214, 244, a);
    }

    (px, SIZE, SIZE)
}
