// The droplet, drawn in code from the same shape the island draws on canvas
// (see draw() in src/main.ts): a circle for the bottom half, two cubic curves
// meeting at the tip, a highlight and two eyes. Pure Rust, no dependencies, so
// scripts/gen_icon.rs can include it too.

const DRIP: [f32; 3] = [0x4C as f32, 0xC2 as f32, 0xDB as f32];
const EYE: [f32; 3] = [0x0A as f32, 0x11 as f32, 0x16 as f32];
const EYE_WHITE: [f32; 3] = [0xF5 as f32, 0xFA as f32, 0xFC as f32];

/// Outline of the drop centred on the circle's centre, radius 1.
fn outline() -> Vec<(f32, f32)> {
    let bez = |p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32), t: f32| {
        let u = 1.0 - t;
        let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        (
            w[0] * p0.0 + w[1] * p1.0 + w[2] * p2.0 + w[3] * p3.0,
            w[0] * p0.1 + w[1] * p1.1 + w[2] * p2.1 + w[3] * p3.1,
        )
    };
    let mut pts = Vec::new();
    for i in 0..=24 {
        pts.push(bez((0.0, -1.8), (0.35, -1.2), (1.0, -0.6), (1.0, 0.0), i as f32 / 24.0));
    }
    for i in 1..48 {
        let a = std::f32::consts::PI * i as f32 / 48.0;
        pts.push((a.cos(), a.sin()));
    }
    for i in 0..=24 {
        pts.push(bez((-1.0, 0.0), (-1.0, -0.6), (-0.35, -1.2), (0.0, -1.8), i as f32 / 24.0));
    }
    pts
}

fn inside(poly: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut hit = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            hit = !hit;
        }
        j = i;
    }
    hit
}

fn in_ellipse(x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> bool {
    let (dx, dy) = ((x - cx) / rx, (y - cy) / ry);
    dx * dx + dy * dy <= 1.0
}

/// Colour at a point in drop units, or None outside the drop.
fn shade(poly: &[(f32, f32)], x: f32, y: f32) -> Option<[f32; 3]> {
    if !inside(poly, x, y) {
        return None;
    }
    // Eyes a little larger than on the island so they survive 16 px.
    for side in [-1.0f32, 1.0] {
        let ex = side * 0.4;
        if in_ellipse(x, y, ex + 0.03, 0.1, 0.13, 0.15) {
            return Some(EYE);
        }
        if in_ellipse(x, y, ex, 0.08, 0.25, 0.32) {
            return Some(EYE_WHITE);
        }
    }
    // Highlight, then a gentle top-left to bottom-right fall-off.
    let (hx, hy) = (x + 0.48, y + 0.7);
    let (c, s) = (0.5f32.cos(), 0.5f32.sin());
    let (rx, ry) = (hx * c - hy * s, hx * s + hy * c);
    if in_ellipse(rx, ry, 0.0, 0.0, 0.16, 0.32) {
        return Some([235.0, 248.0, 252.0]);
    }
    let d = (((x + 0.3).powi(2) + (y + 0.4).powi(2)).sqrt() / 1.6).min(1.0);
    let k = 1.12 - 0.3 * d;
    Some(DRIP.map(|v| (v * k).min(255.0)))
}

/// An RGBA square image of the drop, supersampled 4×4.
pub fn droplet_rgba(size: u32) -> Vec<u8> {
    let poly = outline();
    // The drop spans y ∈ [-1.8, 1] and x ∈ [-1, 1]; fit 2.9 units in the square.
    let scale = size as f32 / 2.9;
    let (ox, oy) = (size as f32 / 2.0, size as f32 * (1.85 / 2.9));
    let mut out = vec![0u8; (size * size * 4) as usize];
    const N: u32 = 4;
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0f32; 3];
            let mut hits = 0u32;
            for sy in 0..N {
                for sx in 0..N {
                    let x = (px as f32 + (sx as f32 + 0.5) / N as f32 - ox) / scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / N as f32 - oy) / scale;
                    if let Some(c) = shade(&poly, x, y) {
                        for k in 0..3 {
                            acc[k] += c[k];
                        }
                        hits += 1;
                    }
                }
            }
            if hits > 0 {
                let i = ((py * size + px) * 4) as usize;
                for k in 0..3 {
                    out[i + k] = (acc[k] / hits as f32).round() as u8;
                }
                out[i + 3] = (255 * hits / (N * N)) as u8;
            }
        }
    }
    out
}

