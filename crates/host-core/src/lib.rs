//! The part of a Hostable host that isn't drawing: seats, game state, the
//! action log, and working out what each phone should show.
//!
//! [`Room`] is shared by the desktop host and the cloud host. It speaks in
//! [`HostToRelay`] messages and [`Event`]s and does no I/O itself, so the
//! shells decide how to send messages and how to draw the table.

use std::collections::BTreeMap;

use hostable_protocol::{HostToPhone, HostToRelay, PhoneInput, PlayerId, Widget};
use hostable_rules_api::{Event, Outcome, Results, Rules, Seat, SeatId};
use serde::{Deserialize, Serialize};

/// Most widgets a game may put on one phone screen.
pub const MAX_UI_NODES: usize = 500;

/// Something the shell should do after a call into the room.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Send this to the relay.
    Send(HostToRelay),
    /// Play this on the shared screen.
    Event(Event),
    /// The game ended.
    Finished(Results),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomError {
    AlreadyStarted,
    NotStarted,
    TableFull,
    TooFewPlayers { have: u32, need: u32 },
    UnknownPlayer(PlayerId),
}

/// One action, kept so a game can be replayed from its seed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoggedAction {
    pub seat: SeatId,
    pub input: PhoneInput,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PlayerSlot {
    seat: SeatId,
    name: String,
    connected: bool,
    /// The last screen sent to this player, to skip resending the same one.
    last_ui: Option<Widget>,
}

/// Everything needed to bring a room back after the host restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(serialize = "", deserialize = ""))]
pub struct Snapshot<R: Rules> {
    seed: u64,
    players: BTreeMap<PlayerId, PlayerSlot>,
    state: Option<R::State>,
    log: Vec<LoggedAction>,
    ui_version: u64,
}

/// A room on the host: the players in it and the game they're playing.
pub struct Room<R: Rules> {
    rules: R,
    seed: u64,
    players: BTreeMap<PlayerId, PlayerSlot>,
    state: Option<R::State>,
    log: Vec<LoggedAction>,
    ui_version: u64,
}

impl<R: Rules> Room<R> {
    pub fn new(rules: R, seed: u64) -> Self {
        Self {
            rules,
            seed,
            players: BTreeMap::new(),
            state: None,
            log: Vec::new(),
            ui_version: 0,
        }
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

    pub fn seats(&self) -> Vec<Seat> {
        let mut seats: Vec<Seat> = self
            .players
            .values()
            .map(|p| Seat {
                id: p.seat,
                name: p.name.clone(),
            })
            .collect();
        seats.sort_by_key(|s| s.id);
        seats
    }

    /// The relay reported a new player. They get the next free seat.
    pub fn player_joined(
        &mut self,
        player: PlayerId,
        name: String,
    ) -> Result<Vec<Effect>, RoomError> {
        if self.is_started() {
            return Err(RoomError::AlreadyStarted);
        }
        let (_, max) = self.rules.seats();
        if self.players.len() as u32 >= max {
            return Err(RoomError::TableFull);
        }
        let seat = self.players.len() as SeatId;
        self.players.insert(
            player,
            PlayerSlot {
                seat,
                name,
                connected: true,
                last_ui: None,
            },
        );
        Ok(self.refresh_phones())
    }

    /// A player's connection dropped. Their seat is kept.
    pub fn player_away(&mut self, player: PlayerId) {
        if let Some(slot) = self.players.get_mut(&player) {
            slot.connected = false;
        }
    }

    /// A player came back. They get their current screen again.
    pub fn player_returned(&mut self, player: PlayerId) -> Result<Vec<Effect>, RoomError> {
        let slot = self
            .players
            .get_mut(&player)
            .ok_or(RoomError::UnknownPlayer(player))?;
        slot.connected = true;
        slot.last_ui = None;
        Ok(self.refresh_phones())
    }

    /// Start the game with everyone who has joined.
    pub fn start(&mut self) -> Result<Vec<Effect>, RoomError> {
        if self.is_started() {
            return Err(RoomError::AlreadyStarted);
        }
        let (min, _) = self.rules.seats();
        let have = self.players.len() as u32;
        if have < min {
            return Err(RoomError::TooFewPlayers { have, need: min });
        }
        self.state = Some(self.rules.setup(&self.seats(), self.seed));
        Ok(self.refresh_phones())
    }

    /// Input from a phone. Input for a widget that isn't on that player's
    /// current screen is dropped, so a stale or tampered phone can't act.
    pub fn handle_input(
        &mut self,
        player: PlayerId,
        input: PhoneInput,
    ) -> Result<Vec<Effect>, RoomError> {
        let slot = self
            .players
            .get(&player)
            .ok_or(RoomError::UnknownPlayer(player))?;
        let seat = slot.seat;
        let allowed = slot
            .last_ui
            .as_ref()
            .is_some_and(|ui| ui.has_input(&input.widget));
        let Some(state) = self.state.take() else {
            return Err(RoomError::NotStarted);
        };
        if !allowed {
            self.state = Some(state);
            return Ok(Vec::new());
        }
        let outcome = self.rules.on_action(state, seat, &input);
        self.log.push(LoggedAction { seat, input });
        Ok(self.apply(outcome))
    }

    pub fn snapshot(&self) -> Snapshot<R> {
        Snapshot {
            seed: self.seed,
            players: self.players.clone(),
            state: self.state.clone(),
            log: self.log.clone(),
            ui_version: self.ui_version,
        }
    }

    /// Rebuild a room from a snapshot. Every player is marked away until
    /// they reconnect.
    pub fn restore(rules: R, snapshot: Snapshot<R>) -> Self {
        let mut players = snapshot.players;
        for slot in players.values_mut() {
            slot.connected = false;
            slot.last_ui = None;
        }
        Self {
            rules,
            seed: snapshot.seed,
            players,
            state: snapshot.state,
            log: snapshot.log,
            ui_version: snapshot.ui_version,
        }
    }

    fn apply(&mut self, outcome: Outcome<R::State>) -> Vec<Effect> {
        let Outcome {
            state,
            events,
            finished,
            ..
        } = outcome;
        self.state = Some(state);
        let mut effects: Vec<Effect> = events.into_iter().map(Effect::Event).collect();
        effects.extend(self.refresh_phones());
        if let Some(results) = finished {
            effects.push(Effect::Finished(results));
        }
        effects
    }

    /// Send each connected player their screen if it changed.
    fn refresh_phones(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        let players: Vec<PlayerId> = self.players.keys().copied().collect();
        for player in players {
            let slot = &self.players[&player];
            if !slot.connected {
                continue;
            }
            let ui = match &self.state {
                Some(state) => self.rules.phone_view(state, slot.seat),
                None => lobby_view(&slot.name),
            };
            let ui = if ui.node_count() > MAX_UI_NODES {
                Widget::text("This game sent a screen that is too big to show.")
            } else {
                ui
            };
            if slot.last_ui.as_ref() == Some(&ui) {
                continue;
            }
            self.ui_version += 1;
            let version = self.ui_version;
            self.players.get_mut(&player).unwrap().last_ui = Some(ui.clone());
            effects.push(Effect::Send(HostToRelay::ToPlayer {
                player,
                message: HostToPhone::Ui { version, ui },
            }));
        }
        effects
    }
}

fn lobby_view(name: &str) -> Widget {
    Widget::Column {
        children: vec![
            Widget::title(format!("Hi, {name}!")),
            Widget::text("Waiting for the game to start."),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hostable_buzzer::{Buzzer, Phase};

    fn sent_to(effects: &[Effect]) -> Vec<PlayerId> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Send(HostToRelay::ToPlayer { player, .. }) => Some(*player),
                _ => None,
            })
            .collect()
    }

