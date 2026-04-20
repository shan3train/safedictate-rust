//! Generates the SafeDictate feather icon as raw RGBA bytes for the system tray.

/// Render a 32×32 RGBA feather icon. Returns (rgba_bytes, width, height).
pub fn feather_rgba() -> (Vec<u8>, u32, u32) {
    const SIZE: u32 = 32;
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];

    let set = |px: &mut Vec<u8>, x: i32, y: i32, r: u8, g: u8, b: u8, a: u8| {
        if x < 0 || y < 0 || x >= SIZE as i32 || y >= SIZE as i32 { return; }
        let i = ((y as u32 * SIZE + x as u32) * 4) as usize;
        px[i]     = r;
        px[i + 1] = g;
        px[i + 2] = b;
        px[i + 3] = a;
    };

    // Background circle (dark)
    let cx = SIZE as f32 / 2.0;
    let cy = SIZE as f32 / 2.0;
    let r_outer = 15.0f32;
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= r_outer {
                let aa = ((r_outer - dist) * 3.0).clamp(0.0, 1.0);
                set(&mut px, x, y, 30, 30, 46, (aa * 220.0) as u8);
            }
        }
    }

    // Feather quill — diagonal spine from bottom-left to top-right
    let spine: &[(i32, i32)] = &[
        (8,24),(9,23),(10,22),(11,21),(12,20),(13,19),(14,18),
        (15,17),(16,16),(17,15),(18,14),(19,13),(20,12),(21,11),(22,10),
    ];
    for &(x, y) in spine {
        for dx in -1..=1i32 {
            set(&mut px, x + dx, y, 205, 214, 244, 255);
        }
    }

    // Barbs — short strokes fanning left and right of the spine
    let barbs: &[(i32, i32, i32, i32)] = &[
        // (spine_x, spine_y, barb_dx, barb_dy)
        (10,22, -2,1),  (10,22, 1,-2),
        (12,20, -2,1),  (12,20, 1,-2),
        (14,18, -2,1),  (14,18, 1,-2),
        (16,16, -2,1),  (16,16, 1,-2),
        (18,14, -2,1),  (18,14, 1,-2),
        (20,12, -2,1),  (20,12, 1,-2),
        (22,10, -2,1),  (22,10, 1,-2),
    ];
    for &(sx, sy, bdx, bdy) in barbs {
        for step in 1..=4i32 {
            let alpha = (255.0 * (1.0 - step as f32 / 5.0)) as u8;
            set(&mut px, sx + bdx * step, sy + bdy * step, 180, 190, 254, alpha);
        }
    }

    // Quill tip — small dot at top-right
    set(&mut px, 23, 9, 205, 214, 244, 255);
    set(&mut px, 24, 8, 205, 214, 244, 200);

    (px, SIZE, SIZE)
}
