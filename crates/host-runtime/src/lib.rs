//! The host loop shared by the desktop host, the cloud host and the
//! end-to-end tests: it connects a [`Room`] to the relay, feeds it relay
//! messages and the clock, sends what it says, saves it after every change,
//! and reconnects (reclaiming the room with its host token) when the relay
//! connection drops.
//!
//! [`run`] is one game. The shell talks to it with [`Command`]s and hears
//! back [`Update`]s.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use hostable_host_core::{Effect, Pace, Room, Snapshot};
use hostable_protocol::{HostToRelay, PlayerId, RelayToHost};
use hostable_rules_api::{DynRules, Results};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

pub mod catalog;
pub mod quota;

pub use catalog::{Catalog, PackEntry};
pub use quota::{Metered, Quota};

pub type GameRoom = Room<Box<DynRules>>;

/// Wall-clock milliseconds since 1970. Game deadlines are absolute, so they
/// survive a host restart.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Everything needed to bring a game back: on this host after a restart, or
/// on another host (desktop to cloud).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub pack: String,
    pub code: String,
    pub host_token: String,
    /// A [`Snapshot`] as JSON.
    pub snapshot: Value,
}

impl Saved {
    pub fn restore(&self, rules: Box<DynRules>) -> Result<GameRoom, String> {
        let snap: Snapshot<Box<DynRules>> =
            serde_json::from_value(self.snapshot.clone()).map_err(|e| e.to_string())?;
        Ok(Room::restore(rules, snap))
    }
}

/// Where a game is saved after every change.
#[derive(Clone, Default)]
pub enum SaveTo {
    #[default]
    Nowhere,
    /// A JSON file, replaced atomically.
    File(PathBuf),
    /// Handed to the shell (the cloud host writes it to its database).
    Callback(Arc<dyn Fn(&Saved) + Send + Sync>),
}

#[derive(Clone)]
pub struct Options {
    /// Base URL of the relay, e.g. `ws://127.0.0.1:8787`.
    pub relay: String,
    pub pack: String,
    /// Save the game here after every change.
    pub save_to: SaveTo,
    /// Stop reconnecting after this many failed tries in a row (`None`:
    /// forever).
    pub max_reconnects: Option<u32>,
}

/// What the shell asks the game to do.
pub enum Command {
    Start,
    /// Make a seat for someone arriving with a link.
    AddPlayer(String),
    Kick(PlayerId),
    SetPace(Pace),
    /// Clear a pause after a rules error.
    Resume,
    /// Pause play with a message (an admin took the pack down).
    Suspend(String),
    /// Read the current save.
    Save(oneshot::Sender<Option<Saved>>),
    /// Look at the room.
    Inspect(Box<dyn FnOnce(&GameRoom) + Send>),
    /// Stop without closing the room on the relay, so another host can
    /// reclaim it. Answers with the final save.
    Detach(oneshot::Sender<Option<Saved>>),
    /// Close the room for good.
    Close,
}

/// What the game tells the shell.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Opened {
        code: String,
        host_token: String,
    },
    PlayerAdded {
        player: PlayerId,
        name: String,
        player_token: String,
    },
    /// Players or their connections changed.
    Players,
    /// Effects from the room (events, table changes, notifications...).
    Effects(Vec<Effect>),
    Finished(Results),
    /// Something went wrong that the shell may want to show.
    Error(String),
    /// The relay connection dropped; reconnecting.
    Reconnecting,
    Closed,
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(relay: &str) -> Result<Ws, String> {
    let url = format!("{}/ws/host", relay.trim_end_matches('/'));
    let (ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ws)
}

async fn send(ws: &mut Ws, msg: &HostToRelay) -> Result<(), String> {
    let text = serde_json::to_string(msg).map_err(|e| e.to_string())?;
    ws.send(Message::Text(text.into()))
        .await
        .map_err(|e| e.to_string())
}

async fn next_message(ws: &mut Ws) -> Option<RelayToHost> {
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(t))) => match serde_json::from_str(&t) {
                Ok(m) => return Some(m),
                Err(_) => continue,
            },
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            Some(Ok(_)) => continue,
        }
    }
}

