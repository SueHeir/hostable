//! The Hostable relay: rooms, join codes and message forwarding.
//!
//! The relay connects one host and many phones over WebSockets. It hands out
//! room codes and tokens and forwards messages, but never runs game logic and
//! never reads what the host and phones say to each other.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use hostable_protocol::{HostToRelay, PhoneInput, PlayerId, RelayToHost, RelayToPhone, RoomCode};
use rand::{distributions::Alphanumeric, Rng};
use tokio::sync::mpsc::UnboundedSender;

pub mod server;

/// Letters used in room codes. No look-alikes (I, L, O, 0, 1) and no vowels,
/// so codes can't spell words.
const CODE_LETTERS: &[u8] = b"BCDFGHJKMNPQRSTVWXZ";
const CODE_LEN: usize = 4;
const TOKEN_LEN: usize = 32;
pub const MAX_NAME_CHARS: usize = 24;
/// How long a room waits for its host to come back.
pub const HOST_GRACE: Duration = Duration::from_secs(10 * 60);

/// A queue of JSON text frames for one socket.
pub type Outbox = UnboundedSender<String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayError {
    NoSuchRoom,
    BadToken,
    RoomLocked,
    BadName,
}

impl RelayError {
    pub fn message(&self) -> &'static str {
        match self {
            RelayError::NoSuchRoom => "No game with that code.",
            RelayError::BadToken => "That seat is no longer valid.",
            RelayError::RoomLocked => "This game has already started.",
            RelayError::BadName => "Please enter a name.",
        }
    }
}

struct Player {
    token: String,
    outbox: Option<Outbox>,
}

struct Room {
    host_token: String,
    host: Option<Outbox>,
    host_left_at: Option<Instant>,
    players: HashMap<PlayerId, Player>,
    next_player: PlayerId,
    locked: bool,
}

impl Room {
    fn to_host(&self, msg: &RelayToHost) {
        if let Some(host) = &self.host {
            let _ = host.send(to_json(msg));
        }
    }

    fn to_player(&self, player: PlayerId, msg: &RelayToPhone) {
        if let Some(outbox) = self.players.get(&player).and_then(|p| p.outbox.as_ref()) {
            let _ = outbox.send(to_json(msg));
        }
    }
}

/// All rooms on this relay.
#[derive(Default)]
pub struct Registry {
    rooms: HashMap<RoomCode, Room>,
}

impl Registry {
    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    /// Open a room for a new host. Returns its code.
    pub fn create_room(&mut self, host: Outbox) -> RoomCode {
        let code = loop {
            let code = new_code();
            if !self.rooms.contains_key(&code) {
                break code;
            }
        };
        let host_token = new_token();
        let room = Room {
            host_token: host_token.clone(),
            host: Some(host),
            host_left_at: None,
            players: HashMap::new(),
            next_player: 1,
            locked: false,
        };
        room.to_host(&RelayToHost::RoomCreated {
            code: code.clone(),
            host_token,
        });
        self.rooms.insert(code.clone(), room);
        code
    }

    /// A host that dropped takes its room back.
    pub fn reclaim_room(
        &mut self,
        code: &str,
        host_token: &str,
        host: Outbox,
    ) -> Result<(), RelayError> {
        let room = self.rooms.get_mut(code).ok_or(RelayError::NoSuchRoom)?;
        if room.host_token != host_token {
            return Err(RelayError::BadToken);
        }
        room.host = Some(host);
        room.host_left_at = None;
        room.to_host(&RelayToHost::RoomReclaimed {
            code: code.to_owned(),
        });
        Ok(())
    }

    /// A message from the host of `code`. Returns false if the room closed.
    pub fn from_host(&mut self, code: &str, msg: HostToRelay) -> bool {
        let Some(room) = self.rooms.get_mut(code) else {
            return false;
        };
        match msg {
            HostToRelay::ToPlayer { player, message } => {
                room.to_player(player, &RelayToPhone::FromHost { message });
            }
            HostToRelay::Kick { player } => {
                room.to_player(
                    player,
                    &RelayToPhone::Error {
                        message: "You were removed from the game.".into(),
                    },
                );
                room.players.remove(&player);
            }
            HostToRelay::Lock { locked } => room.locked = locked,
            HostToRelay::CloseRoom => {
                for id in room.players.keys() {
                    room.to_player(
                        *id,
                        &RelayToPhone::Error {
                            message: "The game has ended.".into(),
                        },
                    );
                }
                self.rooms.remove(code);
                return false;
            }
            HostToRelay::CreateRoom | HostToRelay::ReclaimRoom { .. } => {
                room.to_host(&RelayToHost::Error {
                    message: "Already in a room.".into(),
                });
            }
        }
        true
    }

