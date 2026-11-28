//! CSS colours as games send them (`#rgb`, `#rrggbb`, `#rrggbbaa`, a few
//! names), and the fallbacks used when a game sends none.

/// An sRGB colour with alpha, each 0..=1.
pub type Rgba = [f32; 4];

/// Parse a CSS colour. Unknown text gives `None`.
pub fn parse(css: &str) -> Option<Rgba> {
    let s = css.trim().to_ascii_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        let digit = |i: usize| u8::from_str_radix(hex.get(i..i + 1)?, 16).ok();
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        let f = |b: u8| b as f32 / 255.0;
        return match hex.len() {
            3 | 4 => {
                let c = |i| digit(i).map(|d| f(d * 17));
                let a = if hex.len() == 4 { c(3)? } else { 1.0 };
                Some([c(0)?, c(1)?, c(2)?, a])
            }
            6 | 8 => {
                let a = if hex.len() == 8 { f(byte(6)?) } else { 1.0 };
                Some([f(byte(0)?), f(byte(2)?), f(byte(4)?), a])
            }
            _ => None,
        };
    }
    if let Some(inner) = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))
        .and_then(|r| r.strip_suffix(')'))
    {
        let parts: Vec<f32> = inner
            .split(',')
            .map(|p| p.trim().parse::<f32>())
            .collect::<Result<_, _>>()
            .ok()?;
        return match parts.as_slice() {
            [r, g, b] => Some([r / 255.0, g / 255.0, b / 255.0, 1.0]),
            [r, g, b, a] => Some([r / 255.0, g / 255.0, b / 255.0, *a]),
            _ => None,
        };
    }
    let named = match s.as_str() {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [220, 50, 47],
        "green" => [60, 160, 70],
        "blue" => [50, 100, 220],
        "yellow" => [240, 200, 40],
        "orange" => [240, 140, 30],
        "purple" => [140, 70, 180],
        "gray" | "grey" => [128, 128, 128],
        "brown" => [140, 90, 50],
        "pink" => [240, 130, 180],
        "cyan" => [40, 200, 220],
        "transparent" => return Some([0.0, 0.0, 0.0, 0.0]),
        _ => return None,
    };
    Some([
        named[0] as f32 / 255.0,
        named[1] as f32 / 255.0,
        named[2] as f32 / 255.0,
        1.0,
    ])
}

/// Seat colours for players who didn't pick one.
pub const SEAT_COLORS: [Rgba; 8] = [
    [0.86, 0.30, 0.27, 1.0],
    [0.25, 0.50, 0.86, 1.0],
    [0.95, 0.75, 0.20, 1.0],
    [0.35, 0.70, 0.40, 1.0],
    [0.62, 0.38, 0.80, 1.0],
    [0.95, 0.55, 0.20, 1.0],
    [0.30, 0.75, 0.78, 1.0],
    [0.90, 0.45, 0.65, 1.0],
];

/// A player's colour, or their seat's default.
pub fn player(css: &str, seat: usize) -> Rgba {
    parse(css).unwrap_or(SEAT_COLORS[seat % SEAT_COLORS.len()])
}

/// The same colour, darker (`amount` 0..1).
pub fn darker(c: Rgba, amount: f32) -> Rgba {
    let k = 1.0 - amount.clamp(0.0, 1.0);
    [c[0] * k, c[1] * k, c[2] * k, c[3]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_colours_parse() {
        assert_eq!(parse("#fff"), Some([1.0, 1.0, 1.0, 1.0]));
        assert_eq!(parse("#FF0000"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(
            parse("#00000080").map(|c| (c[3] * 255.0).round()),
            Some(128.0)
        );
        assert_eq!(parse("rgb(255, 0, 255)"), Some([1.0, 0.0, 1.0, 1.0]));
        assert_eq!(parse("rgba(0,0,0,0.5)"), Some([0.0, 0.0, 0.0, 0.5]));
        assert!(parse("white").is_some());
        assert_eq!(parse(""), None);
        assert_eq!(parse("#12"), None);
        assert_eq!(parse("#zzz"), None);
        assert_eq!(parse("url(x)"), None);
    }

    #[test]
    fn missing_colours_fall_back_per_seat() {
        assert_eq!(player("", 1), SEAT_COLORS[1]);
        assert_eq!(player("", 9), SEAT_COLORS[1]);
        assert_eq!(player("#000", 1), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(darker([1.0, 0.5, 0.0, 1.0], 0.5), [0.5, 0.25, 0.0, 1.0]);
    }
}
