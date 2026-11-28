//! The Bevy side of [`hostable_host_runtime::run`]: a background tokio
//! runtime owns the game, and channels carry [`Command`]s out and
//! [`Update`]s in. Nothing here touches windows or rendering, so the
//! headless smoke test and the GUI share it.

use std::collections::BTreeMap;
use std::sync::Mutex;

use bevy::prelude::*;
use hostable_host_core::{Effect, Pace};
use hostable_host_runtime::{hand_off, run, Command, GameRoom, Update};
use hostable_protocol::{PlayerId, TableView, Widget};
use hostable_rules_api::{Event, Results, SeatId};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::game::GameSetup;

/// A player as the room sees them.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerInfo {
    pub player: PlayerId,
    pub seat: Option<SeatId>,
    pub name: String,
    pub connected: bool,
}

/// A look at the room, taken with [`Command::Inspect`].
#[derive(Debug, Clone, Default)]
pub struct RoomInfo {
    pub players: Vec<PlayerInfo>,
    pub min_seats: u32,
    pub max_seats: u32,
    pub started: bool,
    pub finished: Option<Results>,
    pub paused: Option<String>,
    /// The shared table (`None` before the game starts).
    pub table: Option<Result<TableView, String>>,
    /// What each player's phone was last sent.
    pub screens: BTreeMap<PlayerId, Widget>,
}

impl RoomInfo {
    pub fn read(room: &GameRoom) -> RoomInfo {
        let (min_seats, max_seats) = room.rules().seats();
        let players: Vec<PlayerInfo> = room
            .players()
            .into_iter()
            .map(|(player, seat, name, connected)| PlayerInfo {
                player,
                seat,
                name,
                connected,
            })
            .collect();
        let screens = players
            .iter()
            .filter_map(|p| room.current_ui(p.player).map(|w| (p.player, w.clone())))
            .collect();
        let table = room.table_view().map(|r| {
            r.map_err(|e| e.to_string()).and_then(|v| {
                serde_json::from_value::<TableView>(v).map_err(|e| format!("bad table: {e}"))
            })
        });
        RoomInfo {
            players,
            min_seats,
            max_seats,
            started: room.is_started(),
            finished: room.finished().cloned(),
            paused: room.paused().map(str::to_string),
            table,
            screens,
        }
    }

    pub fn seated(&self) -> usize {
        self.players.iter().filter(|p| p.seat.is_some()).count()
    }

    pub fn can_start(&self) -> bool {
        !self.started && self.seated() as u32 >= self.min_seats
    }

    pub fn seat_name(&self, seat: SeatId) -> Option<&str> {
        self.players
            .iter()
            .find(|p| p.seat == Some(seat))
            .map(|p| p.name.as_str())
    }

