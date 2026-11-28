//! The generic 2D table renderer: draws a game's [`Scene`] (hexes, circles,
//! rects, lines, text) with Bevy 2D meshes, scaled to fit the table area.

use bevy::prelude::*;
use bevy::sprite::Anchor;
use hostable_protocol::{Scene, Shape};

use crate::color;
use crate::geometry::{segment, Fit};

/// Everything drawn for the current scene; despawned on redraw.
#[derive(Component)]
pub struct SceneShape;

/// Outline width in pixels.
const STROKE_PX: f32 = 2.0;

pub fn to_color(c: color::Rgba) -> Color {
    Color::srgba(c[0], c[1], c[2], c[3])
}

fn css(text: &str, fallback: Color) -> Color {
    color::parse(text).map(to_color).unwrap_or(fallback)
}

/// Draw `scene` into `fit`. Later shapes are drawn on top.
pub fn spawn_scene(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    scene: &Scene,
    fit: &Fit,
) {
    let mut z = 0.0f32;
    let mut next_z = || {
        z += 0.01;
        z
    };
    if let Some(bg) = color::parse(&scene.background) {
        let (x, y) = fit.point(scene.width / 2.0, scene.height / 2.0);
        commands.spawn((
            SceneShape,
            Mesh2d(meshes.add(Rectangle::new(
                fit.length(scene.width),
                fit.length(scene.height),
            ))),
            MeshMaterial2d(materials.add(to_color(bg))),
            Transform::from_xyz(x, y, next_z()),
        ));
    }
    let label = |commands: &mut Commands, x: f32, y: f32, size: f32, text: &str, c: Color, z| {
        if text.is_empty() {
            return;
        }
        commands.spawn((
            SceneShape,
            Text2d::new(text.to_string()),
            TextFont {
                font_size: FontSize::Px(size.clamp(6.0, 96.0)),
                ..default()
            },
            TextColor(c),
            Anchor::CENTER,
            Transform::from_xyz(x, y, z),
        ));
    };
    for shape in &scene.shapes {
        match shape {
            Shape::Hex {
                x,
                y,
                r,
                fill,
                stroke,
                label: text,
            } => {
                let (px, py) = fit.point(*x, *y);
                let r = fit.length(*r);
                if let Some(s) = color::parse(stroke) {
                    commands.spawn((
                        SceneShape,
                        Mesh2d(meshes.add(RegularPolygon::new(r + STROKE_PX, 6))),
                        MeshMaterial2d(materials.add(to_color(s))),
                        Transform::from_xyz(px, py, next_z()),
                    ));
                }
                commands.spawn((
                    SceneShape,
                    Mesh2d(meshes.add(RegularPolygon::new(r, 6))),
                    MeshMaterial2d(materials.add(css(fill, Color::srgb(0.6, 0.6, 0.6)))),
                    Transform::from_xyz(px, py, next_z()),
                ));
                label(commands, px, py, r * 0.6, text, Color::BLACK, next_z());
            }
            Shape::Circle {
                x,
                y,
                r,
                fill,
                stroke,
                label: text,
            } => {
                let (px, py) = fit.point(*x, *y);
                let r = fit.length(*r);
                if let Some(s) = color::parse(stroke) {
                    commands.spawn((
                        SceneShape,
                        Mesh2d(meshes.add(Circle::new(r + STROKE_PX))),
                        MeshMaterial2d(materials.add(to_color(s))),
                        Transform::from_xyz(px, py, next_z()),
                    ));
                }
                commands.spawn((
                    SceneShape,
                    Mesh2d(meshes.add(Circle::new(r))),
                    MeshMaterial2d(materials.add(css(fill, Color::srgb(0.6, 0.6, 0.6)))),
                    Transform::from_xyz(px, py, next_z()),
                ));
                label(commands, px, py, r * 0.8, text, Color::BLACK, next_z());
            }
            Shape::Rect {
                x,
                y,
                w,
                h,
                fill,
                stroke,
                label: text,
            } => {
                // `(x, y)` is the top-left corner.
                let (px, py) = fit.point(x + w / 2.0, y + h / 2.0);
                let (w, h) = (fit.length(*w), fit.length(*h));
                if let Some(s) = color::parse(stroke) {
                    commands.spawn((
                        SceneShape,
                        Mesh2d(
                            meshes.add(Rectangle::new(w + 2.0 * STROKE_PX, h + 2.0 * STROKE_PX)),
                        ),
                        MeshMaterial2d(materials.add(to_color(s))),
                        Transform::from_xyz(px, py, next_z()),
                    ));
                }
                commands.spawn((
                    SceneShape,
                    Mesh2d(meshes.add(Rectangle::new(w, h))),
                    MeshMaterial2d(materials.add(css(fill, Color::srgb(0.6, 0.6, 0.6)))),
                    Transform::from_xyz(px, py, next_z()),
                ));
                label(
                    commands,
                    px,
                    py,
                    h.min(w) * 0.4,
                    text,
                    Color::BLACK,
                    next_z(),
                );
            }
            Shape::Line {
                x1,
                y1,
                x2,
                y2,
                color: c,
                width,
            } => {
                let (ax, ay) = fit.point(*x1, *y1);
                let (bx, by) = fit.point(*x2, *y2);
                let (angle, len) = segment(ax, ay, bx, by);
                commands.spawn((
                    SceneShape,
                    Mesh2d(meshes.add(Rectangle::new(len, fit.length(*width).max(1.0)))),
                    MeshMaterial2d(materials.add(css(c, Color::BLACK))),
                    Transform::from_xyz((ax + bx) / 2.0, (ay + by) / 2.0, next_z())
                        .with_rotation(Quat::from_rotation_z(angle)),
                ));
            }
            Shape::Text {
                x,
                y,
                text,
                size,
                color: c,
            } => {
                let (px, py) = fit.point(*x, *y);
                let z = next_z();
                label(
                    commands,
                    px,
                    py,
                    fit.length(*size),
                    text,
                    css(c, Color::WHITE),
                    z,
                );
            }
        }
    }
}