    /// The host's socket closed. The room waits [`HOST_GRACE`] for it.
    pub fn host_left(&mut self, code: &str, now: Instant) {
        if let Some(room) = self.rooms.get_mut(code) {
            room.host = None;
            room.host_left_at = Some(now);
            for id in room.players.keys() {
                room.to_player(*id, &RelayToPhone::HostAway);
            }
        }
    }

    /// A new player joins. Returns their id.
    pub fn join(&mut self, code: &str, name: &str, outbox: Outbox) -> Result<PlayerId, RelayError> {
        let code = code.trim().to_ascii_uppercase();
        let room = self.rooms.get_mut(&code).ok_or(RelayError::NoSuchRoom)?;
        if room.locked {
            return Err(RelayError::RoomLocked);
        }
        let name = clean_name(name).ok_or(RelayError::BadName)?;
        let player = room.next_player;
        room.next_player += 1;
        let player_token = new_token();
        room.players.insert(
            player,
            Player {
                token: player_token.clone(),
                outbox: Some(outbox),
            },
        );
        room.to_player(
            player,
            &RelayToPhone::Joined {
                player,
                player_token,
            },
        );
        room.to_host(&RelayToHost::PlayerJoined { player, name });
        Ok(player)
    }

    /// A player comes back with their token.
    pub fn rejoin(
        &mut self,
        code: &str,
        token: &str,
        outbox: Outbox,
    ) -> Result<PlayerId, RelayError> {
        let code = code.trim().to_ascii_uppercase();
        let room = self.rooms.get_mut(&code).ok_or(RelayError::NoSuchRoom)?;
        let (&player, slot) = room
            .players
            .iter_mut()
            .find(|(_, p)| p.token == token)
            .ok_or(RelayError::BadToken)?;
        slot.outbox = Some(outbox);
        room.to_player(player, &RelayToPhone::Rejoined { player });
        if room.host.is_none() {
            room.to_player(player, &RelayToPhone::HostAway);
        }
        room.to_host(&RelayToHost::PlayerReturned { player });
        Ok(player)
    }

    pub fn from_player(&mut self, code: &str, player: PlayerId, input: PhoneInput) {
        if let Some(room) = self.rooms.get(code) {
            if room.players.contains_key(&player) {
                room.to_host(&RelayToHost::FromPlayer { player, input });
            }
        }
    }

    pub fn player_left(&mut self, code: &str, player: PlayerId) {
        if let Some(room) = self.rooms.get_mut(code) {
            if let Some(slot) = room.players.get_mut(&player) {
                slot.outbox = None;
                room.to_host(&RelayToHost::PlayerAway { player });
            }
        }
    }

    /// Close rooms whose host has been gone longer than [`HOST_GRACE`].
    pub fn sweep(&mut self, now: Instant) {
        self.rooms.retain(|_, room| {
            room.host_left_at
                .is_none_or(|left| now.duration_since(left) < HOST_GRACE)
        });
    }
}

fn new_code() -> RoomCode {
    let mut rng = rand::thread_rng();
    (0..CODE_LEN)
        .map(|_| CODE_LETTERS[rng.gen_range(0..CODE_LETTERS.len())] as char)
        .collect()
}

fn new_token() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(TOKEN_LEN)
        .map(char::from)
        .collect()
}

