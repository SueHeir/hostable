//! The windowed host: lobby (room code, QR code, players, Start), the
//! table (generic 2D scene or the duel's 3D board), player panels, log,
//! toasts, pause banner and results.

use std::time::Duration;

use bevy::asset::RenderAssetUsages;
use bevy::camera::ClearColorConfig;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use hostable_host_runtime::Command;
use hostable_protocol::TableView;
use hostable_rules_api::Event;

use crate::cli::Cli;
use crate::color;
use crate::config;
use crate::duel::{self, DuelCamera, DuelPart, DuelShown, DuelView, HpFill, WizardFigure};
use crate::game::GameSetup;
use crate::geometry::{Area, Fit};
use crate::host::{GameEvent, HandOff, HostLink, HostPlugin, HostState};
use crate::qr;
use crate::scene::{spawn_scene, to_color, SceneShape};

/// Width of the side panel, logical pixels.
const PANEL_W: f32 = 360.0;
/// Height of the header over the table.
const HEADER_H: f32 = 76.0;
const TOAST_SECS: f32 = 3.0;

const BG: Color = Color::srgb(0.11, 0.12, 0.15);
const PANEL_BG: Color = Color::srgb(0.16, 0.17, 0.21);
const CARD_BG: Color = Color::srgb(0.21, 0.22, 0.27);
const TEXT: Color = Color::srgb(0.93, 0.93, 0.95);
const DIM: Color = Color::srgb(0.62, 0.64, 0.70);
const ACCENT: Color = Color::srgb(0.32, 0.55, 0.95);
const DISABLED: Color = Color::srgb(0.30, 0.31, 0.35);

/// The windowed app.
pub fn app(setup: GameSetup, cli: &Cli) -> Result<App, String> {
    let mut app = App::new();
    let title = format!("Hostable - {}", setup.name);
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title,
            resolution: (1280u32, 800u32).into(),
            ..default()
        }),
        ..default()
    }))
    .insert_resource(ClearColor(BG))
    .add_plugins(HostPlugin::new(setup, cli.cloud.clone())?)
    .add_plugins(GuiPlugin {
        join_url: cli.join_url.clone(),
        cloud: cli.cloud.clone(),
    });
    Ok(app)
}

pub struct GuiPlugin {
    pub join_url: String,
    pub cloud: Option<String>,
}

#[derive(Resource, Clone)]
struct GuiConfig {
    join_url: String,
    cloud: Option<String>,
}

#[derive(Component)]
struct UiCamera;
#[derive(Component)]
struct LeftPane;
#[derive(Component)]
struct SidePanel;
#[derive(Component)]
struct ToastLayer;
#[derive(Component)]
struct Toast {
    left: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Start,
    MoveToCloud,
    Resume,
}

#[derive(Component)]
struct UiButton {
    action: Action,
    enabled: bool,
}

/// The QR code for the current room.
#[derive(Resource, Default)]
struct QrImage {
    code: String,
    image: Option<Handle<Image>>,
}

/// What the UI was last built from.
#[derive(Resource, Default)]
struct Built {
    version: u64,
    hand_off: Option<HandOff>,
    code: Option<String>,
    reconnecting: bool,
    errors: usize,
}

impl Plugin for GuiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GuiConfig {
            join_url: self.join_url.clone(),
            cloud: self.cloud.clone(),
        })
        .init_resource::<QrImage>()
        .init_resource::<Built>()
        .init_resource::<DuelShown>()
        .add_systems(Startup, setup_ui)
        .add_systems(Update, developer_aids)
        .add_systems(
            Update,
            (
                update_qr,
                rebuild,
                redraw_table,
                clicks,
                button_colors,
                play_events,
                fade_toasts,
                duel::animate,
            )
                .chain(),
        );
    }
}

fn font(size: f32) -> TextFont {
    TextFont {
        font_size: FontSize::Px(size),
        ..default()
    }
}

fn label(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (Text::new(text.into()), font(size), TextColor(color))
}

