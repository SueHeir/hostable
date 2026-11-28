//! The Hostable relay: rooms, join codes and message forwarding.
//!
//! The relay connects one host and many phones over WebSockets. It hands out
//! room codes and tokens and forwards messages, but never runs game logic and
//! never reads what the host and phones say to each other. It stores no
//! personal data beyond the display names players type, and only in memory
//! (or in its optional state file, so rooms survive a relay restart).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use hostable_protocol::{HostToRelay, PhoneInput, PlayerId, RelayToHost, RelayToPhone, RoomCode};
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

pub mod cli;
pub mod limits;
pub mod server;

/// Letters used in room codes. No look-alikes (I, L, O, 0, 1) and no vowels,
/// so codes can't spell words.
const CODE_LETTERS: &[u8] = b"BCDFGHJKMNPQRSTVWXZ";
const CODE_LEN: usize = 4;
const TOKEN_LEN: usize = 32;
pub const MAX_NAME_CHARS: usize = 24;
/// Most players (including spectators) in one room.
pub const MAX_PLAYERS: usize = 64;
/// How long a room waits for its host to come back.
pub const HOST_GRACE: Duration = Duration::from_secs(10 * 60);

/// A queue of JSON text frames for one socket.
pub type Outbox = UnboundedSender<String>;

/// Identifies one socket, so a stale connection can't act after a newer one
/// took its place.
pub type ConnId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayError {
    NoSuchRoom,
    BadToken,
    RoomLocked,
    RoomFull,
    BadName,
    TooManyAttempts,
}

impl RelayError {
    pub fn message(&self) -> &'static str {
        match self {
            RelayError::NoSuchRoom => "No game with that code.",
            RelayError::BadToken => "That seat is no longer valid.",
            RelayError::RoomLocked => "This game isn't taking new players.",
            RelayError::RoomFull => "This game is full.",
            RelayError::BadName => "Please enter a name.",
            RelayError::TooManyAttempts => "Too many wrong codes. Try again in a minute.",
        }
    }
}

struct Player {
    token: String,
    name: String,
    outbox: Option<(ConnId, Outbox)>,
}

struct Room {
    host_token: String,
    host: Option<(ConnId, Outbox)>,
    host_left_at: Option<Instant>,
    players: HashMap<PlayerId, Player>,
    next_player: PlayerId,
    locked: bool,
}

impl Room {
    fn to_host(&self, msg: &RelayToHost) {
        if let Some((_, host)) = &self.host {
            let _ = host.send(to_json(msg));
        }
    }

    fn to_player(&self, player: PlayerId, msg: &RelayToPhone) {
        if let Some((_, outbox)) = self.players.get(&player).and_then(|p| p.outbox.as_ref()) {
            let _ = outbox.send(to_json(msg));
        }
    }

    fn to_all_players(&self, msg: &RelayToPhone) {
        for id in self.players.keys() {
            self.to_player(*id, msg);
        }
    }

    fn host_is(&self, conn: ConnId) -> bool {
        self.host.as_ref().is_some_and(|(c, _)| *c == conn)
    }
}

/// What the relay writes to its state file: rooms and tokens, no sockets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub rooms: Vec<SavedRoom>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedRoom {
    pub code: RoomCode,
    pub host_token: String,
    pub players: Vec<SavedPlayer>,
    pub next_player: PlayerId,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedPlayer {
    pub id: PlayerId,
    pub token: String,
    pub name: String,
}

/// All rooms on this relay.
#[derive(Default)]
pub struct Registry {
    rooms: HashMap<RoomCode, Room>,
    next_conn: ConnId,
    /// Rooms or tokens changed since the last save.
    dirty: bool,
}

impl Registry {
    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    fn conn(&mut self) -> ConnId {
        self.next_conn += 1;
        self.next_conn
    }

    /// Whether anything worth saving changed since [`Registry::mark_saved`].
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub fn save(&self) -> Saved {
        let mut rooms: Vec<SavedRoom> = self
            .rooms
            .iter()
            .map(|(code, r)| {
                let mut players: Vec<SavedPlayer> = r
                    .players
                    .iter()
                    .map(|(id, p)| SavedPlayer {
                        id: *id,
                        token: p.token.clone(),
                        name: p.name.clone(),
                    })
                    .collect();
                players.sort_by_key(|p| p.id);
                SavedRoom {
                    code: code.clone(),
                    host_token: r.host_token.clone(),
                    players,
                    next_player: r.next_player,
                    locked: r.locked,
                }
            })
            .collect();
        rooms.sort_by(|a, b| a.code.cmp(&b.code));
        Saved { rooms }
    }