/// Open (or reclaim) a room and run the game until it is closed or
/// detached. `room` may be a fresh room or one restored from a [`Saved`]
/// (pass its code and token in `reclaim`).
pub async fn run(
    mut room: GameRoom,
    reclaim: Option<(String, String)>,
    options: Options,
    mut commands: mpsc::UnboundedReceiver<Command>,
    updates: mpsc::UnboundedSender<Update>,
) -> Result<(), String> {
    let mut identity = reclaim;
    let mut failures = 0u32;
    loop {
        // Connect, then create or reclaim the room.
        let mut ws = match connect(&options.relay).await {
            Ok(ws) => ws,
            Err(e) => {
                failures += 1;
                if options.max_reconnects.is_some_and(|m| failures > m) {
                    let _ = updates.send(Update::Error(format!("relay unreachable: {e}")));
                    return Err(e);
                }
                let _ = updates.send(Update::Reconnecting);
                tokio::time::sleep(backoff(failures)).await;
                continue;
            }
        };
        let hello = match &identity {
            Some((code, host_token)) => HostToRelay::ReclaimRoom {
                code: code.clone(),
                host_token: host_token.clone(),
            },
            None => HostToRelay::CreateRoom,
        };
        if send(&mut ws, &hello).await.is_err() {
            failures += 1;
            tokio::time::sleep(backoff(failures)).await;
            continue;
        }
        match next_message(&mut ws).await {
            Some(RelayToHost::RoomCreated { code, host_token }) => {
                identity = Some((code.clone(), host_token.clone()));
                let _ = updates.send(Update::Opened { code, host_token });
            }
            Some(RelayToHost::RoomReclaimed { code, connected }) => {
                let token = identity.as_ref().map(|i| i.1.clone()).unwrap_or_default();
                let _ = updates.send(Update::Opened {
                    code,
                    host_token: token,
                });
                let now = now_ms();
                let mut effects = Vec::new();
                for p in connected {
                    if let Ok(e) = room.player_returned(p, now) {
                        effects.extend(e);
                    }
                }
                if deliver(&mut ws, effects, &updates).await.is_err() {
                    continue;
                }
                let _ = updates.send(Update::Players);
            }
            Some(RelayToHost::Error { message }) => {
                // The relay lost the room (restarted without its state):
                // open a fresh one. Phones will need the new code.
                let _ = updates.send(Update::Error(message));
                identity = None;
                failures += 1;
                if options.max_reconnects.is_some_and(|m| failures > m) {
                    return Err("relay refused the room".into());
                }
                continue;
            }
            _ => {
                failures += 1;
                tokio::time::sleep(backoff(failures)).await;
                continue;
            }
        }
        failures = 0;
        save(&room, &identity, &options);

        // The connected loop.
        let end = connected_loop(
            &mut room,
            &mut ws,
            &identity,
            &options,
            &mut commands,
            &updates,
        )
        .await;
        match end {
            LoopEnd::Dropped => {
                let _ = updates.send(Update::Reconnecting);
                tokio::time::sleep(backoff(1)).await;
            }
            LoopEnd::Closed => {
                let _ = send(&mut ws, &HostToRelay::CloseRoom).await;
                let _ = ws.close(None).await;
                if let SaveTo::File(path) = &options.save_to {
                    let _ = std::fs::remove_file(path);
                }
                let _ = updates.send(Update::Closed);
                return Ok(());
            }
            LoopEnd::Detached => {
                let _ = ws.close(None).await;
                let _ = updates.send(Update::Closed);
                return Ok(());
            }
            LoopEnd::ShellGone => {
                // Nobody is listening any more; leave the room for a restart.
                let _ = ws.close(None).await;
                return Ok(());
            }
        }
    }
}

enum LoopEnd {
    Dropped,
    Closed,
    Detached,
    ShellGone,
}

fn backoff(failures: u32) -> Duration {
    Duration::from_millis((250u64 << failures.min(5)).min(5_000))
}

/// The current save, if the room is open.
pub fn saved(room: &GameRoom, identity: &Option<(String, String)>, pack: &str) -> Option<Saved> {
    let (code, host_token) = identity.clone()?;
    Some(Saved {
        pack: pack.to_string(),
        code,
        host_token,
        snapshot: serde_json::to_value(room.snapshot()).ok()?,
    })
}

fn save(room: &GameRoom, identity: &Option<(String, String)>, options: &Options) {
    let path = match &options.save_to {
        SaveTo::Nowhere => return,
        SaveTo::File(path) => path,
        SaveTo::Callback(f) => {
            if let Some(s) = saved(room, identity, &options.pack) {
                f(&s);
            }
            return;
        }
    };
    let Some(s) = saved(room, identity, &options.pack) else {
        return;
    };
    let tmp = path.with_extension("tmp");
    let ok = serde_json::to_vec(&s)
        .ok()
        .and_then(|bytes| std::fs::write(&tmp, bytes).ok())
        .and_then(|()| std::fs::rename(&tmp, path).ok());
    if ok.is_none() {
        eprintln!("hostable: could not save the game to {}", path.display());
    }
}

/// Send the effects' relay messages; pass everything to the shell.
async fn deliver(
    ws: &mut Ws,
    effects: Vec<Effect>,
    updates: &mpsc::UnboundedSender<Update>,
) -> Result<(), String> {
    if effects.is_empty() {
        return Ok(());
    }
    let mut rest = Vec::new();
    for e in effects {
        match e {
            Effect::Send(msg) => send(ws, &msg).await?,
            Effect::Finished(r) => {
                let _ = updates.send(Update::Finished(r.clone()));
                rest.push(Effect::Finished(r));
            }
            other => rest.push(other),
        }
    }
    if !rest.is_empty() {
        let _ = updates.send(Update::Effects(rest));
    }
    Ok(())
}

