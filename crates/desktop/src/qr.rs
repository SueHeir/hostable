//! The join QR code as a grid of dark and light modules.

use qrcode::{Color, EcLevel, QrCode};

/// Light modules around the code, as the QR spec asks.
pub const QUIET_ZONE: usize = 4;

/// A square grid of modules, `true` = dark, including the quiet zone.
#[derive(Debug, Clone, PartialEq)]
pub struct Modules {
    pub size: usize,
    pub dark: Vec<bool>,
}

impl Modules {
    pub fn is_dark(&self, x: usize, y: usize) -> bool {
        self.dark[y * self.size + x]
    }

    /// RGBA8 pixels, `scale` pixels per module (black on white).
    pub fn rgba(&self, scale: usize) -> (usize, Vec<u8>) {
        let side = self.size * scale;
        let mut px = Vec::with_capacity(side * side * 4);
        for y in 0..side {
            for x in 0..side {
                let v = if self.is_dark(x / scale, y / scale) {
                    0
                } else {
                    255
                };
                px.extend_from_slice(&[v, v, v, 255]);
            }
        }
        (side, px)
    }
}

/// Encode `text` (the join link).
pub fn encode(text: &str) -> Result<Modules, String> {
    let code = QrCode::with_error_correction_level(text, EcLevel::M).map_err(|e| e.to_string())?;
    let w = code.width();
    let size = w + 2 * QUIET_ZONE;
    let mut dark = vec![false; size * size];
    for (i, c) in code.to_colors().into_iter().enumerate() {
        let (x, y) = (i % w + QUIET_ZONE, i / w + QUIET_ZONE);
        dark[y * size + x] = c == Color::Dark;
    }
    Ok(Modules { size, dark })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    #[test]
    fn the_join_link_encodes_with_finder_patterns() {
        let link = config::join_link(&config::default_join_url(), "BCDF");
        assert_eq!(link, "https://dinkydelvers.com/play/j/BCDF");
        let m = encode(&link).unwrap();
        assert!(m.size >= 21 + 2 * QUIET_ZONE);
        // Quiet zone is light; the top-left finder pattern starts dark.
        assert!(!m.is_dark(0, 0));
        assert!(m.is_dark(QUIET_ZONE, QUIET_ZONE));
        assert!(m.is_dark(QUIET_ZONE + 6, QUIET_ZONE));
        assert!(!m.is_dark(QUIET_ZONE + 1, QUIET_ZONE + 1));
        let (side, px) = m.rgba(3);
        assert_eq!(side, m.size * 3);
        assert_eq!(px.len(), side * side * 4);
    }

    #[test]
    fn a_different_code_gives_a_different_image() {
        let a = encode(&config::join_link("https://x.test", "AAAA")).unwrap();
        let b = encode(&config::join_link("https://x.test", "BBBB")).unwrap();
        assert_ne!(a, b);
    }
}