    /// Rebuild rooms after a restart. Every host gets [`HOST_GRACE`] from
    /// `now` to come back.
    pub fn restore(saved: Saved, now: Instant) -> Self {
        let rooms = saved
            .rooms
            .into_iter()
            .map(|r| {
                let players = r
                    .players
                    .into_iter()
                    .map(|p| {
                        (
                            p.id,
                            Player {
                                token: p.token,
                                name: p.name,
                                outbox: None,
                            },
                        )
                    })
                    .collect();
                (
                    r.code,
                    Room {
                        host_token: r.host_token,
                        host: None,
                        host_left_at: Some(now),
                        players,
                        next_player: r.next_player,
                        locked: r.locked,
                    },
                )
            })
            .collect();
        Self {
            rooms,
            next_conn: 0,
            dirty: false,
        }
    }

    /// Open a room for a new host. Returns its code and the host's
    /// connection id.
    pub fn create_room(&mut self, host: Outbox) -> (RoomCode, ConnId) {
        let code = loop {
            let code = new_code();
            if !self.rooms.contains_key(&code) {
                break code;
            }
        };
        let conn = self.conn();
        let host_token = new_token();
        let room = Room {
            host_token: host_token.clone(),
            host: Some((conn, host)),
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
        self.dirty = true;
        (code, conn)
    }

    /// A host that dropped takes its room back, or another host with the
    /// code and host token takes it over (desktop to cloud handoff). A host
    /// still connected is told and cut off.
    pub fn reclaim_room(
        &mut self,
        code: &str,
        host_token: &str,
        host: Outbox,
    ) -> Result<ConnId, RelayError> {
        let conn = self.conn();
        let room = self.rooms.get_mut(code).ok_or(RelayError::NoSuchRoom)?;
        if !constant_time_eq(&room.host_token, host_token) {
            return Err(RelayError::BadToken);
        }
        room.to_host(&RelayToHost::Error {
            message: "Another host took over this room.".into(),
        });
        room.host = Some((conn, host));
        room.host_left_at = None;
        let mut connected: Vec<PlayerId> = room
            .players
            .iter()
            .filter(|(_, p)| p.outbox.is_some())
            .map(|(id, _)| *id)
            .collect();
        connected.sort_unstable();
        room.to_host(&RelayToHost::RoomReclaimed {
            code: code.to_owned(),
            connected,
        });
        room.to_all_players(&RelayToPhone::HostBack);
        Ok(conn)
    }

    /// A message from the host of `code`. Returns false if the room closed
    /// or this host was replaced, so its socket should close.
    pub fn from_host(&mut self, code: &str, conn: ConnId, msg: HostToRelay) -> bool {
        let Some(room) = self.rooms.get_mut(code) else {
            return false;
        };
        if !room.host_is(conn) {
            return false;
        }
        match msg {
            HostToRelay::ToPlayer { player, message } => {
                room.to_player(player, &RelayToPhone::FromHost { message });
            }
            HostToRelay::AddPlayer { name } => {
                let Some(name) = clean_name(&name) else {
                    room.to_host(&RelayToHost::Error {
                        message: RelayError::BadName.message().into(),
                    });
                    return true;
                };
                if room.players.len() >= MAX_PLAYERS {
                    room.to_host(&RelayToHost::Error {
                        message: RelayError::RoomFull.message().into(),
                    });
                    return true;
                }
                let player = room.next_player;
                room.next_player += 1;
                let player_token = new_token();
                room.players.insert(
                    player,
                    Player {
                        token: player_token.clone(),
                        name: name.clone(),
                        outbox: None,
                    },
                );
                room.to_host(&RelayToHost::PlayerAdded {
                    player,
                    name,
                    player_token,
                });
                self.dirty = true;
            }
            HostToRelay::Kick { player } => {
                room.to_player(
                    player,
                    &RelayToPhone::Error {
                        message: "You were removed from the game.".into(),
                    },
                );
                room.players.remove(&player);
                self.dirty = true;
            }
            HostToRelay::Lock { locked } => {
                room.locked = locked;
                self.dirty = true;
            }
            HostToRelay::CloseRoom => {
                room.to_all_players(&RelayToPhone::Error {
                    message: "The game has ended.".into(),
                });
                self.rooms.remove(code);
                self.dirty = true;
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
    pub fn host_left(&mut self, code: &str, conn: ConnId, now: Instant) {
        if let Some(room) = self.rooms.get_mut(code) {
            if !room.host_is(conn) {
                return;
            }
            room.host = None;
            room.host_left_at = Some(now);
            room.to_all_players(&RelayToPhone::HostAway);
        }
    }

    /// A new player joins. Returns their id and connection id.
    pub fn join(
        &mut self,
        code: &str,
        name: &str,
        color: &str,
        outbox: Outbox,
    ) -> Result<(PlayerId, ConnId), RelayError> {
        let code = code.trim().to_ascii_uppercase();
        let conn = self.conn();
        let room = self.rooms.get_mut(&code).ok_or(RelayError::NoSuchRoom)?;
        if room.locked {
            return Err(RelayError::RoomLocked);
        }
        if room.players.len() >= MAX_PLAYERS {
            return Err(RelayError::RoomFull);
        }
        let name = clean_name(name).ok_or(RelayError::BadName)?;
        let player = room.next_player;
        room.next_player += 1;
        let player_token = new_token();
        room.players.insert(
            player,
            Player {
                token: player_token.clone(),
                name: name.clone(),
                outbox: Some((conn, outbox)),
            },
        );
        room.to_player(
            player,
            &RelayToPhone::Joined {
                player,
                player_token,
            },
        );
        if room.host.is_none() {
            room.to_player(player, &RelayToPhone::HostAway);
        }
        room.to_host(&RelayToHost::PlayerJoined {
            player,
            name,
            color: clean_color(color),
        });
        self.dirty = true;
        Ok((player, conn))
    }

    /// A player comes back with their token.
    pub fn rejoin(
        &mut self,
        code: &str,
        token: &str,
        outbox: Outbox,
    ) -> Result<(PlayerId, ConnId), RelayError> {
        let code = code.trim().to_ascii_uppercase();
        let conn = self.conn();
        let room = self.rooms.get_mut(&code).ok_or(RelayError::NoSuchRoom)?;
        let (&player, slot) = room
            .players
            .iter_mut()
            .find(|(_, p)| constant_time_eq(&p.token, token))
            .ok_or(RelayError::BadToken)?;
        if let Some((_, old)) = slot.outbox.replace((conn, outbox)) {
            let _ = old.send(to_json(&RelayToPhone::Error {
                message: "This seat was opened somewhere else.".into(),
            }));
        }
        room.to_player(player, &RelayToPhone::Rejoined { player });
        if room.host.is_none() {
            room.to_player(player, &RelayToPhone::HostAway);
        }
        room.to_host(&RelayToHost::PlayerReturned { player });
        Ok((player, conn))
    }

    pub fn from_player(&mut self, code: &str, player: PlayerId, conn: ConnId, input: PhoneInput) {
        if let Some(room) = self.rooms.get(code) {
            let current = room
                .players
                .get(&player)
                .and_then(|p| p.outbox.as_ref())
                .is_some_and(|(c, _)| *c == conn);
            if current {
                room.to_host(&RelayToHost::FromPlayer { player, input });
            }
        }
    }

    pub fn player_left(&mut self, code: &str, player: PlayerId, conn: ConnId) {
        if let Some(room) = self.rooms.get_mut(code) {
            if let Some(slot) = room.players.get_mut(&player) {
                if slot.outbox.as_ref().is_some_and(|(c, _)| *c == conn) {
                    slot.outbox = None;
                    room.to_host(&RelayToHost::PlayerAway { player });
                }
            }
        }
    }

    /// Close rooms whose host has been gone longer than [`HOST_GRACE`].
    pub fn sweep(&mut self, now: Instant) {
        let before = self.rooms.len();
        self.rooms.retain(|_, room| {
            room.host_left_at
                .is_none_or(|left| now.duration_since(left) < HOST_GRACE)
        });
        if self.rooms.len() != before {
            self.dirty = true;
        }
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

fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
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

fn to_json<T: serde::Serialize>(msg: &T) -> String {
    serde_json::to_string(msg).expect("protocol messages always serialize")
}

#[cfg(test)]
mod tests;
