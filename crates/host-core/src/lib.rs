//! The part of a Hostable host that isn't drawing: seats, game state, the
//! action log, reaction windows, and working out what each phone should show.
//!
//! [`Room`] is shared by the desktop host and the cloud host. It speaks in
//! [`HostToRelay`] messages and [`Effect`]s and does no I/O itself, and it
//! never reads the clock: every call takes the current game time in
//! milliseconds, so tests can jump a day ahead in one line.

use std::collections::BTreeMap;

use hostable_protocol::{
    Feedback, HostToPhone, HostToRelay, PhoneInput, PlayerId, TableView, TextStyle, Widget,
    INPUT_AUTO_PASS, INPUT_PUSH, INPUT_WANT_TABLE,
};
use hostable_rules_api::{
    Event, OnTimeout, Outcome, Results, Rules, RulesError, Seat, SeatId, Wait,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod bot;
mod check;
pub use check::{check_outcome, check_table, MAX_EVENTS, MAX_TABLE_BYTES, MAX_TABLE_SHAPES};

/// Most widgets a game may put on one phone screen.
pub const MAX_UI_NODES: usize = 500;
/// Most automatic passes in a row before the room decides the game is stuck.
const MAX_AUTO_STEPS: usize = 256;

/// Whether players are all online now, or playing over days.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pace {
    #[default]
    Live,
    Async,
}

/// Something the shell should do after a call into the room.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Send this to the relay.
    Send(HostToRelay),
    /// Play this on the shared screen.
    Event(Event),
    /// The shared table changed; redraw it from [`Room::table_view`].
    TableChanged,
    /// A seat is now being waited on. The cloud host sends a push
    /// notification if that player has one set up.
    Notify {
        player: PlayerId,
        seat: SeatId,
        prompt: String,
    },
    /// A phone sent its Web Push subscription.
    PushSubscribed {
        player: PlayerId,
        subscription: Value,
    },
    /// The rules failed; the game is paused until [`Room::resume`].
    Paused(String),
    /// The game ended.
    Finished(Results),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomError {
    AlreadyStarted,
    NotStarted,
    TooFewPlayers { have: u32, need: u32 },
    UnknownPlayer(PlayerId),
    Rules(String),
}

impl std::fmt::Display for RoomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoomError::AlreadyStarted => write!(f, "the game has already started"),
            RoomError::NotStarted => write!(f, "the game hasn't started"),
            RoomError::TooFewPlayers { have, need } => {
                write!(f, "{have} players joined, the game needs {need}")
            }
            RoomError::UnknownPlayer(p) => write!(f, "no player {p}"),
            RoomError::Rules(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for RoomError {}

/// What moved the game forward, kept so a game can be replayed from its seed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoggedAction {
    /// A player's input.
    Input { seat: SeatId, input: PhoneInput },
    /// A wait ran out (or was passed automatically) and the rules decided.
    Timeout { seat: SeatId, now_ms: u64 },
    /// A wait ran out (or was passed automatically) with a preset input.
    Passed { seat: SeatId, input: PhoneInput },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PlayerSlot {
    /// `None` for a spectator.
    seat: Option<SeatId>,
    name: String,
    #[serde(default)]
    color: String,
    connected: bool,
    #[serde(default)]
    want_table: bool,
    #[serde(default)]
    auto_pass: bool,
    #[serde(default)]
    push: Option<Value>,
    /// The last screen sent (before the timer was added) and the deadline it
    /// showed, to skip resending the same one.
    #[serde(skip)]
    last_ui: Option<(Widget, Option<u64>)>,
    #[serde(skip)]
    last_table: Option<u64>,
}

/// A wait the room is counting down.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveWait {
    pub wait: Wait,
    pub started_ms: u64,
    pub deadline_ms: u64,
}

/// Everything needed to bring a room back after the host restarts, or to
/// move it to another host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(serialize = "", deserialize = ""))]
pub struct Snapshot<R: Rules> {
    seed: u64,
    #[serde(default)]
    pace: Pace,
    players: BTreeMap<PlayerId, PlayerSlot>,
    state: Option<R::State>,
    log: Vec<LoggedAction>,
    ui_version: u64,
    #[serde(default)]
    waits: Vec<ActiveWait>,
    #[serde(default)]
    finished: Option<Results>,
    #[serde(default)]
    paused: Option<String>,
    #[serde(default)]
    table_version: u64,
}

