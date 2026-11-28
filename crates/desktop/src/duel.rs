//! The wizards duel's native 3D table (pack `wizards-duel`, renderer
//! `wizards-duel`).
//!
//! The original game's art module is a separate binary crate on an older
//! Bevy, so this is a small reimplementation in the same storybook spirit,
//! not a port: extruded hex tiles tinted by mana with a darker rim, grey
//! lumps for rocks, and each wizard as a robe, head and pointed hat with an
//! HP bar, under a soft sun and an angled camera. It only reads the JSON the
//! duel puts in the table view's `extra`, so it doesn't depend on the duel's
//! crates.

use std::collections::BTreeMap;

use bevy::camera::{ClearColorConfig, Viewport};
use bevy::prelude::*;
use serde::Deserialize;
use serde_json::Value;

use crate::color;
use crate::geometry::{board_to_ground, hex_center, vertex_position};
use crate::scene::to_color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub struct AxialHex {
    pub q: i32,
    pub r: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum Corner {
    #[default]
    N,
    S,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub struct BoardVertex {
    pub hex: AxialHex,
    #[serde(default)]
    pub corner: Corner,
}

impl BoardVertex {
    /// Ground position (x, z).
    pub fn ground(&self) -> (f32, f32) {
        let (x, y) = vertex_position(self.hex.q, self.hex.r, self.corner == Corner::N);
        board_to_ground(x, y)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HexTile {
    pub hex: AxialHex,
    #[serde(default)]
    pub mana: Option<String>,
    #[serde(default)]
    pub terrain: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Board {
    #[serde(default)]
    pub hexes: Vec<HexTile>,
    #[serde(default)]
    pub rocks: Vec<BoardVertex>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WizardView {
    #[serde(default)]
    pub seat: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub color: String,
    pub at: BoardVertex,
    #[serde(default)]
    pub hp: i64,
    #[serde(default)]
    pub max_hp: i64,
    #[serde(default = "yes")]
    pub alive: bool,
}

fn yes() -> bool {
    true
}

/// The duel's part of `TableView::extra`.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct DuelView {
    pub board: Board,
    #[serde(default)]
    pub wizards: Vec<WizardView>,
}

impl DuelView {
    /// The duel view, if `extra` holds one (a board with hexes).
    pub fn from_extra(extra: &Value) -> Option<DuelView> {
        if !extra.get("board")?.get("hexes")?.is_array() {
            return None;
        }
        let view: DuelView = serde_json::from_value(extra.clone()).ok()?;
        (!view.board.hexes.is_empty()).then_some(view)
    }

    /// Centre and radius of the board on the ground (x, z).
    pub fn extent(&self) -> (Vec2, f32) {
        let pts: Vec<Vec2> = self
            .board
            .hexes
            .iter()
            .map(|h| {
                let (x, y) = hex_center(h.hex.q, h.hex.r);
                let (x, z) = board_to_ground(x, y);
                Vec2::new(x, z)
            })
            .collect();
        if pts.is_empty() {
            return (Vec2::ZERO, 1.0);
        }
        let min = pts.iter().fold(Vec2::splat(f32::MAX), |a, p| a.min(*p));
        let max = pts.iter().fold(Vec2::splat(f32::MIN), |a, p| a.max(*p));
        let center = (min + max) / 2.0;
        let radius = pts.iter().map(|p| p.distance(center)).fold(0.0, f32::max) + 1.0;
        (center, radius)
    }
}

/// Tile colour for a mana kind (or plain ground).
pub fn mana_color(mana: Option<&str>) -> color::Rgba {
    match mana {
        Some("Red") => [0.84, 0.30, 0.24, 1.0],
        Some("Yellow") => [0.95, 0.82, 0.32, 1.0],
        Some("Blue") => [0.30, 0.50, 0.86, 1.0],
        Some("Orange") => [0.95, 0.58, 0.24, 1.0],
        Some("Purple") => [0.58, 0.36, 0.74, 1.0],
        Some("Green") => [0.38, 0.68, 0.34, 1.0],
        _ => [0.86, 0.80, 0.64, 1.0],
    }
}

/// Height of the walkable tile tops.
const TILE_TOP: f32 = 0.3;

/// Tile colour and height for a terrain (`None`: no tile at all).
fn terrain_look(tile: &HexTile) -> Option<(color::Rgba, f32)> {
    match tile.terrain.as_deref() {
        Some("Hole") => None,
        Some("Water") => Some((
            tile.mana
                .as_deref()
                .map_or([0.32, 0.55, 0.78, 1.0], |m| mana_color(Some(m))),
            0.12,
        )),
        Some("Lava") => Some(([0.90, 0.35, 0.12, 1.0], 0.12)),
        Some("Ice") => Some(([0.82, 0.92, 0.97, 1.0], TILE_TOP)),
        _ => Some((mana_color(tile.mana.as_deref()), TILE_TOP)),
    }
}

/// Everything the duel renderer spawned.
#[derive(Component)]
pub struct DuelPart;

#[derive(Component)]
pub struct DuelCamera;

#[derive(Component)]
pub struct WizardFigure {
    pub seat: u32,
    pub target: Vec3,
}

#[derive(Component)]
pub struct HpFill {
    pub seat: u32,
}

/// What is on screen, to rebuild only what changed.
#[derive(Resource, Default)]
pub struct DuelShown {
    pub board: Option<Board>,
    pub figures: BTreeMap<u32, Entity>,
}

/// Show `view` (or tear the duel down when `None`). `viewport` is the
/// table area in physical pixels.
#[allow(clippy::too_many_arguments)]
pub fn show(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    shown: &mut DuelShown,
    parts: &Query<Entity, With<DuelPart>>,
    figures: &mut Query<(&mut WizardFigure, &mut Transform)>,
    fills: &mut Query<(&HpFill, &mut Transform), Without<WizardFigure>>,
    view: Option<&DuelView>,
) {
    let Some(view) = view else {
        if shown.board.is_some() {
            for e in parts.iter() {
                commands.entity(e).despawn();
            }
            *shown = DuelShown::default();
        }
        return;
    };
    if shown.board.as_ref() != Some(&view.board) {
        for e in parts.iter() {
            commands.entity(e).despawn();
        }
        shown.figures.clear();
        build_board(commands, meshes, materials, view);
        shown.board = Some(view.board.clone());
    }
    for w in &view.wizards {
        let (x, z) = w.at.ground();
        let target = Vec3::new(x, TILE_TOP, z);
        match shown.figures.get(&w.seat) {
            Some(&e) => {
                if let Ok((mut fig, mut t)) = figures.get_mut(e) {
                    fig.target = target;
                    let lying = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
                    t.rotation = if w.alive { Quat::IDENTITY } else { lying };
                }
            }
            None => {
                let e = spawn_wizard(commands, meshes, materials, w, target);
                shown.figures.insert(w.seat, e);
            }
        }
        let frac = if w.max_hp > 0 {
            (w.hp as f32 / w.max_hp as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        for (fill, mut t) in fills.iter_mut() {
            if fill.seat == w.seat {
                t.scale.x = frac.max(0.001);
                t.translation.x = -0.35 * (1.0 - frac);
            }
        }
    }
}

fn build_board(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    view: &DuelView,
) {
    let (center, radius) = view.extent();
    // An angled camera from the south, looking at the middle of the board.
    let eye = Vec3::new(center.x, radius * 2.0, center.y + radius * 1.75);
    commands.spawn((
        DuelPart,
        DuelCamera,
        Camera3d::default(),
        Camera {
            order: 0,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.55, 0.70, 0.78)),
            ..default()
        },
        Transform::from_translation(eye).looking_at(Vec3::new(center.x, 0.0, center.y), Vec3::Y),
    ));
    commands.spawn((
        DuelPart,
        DirectionalLight {
            illuminance: 9_000.0,
            shadow_maps_enabled: true,
            color: Color::srgb(1.0, 0.96, 0.88),
            ..default()
        },
        Transform::from_xyz(center.x - radius, radius * 2.0, center.y + radius * 0.5)
            .looking_at(Vec3::new(center.x, 0.0, center.y), Vec3::Y),
    ));
    // Tiles: a darker, slightly wider base under a lighter top.
    let base_mesh = meshes.add(Extrusion::new(RegularPolygon::new(1.0, 6), 1.0));
    let top_mesh = meshes.add(Extrusion::new(RegularPolygon::new(0.9, 6), 1.0));
    // Lying flat: the extrusion's depth (z) becomes height (y).
    let flat = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    for tile in &view.board.hexes {
        let Some((c, height)) = terrain_look(tile) else {
            continue;
        };
        let (x, y) = hex_center(tile.hex.q, tile.hex.r);
        let (x, z) = board_to_ground(x, y);
        let rough = |c: color::Rgba| StandardMaterial {
            base_color: to_color(c),
            perceptual_roughness: 0.85,
            ..default()
        };
        let base_h = height - 0.04;
        commands.spawn((
            DuelPart,
            Mesh3d(base_mesh.clone()),
            MeshMaterial3d(materials.add(rough(color::darker(c, 0.45)))),
            Transform::from_xyz(x, base_h / 2.0, z)
                .with_rotation(flat)
                .with_scale(Vec3::new(1.0, 1.0, base_h)),
        ));
        let mut top = rough(c);
        if tile.terrain.as_deref() == Some("Lava") {
            top.emissive = LinearRgba::rgb(1.2, 0.3, 0.05);
        }
        commands.spawn((
            DuelPart,
            Mesh3d(top_mesh.clone()),
            MeshMaterial3d(materials.add(top)),
            Transform::from_xyz(x, height - 0.02, z)
                .with_rotation(flat)
                .with_scale(Vec3::new(1.0, 1.0, 0.04)),
        ));
    }
    let rock_mesh = meshes.add(Sphere::new(0.32).mesh().ico(2).expect("small ico sphere"));
    let rock_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.48, 0.47, 0.45),
        perceptual_roughness: 1.0,
        ..default()
    });
    for (i, rock) in view.board.rocks.iter().enumerate() {
        let (x, z) = rock.ground();
        // A little variety so rocks read as lumps, not balls.
        let k = 0.85 + 0.1 * ((i * 7 % 5) as f32 / 4.0);
        commands.spawn((
            DuelPart,
            Mesh3d(rock_mesh.clone()),
            MeshMaterial3d(rock_mat.clone()),
            Transform::from_xyz(x, TILE_TOP + 0.08, z)
                .with_rotation(Quat::from_rotation_y(i as f32 * 1.3))
                .with_scale(Vec3::new(1.15 * k, 0.7 * k, 0.95 * k)),
        ));
    }
}

fn spawn_wizard(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    w: &WizardView,
    at: Vec3,
) -> Entity {
    let robe = color::player(&w.color, w.seat as usize);
    let mat = |c: Color, materials: &mut Assets<StandardMaterial>| {
        materials.add(StandardMaterial {
            base_color: c,
            perceptual_roughness: 0.7,
            ..default()
        })
    };
    let robe_mat = mat(to_color(robe), materials);
    let hat_mat = mat(to_color(color::darker(robe, 0.35)), materials);
    let skin = mat(Color::srgb(0.96, 0.82, 0.68), materials);
    let bar_bg = materials.add(StandardMaterial {
        base_color: Color::srgb(0.25, 0.05, 0.05),
        unlit: true,
        ..default()
    });
    let bar_fg = materials.add(StandardMaterial {
        base_color: Color::srgb(0.3, 0.85, 0.35),
        unlit: true,
        ..default()
    });
    let seat = w.seat;
    commands
        .spawn((
            DuelPart,
            WizardFigure { seat, target: at },
            Transform::from_translation(at),
            Visibility::default(),
        ))
        .with_children(|p| {
            p.spawn((
                Mesh3d(meshes.add(Cone::new(0.28, 0.62))),
                MeshMaterial3d(robe_mat),
                Transform::from_xyz(0.0, 0.31, 0.0),
            ));
            p.spawn((
                Mesh3d(meshes.add(Sphere::new(0.15))),
                MeshMaterial3d(skin),
                Transform::from_xyz(0.0, 0.72, 0.0),
            ));
            p.spawn((
                Mesh3d(meshes.add(Cylinder::new(0.26, 0.03))),
                MeshMaterial3d(hat_mat.clone()),
                Transform::from_xyz(0.0, 0.82, 0.0),
            ));
            p.spawn((
                Mesh3d(meshes.add(Cone::new(0.18, 0.5))),
                MeshMaterial3d(hat_mat),
                Transform::from_xyz(0.0, 1.08, 0.0),
            ));
            p.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.72, 0.08, 0.02))),
                MeshMaterial3d(bar_bg),
                Transform::from_xyz(0.0, 1.5, 0.0),
            ))
            .with_children(|bar| {
                bar.spawn((
                    HpFill { seat },
                    Mesh3d(meshes.add(Cuboid::new(0.7, 0.07, 0.02))),
                    MeshMaterial3d(bar_fg),
                    Transform::from_xyz(0.0, 0.0, 0.015),
                ));
            });
        })
        .id()
}

