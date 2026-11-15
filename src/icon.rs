//! Dessin de l'icône Echoes (ondes concentriques), sans fichier image : barre des menus et icône d'app.

pub const GREEN: [u8; 4] = [52, 199, 89, 255];
pub const ORANGE: [u8; 4] = [255, 149, 0, 255];
pub const GREY: [u8; 4] = [142, 142, 147, 255];

/// Gris neutre lisible sur une barre des menus claire comme sombre.
const GLYPH_MENU: [u8; 3] = [142, 142, 147];
/// Registre maison : fond encre, glyphe cuivre.
const APP_BG: [u8; 3] = [31, 36, 48];
const APP_GLYPH: [u8; 3] = [200, 121, 74];

/// Centre de la pastille d'état, en pixels.
pub fn dot_center(size: u32) -> (u32, u32) {
    let c = (size as f32 * 0.78) as u32;
    (c, c)
}

/// Ondes : un point au centre et trois cercles concentriques de plus en plus fins, comme un écho
/// qui s'éteint. Coordonnées normalisées 0..1.
fn in_glyph(x: f32, y: f32) -> bool {
    let r = ((x - 0.5).powi(2) + (y - 0.5).powi(2)).sqrt();
    r <= 0.10 || [(0.23_f32, 0.040_f32), (0.35, 0.032), (0.46, 0.024)].iter().any(|&(ring, half)| (r - ring).abs() <= half)
}

/// Carré arrondi de l'icône d'app (marge 8 %, rayon 22 %).
fn in_rounded_square(x: f32, y: f32) -> bool {
    let (m, rad) = (0.08_f32, 0.22_f32);
    let (lo, hi) = (m + rad, 1.0 - m - rad);
    let cx = x.clamp(lo, hi);
    let cy = y.clamp(lo, hi);
    let (dx, dy) = (x - cx, y - cy);
    x >= m && x <= 1.0 - m && y >= m && y <= 1.0 - m && dx * dx + dy * dy <= rad * rad
}

/// Image RGBA `size × size`. `dot` : pastille d'état (barre des menus seulement).
/// `app_style` : carré arrondi plein, pour l'icône d'app.
pub fn render(size: u32, dot: Option<[u8; 4]>, app_style: bool) -> Vec<u8> {
    const SS: u32 = 4; // sur-échantillonnage 4 × 4 pour lisser les bords
    let s = size as f32;
    let (dcx, dcy) = dot_center(size);
    let (dcx, dcy) = ((dcx as f32 + 0.5) / s, (dcy as f32 + 0.5) / s);
    let dot_r = 0.20_f32;
    let mut img = vec![0u8; (size * size * 4) as usize];
    for py in 0..size {
        for px in 0..size {
            let (mut glyph, mut bg, mut pastille) = (0u32, 0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / s;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / s;
                    if app_style {
                        if in_rounded_square(x, y) {
                            bg += 1;
                            // Glyphe réduit et centré dans le carré.
                            if in_glyph((x - 0.5) / 0.70 + 0.5, (y - 0.5) / 0.70 + 0.5) {
                                glyph += 1;
                            }
                        }
                        continue;
                    }
                    let d = ((x - dcx).powi(2) + (y - dcy).powi(2)).sqrt();
                    if dot.is_some() && d <= dot_r {
                        pastille += 1;
                    } else if (dot.is_none() || d > dot_r + 0.07) && in_glyph(x, y) {
                        // Un liseré transparent de 7 % détache la pastille des ondes.
                        glyph += 1;
                    }
                }
            }
            let n = (SS * SS) as f32;
            let i = ((py * size + px) * 4) as usize;
            let (rgb, alpha) = if app_style {
                let g = glyph as f32 / bg.max(1) as f32;
                let mix = |a: u8, b: u8| (a as f32 * (1.0 - g) + b as f32 * g).round() as u8;
                ([mix(APP_BG[0], APP_GLYPH[0]), mix(APP_BG[1], APP_GLYPH[1]), mix(APP_BG[2], APP_GLYPH[2])], bg as f32 / n)
            } else if pastille > 0 {
                let c = dot.unwrap_or(GREY);
                ([c[0], c[1], c[2]], pastille as f32 / n)
            } else {
                (GLYPH_MENU, glyph as f32 / n)
            };
            img[i..i + 3].copy_from_slice(&rgb);
            img[i + 3] = (alpha * 255.0).round() as u8;
        }
    }
    img
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Encode une image RGBA en PNG (zlib non compressé : blocs « stored ») — suffisant pour générer l'iconset.
pub fn png(size: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(rgba.len() + size as usize);
    for row in rgba.chunks(size as usize * 4) {
        raw.push(0); // filtre « aucun »
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8; 4], body: &[u8]| {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(body);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bits, RGBA
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

/// Jeu d'images `Echoes.iconset` attendu par `iconutil`.
pub fn write_iconset(dir: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    for base in [16u32, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let size = base * scale;
            let file = dir.join(format!("icon_{base}x{base}{suffix}.png"));
            std::fs::write(file, png(size, &render(size, None, true)))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(img: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    }

    #[test]
    fn render_has_expected_len() {
        assert_eq!(render(36, None, false).len(), 36 * 36 * 4);
    }

    #[test]
    fn menu_icon_corners_transparent() {
        let img = render(36, Some(GREEN), false);
        assert_eq!(px(&img, 36, 0, 0)[3], 0);
    }

    #[test]
    fn dot_is_coloured() {
        let size = 36;
        let img = render(size, Some(GREEN), false);
        let (cx, cy) = dot_center(size);
        let p = px(&img, size, cx, cy);
        for c in 0..4 {
            assert!((p[c] as i32 - GREEN[c] as i32).abs() <= 8, "pixel {p:?} au lieu de {GREEN:?}");
        }
    }

    #[test]
    fn png_has_signature_and_iend() {
        let data = png(2, &[255; 16]);
        assert_eq!(&data[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&data[data.len() - 8..data.len() - 4], b"IEND");
        // CRC de IEND, valeur connue.
        assert_eq!(&data[data.len() - 4..], &[0xAE, 0x42, 0x60, 0x82]);
    }

    #[test]
    fn app_icon_is_opaque_inside() {
        let img = render(256, None, true);
        assert_eq!(px(&img, 256, 128, 128)[3], 255);
    }
}
