//! Pure geometry: fitting a game's scene into the window, hexagons, and the
//! duel board's axial hex grid. No Bevy state, so it is easy to test.

/// A rectangle in Bevy 2D world units (origin at the window centre, y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub center_x: f32,
    pub center_y: f32,
    pub width: f32,
    pub height: f32,
}

impl Area {
    /// The part of a `window_w` by `window_h` window left after taking
    /// `left`, `right`, `top` and `bottom` pixels off its edges.
    pub fn inside_window(
        window_w: f32,
        window_h: f32,
        left: f32,
        right: f32,
        top: f32,
        bottom: f32,
    ) -> Area {
        let width = (window_w - left - right).max(1.0);
        let height = (window_h - top - bottom).max(1.0);
        Area {
            center_x: -window_w / 2.0 + left + width / 2.0,
            center_y: window_h / 2.0 - top - height / 2.0,
            width,
            height,
        }
    }
}

/// How a scene (`width` by `height` units, y down) maps into an [`Area`]:
/// uniformly scaled to fit, centred, never stretched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub scale: f32,
    scene_w: f32,
    scene_h: f32,
    area: Area,
}

impl Fit {
    /// Fit the scene inside `area`, leaving `margin` (a fraction, e.g.
    /// 0.05) free on each side.
    pub fn new(scene_w: f32, scene_h: f32, area: Area, margin: f32) -> Fit {
        let scene_w = if scene_w > 0.0 { scene_w } else { 1.0 };
        let scene_h = if scene_h > 0.0 { scene_h } else { 1.0 };
        let usable = (1.0 - 2.0 * margin.clamp(0.0, 0.45)).max(0.1);
        let scale = (area.width * usable / scene_w).min(area.height * usable / scene_h);
        Fit {
            scale,
            scene_w,
            scene_h,
            area,
        }
    }

    /// A scene point in world units.
    pub fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.area.center_x + (x - self.scene_w / 2.0) * self.scale,
            self.area.center_y - (y - self.scene_h / 2.0) * self.scale,
        )
    }

    /// A scene length in world units.
    pub fn length(&self, l: f32) -> f32 {
        l * self.scale
    }
}

/// Corners of a pointy-top hexagon centred on `(x, y)` with circumradius
/// `r`, clockwise from the top, in y-down coordinates.
pub fn hex_corners(x: f32, y: f32, r: f32) -> [(f32, f32); 6] {
    let mut out = [(0.0, 0.0); 6];
    for (i, c) in out.iter_mut().enumerate() {
        // Top first: -90 degrees in y-down space, then clockwise.
        let a = (-90.0 + 60.0 * i as f32).to_radians();
        *c = (x + r * a.cos(), y + r * a.sin());
    }
    out
}

/// Centre of axial hex `(q, r)` on the duel board (pointy-top, circumradius
/// 1), x right, y up.
pub fn hex_center(q: i32, r: i32) -> (f32, f32) {
    let s3 = 3f32.sqrt();
    (s3 * (q as f32 + r as f32 / 2.0), -1.5 * r as f32)
}

/// A board vertex: the top (`north`) or bottom corner of hex `(q, r)`, x
/// right, y up.
pub fn vertex_position(q: i32, r: i32, north: bool) -> (f32, f32) {
    let (x, y) = hex_center(q, r);
    if north {
        (x, y + 1.0)
    } else {
        (x, y - 1.0)
    }
}

/// Board coordinates (x right, y up) to Bevy 3D ground coordinates
/// (`x`, `z`): y up on the board becomes -z, away from a camera on +z.
pub fn board_to_ground(x: f32, y: f32) -> (f32, f32) {
    (x, -y)
}