/// Trim a player name, drop control characters and cap its length.
fn clean_name(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

fn to_json<T: serde::Serialize>(msg: &T) -> String {
    serde_json::to_string(msg).expect("protocol messages always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hostable_protocol::{HostToPhone, Widget};
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    fn drain<T: serde::de::DeserializeOwned>(rx: &mut UnboundedReceiver<String>) -> Vec<T> {
        let mut out = Vec::new();
        while let Ok(text) = rx.try_recv() {
            out.push(serde_json::from_str(&text).unwrap());
        }
        out
    }

    #[test]
    fn codes_use_only_unambiguous_letters() {
        for _ in 0..200 {
            let code = new_code();
            assert_eq!(code.len(), CODE_LEN);
            assert!(code.bytes().all(|b| CODE_LETTERS.contains(&b)));
        }
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  Ana \n  B\u{7}o "), Some("Ana Bo".into()));
        assert_eq!(clean_name("   "), None);
        assert_eq!(clean_name(&"x".repeat(100)).unwrap().len(), MAX_NAME_CHARS);
    }

    #[test]
    fn join_play_and_rejoin() {
        let mut reg = Registry::default();
        let (host_tx, mut host_rx) = unbounded_channel();
        let code = reg.create_room(host_tx);
        let created: Vec<RelayToHost> = drain(&mut host_rx);
        assert!(matches!(&created[0], RelayToHost::RoomCreated { code: c, .. } if *c == code));

        // A phone joins with a lower-case code.
        let (phone_tx, mut phone_rx) = unbounded_channel();
        let player = reg.join(&code.to_lowercase(), "Ana", phone_tx).unwrap();
        let RelayToPhone::Joined { player_token, .. } = drain(&mut phone_rx).remove(0) else {
            panic!("expected Joined");
        };
        assert_eq!(
            drain::<RelayToHost>(&mut host_rx),
            vec![RelayToHost::PlayerJoined {
                player,
                name: "Ana".into()
            }]
        );

        // Host to phone and phone to host.
        let message = HostToPhone::Ui {
            version: 1,
            ui: Widget::text("hi"),
        };
        reg.from_host(
            &code,
            HostToRelay::ToPlayer {
                player,
                message: message.clone(),
            },
        );
        assert_eq!(
            drain::<RelayToPhone>(&mut phone_rx),
            vec![RelayToPhone::FromHost { message }]
        );
        reg.from_player(&code, player, PhoneInput::press("buzz"));
        assert_eq!(
            drain::<RelayToHost>(&mut host_rx),
            vec![RelayToHost::FromPlayer {
                player,
                input: PhoneInput::press("buzz")
            }]
        );

        // The phone drops and comes back with its token.
        reg.player_left(&code, player);
        assert_eq!(
            drain::<RelayToHost>(&mut host_rx),
            vec![RelayToHost::PlayerAway { player }]
        );
        let (phone_tx, mut phone_rx) = unbounded_channel();
        assert_eq!(
            reg.rejoin(&code, "wrong", phone_tx.clone()),
            Err(RelayError::BadToken)
        );
        assert_eq!(reg.rejoin(&code, &player_token, phone_tx), Ok(player));
        assert_eq!(
            drain::<RelayToPhone>(&mut phone_rx),
            vec![RelayToPhone::Rejoined { player }]
        );
    }

    #[test]
    fn locked_rooms_refuse_new_players() {
        let mut reg = Registry::default();
        let (host_tx, _host_rx) = unbounded_channel();
        let code = reg.create_room(host_tx);
        reg.from_host(&code, HostToRelay::Lock { locked: true });
        let (phone_tx, _rx) = unbounded_channel();
        assert_eq!(
            reg.join(&code, "Ana", phone_tx),
            Err(RelayError::RoomLocked)
        );
    }

    #[test]
    fn a_room_waits_for_its_host_then_closes() {
        let mut reg = Registry::default();
        let (host_tx, mut host_rx) = unbounded_channel();
        let code = reg.create_room(host_tx);
        let RelayToHost::RoomCreated { host_token, .. } = drain(&mut host_rx).remove(0) else {
            panic!("expected RoomCreated");
        };
        let (phone_tx, mut phone_rx) = unbounded_channel();
        reg.join(&code, "Ana", phone_tx).unwrap();
        drain::<RelayToPhone>(&mut phone_rx);

        let t0 = Instant::now();
        reg.host_left(&code, t0);
        assert_eq!(
            drain::<RelayToPhone>(&mut phone_rx),
            vec![RelayToPhone::HostAway]
        );
        reg.sweep(t0 + Duration::from_secs(60));
        let (host_tx, _host_rx) = unbounded_channel();
        assert_eq!(reg.reclaim_room(&code, &host_token, host_tx), Ok(()));

        reg.host_left(&code, t0);
        reg.sweep(t0 + HOST_GRACE + Duration::from_secs(1));
        assert_eq!(reg.room_count(), 0);
    }
}