/// Glide wizards toward where the game says they are.
pub fn animate(time: Res<Time>, mut figures: Query<(&WizardFigure, &mut Transform)>) {
    let k = 1.0 - (-8.0 * time.delta_secs()).exp();
    for (fig, mut t) in figures.iter_mut() {
        let next = t.translation.lerp(fig.target, k);
        t.translation = next;
    }
}

/// Keep the duel camera inside the table area.
pub fn set_viewport(camera: &mut Camera, pos: UVec2, size: UVec2) {
    if size.x == 0 || size.y == 0 {
        return;
    }
    camera.viewport = Some(Viewport {
        physical_position: pos,
        physical_size: size,
        ..default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A hand-written sample in the shape the duel sends.
    fn sample() -> Value {
        json!({
            "board": {
                "id": "arena",
                "hexes": [
                    {"hex": {"q": 0, "r": 0}, "mana": "Red"},
                    {"hex": {"q": 1, "r": 0}},
                    {"hex": {"q": 0, "r": 1}, "terrain": "Water"},
                    {"hex": {"q": -1, "r": 1}, "terrain": "Hole"}
                ],
                "rocks": [{"hex": {"q": 1, "r": 0}, "corner": "N"}]
            },
            "wizards": [
                {"seat": 0, "name": "Ana", "color": "#d33", "at": {"hex": {"q": 0, "r": 0}, "corner": "N"}, "hp": 7, "max_hp": 10, "alive": true, "status": {}},
                {"seat": 1, "name": "Bo", "at": {"hex": {"q": 0, "r": 1}, "corner": "S"}, "hp": 0, "max_hp": 10, "alive": false}
            ],
            "zones": [], "current": 0, "turn": 3, "over": false
        })
    }

    #[test]
    fn the_duel_sample_parses() {
        let view = DuelView::from_extra(&sample()).unwrap();
        assert_eq!(view.board.hexes.len(), 4);
        assert_eq!(view.board.hexes[0].mana.as_deref(), Some("Red"));
        assert_eq!(view.board.rocks[0].corner, Corner::N);
        assert_eq!(view.wizards[1].at.corner, Corner::S);
        assert!(!view.wizards[1].alive);
        assert!(terrain_look(&view.board.hexes[3]).is_none());
        assert_eq!(terrain_look(&view.board.hexes[2]).unwrap().1, 0.12);
    }

    #[test]
    fn other_games_extra_is_not_a_duel() {
        assert!(DuelView::from_extra(&json!({"round": 1})).is_none());
        assert!(DuelView::from_extra(&Value::Null).is_none());
        assert!(DuelView::from_extra(&json!({"board": {"hexes": []}})).is_none());
    }

    #[test]
    fn wizards_stand_on_their_vertex() {
        let view = DuelView::from_extra(&sample()).unwrap();
        // N corner of (0,0): board (0, 1) -> ground z = -1.
        assert_eq!(view.wizards[0].at.ground(), (0.0, -1.0));
        let (x, z) = view.wizards[1].at.ground();
        assert!((x - 3f32.sqrt() / 2.0).abs() < 1e-5 && (z - 2.5).abs() < 1e-5);
        let (c, r) = view.extent();
        assert!(r > 1.0 && c.x.is_finite());
    }

    #[test]
    fn mana_colours_differ() {
        let all = ["Red", "Yellow", "Blue", "Orange", "Purple", "Green"];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(mana_color(Some(a)), mana_color(Some(b)));
            }
        }
        assert_eq!(mana_color(None), mana_color(Some("Nothing")));
    }
}