/// The angle (radians, counter-clockwise from +x in y-up space) and length
/// of a line segment, for drawing it as a rotated thin rectangle.
pub fn segment(x1: f32, y1: f32, x2: f32, y2: f32) -> (f32, f32) {
    let (dx, dy) = (x2 - x1, y2 - y1);
    (dy.atan2(dx), (dx * dx + dy * dy).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn a_wide_scene_fills_the_width_and_is_centred() {
        let area = Area::inside_window(1000.0, 500.0, 0.0, 0.0, 0.0, 0.0);
        let fit = Fit::new(200.0, 50.0, area, 0.0);
        assert!(close(fit.scale, 5.0));
        // Top left of the scene sits on the left edge, a quarter down.
        let (x, y) = fit.point(0.0, 0.0);
        assert!(close(x, -500.0) && close(y, 125.0), "{x} {y}");
        let (x, y) = fit.point(100.0, 25.0);
        assert!(close(x, 0.0) && close(y, 0.0));
    }

    #[test]
    fn a_tall_scene_fills_the_height() {
        let area = Area::inside_window(800.0, 600.0, 0.0, 0.0, 0.0, 0.0);
        let fit = Fit::new(10.0, 100.0, area, 0.0);
        assert!(close(fit.scale, 6.0));
        let (_, top) = fit.point(5.0, 0.0);
        let (_, bottom) = fit.point(5.0, 100.0);
        assert!(close(top, 300.0) && close(bottom, -300.0));
    }

    #[test]
    fn margins_and_panels_shrink_the_area() {
        // A 300 px side panel on the right, 100 px header.
        let area = Area::inside_window(1300.0, 800.0, 0.0, 300.0, 100.0, 0.0);
        assert!(close(area.width, 1000.0) && close(area.height, 700.0));
        assert!(close(area.center_x, -150.0) && close(area.center_y, -50.0));
        let fit = Fit::new(100.0, 100.0, area, 0.05);
        assert!(close(fit.scale, 6.3));
        let (cx, cy) = fit.point(50.0, 50.0);
        assert!(close(cx, -150.0) && close(cy, -50.0));
        assert!(close(fit.length(10.0), 63.0));
    }

    #[test]
    fn a_degenerate_scene_does_not_divide_by_zero() {
        let area = Area::inside_window(100.0, 100.0, 0.0, 0.0, 0.0, 0.0);
        let fit = Fit::new(0.0, -3.0, area, 0.0);
        assert!(fit.scale.is_finite() && fit.scale > 0.0);
    }

    #[test]
    fn hexes_are_pointy_top() {
        let c = hex_corners(10.0, 20.0, 2.0);
        assert!(close(c[0].0, 10.0) && close(c[0].1, 18.0), "top {:?}", c[0]);
        assert!(
            close(c[3].0, 10.0) && close(c[3].1, 22.0),
            "bottom {:?}",
            c[3]
        );
        // Flat sides left and right.
        assert!(close(c[1].0, c[2].0) && close(c[4].0, c[5].0));
        assert!(close(c[1].0 - 10.0, 3f32.sqrt()));
    }

    #[test]
    fn axial_hex_centres_and_vertices() {
        assert_eq!(hex_center(0, 0), (0.0, 0.0));
        let (x, y) = hex_center(1, 0);
        assert!(close(x, 3f32.sqrt()) && close(y, 0.0));
        let (x, y) = hex_center(0, 1);
        assert!(close(x, 3f32.sqrt() / 2.0) && close(y, -1.5));
        let (x, y) = vertex_position(2, -1, true);
        assert!(close(x, 1.5 * 3f32.sqrt()) && close(y, 2.5));
        let (_, y) = vertex_position(0, 0, false);
        assert!(close(y, -1.0));
        // The S corner of (0,0) and the N corner of (0,1) share an edge, so
        // they are one circumradius apart.
        let a = vertex_position(0, 0, false);
        let b = vertex_position(0, 1, true);
        let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
        assert!(close(d, 1.0), "{d}");
    }

    #[test]
    fn board_maps_up_to_minus_z() {
        assert_eq!(board_to_ground(1.0, 2.0), (1.0, -2.0));
    }

    #[test]
    fn segments() {
        let (a, l) = segment(0.0, 0.0, 0.0, 2.0);
        assert!(close(a, std::f32::consts::FRAC_PI_2) && close(l, 2.0));
    }
}