    fn room_with(names: &[&str]) -> Room<Buzzer> {
        let mut room = Room::new(Buzzer, 7);
        for (i, name) in names.iter().enumerate() {
            room.player_joined(100 + i as PlayerId, name.to_string())
                .unwrap();
        }
        room
    }

    #[test]
    fn joining_shows_the_lobby_and_start_needs_enough_players() {
        let mut room = Room::new(Buzzer, 7);
        let effects = room.player_joined(100, "Ana".into()).unwrap();
        assert_eq!(sent_to(&effects), vec![100]);
        assert_eq!(
            room.start(),
            Err(RoomError::TooFewPlayers { have: 1, need: 2 })
        );
        room.player_joined(101, "Bo".into()).unwrap();
        let effects = room.start().unwrap();
        assert_eq!(sent_to(&effects), vec![100, 101]);
        assert_eq!(
            room.player_joined(102, "Cy".into()),
            Err(RoomError::AlreadyStarted)
        );
    }

    #[test]
    fn input_runs_the_rules_and_only_changed_screens_are_sent() {
        let mut room = room_with(&["Ana", "Bo"]);
        room.start().unwrap();
        let effects = room.handle_input(101, PhoneInput::press("buzz")).unwrap();
        assert_eq!(room.state().unwrap().phase, Phase::Buzzed { by: 1 });
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::Event(Event::Sound { .. }))));
        // Both screens changed: Bo sees "You were first", Ana sees a Next round button.
        assert_eq!(sent_to(&effects), vec![100, 101]);
        assert_eq!(room.log().len(), 1);
    }

    #[test]
    fn input_for_a_widget_not_on_screen_is_dropped() {
        let mut room = room_with(&["Ana", "Bo"]);
        room.start().unwrap();
        // Bo isn't the leader, so his screen never has a Next round button.
        let effects = room
            .handle_input(101, PhoneInput::press("next_round"))
            .unwrap();
        assert!(effects.is_empty());
        assert!(room.log().is_empty());
    }

    #[test]
    fn a_returning_player_gets_their_screen_again() {
        let mut room = room_with(&["Ana", "Bo"]);
        room.start().unwrap();
        room.player_away(101);
        let effects = room.handle_input(100, PhoneInput::press("buzz")).unwrap();
        assert_eq!(sent_to(&effects), vec![100]);
        let effects = room.player_returned(101).unwrap();
        assert_eq!(sent_to(&effects), vec![101]);
    }

    #[test]
    fn a_snapshot_survives_a_host_restart() {
        let mut room = room_with(&["Ana", "Bo"]);
        room.start().unwrap();
        room.handle_input(101, PhoneInput::press("buzz")).unwrap();
        let saved = serde_json::to_string(&room.snapshot()).unwrap();

        let mut room = Room::restore(Buzzer, serde_json::from_str(&saved).unwrap());
        assert_eq!(room.state().unwrap().scores, vec![0, 1]);
        assert_eq!(room.log().len(), 1);
        let effects = room.player_returned(100).unwrap();
        assert_eq!(sent_to(&effects), vec![100]);
        room.handle_input(100, PhoneInput::press("next_round"))
            .unwrap();
        assert_eq!(room.state().unwrap().round, 2);
    }
}