/// A room on the host: the players in it and the game they're playing.
pub struct Room<R: Rules> {
    rules: R,
    game_name: String,
    seed: u64,
    pace: Pace,
    players: BTreeMap<PlayerId, PlayerSlot>,
    state: Option<R::State>,
    log: Vec<LoggedAction>,
    ui_version: u64,
    table_version: u64,
    waits: Vec<ActiveWait>,
    finished: Option<Results>,
    paused: Option<String>,
    now_ms: u64,
}

impl<R: Rules> Room<R> {
    pub fn new(rules: R, seed: u64) -> Self {
        Self {
            rules,
            game_name: String::new(),
            seed,
            pace: Pace::Live,
            players: BTreeMap::new(),
            state: None,
            log: Vec::new(),
            ui_version: 0,
            table_version: 0,
            waits: Vec::new(),
            finished: None,
            paused: None,
            now_ms: 0,
        }
    }

    /// The game's display name, shown on phones.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.game_name = name.into();
        self
    }

    pub fn with_pace(mut self, pace: Pace) -> Self {
        self.pace = pace;
        self
    }

    pub fn rules(&self) -> &R {
        &self.rules
    }

    pub fn pace(&self) -> Pace {
        self.pace
    }

    /// Switch between live and async. Running deadlines are kept.
    pub fn set_pace(&mut self, pace: Pace) {
        self.pace = pace;
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn is_started(&self) -> bool {
        self.state.is_some()
    }

    pub fn state(&self) -> Option<&R::State> {
        self.state.as_ref()
    }

    pub fn log(&self) -> &[LoggedAction] {
        &self.log
    }

    pub fn waits(&self) -> &[ActiveWait] {
        &self.waits
    }

    pub fn finished(&self) -> Option<&Results> {
        self.finished.as_ref()
    }

    pub fn paused(&self) -> Option<&str> {
        self.paused.as_deref()
    }

    /// The earliest deadline, so the shell knows when to call [`Room::tick`].
    pub fn next_deadline(&self) -> Option<u64> {
        self.waits.iter().map(|w| w.deadline_ms).min()
    }

    /// The seated players, by seat.
    pub fn seats(&self) -> Vec<Seat> {
        let mut seats: Vec<Seat> = self
            .players
            .values()
            .filter_map(|p| {
                p.seat.map(|id| Seat {
                    id,
                    name: p.name.clone(),
                    color: p.color.clone(),
                })
            })
            .collect();
        seats.sort_by_key(|s| s.id);
        seats
    }

    /// The player in a seat.
    pub fn player_in(&self, seat: SeatId) -> Option<PlayerId> {
        self.players
            .iter()
            .find(|(_, s)| s.seat == Some(seat))
            .map(|(p, _)| *p)
    }

    pub fn seat_of(&self, player: PlayerId) -> Option<SeatId> {
        self.players.get(&player).and_then(|s| s.seat)
    }

    pub fn is_connected(&self, player: PlayerId) -> bool {
        self.players.get(&player).is_some_and(|s| s.connected)
    }

    pub fn push_subscription(&self, player: PlayerId) -> Option<&Value> {
        self.players.get(&player).and_then(|s| s.push.as_ref())
    }

    /// Players in the room, with their seat (`None` for spectators).
    pub fn players(&self) -> Vec<(PlayerId, Option<SeatId>, String, bool)> {
        self.players
            .iter()
            .map(|(p, s)| (*p, s.seat, s.name.clone(), s.connected))
            .collect()
    }

    /// The relay reported a new player. Before the game starts they get the
    /// next free seat; after that, or once the table is full, they watch.
    pub fn player_joined(
        &mut self,
        player: PlayerId,
        name: String,
        color: String,
        now_ms: u64,
    ) -> Vec<Effect> {
        self.add_player(player, name, color, true, now_ms)
    }

    /// A seat made ahead of time for someone arriving with a share link.
    pub fn player_added(&mut self, player: PlayerId, name: String, now_ms: u64) -> Vec<Effect> {
        self.add_player(player, name, String::new(), false, now_ms)
    }

    fn add_player(
        &mut self,
        player: PlayerId,
        name: String,
        color: String,
        connected: bool,
        now_ms: u64,
    ) -> Vec<Effect> {
        self.now_ms = self.now_ms.max(now_ms);
        let (_, max) = self.rules.seats();
        let seated = self.players.values().filter(|p| p.seat.is_some()).count() as u32;
        let seat = (!self.is_started() && seated < max).then_some(seated);
        self.players.insert(
            player,
            PlayerSlot {
                seat,
                name,
                color: clean_color(&color),
                connected,
                want_table: false,
                auto_pass: false,
                push: None,
                last_ui: None,
                last_table: None,
            },
        );
        let mut effects = vec![self.seat_message(player)];
        effects.extend(self.refresh_phones());
        effects
    }

    fn seat_message(&self, player: PlayerId) -> Effect {
        let slot = &self.players[&player];
        Effect::Send(HostToRelay::ToPlayer {
            player,
            message: HostToPhone::Seat {
                seat: slot.seat,
                name: slot.name.clone(),
                game: self.game_name.clone(),
            },
        })
    }

    /// A player's connection dropped. Their seat is kept.
    pub fn player_away(&mut self, player: PlayerId) {
        if let Some(slot) = self.players.get_mut(&player) {
            slot.connected = false;
        }
    }

    /// A player came back. They get their seat and current screen again.
    pub fn player_returned(
        &mut self,
        player: PlayerId,
        now_ms: u64,
    ) -> Result<Vec<Effect>, RoomError> {
        self.now_ms = self.now_ms.max(now_ms);
        let slot = self
            .players
            .get_mut(&player)
            .ok_or(RoomError::UnknownPlayer(player))?;
        slot.connected = true;
        slot.last_ui = None;
        slot.last_table = None;
        let mut effects = vec![self.seat_message(player)];
        effects.extend(self.refresh_phones());
        Ok(effects)
    }

    /// Start the game with everyone seated.
    pub fn start(&mut self, now_ms: u64) -> Result<Vec<Effect>, RoomError> {
        if self.is_started() {
            return Err(RoomError::AlreadyStarted);
        }
        self.now_ms = self.now_ms.max(now_ms);
        let (min, _) = self.rules.seats();
        let seats = self.seats();
        let have = seats.len() as u32;
        if have < min {
            return Err(RoomError::TooFewPlayers { have, need: min });
        }
        let outcome = self
            .rules
            .setup(&seats, self.seed)
            .map_err(|e| RoomError::Rules(e.to_string()))?;
        check_outcome(&outcome, have).map_err(|e| RoomError::Rules(e.to_string()))?;
        let mut effects = Vec::new();
        self.apply(outcome, &mut effects);
        self.settle(&mut effects);
        effects.extend(self.refresh_phones());
        Ok(effects)
    }

    /// Input from a phone. Input for a widget that isn't on that player's
    /// current screen, or with a value that screen couldn't send, is dropped,
    /// so a stale or tampered phone can't act.
    pub fn handle_input(
        &mut self,
        player: PlayerId,
        input: PhoneInput,
        now_ms: u64,
    ) -> Result<Vec<Effect>, RoomError> {
        self.now_ms = self.now_ms.max(now_ms);
        if !self.players.contains_key(&player) {
            return Err(RoomError::UnknownPlayer(player));
        }
        if input.is_host_input() {
            return Ok(self.host_input(player, input));
        }
        let mut effects = Vec::new();
        // Deadlines that passed while nobody was looking come first.
        self.fire_due(&mut effects);
        let slot = &self.players[&player];
        let Some(seat) = slot.seat else {
            return Ok(effects);
        };
        let allowed = slot
            .last_ui
            .as_ref()
            .is_some_and(|(ui, _)| ui.accepts(&input));
        if self.state.is_none() {
            return Err(RoomError::NotStarted);
        }
        if !allowed || self.paused.is_some() || self.finished.is_some() {
            effects.extend(self.refresh_phones());
            return Ok(effects);
        }
        let state = self.state.clone().expect("checked above");
        let result = self.rules.on_action(state, seat, &input);
        if self.accept(result, seat, &mut effects) {
            self.log.push(LoggedAction::Input { seat, input });
            self.settle(&mut effects);
        }
        effects.extend(self.refresh_phones());
        Ok(effects)
    }

    /// Run any waits whose deadline has passed.
    pub fn tick(&mut self, now_ms: u64) -> Vec<Effect> {
        self.now_ms = self.now_ms.max(now_ms);
        let mut effects = Vec::new();
        self.fire_due(&mut effects);
        effects.extend(self.refresh_phones());
        effects
    }

    /// Clear a pause so play can go on. The call that failed is dropped.
    pub fn resume(&mut self) -> Vec<Effect> {
        self.paused = None;
        for slot in self.players.values_mut() {
            slot.last_ui = None;
        }
        self.refresh_phones()
    }

    /// The current shared table, or `None` before the game starts.
    pub fn table_view(&self) -> Option<Result<Value, RulesError>> {
        self.state.as_ref().map(|s| self.rules.table_view(s))
    }

    /// What a player's phone shows right now (as last sent).
    pub fn current_ui(&self, player: PlayerId) -> Option<&Widget> {
        self.players
            .get(&player)
            .and_then(|s| s.last_ui.as_ref())
            .map(|(w, _)| w)
    }

    fn host_input(&mut self, player: PlayerId, input: PhoneInput) -> Vec<Effect> {
        let mut effects = Vec::new();
        let slot = self.players.get_mut(&player).expect("checked by caller");
        match input.widget.as_str() {
            INPUT_WANT_TABLE => {
                slot.want_table = input.value.as_bool().unwrap_or(false);
                slot.last_table = None;
            }
            INPUT_AUTO_PASS => {
                slot.auto_pass = input.value.as_bool().unwrap_or(false);
                slot.last_ui = None;
                self.settle(&mut effects);
            }
            INPUT_PUSH if is_push_subscription(&input.value) => {
                slot.push = Some(input.value.clone());
                effects.push(Effect::PushSubscribed {
                    player,
                    subscription: input.value,
                });
            }
            _ => {}
        }
        effects.extend(self.refresh_phones());
        effects
    }

    /// Take the rules' answer, or pause the game if it failed. Returns
    /// whether the game moved on.
    fn accept(
        &mut self,
        result: Result<Outcome<R::State>, RulesError>,
        _seat: SeatId,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let seats = self.seats().len() as u32;
        let checked = result.and_then(|o| check_outcome(&o, seats).map(|()| o));
        match checked {
            Ok(outcome) => {
                self.apply(outcome, effects);
                true
            }
            Err(e) => {
                self.pause(e.to_string(), effects);
                false
            }
        }
    }

    fn pause(&mut self, message: String, effects: &mut Vec<Effect>) {
        self.paused = Some(message.clone());
        effects.push(Effect::Paused(message));
    }

    fn apply(&mut self, outcome: Outcome<R::State>, effects: &mut Vec<Effect>) {
        let Outcome {
            state,
            events,
            waiting_on,
            finished,
        } = outcome;
        self.state = Some(state);
        for event in events {
            if let Event::Feedback { seat, kind } = &event {
                if let Some(player) = self.player_in(*seat) {
                    effects.push(Effect::Send(HostToRelay::ToPlayer {
                        player,
                        message: HostToPhone::Feedback { kind: *kind },
                    }));
                }
            }
            effects.push(Effect::Event(event));
        }
        effects.push(Effect::TableChanged);
        if let Some(results) = finished {
            self.waits.clear();
            self.finished = Some(results.clone());
            effects.push(Effect::Finished(results));
            return;
        }
        let now = self.now_ms;
        let pace = self.pace;
        let old = std::mem::take(&mut self.waits);
        for wait in waiting_on {
            let same = old
                .iter()
                .find(|w| {
                    w.wait.seat == wait.seat
                        && w.wait.prompt == wait.prompt
                        && w.wait.reaction == wait.reaction
                })
                .cloned();
            let active = match same {
                Some(mut kept) => {
                    kept.wait = wait;
                    kept
                }
                None => {
                    let timeout = match pace {
                        Pace::Live => wait.live_timeout_ms,
                        Pace::Async => wait.async_timeout_ms,
                    };
                    if let Some(player) = self.player_in(wait.seat) {
                        effects.push(Effect::Notify {
                            player,
                            seat: wait.seat,
                            prompt: wait.prompt.clone(),
                        });
                        effects.push(Effect::Send(HostToRelay::ToPlayer {
                            player,
                            message: HostToPhone::Feedback {
                                kind: Feedback::YourTurn,
                            },
                        }));
                    }
                    ActiveWait {
                        wait,
                        started_ms: now,
                        deadline_ms: now.saturating_add(timeout),
                    }
                }
            };
            self.waits.push(active);
        }
    }

    /// Pass automatically for seats that can't respond to a reaction window
    /// (always in async games, and for players who asked for it).
    fn settle(&mut self, effects: &mut Vec<Effect>) {
        for _ in 0..MAX_AUTO_STEPS {
            if self.paused.is_some() || self.finished.is_some() {
                return;
            }
            let Some(state) = self.state.as_ref() else {
                return;
            };
            let mut pass = None;
            for (i, w) in self.waits.iter().enumerate() {
                if !w.wait.reaction {
                    continue;
                }
                let wants = self.pace == Pace::Async
                    || self
                        .player_in(w.wait.seat)
                        .and_then(|p| self.players.get(&p))
                        .is_some_and(|s| s.auto_pass);
                if !wants {
                    continue;
                }
                match self.rules.can_respond(state, w.wait.seat) {
                    Ok(false) => {
                        pass = Some(i);
                        break;
                    }
                    Ok(true) => {}
                    Err(e) => {
                        self.pause(e.to_string(), effects);
                        return;
                    }
                }
            }
            let Some(i) = pass else {
                return;
            };
            let wait = self.waits.remove(i);
            self.run_timeout(wait.wait, effects);
        }
        self.pause(
            "the game kept passing on its own and never settled".into(),
            effects,
        );
    }

    fn fire_due(&mut self, effects: &mut Vec<Effect>) {
        for _ in 0..MAX_AUTO_STEPS {
            if self.paused.is_some() || self.finished.is_some() {
                return;
            }
            let now = self.now_ms;
            let due = self
                .waits
                .iter()
                .enumerate()
                .filter(|(_, w)| w.deadline_ms <= now)
                .min_by_key(|(_, w)| w.deadline_ms)
                .map(|(i, _)| i);
            let Some(i) = due else {
                return;
            };
            let wait = self.waits.remove(i);
            self.run_timeout(wait.wait, effects);
            self.settle(effects);
        }
    }

    fn run_timeout(&mut self, wait: Wait, effects: &mut Vec<Effect>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let seat = wait.seat;
        let now = self.now_ms;
        match wait.on_timeout {
            OnTimeout::Rules => {
                let result = self.rules.on_timeout(state, seat, now);
                if self.accept(result, seat, effects) {
                    self.log.push(LoggedAction::Timeout { seat, now_ms: now });
                }
            }
            OnTimeout::Input { input } => {
                let result = self.rules.on_action(state, seat, &input);
                if self.accept(result, seat, effects) {
                    self.log.push(LoggedAction::Passed { seat, input });
                }
            }
        }
    }

    pub fn snapshot(&self) -> Snapshot<R> {
        Snapshot {
            seed: self.seed,
            pace: self.pace,
            players: self.players.clone(),
            state: self.state.clone(),
            log: self.log.clone(),
            ui_version: self.ui_version,
            waits: self.waits.clone(),
            finished: self.finished.clone(),
            paused: self.paused.clone(),
            table_version: self.table_version,
        }
    }

    /// Rebuild a room from a snapshot. Every player is marked away until
    /// they reconnect.
    pub fn restore(rules: R, snapshot: Snapshot<R>) -> Self {
        let mut players = snapshot.players;
        for slot in players.values_mut() {
            slot.connected = false;
            slot.last_ui = None;
            slot.last_table = None;
        }
        let now_ms = snapshot
            .waits
            .iter()
            .map(|w| w.started_ms)
            .max()
            .unwrap_or(0);
        Self {
            rules,
            game_name: String::new(),
            seed: snapshot.seed,
            pace: snapshot.pace,
            players,
            state: snapshot.state,
            log: snapshot.log,
            ui_version: snapshot.ui_version,
            table_version: snapshot.table_version,
            waits: snapshot.waits,
            finished: snapshot.finished,
            paused: snapshot.paused,
            now_ms,
        }
    }

    /// Send each connected player their screen (and table) if it changed.
    fn refresh_phones(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        let table: Option<Value> = match (&self.state, self.players.values().any(|p| p.want_table))
        {
            (Some(state), true) => self.rules.table_view(state).ok(),
            _ => None,
        };
        let table_version = table.as_ref().map(hash_value);
        let players: Vec<PlayerId> = self.players.keys().copied().collect();
        for player in players {
            let slot = &self.players[&player];
            if !slot.connected {
                continue;
            }
            let base = self.base_view(slot);
            let wait = slot
                .seat
                .and_then(|seat| self.waits.iter().find(|w| w.wait.seat == seat))
                .cloned();
            let deadline = wait.as_ref().map(|w| w.deadline_ms);
            let unchanged = slot
                .last_ui
                .as_ref()
                .is_some_and(|(ui, d)| *ui == base && *d == deadline);
            let send_table =
                slot.want_table && table_version.is_some() && slot.last_table != table_version;
            if !unchanged {
                let shown = match &wait {
                    Some(w) => with_timer(base.clone(), w, self.now_ms),
                    None => base.clone(),
                };
                self.ui_version += 1;
                let version = self.ui_version;
                self.players.get_mut(&player).unwrap().last_ui = Some((base, deadline));
                effects.push(Effect::Send(HostToRelay::ToPlayer {
                    player,
                    message: HostToPhone::Ui { version, ui: shown },
                }));
            }
            if send_table {
                if let Some(Ok(view)) = table
                    .as_ref()
                    .map(|t| serde_json::from_value::<TableView>(t.clone()))
                {
                    self.table_version += 1;
                    self.players.get_mut(&player).unwrap().last_table = table_version;
                    effects.push(Effect::Send(HostToRelay::ToPlayer {
                        player,
                        message: HostToPhone::Table {
                            version: self.table_version,
                            table: view,
                        },
                    }));
                }
            }
        }
        effects
    }

    fn base_view(&self, slot: &PlayerSlot) -> Widget {
        if let Some(message) = &self.paused {
            return Widget::column(vec![
                Widget::title("Game paused"),
                Widget::styled(message.clone(), TextStyle::Warning),
            ]);
        }
        let Some(state) = &self.state else {
            return lobby_view(slot);
        };
        let Some(seat) = slot.seat else {
            return self.spectator_view(state);
        };
        let ui = match self.rules.phone_view(state, seat) {
            Ok(ui) => ui,
            Err(e) => {
                return Widget::column(vec![
                    Widget::title("Something went wrong"),
                    Widget::styled(e.to_string(), TextStyle::Warning),
                ])
            }
        };
        if ui.node_count() > MAX_UI_NODES {
            Widget::styled(
                "This game sent a screen that is too big to show.",
                TextStyle::Warning,
            )
        } else {
            ui
        }
    }

    fn spectator_view(&self, state: &R::State) -> Widget {
        let status = self
            .rules
            .table_view(state)
            .ok()
            .and_then(|v| serde_json::from_value::<TableView>(v).ok())
            .map(|t| t.status)
            .unwrap_or_default();
        let mut children = vec![Widget::title("Watching")];
        if !status.is_empty() {
            children.push(Widget::text(status));
        }
        if let Some(r) = &self.finished {
            children.push(Widget::text(r.summary.clone()));
        }
        Widget::column(children)
    }
}