async fn connected_loop(
    room: &mut GameRoom,
    ws: &mut Ws,
    identity: &Option<(String, String)>,
    options: &Options,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    updates: &mpsc::UnboundedSender<Update>,
) -> LoopEnd {
    loop {
        let sleep = match room.next_deadline() {
            Some(deadline) => Duration::from_millis(deadline.saturating_sub(now_ms()).min(60_000)),
            None => Duration::from_secs(60),
        };
        let effects: Vec<Effect> = tokio::select! {
            msg = next_message(ws) => {
                let Some(msg) = msg else { return LoopEnd::Dropped };
                let now = now_ms();
                match msg {
                    RelayToHost::PlayerJoined { player, name, color } => {
                        let e = room.player_joined(player, name, color, now);
                        let _ = updates.send(Update::Players);
                        e
                    }
                    RelayToHost::PlayerAdded { player, name, player_token } => {
                        let e = room.player_added(player, name.clone(), now);
                        let _ = updates.send(Update::PlayerAdded { player, name, player_token });
                        let _ = updates.send(Update::Players);
                        e
                    }
                    RelayToHost::PlayerReturned { player } => {
                        let e = room.player_returned(player, now).unwrap_or_default();
                        let _ = updates.send(Update::Players);
                        e
                    }
                    RelayToHost::PlayerAway { player } => {
                        room.player_away(player);
                        let _ = updates.send(Update::Players);
                        Vec::new()
                    }
                    RelayToHost::FromPlayer { player, input } => {
                        room.handle_input(player, input, now).unwrap_or_default()
                    }
                    RelayToHost::Error { message } => {
                        let _ = updates.send(Update::Error(message.clone()));
                        if message.contains("took over") {
                            // Another host has this room now.
                            return LoopEnd::Detached;
                        }
                        Vec::new()
                    }
                    RelayToHost::RoomCreated { .. } | RelayToHost::RoomReclaimed { .. } => Vec::new(),
                }
            }
            cmd = commands.recv() => {
                let Some(cmd) = cmd else { return LoopEnd::ShellGone };
                let now = now_ms();
                match cmd {
                    Command::Start => match room.start(now) {
                        Ok(e) => e,
                        Err(e) => {
                            let _ = updates.send(Update::Error(e.to_string()));
                            Vec::new()
                        }
                    },
                    Command::AddPlayer(name) => {
                        if send(ws, &HostToRelay::AddPlayer { name }).await.is_err() {
                            return LoopEnd::Dropped;
                        }
                        Vec::new()
                    }
                    Command::Kick(player) => {
                        if send(ws, &HostToRelay::Kick { player }).await.is_err() {
                            return LoopEnd::Dropped;
                        }
                        Vec::new()
                    }
                    Command::SetPace(pace) => {
                        room.set_pace(pace);
                        Vec::new()
                    }
                    Command::Resume => room.resume(),
                    Command::Suspend(reason) => room.suspend(reason),
                    Command::Save(reply) => {
                        let _ = reply.send(saved(room, identity, &options.pack));
                        Vec::new()
                    }
                    Command::Inspect(f) => {
                        f(room);
                        Vec::new()
                    }
                    Command::Detach(reply) => {
                        save(room, identity, options);
                        let _ = reply.send(saved(room, identity, &options.pack));
                        return LoopEnd::Detached;
                    }
                    Command::Close => return LoopEnd::Closed,
                }
            }
            _ = tokio::time::sleep(sleep) => room.tick(now_ms()),
        };
        let changed = !effects.is_empty();
        if deliver(ws, effects, updates).await.is_err() {
            save(room, identity, options);
            return LoopEnd::Dropped;
        }
        if changed {
            save(room, identity, options);
        }
    }
}

#[cfg(test)]
mod tests;

/// Move a running game to a cloud host: detach it here, then hand the save
/// to the cloud, which reclaims the same relay room with the same host
/// token, so every phone keeps its seat and token. Returns the cloud's game
/// id and share path.
pub async fn hand_off(
    commands: &mpsc::UnboundedSender<Command>,
    cloud: &str,
    pace: Pace,
) -> Result<(String, String), String> {
    let (tx, rx) = oneshot::channel();
    commands
        .send(Command::Detach(tx))
        .map_err(|_| "the game isn't running".to_string())?;
    let saved = rx
        .await
        .map_err(|_| "the game stopped".to_string())?
        .ok_or("the game has no room yet")?;
    let body = serde_json::json!({
        "pack": saved.pack,
        "code": saved.code,
        "host_token": saved.host_token,
        "snapshot": saved.snapshot,
        "pace": match pace { Pace::Live => "live", Pace::Async => "async" },
    });
    let url = format!("{}/api/handoff", cloud.trim_end_matches('/'));
    let res = reqwest::Client::new()
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("the cloud refused: {}", res.status()));
    }
    let v: Value = res.json().await.map_err(|e| e.to_string())?;
    Ok((
        v["id"].as_str().unwrap_or_default().to_string(),
        v["url"].as_str().unwrap_or_default().to_string(),
    ))
}