fn setup_ui(mut commands: Commands) {
    commands.spawn((
        UiCamera,
        Camera2d,
        Camera {
            order: 1,
            ..default()
        },
        IsDefaultUiCamera,
    ));
    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Row,
            ..default()
        })
        .with_children(|root| {
            root.spawn((
                LeftPane,
                Node {
                    flex_grow: 1.0,
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
            ));
            root.spawn((
                SidePanel,
                Node {
                    width: Val::Px(PANEL_W),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(16.0)),
                    row_gap: Val::Px(10.0),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
            ));
        });
    commands.spawn((
        ToastLayer,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(HEADER_H + 12.0),
            left: Val::Px(0.0),
            right: Val::Px(PANEL_W),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(8.0),
            ..default()
        },
        GlobalZIndex(10),
    ));
}

fn update_qr(
    state: Res<HostState>,
    config: Res<GuiConfig>,
    mut qr_image: ResMut<QrImage>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(code) = &state.code else {
        return;
    };
    if &qr_image.code == code {
        return;
    }
    qr_image.code = code.clone();
    qr_image.image = qr::encode(&config::join_link(&config.join_url, code))
        .ok()
        .map(|m| {
            let (side, px) = m.rgba(8);
            let mut image = Image::new(
                Extent3d {
                    width: side as u32,
                    height: side as u32,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                px,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            );
            image.sampler = ImageSampler::nearest();
            images.add(image)
        });
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    state: Res<HostState>,
    config: Res<GuiConfig>,
    qr_image: Res<QrImage>,
    mut built: ResMut<Built>,
    left: Single<Entity, With<LeftPane>>,
    side: Single<Entity, With<SidePanel>>,
) {
    let changed = built.version != state.version
        || built.hand_off != state.hand_off
        || built.code != state.code
        || built.reconnecting != state.reconnecting
        || built.errors != state.errors.len()
        || (built.code.is_some() && qr_image.is_changed());
    if !changed {
        return;
    }
    built.version = state.version;
    built.hand_off = state.hand_off.clone();
    built.code = state.code.clone();
    built.reconnecting = state.reconnecting;
    built.errors = state.errors.len();
    let left = *left;
    let side = *side;
    commands.entity(left).despawn_children();
    commands.entity(side).despawn_children();
    let link = state
        .code
        .as_ref()
        .map(|c| config::join_link(&config.join_url, c));
    let table = match &state.info.table {
        Some(Ok(t)) => Some(t.clone()),
        _ => None,
    };

    // The table side: lobby before the game, a header during it.
    commands.entity(left).with_children(|p| {
        if !state.info.started {
            lobby(p, &state, link.as_deref(), qr_image.image.clone());
            return;
        }
        let (title, status) = table
            .as_ref()
            .map(|t| (t.title.clone(), t.status.clone()))
            .unwrap_or((state.game_name.clone(), String::new()));
        p.spawn((
            Node {
                height: Val::Px(HEADER_H),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                padding: UiRect::horizontal(Val::Px(20.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.09, 0.12, 0.85)),
        ))
        .with_children(|h| {
            h.spawn(label(title, 30.0, TEXT));
            h.spawn(label(status, 20.0, DIM));
        });
        if let Some(Err(e)) = &state.info.table {
            p.spawn(label(
                format!("The table can't be drawn: {e}"),
                18.0,
                Color::srgb(1.0, 0.5, 0.4),
            ));
        }
        if let Some(why) = &state.paused {
            banner(p, why);
        }
        if let Some(results) = &state.finished {
            p.spawn(Node {
                margin: UiRect::all(Val::Auto),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::all(Val::Px(28.0)),
                row_gap: Val::Px(10.0),
                border_radius: BorderRadius::all(Val::Px(14.0)),
                ..default()
            })
            .insert(BackgroundColor(Color::srgba(0.05, 0.05, 0.08, 0.88)))
            .with_children(|r| {
                r.spawn(label("Game over", 44.0, TEXT));
                r.spawn(label(results.summary.clone(), 26.0, TEXT));
                for (place, seats) in results.ranking.iter().enumerate() {
                    let names: Vec<String> = seats
                        .iter()
                        .map(|s| state.info.seat_name(*s).unwrap_or("?").to_string())
                        .collect();
                    r.spawn(label(
                        format!("{}. {}", place + 1, names.join(", ")),
                        22.0,
                        DIM,
                    ));
                }
            });
        }
    });

    // The side panel.
    commands.entity(side).with_children(|p| {
        if state.info.started {
            if let Some(t) = &table {
                player_panels(p, t);
                if !t.log.is_empty() {
                    p.spawn(label("Log", 18.0, DIM));
                    for line in t.log.iter().rev().take(8).rev() {
                        p.spawn(label(line.clone(), 15.0, TEXT));
                    }
                }
            }
        } else {
            players(p, &state);
            let can = state.info.can_start();
            button(p, "Start game", Action::Start, can);
            if !can && state.info.min_seats > 0 {
                p.spawn(label(
                    format!("Needs at least {} players", state.info.min_seats),
                    15.0,
                    DIM,
                ));
            }
        }
        if config.cloud.is_some() {
            match &state.hand_off {
                None => button(
                    p,
                    "Move to cloud",
                    Action::MoveToCloud,
                    state.code.is_some(),
                ),
                Some(HandOff::Moving) => {
                    p.spawn(label("Moving to the cloud...", 16.0, DIM));
                }
                Some(HandOff::Done(url)) => {
                    p.spawn(label("Moved to the cloud. Share:", 16.0, DIM));
                    p.spawn(label(url.clone(), 16.0, ACCENT));
                }
                Some(HandOff::Failed(e)) => {
                    p.spawn(label(
                        format!("Move failed: {e}"),
                        15.0,
                        Color::srgb(1.0, 0.5, 0.4),
                    ));
                    button(p, "Try again", Action::MoveToCloud, true);
                }
            }
        }
        // Footer: how to join, connection state.
        p.spawn(Node {
            flex_grow: 1.0,
            ..default()
        });
        if state.info.started {
            if let (Some(code), Some(link)) = (&state.code, &link) {
                p.spawn(label(format!("Room {code}"), 22.0, TEXT));
                p.spawn(label(link.clone(), 14.0, DIM));
            }
        }
        if state.reconnecting {
            p.spawn(label(
                "Relay connection lost, reconnecting...",
                15.0,
                Color::srgb(1.0, 0.8, 0.4),
            ));
        }
        if let Some(e) = state.errors.last() {
            p.spawn(label(e.clone(), 14.0, Color::srgb(1.0, 0.5, 0.4)));
        }
    });
}

fn lobby(
    p: &mut ChildSpawnerCommands,
    state: &HostState,
    link: Option<&str>,
    qr: Option<Handle<Image>>,
) {
    p.spawn(Node {
        flex_grow: 1.0,
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        row_gap: Val::Px(14.0),
        ..default()
    })
    .with_children(|c| {
        c.spawn(label(state.game_name.clone(), 36.0, TEXT));
        let Some(link) = link else {
            c.spawn(label("Opening a room...", 26.0, DIM));
            return;
        };
        c.spawn(label("Scan to join, or open", 22.0, DIM));
        c.spawn(label(link.to_string(), 24.0, ACCENT));
        if let Some(image) = qr {
            c.spawn((
                Node {
                    width: Val::Px(300.0),
                    height: Val::Px(300.0),
                    ..default()
                },
                ImageNode::new(image),
            ));
        }
        c.spawn(label("Room code", 22.0, DIM));
        c.spawn(label(state.code.clone().unwrap_or_default(), 120.0, TEXT));
    });
}

fn players(p: &mut ChildSpawnerCommands, state: &HostState) {
    let info = &state.info;
    p.spawn(label(
        format!("Players ({}/{})", info.seated(), info.max_seats.max(1)),
        22.0,
        TEXT,
    ));
    if info.players.is_empty() {
        p.spawn(label("Nobody yet", 16.0, DIM));
    }
    for (i, player) in info.players.iter().enumerate() {
        let seat = player
            .seat
            .map_or("watching".to_string(), |s| format!("seat {}", s + 1));
        let conn = if player.connected {
            "connected"
        } else {
            "away"
        };
        let swatch = to_color(color::player("", player.seat.unwrap_or(i as u32) as usize));
        p.spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(10.0),
                padding: UiRect::all(Val::Px(8.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(CARD_BG),
        ))
        .with_children(|row| {
            row.spawn((
                Node {
                    width: Val::Px(14.0),
                    height: Val::Px(14.0),
                    border_radius: BorderRadius::all(Val::Px(7.0)),
                    ..default()
                },
                BackgroundColor(if player.connected { swatch } else { DISABLED }),
            ));
            row.spawn(label(player.name.clone(), 18.0, TEXT));
            row.spawn(label(format!("{seat} - {conn}"), 14.0, DIM));
        });
    }
}

fn player_panels(p: &mut ChildSpawnerCommands, table: &TableView) {
    for (i, panel) in table.players.iter().enumerate() {
        let c = to_color(color::player(&panel.color, i));
        p.spawn((
            Node {
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(10.0)),
                border: UiRect::left(Val::Px(6.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(if panel.active {
                Color::srgb(0.27, 0.30, 0.40)
            } else {
                CARD_BG
            }),
            BorderColor::all(c),
        ))
        .with_children(|card| {
            let name = if panel.active {
                format!("> {}", panel.name)
            } else {
                panel.name.clone()
            };
            card.spawn(label(name, 20.0, TEXT));
            for line in &panel.lines {
                card.spawn(label(line.clone(), 15.0, DIM));
            }
        });
    }
}

fn banner(p: &mut ChildSpawnerCommands, why: &str) {
    p.spawn((
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(16.0),
            padding: UiRect::all(Val::Px(12.0)),
            margin: UiRect::horizontal(Val::Px(20.0)),
            border_radius: BorderRadius::all(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(Color::srgb(0.55, 0.22, 0.18)),
    ))
    .with_children(|b| {
        b.spawn(label(format!("Paused: {why}"), 20.0, TEXT));
        button(b, "Resume", Action::Resume, true);
    });
}

fn button(p: &mut ChildSpawnerCommands, text: &str, action: Action, enabled: bool) {
    p.spawn((
        UiButton { action, enabled },
        Node {
            padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
            justify_content: JustifyContent::Center,
            border_radius: BorderRadius::all(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(if enabled { ACCENT } else { DISABLED }),
    ))
    .with_child(label(text, 20.0, if enabled { TEXT } else { DIM }));
}

/// Hit-test buttons ourselves (cursor and nodes are both in physical
/// pixels).
fn clicks(
    mouse: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    buttons: Query<(&UiButton, &ComputedNode, &UiGlobalTransform)>,
    link: Res<HostLink>,
    config: Res<GuiConfig>,
    mut state: ResMut<HostState>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(cursor) = window.physical_cursor_position() else {
        return;
    };
    for (button, node, transform) in &buttons {
        if !button.enabled || !node.contains_point(*transform, cursor) {
            continue;
        }
        match button.action {
            Action::Start => link.send(Command::Start),
            Action::Resume => link.send(Command::Resume),
            Action::MoveToCloud => {
                if let Some(cloud) = &config.cloud {
                    state.hand_off = Some(HandOff::Moving);
                    link.move_to_cloud(cloud.clone());
                }
            }
        }
    }
}

fn button_colors(
    window: Single<&Window, With<PrimaryWindow>>,
    mut buttons: Query<(
        &UiButton,
        &ComputedNode,
        &UiGlobalTransform,
        &mut BackgroundColor,
    )>,
) {
    let cursor = window.physical_cursor_position();
    for (button, node, transform, mut bg) in &mut buttons {
        let hover = cursor.is_some_and(|c| node.contains_point(*transform, c));
        let want = match (button.enabled, hover) {
            (false, _) => DISABLED,
            (true, true) => Color::srgb(0.45, 0.66, 1.0),
            (true, false) => ACCENT,
        };
        if bg.0 != want {
            bg.0 = want;
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn redraw_table(
    mut commands: Commands,
    state: Res<HostState>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut built_for: Local<(u64, Vec2)>,
    shapes: Query<Entity, With<SceneShape>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials_2d: ResMut<Assets<ColorMaterial>>,
    mut materials_3d: ResMut<Assets<StandardMaterial>>,
    mut duel_shown: ResMut<DuelShown>,
    parts: Query<Entity, With<DuelPart>>,
    mut figures: Query<(&mut WizardFigure, &mut Transform)>,
    mut fills: Query<(&HpFill, &mut Transform), Without<WizardFigure>>,
    mut ui_camera: Single<&mut Camera, (With<UiCamera>, Without<DuelCamera>)>,
    mut duel_camera: Query<&mut Camera, (With<DuelCamera>, Without<UiCamera>)>,
) {
    let size = Vec2::new(window.width(), window.height());
    let scale = window.scale_factor();
    if built_for.0 == state.version && built_for.1 == size {
        // The duel camera may have been spawned last frame; keep it placed.
        for mut cam in &mut duel_camera {
            if cam.viewport.is_none() {
                place_duel_camera(&mut cam, size, scale);
            }
        }
        return;
    }
    *built_for = (state.version, size);
    for e in &shapes {
        commands.entity(e).despawn();
    }
    let table = match (&state.info.table, state.info.started) {
        (Some(Ok(t)), true) => Some(t),
        _ => None,
    };
    let duel_view = table.and_then(|t| DuelView::from_extra(&t.extra));
    duel::show(
        &mut commands,
        &mut meshes,
        &mut materials_3d,
        &mut duel_shown,
        &parts,
        &mut figures,
        &mut fills,
        duel_view.as_ref(),
    );
    // With the 3D board under it, the UI camera must not clear.
    ui_camera.clear_color = if duel_view.is_some() {
        ClearColorConfig::None
    } else {
        ClearColorConfig::Default
    };
    for mut cam in &mut duel_camera {
        place_duel_camera(&mut cam, size, scale);
    }
    if duel_view.is_some() {
        return;
    }
    let Some(scene) = table.and_then(|t| t.scene.as_ref()) else {
        return;
    };
    let area = Area::inside_window(size.x, size.y, 0.0, PANEL_W, HEADER_H, 0.0);
    let fit = Fit::new(scene.width, scene.height, area, 0.03);
    spawn_scene(&mut commands, &mut meshes, &mut materials_2d, scene, &fit);
}

fn place_duel_camera(cam: &mut Camera, size: Vec2, scale: f32) {
    let w = ((size.x - PANEL_W).max(1.0) * scale) as u32;
    let h = (size.y * scale) as u32;
    duel::set_viewport(cam, UVec2::ZERO, UVec2::new(w, h));
}

fn play_events(
    mut commands: Commands,
    mut events: MessageReader<GameEvent>,
    layer: Single<Entity, With<ToastLayer>>,
    #[cfg(feature = "sound")] mut pitches: ResMut<Assets<bevy::audio::Pitch>>,
) {
    for GameEvent(event) in events.read() {
        match event {
            Event::Toast { text } => {
                commands.entity(*layer).with_children(|l| {
                    l.spawn((
                        Toast { left: TOAST_SECS },
                        Node {
                            padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
                            border_radius: BorderRadius::all(Val::Px(10.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.05, 0.05, 0.08, 0.9)),
                    ))
                    .with_child(label(text.clone(), 24.0, TEXT));
                });
            }
            #[cfg(feature = "sound")]
            Event::Sound { name } => {
                // A short tone per sound name, no audio files needed.
                let hash = name
                    .bytes()
                    .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
                let freq = 330.0 + (hash % 12) as f32 * 55.0;
                commands.spawn((
                    AudioPlayer(
                        pitches.add(bevy::audio::Pitch::new(freq, Duration::from_millis(160))),
                    ),
                    PlaybackSettings::DESPAWN,
                ));
            }
            _ => {}
        }
    }
}

fn fade_toasts(mut commands: Commands, time: Res<Time>, mut toasts: Query<(Entity, &mut Toast)>) {
    let dt = time
        .delta_secs()
        .min(Duration::from_millis(250).as_secs_f32());
    for (e, mut toast) in &mut toasts {
        toast.left -= dt;
        if toast.left <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

/// Developer aids, read from the environment:
///
/// - `HOSTABLE_PREVIEW_TABLE=FILE.json`: show this table view as if a game
///   were running (to look at a renderer without phones).
/// - `HOSTABLE_SCREENSHOT=FILE.png`: save a screenshot after a moment, then
///   quit.
fn developer_aids(
    mut commands: Commands,
    mut frames: Local<u32>,
    mut state: ResMut<HostState>,
    mut exit: MessageWriter<AppExit>,
) {
    *frames += 1;
    if *frames == 5 {
        if let Some(path) = std::env::var("HOSTABLE_PREVIEW_TABLE")
            .ok()
            .filter(|p| !p.is_empty())
        {
            match std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| serde_json::from_slice::<TableView>(&b).map_err(|e| e.to_string()))
            {
                Ok(table) => {
                    state.info.started = true;
                    state.info.table = Some(Ok(table));
                    state.version += 1_000_000;
                }
                Err(e) => eprintln!("hostable: HOSTABLE_PREVIEW_TABLE {path}: {e}"),
            }
        }
    }
    let Ok(path) = std::env::var("HOSTABLE_SCREENSHOT") else {
        return;
    };
    if *frames == 90 {
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path));
    }
    if *frames == 130 {
        exit.write(AppExit::Success);
    }
}