/// Put the countdown and prompt above a waiting seat's screen.
fn with_timer(base: Widget, wait: &ActiveWait, now_ms: u64) -> Widget {
    let total = wait.deadline_ms.saturating_sub(wait.started_ms);
    let timer = Widget::Timer {
        ends_in_ms: wait.deadline_ms.saturating_sub(now_ms),
        total_ms: Some(total),
        label: wait.wait.prompt.clone(),
    };
    Widget::column(vec![timer, base])
}

fn lobby_view(slot: &PlayerSlot) -> Widget {
    let mut children = vec![Widget::title(format!("Hi, {}!", slot.name))];
    match slot.seat {
        Some(_) => children.push(Widget::text("Waiting for the game to start.")),
        None => children.push(Widget::text("The table is full, so you're watching.")),
    }
    Widget::column(children)
}

/// Keep a `#rrggbb` colour; drop anything else.
fn clean_color(color: &str) -> String {
    let ok = color.len() == 7
        && color.starts_with('#')
        && color[1..].chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        color.to_ascii_lowercase()
    } else {
        String::new()
    }
}

fn is_push_subscription(v: &Value) -> bool {
    let endpoint_ok = v
        .get("endpoint")
        .and_then(Value::as_str)
        .is_some_and(|e| e.starts_with("https://") && e.len() < 1024);
    let keys_ok = ["p256dh", "auth"].iter().all(|k| {
        v.get("keys")
            .and_then(|keys| keys.get(*k))
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty() && s.len() < 256)
    });
    endpoint_ok && keys_ok
}

fn hash_value(v: &Value) -> u64 {
    // FNV-1a over the JSON text: cheap change detection, not security.
    let text = v.to_string();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Replay a game from its seed and log, as a check that it is deterministic.
pub fn replay<R: Rules>(
    rules: &R,
    seats: &[Seat],
    seed: u64,
    log: &[LoggedAction],
) -> Result<R::State, RulesError> {
    let mut state = rules.setup(seats, seed)?.state;
    for entry in log {
        state = match entry {
            LoggedAction::Input { seat, input } | LoggedAction::Passed { seat, input } => {
                rules.on_action(state, *seat, input)?.state
            }
            LoggedAction::Timeout { seat, now_ms } => {
                rules.on_timeout(state, *seat, *now_ms)?.state
            }
        };
    }
    Ok(state)
}

#[cfg(test)]
mod tests;