    /// Names of the first-ranked seats.
    pub fn winners(&self) -> Vec<String> {
        let Some(r) = &self.finished else {
            return Vec::new();
        };
        r.ranking
            .first()
            .map(|seats| {
                seats
                    .iter()
                    .map(|s| self.seat_name(*s).unwrap_or("?").to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Something the rules want played on the shared screen.
#[derive(Message, Debug, Clone)]
pub struct GameEvent(pub Event);

/// The cloud hand-off, once asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum HandOff {
    Moving,
    /// The share link.
    Done(String),
    Failed(String),
}

/// What the shell knows about the game.
#[derive(Resource, Default)]
pub struct HostState {
    pub game_name: String,
    pub renderer: Option<String>,
    pub code: Option<String>,
    pub host_token: Option<String>,
    pub info: RoomInfo,
    /// Bumped whenever `info` is replaced.
    pub version: u64,
    pub finished: Option<Results>,
    pub paused: Option<String>,
    pub reconnecting: bool,
    pub closed: bool,
    pub errors: Vec<String>,
    /// The game loop returned (`Err` when it gave up).
    pub ended: Option<Result<(), String>>,
    pub hand_off: Option<HandOff>,
}

/// Wraps the runtime so dropping it never blocks exit.
struct Background(Option<Runtime>);

impl Drop for Background {
    fn drop(&mut self) {
        if let Some(rt) = self.0.take() {
            rt.shutdown_background();
        }
    }
}

/// The channels to and from the running game.
#[derive(Resource)]
pub struct HostLink {
    runtime: Background,
    commands: mpsc::UnboundedSender<Command>,
    updates: Mutex<mpsc::UnboundedReceiver<Update>>,
    infos_tx: mpsc::UnboundedSender<RoomInfo>,
    infos: Mutex<mpsc::UnboundedReceiver<RoomInfo>>,
    ended: Mutex<mpsc::UnboundedReceiver<Result<(), String>>>,
    hand_off_tx: mpsc::UnboundedSender<Result<(String, String), String>>,
    hand_off_rx: Mutex<mpsc::UnboundedReceiver<Result<(String, String), String>>>,
}

impl HostLink {
    /// Start a background runtime and the game loop on it.
    pub fn start(setup: GameSetup) -> Result<HostLink, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("hostable-host")
            .build()
            .map_err(|e| e.to_string())?;
        let (commands, commands_rx) = mpsc::unbounded_channel();
        let (updates_tx, updates) = mpsc::unbounded_channel();
        let (ended_tx, ended) = mpsc::unbounded_channel();
        let (infos_tx, infos) = mpsc::unbounded_channel();
        let (hand_off_tx, hand_off_rx) = mpsc::unbounded_channel();
        let GameSetup {
            room,
            reclaim,
            options,
            ..
        } = setup;
        runtime.spawn(async move {
            let result = run(room, reclaim, options, commands_rx, updates_tx).await;
            let _ = ended_tx.send(result);
        });
        Ok(HostLink {
            runtime: Background(Some(runtime)),
            commands,
            updates: Mutex::new(updates),
            infos_tx,
            infos: Mutex::new(infos),
            ended: Mutex::new(ended),
            hand_off_tx,
            hand_off_rx: Mutex::new(hand_off_rx),
        })
    }

    pub fn runtime(&self) -> &tokio::runtime::Handle {
        self.runtime
            .0
            .as_ref()
            .expect("runtime lives as long as the link")
            .handle()
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Ask for a fresh [`RoomInfo`]; it arrives in [`HostState::info`].
    pub fn inspect(&self) {
        let tx = self.infos_tx.clone();
        self.send(Command::Inspect(Box::new(move |room| {
            let _ = tx.send(RoomInfo::read(room));
        })));
    }

    /// Move the game to a cloud host.
    pub fn move_to_cloud(&self, cloud: String) {
        let commands = self.commands.clone();
        let done = self.hand_off_tx.clone();
        self.runtime().spawn(async move {
            let _ = done.send(hand_off(&commands, &cloud, Pace::Async).await);
        });
    }
}

/// Owns the game for the app: inserts [`HostLink`] and [`HostState`] and
/// pumps updates every frame.
pub struct HostPlugin {
    link: Mutex<Option<HostLink>>,
    game_name: String,
    renderer: Option<String>,
    /// The cloud root, for share links.
    cloud: Option<String>,
}

impl HostPlugin {
    pub fn new(setup: GameSetup, cloud: Option<String>) -> Result<HostPlugin, String> {
        let game_name = setup.name.clone();
        let renderer = setup.renderer.clone();
        Ok(HostPlugin {
            link: Mutex::new(Some(HostLink::start(setup)?)),
            game_name,
            renderer,
            cloud,
        })
    }
}

/// The cloud root given on the command line.
#[derive(Resource, Clone, Default)]
pub struct CloudRoot(pub Option<String>);

impl Plugin for HostPlugin {
    fn build(&self, app: &mut App) {
        let link = self
            .link
            .lock()
            .expect("host plugin lock")
            .take()
            .expect("HostPlugin is added once");
        app.insert_resource(link)
            .insert_resource(HostState {
                game_name: self.game_name.clone(),
                renderer: self.renderer.clone(),
                ..HostState::default()
            })
            .insert_resource(CloudRoot(self.cloud.clone()))
            .add_message::<GameEvent>()
            .add_systems(PreUpdate, pump_updates);
    }
}

/// Drain everything the game said since the last frame.
pub fn pump_updates(
    link: Res<HostLink>,
    mut state: ResMut<HostState>,
    cloud: Res<CloudRoot>,
    mut events: MessageWriter<GameEvent>,
) {
    let mut want_inspect = false;
    {
        let mut updates = link.updates.lock().expect("updates lock");
        while let Ok(update) = updates.try_recv() {
            want_inspect = true;
            match update {
                Update::Opened { code, host_token } => {
                    state.code = Some(code);
                    state.host_token = Some(host_token);
                    state.reconnecting = false;
                }
                Update::PlayerAdded { .. } | Update::Players => {}
                Update::Effects(effects) => {
                    for effect in effects {
                        match effect {
                            Effect::Event(e) => {
                                events.write(GameEvent(e));
                            }
                            Effect::Paused(why) => state.paused = Some(why),
                            Effect::Finished(r) => state.finished = Some(r),
                            Effect::TableChanged
                            | Effect::Send(_)
                            | Effect::Notify { .. }
                            | Effect::PushSubscribed { .. } => {}
                        }
                    }
                }
                Update::Finished(r) => state.finished = Some(r),
                Update::Error(e) => {
                    state.errors.push(e);
                    let n = state.errors.len();
                    if n > 20 {
                        state.errors.drain(..n - 20);
                    }
                }
                Update::Reconnecting => state.reconnecting = true,
                Update::Closed => state.closed = true,
            }
        }
    }
    if let Ok(result) = link.ended.lock().expect("ended lock").try_recv() {
        state.ended = Some(result);
    }
    if let Ok(result) = link.hand_off_rx.lock().expect("hand-off lock").try_recv() {
        state.hand_off = Some(match result {
            Ok((_, path)) => HandOff::Done(match &cloud.0 {
                Some(root) => crate::config::share_link(root, &path),
                None => path,
            }),
            Err(e) => HandOff::Failed(e),
        });
    }
    let mut latest = None;
    {
        let mut infos = link.infos.lock().expect("infos lock");
        while let Ok(info) = infos.try_recv() {
            latest = Some(info);
        }
    }
    if let Some(info) = latest {
        // A resumed or cleared pause shows in the room itself.
        state.paused = info.paused.clone();
        if info.finished.is_some() {
            state.finished = info.finished.clone();
        }
        state.info = info;
        state.version += 1;
    }
    if want_inspect && state.ended.is_none() {
        link.inspect();
    }
}
