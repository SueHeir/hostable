//! Tideholm: islanders of a tidal archipelago raise huts and lodges, link
//! them with causeways and trade what the tides bring in. First to 10 points
//! on their own turn wins.
//!
//! The rules are a pure state machine. All randomness comes from the seeded
//! [`Rng`] kept in the [`State`], so a game replays exactly from its seed and
//! action log.

use hostable_sdk::prelude::*;

pub mod board;
pub mod bot;
mod view;

#[cfg(test)]
mod tests;

use board::{topo, HEX_COUNT};

hostable_sdk::export!(Tideholm);

pub const RES: [&str; 5] = ["Reed", "Clay", "Salt", "Fish", "Copper"];
pub const RES_KEYS: [&str; 5] = ["reed", "clay", "salt", "fish", "copper"];

pub const COST_CAUSEWAY: [u8; 5] = [0, 1, 1, 0, 0];
pub const COST_HUT: [u8; 5] = [1, 1, 0, 1, 0];
pub const COST_LODGE: [u8; 5] = [0, 0, 2, 1, 2];
pub const COST_ALMANAC: [u8; 5] = [1, 0, 0, 1, 1];

pub const WIN_POINTS: u32 = 10;
pub const BANK_EACH: u8 = 19;
pub const MAX_HUTS: u8 = 5;
pub const MAX_LODGES: u8 = 4;
pub const MAX_CAUSEWAYS: u8 = 14;
/// Player-to-player offers a player may make per turn.
pub const MAX_PROPOSALS: u8 = 2;
pub const LONGEST_MIN: u8 = 5;
pub const FLEET_MIN: u8 = 3;
const LOG_KEEP: usize = 40;

const ASYNC_MS: u64 = 24 * 60 * 60 * 1000;
const TURN_MS: u64 = 120_000;
const SETUP_MS: u64 = 90_000;
const STEP_MS: u64 = 60_000;
const DISCARD_MS: u64 = 45_000;
const TRADE_MS: u64 = 30_000;

#[derive(Debug, Default, Clone, Copy)]
pub struct Tideholm;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Card {
    Ferryman,
    Surveyor,
    BountifulTide,
    MarketDay,
    OldLore,
}

impl Card {
    pub const ALL: [Card; 5] = [
        Card::Ferryman,
        Card::Surveyor,
        Card::BountifulTide,
        Card::MarketDay,
        Card::OldLore,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Card::Ferryman => "Ferryman",
            Card::Surveyor => "Surveyor",
            Card::BountifulTide => "Bountiful Tide",
            Card::MarketDay => "Market Day",
            Card::OldLore => "Old Lore",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Card::Ferryman => "ferryman",
            Card::Surveyor => "surveyor",
            Card::BountifulTide => "bountiful_tide",
            Card::MarketDay => "market_day",
            Card::OldLore => "old_lore",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Card::Ferryman => "Row the Squall to a new hex and take a random card from someone beside it. Counts toward Largest Fleet.",
            Card::Surveyor => "Lay up to 2 causeways for free.",
            Card::BountifulTide => "Take any 2 resources from the bank.",
            Card::MarketDay => "Name a resource. Every opponent hands you all of theirs.",
            Card::OldLore => "Worth 1 hidden point, revealed when you win.",
        }
    }

    fn from_key(k: &str) -> Option<Card> {
        Card::ALL.into_iter().find(|c| c.key() == k)
    }

    /// How many of each card the Almanac deck holds.
    pub fn count(self) -> usize {
        match self {
            Card::Ferryman => 12,
            Card::Surveyor => 3,
            Card::BountifulTide => 2,
            Card::MarketDay => 2,
            Card::OldLore => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    /// A resource index, or `None` for the Dunes.
    pub res: Option<usize>,
    /// The number token, 0 for the Dunes.
    pub number: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Building {
    pub seat: SeatId,
    pub lodge: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Player {
    pub res: [u8; 5],
    /// Almanac cards held (Old Lore stays here, hidden).
    pub cards: Vec<Card>,
    /// Bought this turn; can't be played until next turn.
    pub fresh: Vec<Card>,
    pub ferrymen: u8,
    pub causeway_len: u8,
    pub huts: u8,
    pub lodges: u8,
    pub causeways: u8,
    /// Private notes for this player's phone (e.g. what was taken).
    pub notes: Vec<String>,
}

impl Player {
    pub fn total(&self) -> u32 {
        self.res.iter().map(|&n| n as u32).sum()
    }

    fn has(&self, cost: &[u8; 5]) -> bool {
        self.res.iter().zip(cost).all(|(h, c)| h >= c)
    }

    fn pay(&mut self, cost: &[u8; 5], bank: &mut [u8; 5]) {
        for i in 0..5 {
            self.res[i] -= cost[i];
            bank[i] += cost[i];
        }
    }

    pub fn old_lore(&self) -> u32 {
        self.cards
            .iter()
            .chain(&self.fresh)
            .filter(|c| **c == Card::OldLore)
            .count() as u32
    }

    fn note(&mut self, text: String) {
        self.notes.push(text);
        if self.notes.len() > 3 {
            self.notes.remove(0);
        }
    }
}

/// A trade from the proposer's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub give: [u8; 5],
    pub get: [u8; 5],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resp {
    /// The proposer themselves.
    Proposer,
    Pending,
    Accept,
    Decline,
    Counter(Offer),
}

/// Where play returns to after a side step (the Squall, an Almanac card).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Back {
    PreRoll,
    Main,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Phase {
    /// Snake-order placement. `hut` is set while placing the causeway.
    Setup {
        round: u8,
        idx: u8,
        hut: Option<usize>,
    },
    PreRoll,
    Main,
    Discard {
        owed: Vec<u8>,
    },
    MoveSquall {
        back: Back,
    },
    Steal {
        from: Vec<SeatId>,
        back: Back,
    },
    Surveyor {
        left: u8,
        back: Back,
    },
    Bounty {
        back: Back,
    },
    Market {
        back: Back,
    },
    TradeRespond {
        offer: Offer,
        responses: Vec<Resp>,
    },
    TradeConfirm {
        offer: Offer,
        responses: Vec<Resp>,
    },
    Over {
        winner: SeatId,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub seats: Vec<Seat>,
    pub rng: Rng,
    pub tiles: Vec<Tile>,
    pub squall: usize,
    /// One slot per vertex.
    pub buildings: Vec<Option<Building>>,
    /// One slot per edge: the owner of the causeway there.
    pub causeways: Vec<Option<SeatId>>,
    pub players: Vec<Player>,
    pub bank: [u8; 5],
    pub deck: Vec<Card>,
    pub phase: Phase,
    pub current: SeatId,
    pub turn: u32,
    pub dice: Option<(u8, u8)>,
    pub longest: Option<SeatId>,
    pub fleet: Option<SeatId>,
    pub proposals: u8,
    pub played_card: bool,
    pub trades_completed: u32,
    pub log: Vec<String>,
}

impl State {
    pub fn name(&self, seat: SeatId) -> &str {
        self.seats
            .get(seat as usize)
            .map_or("?", |s| s.name.as_str())
    }

    fn n(&self) -> usize {
        self.players.len()
    }

    fn log(&mut self, line: String) {
        self.log.push(line);
        if self.log.len() > LOG_KEEP {
            let extra = self.log.len() - LOG_KEEP;
            self.log.drain(..extra);
        }
    }

    /// Points everyone can see.
    pub fn visible_points(&self, seat: SeatId) -> u32 {
        let Some(p) = self.players.get(seat as usize) else {
            return 0;
        };
        let mut pts = p.huts as u32 + 2 * p.lodges as u32;
        if self.longest == Some(seat) {
            pts += 2;
        }
        if self.fleet == Some(seat) {
            pts += 2;
        }
        pts
    }

    /// Points including hidden Old Lore.
    pub fn total_points(&self, seat: SeatId) -> u32 {
        self.visible_points(seat) + self.players.get(seat as usize).map_or(0, Player::old_lore)
    }

    /// The seat placing during setup.
    pub fn setup_seat(&self) -> Option<SeatId> {
        match self.phase {
            Phase::Setup { round, idx, .. } => {
                let n = self.n() as u8;
                Some(if round == 0 { idx } else { n - 1 - idx } as SeatId)
            }
            _ => None,
        }
    }

    fn other_building(&self, v: usize, seat: SeatId) -> bool {
        self.buildings[v].is_some_and(|b| b.seat != seat)
    }

    /// Distance rule: the vertex and all its neighbours are empty.
    pub fn hut_spot_free(&self, v: usize) -> bool {
        v < self.buildings.len()
            && self.buildings[v].is_none()
            && topo().vert_neighbors[v]
                .iter()
                .all(|&u| self.buildings[u].is_none())
    }

    /// A hut placed during play must touch one of the seat's causeways.
    pub fn hut_legal(&self, seat: SeatId, v: usize) -> bool {
        self.hut_spot_free(v)
            && topo().vert_edges[v]
                .iter()
                .any(|&e| self.causeways[e] == Some(seat))
    }

    pub fn hut_spots(&self, seat: SeatId) -> Vec<usize> {
        (0..self.buildings.len())
            .filter(|&v| self.hut_legal(seat, v))
            .collect()
    }

    /// A causeway must be empty and join the seat's building, or the seat's
    /// causeway through a vertex no opponent has built on.
    pub fn causeway_legal(&self, seat: SeatId, e: usize) -> bool {
        let t = topo();
        if e >= self.causeways.len() || self.causeways[e].is_some() {
            return false;
        }
        let (a, b) = t.edges[e];
        [a, b].into_iter().any(|v| {
            if self.buildings[v].is_some_and(|b| b.seat == seat) {
                return true;
            }
            !self.other_building(v, seat)
                && t.vert_edges[v]
                    .iter()
                    .any(|&f| f != e && self.causeways[f] == Some(seat))
        })
    }

    pub fn causeway_spots(&self, seat: SeatId) -> Vec<usize> {
        (0..self.causeways.len())
            .filter(|&e| self.causeway_legal(seat, e))
            .collect()
    }

    /// Seats (not `by`) with a building beside `hex` and at least one card.
    pub fn robbable(&self, hex: usize, by: SeatId) -> Vec<SeatId> {
        let mut out: Vec<SeatId> = topo().hex_verts[hex]
            .iter()
            .filter_map(|&v| self.buildings[v].map(|b| b.seat))
            .filter(|&s| s != by && self.players[s as usize].total() > 0)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Longest connected trail of the seat's causeways, not passing through
    /// an opponent's building.
    pub fn longest_trail(&self, seat: SeatId) -> u8 {
        let t = topo();
        let mine: Vec<usize> = (0..self.causeways.len())
            .filter(|&e| self.causeways[e] == Some(seat))
            .collect();
        if mine.is_empty() {
            return 0;
        }
        let mut best = 0u8;
        let mut starts: Vec<usize> = mine
            .iter()
            .flat_map(|&e| [t.edges[e].0, t.edges[e].1])
            .collect();
        starts.sort_unstable();
        starts.dedup();
        for v in starts {
            self.trail_dfs(seat, &mine, v, 0, 0, &mut best);
        }
        best
    }

    fn trail_dfs(&self, seat: SeatId, mine: &[usize], v: usize, used: u64, len: u8, best: &mut u8) {
        *best = (*best).max(len);
        if len > 0 && self.other_building(v, seat) {
            return;
        }
        let t = topo();
        for (i, &e) in mine.iter().enumerate().take(64) {
            if used & (1 << i) != 0 {
                continue;
            }
            let (a, b) = t.edges[e];
            let next = if a == v {
                b
            } else if b == v {
                a
            } else {
                continue;
            };
            self.trail_dfs(seat, mine, next, used | (1 << i), len + 1, best);
        }
    }

    fn update_longest(&mut self) {
        for s in 0..self.n() {
            self.players[s].causeway_len = self.longest_trail(s as SeatId);
        }
        let lens: Vec<u8> = self.players.iter().map(|p| p.causeway_len).collect();
        let max = lens.iter().copied().max().unwrap_or(0);
        let before = self.longest;
        if max < LONGEST_MIN {
            self.longest = None;
        } else if self.longest.is_some_and(|h| lens[h as usize] == max) {
            // The holder keeps it while still tied for the longest.
        } else {
            let top: Vec<usize> = (0..lens.len()).filter(|&i| lens[i] == max).collect();
            self.longest = (top.len() == 1).then(|| top[0] as SeatId);
        }
        if self.longest != before {
            match self.longest {
                Some(h) => {
                    let line = format!("{} now has the Longest Causeway (+2)", self.name(h));
                    self.log(line)
                }
                None => self.log("Nobody holds the Longest Causeway now".into()),
            }
        }
    }

    fn update_fleet(&mut self, seat: SeatId) {
        let f = self.players[seat as usize].ferrymen;
        let beats = match self.fleet {
            None => true,
            Some(h) => h != seat && f > self.players[h as usize].ferrymen,
        };
        if f >= FLEET_MIN && beats && self.fleet != Some(seat) {
            self.fleet = Some(seat);
            let line = format!("{} now has the Largest Fleet (+2)", self.name(seat));
            self.log(line);
        }
    }

    fn produce(&mut self, roll: u8) {
        let t = topo();
        let mut gains = vec![[0u8; 5]; self.n()];
        for h in 0..HEX_COUNT {
            let tile = &self.tiles[h];
            let Some(r) = tile.res else { continue };
            if tile.number != roll || self.squall == h {
                continue;
            }
            for &v in &t.hex_verts[h] {
                if let Some(b) = self.buildings[v] {
                    gains[b.seat as usize][r] += if b.lodge { 2 } else { 1 };
                }
            }
        }
        for r in 0..5 {
            let want: u32 = gains.iter().map(|g| g[r] as u32).sum();
            if want == 0 {
                continue;
            }
            if want > self.bank[r] as u32 {
                let line = format!("The bank is short of {}: nobody gets any", RES[r]);
                self.log(line);
                continue;
            }
            for (s, g) in gains.iter().enumerate() {
                self.players[s].res[r] += g[r];
            }
            self.bank[r] -= want as u8;
        }
        let parts: Vec<String> = gains
            .iter()
            .enumerate()
            .filter(|(_, g)| g.iter().any(|&x| x > 0))
            .map(|(s, g)| format!("{} +{}", self.name(s as SeatId), g.iter().sum::<u8>()))
            .collect();
        if !parts.is_empty() {
            let line = format!("Harvest: {}", parts.join(", "));
            self.log(line);
        }
    }

    fn steal(&mut self, by: SeatId, from: SeatId) {
        let total = self.players[from as usize].total();
        if total == 0 || by == from {
            return;
        }
        let mut pick = self.rng.below(total);
        let mut r = 0;
        for i in 0..5 {
            let have = self.players[from as usize].res[i] as u32;
            if pick < have {
                r = i;
                break;
            }
            pick -= have;
        }
        self.players[from as usize].res[r] -= 1;
        self.players[by as usize].res[r] += 1;
        let by_name = self.name(by).to_string();
        let from_name = self.name(from).to_string();
        self.players[from as usize].note(format!("{by_name} took your {}", RES[r]));
        self.players[by as usize].note(format!("You took {} from {from_name}", RES[r]));
        self.log(format!("{by_name} took a card from {from_name}"));
    }

    fn go_back(&mut self, back: Back) {
        self.phase = match back {
            Back::PreRoll => Phase::PreRoll,
            Back::Main => Phase::Main,
        };
    }

    fn next_turn(&mut self) {
        let cur = self.current as usize;
        let fresh = std::mem::take(&mut self.players[cur].fresh);
        self.players[cur].cards.extend(fresh);
        self.played_card = false;
        self.proposals = 0;
        self.dice = None;
        self.current = ((cur + 1) % self.n()) as SeatId;
        self.turn += 1;
        self.phase = Phase::PreRoll;
    }

    /// Whether this seat owes something in the current window.
    pub fn owes_action(&self, seat: SeatId) -> bool {
        let s = seat as usize;
        if s >= self.n() {
            return false;
        }
        match &self.phase {
            Phase::Setup { .. } => self.setup_seat() == Some(seat),
            Phase::Discard { owed } => owed.get(s).is_some_and(|&n| n > 0),
            Phase::TradeRespond { responses, .. } => {
                responses.get(s) == Some(&Resp::Pending) && self.players[s].total() > 0
            }
            Phase::Over { .. } => false,
            _ => self.current == seat,
        }
    }
}

// ---------------------------------------------------------------------------
// Setup

fn new_game(seats: &[Seat], seed: u64) -> State {
    let mut rng = Rng::new(seed);
    let mut kinds: Vec<Option<usize>> = Vec::new();
    for (r, n) in [4, 3, 4, 4, 3].into_iter().enumerate() {
        kinds.extend(std::iter::repeat_n(Some(r), n));
    }
    kinds.push(None);
    rng.shuffle(&mut kinds);
    let base: [u8; 18] = [2, 3, 3, 4, 4, 5, 5, 6, 6, 8, 8, 9, 9, 10, 10, 11, 11, 12];
    let t = topo();
    let mut numbers = base.to_vec();
    // Keep the 6s and 8s apart: reshuffle a bounded number of times.
    for _ in 0..200 {
        rng.shuffle(&mut numbers);
        let mut it = numbers.iter();
        let placed: Vec<u8> = kinds
            .iter()
            .map(|k| {
                if k.is_some() {
                    *it.next().unwrap_or(&0)
                } else {
                    0
                }
            })
            .collect();
        let red = |n: u8| n == 6 || n == 8;
        let clash = (0..HEX_COUNT)
            .any(|h| red(placed[h]) && t.hex_neighbors[h].iter().any(|&o| red(placed[o])));
        if !clash {
            break;
        }
    }
    let mut it = numbers.into_iter();
    let tiles: Vec<Tile> = kinds
        .iter()
        .map(|&res| Tile {
            res,
            number: if res.is_some() {
                it.next().unwrap_or(0)
            } else {
                0
            },
        })
        .collect();
    let squall = tiles.iter().position(|t| t.res.is_none()).unwrap_or(0);
    let mut deck: Vec<Card> = Card::ALL
        .into_iter()
        .flat_map(|c| std::iter::repeat_n(c, c.count()))
        .collect();
    rng.shuffle(&mut deck);
    let seats: Vec<Seat> = seats.to_vec();
    let mut state = State {
        players: vec![Player::default(); seats.len()],
        seats,
        rng,
        tiles,
        squall,
        buildings: vec![None; t.verts.len()],
        causeways: vec![None; t.edges.len()],
        bank: [BANK_EACH; 5],
        deck,
        phase: Phase::Setup {
            round: 0,
            idx: 0,
            hut: None,
        },
        current: 0,
        turn: 0,
        dice: None,
        longest: None,
        fleet: None,
        proposals: 0,
        played_card: false,
        trades_completed: 0,
        log: Vec::new(),
    };
    state.log("The tide goes out on Tideholm. Place your huts.".into());
    state
}

fn setup_place_hut(s: &mut State, seat: SeatId, v: usize) -> Result<(), &'static str> {
    if !s.hut_spot_free(v) {
        return Err("That spot is taken or too close to another building");
    }
    let Phase::Setup { round, idx, .. } = s.phase else {
        return Err("Not placing now");
    };
    s.buildings[v] = Some(Building { seat, lodge: false });
    s.players[seat as usize].huts += 1;
    if round == 1 {
        let t = topo();
        let mut got = Vec::new();
        for &h in &t.vert_hexes[v] {
            if let Some(r) = s.tiles[h].res {
                if s.bank[r] > 0 {
                    s.bank[r] -= 1;
                    s.players[seat as usize].res[r] += 1;
                    got.push(RES[r]);
                }
            }
        }
        if !got.is_empty() {
            let line = format!("{} starts with {}", s.name(seat), got.join(", "));
            s.log(line);
        }
    }
    let line = format!("{} raised a hut at {}", s.name(seat), topo().vert_labels[v]);
    s.log(line);
    s.phase = Phase::Setup {
        round,
        idx,
        hut: Some(v),
    };
    Ok(())
}

fn setup_place_causeway(s: &mut State, seat: SeatId, e: usize) -> Result<(), &'static str> {
    let Phase::Setup {
        round,
        idx,
        hut: Some(v),
    } = s.phase
    else {
        return Err("Not placing a causeway now");
    };
    let t = topo();
    if e >= t.edges.len() || s.causeways[e].is_some() || !t.vert_edges[v].contains(&e) {
        return Err("The causeway must touch the hut you just raised");
    }
    s.causeways[e] = Some(seat);
    s.players[seat as usize].causeways += 1;
    let line = format!("{} laid a causeway at {}", s.name(seat), t.edge_labels[e]);
    s.log(line);
    let n = s.n() as u8;
    s.phase = if idx + 1 < n {
        Phase::Setup {
            round,
            idx: idx + 1,
            hut: None,
        }
    } else if round == 0 {
        Phase::Setup {
            round: 1,
            idx: 0,
            hut: None,
        }
    } else {
        s.current = 0;
        s.turn = 1;
        s.log("Setup done. The tide comes in!".into());
        Phase::PreRoll
    };
    s.update_longest();
    Ok(())
}

fn setup_causeway_spots(s: &State) -> Vec<usize> {
    match s.phase {
        Phase::Setup { hut: Some(v), .. } => topo().vert_edges[v]
            .iter()
            .copied()
            .filter(|&e| s.causeways[e].is_none())
            .collect(),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Turn actions

fn roll(s: &mut State) {
    let a = s.rng.die(6) as u8;
    let b = s.rng.die(6) as u8;
    s.dice = Some((a, b));
    let sum = a + b;
    let line = format!("{} rolled {} ({a}+{b})", s.name(s.current), sum);
    s.log(line);
    if sum == 7 {
        s.log("A Squall blows in!".into());
        let owed: Vec<u8> = s
            .players
            .iter()
            .map(|p| {
                let t = p.total();
                if t > 7 {
                    (t / 2) as u8
                } else {
                    0
                }
            })
            .collect();
        s.phase = if owed.iter().any(|&n| n > 0) {
            Phase::Discard { owed }
        } else {
            Phase::MoveSquall { back: Back::Main }
        };
    } else {
        s.produce(sum);
        s.phase = Phase::Main;
    }
}

fn res_index(key: &str) -> Option<usize> {
    RES_KEYS.iter().position(|k| *k == key)
}

/// Parses picks like `reed:0` / `give:reed:2` into per-resource counts.
fn parse_units(values: &[String], prefix: Option<&str>) -> Option<[u8; 5]> {
    let mut out = [0u8; 5];
    let mut seen: Vec<&str> = Vec::new();
    for v in values {
        if seen.contains(&v.as_str()) {
            return None;
        }
        seen.push(v);
        let mut parts = v.split(':');
        if let Some(p) = prefix {
            if parts.next() != Some(p) {
                continue;
            }
        }
        let r = res_index(parts.next()?)?;
        parts.next()?.parse::<u8>().ok()?;
        out[r] = out[r].checked_add(1)?;
    }
    Some(out)
}

fn list_of(input: &PhoneInput) -> Option<Vec<String>> {
    input
        .value
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect()
}

fn str_of(input: &PhoneInput) -> Option<&str> {
    input.value.as_str()
}

fn index_of(input: &PhoneInput) -> Option<usize> {
    str_of(input)?.parse().ok()
}

fn apply_discard(s: &mut State, seat: SeatId, counts: [u8; 5]) -> Result<(), &'static str> {
    let Phase::Discard { owed } = &mut s.phase else {
        return Err("No discard now");
    };
    let need = owed.get(seat as usize).copied().unwrap_or(0);
    if need == 0 {
        return Err("You don't need to discard");
    }
    let p = &s.players[seat as usize];
    if counts.iter().map(|&c| c as u32).sum::<u32>() != need as u32 || !p.has(&counts) {
        return Err("Pick exactly the right number of your cards");
    }
    owed[seat as usize] = 0;
    let done = owed.iter().all(|&n| n == 0);
    let p = &mut s.players[seat as usize];
    p.pay(&counts, &mut s.bank);
    let line = format!("{} discarded {} cards to the Squall", s.name(seat), need);
    s.log(line);
    if done {
        s.phase = Phase::MoveSquall { back: Back::Main };
    }
    Ok(())
}

fn random_discard(s: &mut State, seat: SeatId) {
    let need = match &s.phase {
        Phase::Discard { owed } => owed.get(seat as usize).copied().unwrap_or(0),
        _ => 0,
    };
    let mut res = s.players[seat as usize].res;
    let mut counts = [0u8; 5];
    for _ in 0..need {
        let total: u32 = res.iter().map(|&n| n as u32).sum();
        if total == 0 {
            break;
        }
        let mut pick = s.rng.below(total);
        for r in 0..5 {
            if pick < res[r] as u32 {
                res[r] -= 1;
                counts[r] += 1;
                break;
            }
            pick -= res[r] as u32;
        }
    }
    let _ = apply_discard(s, seat, counts);
}

fn move_squall(s: &mut State, hex: usize, back: Back) -> Result<(), &'static str> {
    if hex >= HEX_COUNT || hex == s.squall {
        return Err("The Squall must move to a different hex");
    }
    s.squall = hex;
    let line = format!(
        "{} sent the Squall to {}",
        s.name(s.current),
        board::LETTERS[hex]
    );
    s.log(line);
    let from = s.robbable(hex, s.current);
    match from.len() {
        0 => s.go_back(back),
        1 => {
            s.steal(s.current, from[0]);
            s.go_back(back);
        }
        _ => s.phase = Phase::Steal { from, back },
    }
    Ok(())
}

fn play_card(s: &mut State, seat: SeatId, key: &str, back: Back) -> Result<(), &'static str> {
    if s.played_card {
        return Err("You already played an Almanac card this turn");
    }
    let card = Card::from_key(key).ok_or("Unknown card")?;
    if card == Card::OldLore {
        return Err("Old Lore stays hidden until you win");
    }
    let p = &mut s.players[seat as usize];
    let i = p
        .cards
        .iter()
        .position(|&c| c == card)
        .ok_or("You can't play that card yet")?;
    p.cards.remove(i);
    s.played_card = true;
    let line = format!("{} played {}", s.name(seat), card.name());
    s.log(line);
    match card {
        Card::Ferryman => {
            s.players[seat as usize].ferrymen += 1;
            s.update_fleet(seat);
            s.phase = Phase::MoveSquall { back };
        }
        Card::Surveyor => {
            let left = 2.min(MAX_CAUSEWAYS - s.players[seat as usize].causeways.min(MAX_CAUSEWAYS));
            if left == 0 || s.causeway_spots(seat).is_empty() {
                s.log("...but there was nowhere to lay a causeway".into());
            } else {
                s.phase = Phase::Surveyor { left, back };
            }
        }
        Card::BountifulTide => {
            if s.bank.iter().map(|&b| b as u32).sum::<u32>() < 2 {
                s.log("...but the bank is empty".into());
            } else {
                s.phase = Phase::Bounty { back };
            }
        }
        Card::MarketDay => s.phase = Phase::Market { back },
        Card::OldLore => {}
    }
    Ok(())
}

fn place_causeway(s: &mut State, seat: SeatId, e: usize) {
    s.causeways[e] = Some(seat);
    s.players[seat as usize].causeways += 1;
    let line = format!(
        "{} laid a causeway at {}",
        s.name(seat),
        topo().edge_labels[e]
    );
    s.log(line);
    s.update_longest();
}

fn main_action(s: &mut State, seat: SeatId, input: &PhoneInput) -> Result<(), &'static str> {
    let w = input.widget.as_str();
    let si = seat as usize;
    match w {
        "end_turn" => {
            s.next_turn();
            Ok(())
        }
        "build_causeway" => {
            let e = index_of(input).ok_or("Pick a causeway spot")?;
            if !s.causeway_legal(seat, e) {
                return Err("A causeway can't go there");
            }
            if s.players[si].causeways >= MAX_CAUSEWAYS {
                return Err("You have no causeways left");
            }
            if !s.players[si].has(&COST_CAUSEWAY) {
                return Err("You can't afford a causeway");
            }
            s.players[si].pay(&COST_CAUSEWAY, &mut s.bank);
            place_causeway(s, seat, e);
            Ok(())
        }
        "build_hut" => {
            let v = index_of(input).ok_or("Pick a hut spot")?;
            if !s.hut_legal(seat, v) {
                return Err("A hut can't go there");
            }
            if s.players[si].huts >= MAX_HUTS {
                return Err("You have no huts left");
            }
            if !s.players[si].has(&COST_HUT) {
                return Err("You can't afford a hut");
            }
            s.players[si].pay(&COST_HUT, &mut s.bank);
            s.buildings[v] = Some(Building { seat, lodge: false });
            s.players[si].huts += 1;
            let line = format!("{} raised a hut at {}", s.name(seat), topo().vert_labels[v]);
            s.log(line);
            s.update_longest();
            Ok(())
        }
        "build_lodge" => {
            let v = index_of(input).ok_or("Pick a hut to upgrade")?;
            if s.buildings.get(v).copied().flatten() != Some(Building { seat, lodge: false }) {
                return Err("Pick one of your huts");
            }
            if s.players[si].lodges >= MAX_LODGES {
                return Err("You have no lodges left");
            }
            if !s.players[si].has(&COST_LODGE) {
                return Err("You can't afford a lodge");
            }
            s.players[si].pay(&COST_LODGE, &mut s.bank);
            s.buildings[v] = Some(Building { seat, lodge: true });
            s.players[si].huts -= 1;
            s.players[si].lodges += 1;
            let line = format!(
                "{} built a lodge at {}",
                s.name(seat),
                topo().vert_labels[v]
            );
            s.log(line);
            Ok(())
        }
        "buy_almanac" => {
            if s.deck.is_empty() {
                return Err("The Almanac is used up");
            }
            if !s.players[si].has(&COST_ALMANAC) {
                return Err("You can't afford an Almanac card");
            }
            s.players[si].pay(&COST_ALMANAC, &mut s.bank);
            let card = s.deck.pop().ok_or("The Almanac is used up")?;
            s.players[si].fresh.push(card);
            s.players[si].note(format!("You drew {}", card.name()));
            let line = format!("{} bought an Almanac card", s.name(seat));
            s.log(line);
            Ok(())
        }
        "bank_trade" => {
            let v = str_of(input).ok_or("Pick a trade")?;
            let (a, b) = v.split_once('>').ok_or("Pick a trade")?;
            let a = res_index(a).ok_or("Pick a trade")?;
            let b = res_index(b).ok_or("Pick a trade")?;
            if a == b || s.players[si].res[a] < 4 || s.bank[b] == 0 {
                return Err("You can't make that trade");
            }
            s.players[si].res[a] -= 4;
            s.bank[a] += 4;
            s.bank[b] -= 1;
            s.players[si].res[b] += 1;
            let line = format!(
                "{} traded 4 {} to the bank for 1 {}",
                s.name(seat),
                RES[a],
                RES[b]
            );
            s.log(line);
            Ok(())
        }
        "offer" => {
            if s.proposals >= MAX_PROPOSALS {
                return Err("No more offers this turn");
            }
            let picks = list_of(input).ok_or("Pick what to give and want")?;
            let give = parse_units(&picks, Some("give")).ok_or("Bad offer")?;
            let get = parse_units(&picks, Some("want")).ok_or("Bad offer")?;
            let offer = Offer { give, get };
            check_offer(&offer)?;
            if !s.players[si].has(&give) {
                return Err("You don't have those cards");
            }
            s.proposals += 1;
            let responses: Vec<Resp> = (0..s.n())
                .map(|i| {
                    if i == si {
                        Resp::Proposer
                    } else if s.players[i].total() == 0 {
                        Resp::Decline
                    } else {
                        Resp::Pending
                    }
                })
                .collect();
            let line = format!("{} offers {}", s.name(seat), offer_text(&offer));
            s.log(line);
            s.phase = Phase::TradeRespond { offer, responses };
            settle_trade(s);
            Ok(())
        }
        "play_card" => {
            let key = str_of(input).ok_or("Pick a card")?.to_string();
            play_card(s, seat, &key, Back::Main)
        }
        _ => Err("You can't do that now"),
    }
}

fn check_offer(o: &Offer) -> Result<(), &'static str> {
    if o.give.iter().all(|&n| n == 0) || o.get.iter().all(|&n| n == 0) {
        return Err("An offer needs something to give and something to get");
    }
    if (0..5).any(|r| o.give[r] > 0 && o.get[r] > 0) {
        return Err("Don't give and want the same resource");
    }
    Ok(())
}

pub fn res_list(counts: &[u8; 5]) -> String {
    let parts: Vec<String> = (0..5)
        .filter(|&r| counts[r] > 0)
        .map(|r| format!("{} {}", counts[r], RES[r]))
        .collect();
    if parts.is_empty() {
        "nothing".into()
    } else {
        parts.join(" + ")
    }
}

pub fn offer_text(o: &Offer) -> String {
    format!("{} for {}", res_list(&o.give), res_list(&o.get))
}

/// When nobody is still deciding, move to the proposer's pick (or back).
fn settle_trade(s: &mut State) {
    let Phase::TradeRespond { offer, responses } = &s.phase else {
        return;
    };
    if responses.contains(&Resp::Pending) {
        return;
    }
    let any = responses
        .iter()
        .any(|r| matches!(r, Resp::Accept | Resp::Counter(_)));
    if any {
        s.phase = Phase::TradeConfirm {
            offer: *offer,
            responses: responses.clone(),
        };
    } else {
        s.log("Nobody took the offer".into());
        s.phase = Phase::Main;
    }
}

fn respond(s: &mut State, seat: SeatId, input: &PhoneInput) -> Result<(), &'static str> {
    let si = seat as usize;
    let Phase::TradeRespond { offer, responses } = &s.phase else {
        return Err("No offer now");
    };
    let offer = *offer;
    if seat == s.current {
        if input.widget == "trade_cancel" {
            s.log(format!("{} withdrew the offer", s.name(seat)));
            s.phase = Phase::Main;
            return Ok(());
        }
        return Err("Waiting for answers");
    }
    if responses.get(si) != Some(&Resp::Pending) {
        return Err("You already answered");
    }
    let resp = match input.widget.as_str() {
        "trade_accept" => {
            if !s.players[si].has(&offer.get) {
                return Err("You don't have what they want");
            }
            Resp::Accept
        }
        "trade_decline" => Resp::Decline,
        "counter" => {
            let picks = list_of(input).ok_or("Pick what to give and want")?;
            // From the responder's side: they give what the proposer gets.
            let get = parse_units(&picks, Some("give")).ok_or("Bad counter")?;
            let give = parse_units(&picks, Some("want")).ok_or("Bad counter")?;
            let counter = Offer { give, get };
            check_offer(&counter)?;
            if !s.players[si].has(&counter.get) {
                return Err("You don't have those cards");
            }
            if counter == offer {
                Resp::Accept
            } else {
                Resp::Counter(counter)
            }
        }
        _ => return Err("Answer the offer"),
    };
    let line = match resp {
        Resp::Accept => format!("{} accepts", s.name(seat)),
        Resp::Counter(o) => format!("{} counters: {}", s.name(seat), offer_text(&o)),
        _ => format!("{} declines", s.name(seat)),
    };
    s.log(line);
    if let Phase::TradeRespond { responses, .. } = &mut s.phase {
        responses[si] = resp;
    }
    settle_trade(s);
    Ok(())
}

/// The trade the proposer would make with `partner`, if it is still possible.
pub fn trade_with(s: &State, partner: usize) -> Option<Offer> {
    let (offer, responses) = match &s.phase {
        Phase::TradeConfirm { offer, responses } => (offer, responses),
        _ => return None,
    };
    let deal = match responses.get(partner)? {
        Resp::Accept => *offer,
        Resp::Counter(o) => *o,
        _ => return None,
    };
    let me = s.players.get(s.current as usize)?;
    let them = s.players.get(partner)?;
    (me.has(&deal.give) && them.has(&deal.get)).then_some(deal)
}

fn confirm(s: &mut State, seat: SeatId, input: &PhoneInput) -> Result<(), &'static str> {
    if seat != s.current {
        return Err("Not your trade");
    }
    match input.widget.as_str() {
        "trade_cancel" => {
            s.log(format!("{} called off the trade", s.name(seat)));
            s.phase = Phase::Main;
            Ok(())
        }
        "trade_pick" => {
            let partner = index_of(input).ok_or("Pick a trade")?;
            let deal = trade_with(s, partner).ok_or("That trade isn't possible")?;
            let me = seat as usize;
            for r in 0..5 {
                s.players[me].res[r] = s.players[me].res[r] - deal.give[r] + deal.get[r];
                s.players[partner].res[r] = s.players[partner].res[r] + deal.give[r] - deal.get[r];
            }
            s.trades_completed += 1;
            let line = format!(
                "{} traded with {}: {}",
                s.name(seat),
                s.name(partner as SeatId),
                offer_text(&deal)
            );
            s.log(line);
            s.phase = Phase::Main;
            Ok(())
        }
        _ => Err("Pick a trade or cancel"),
    }
}

fn step(s: &mut State, seat: SeatId, input: &PhoneInput) -> Result<(), &'static str> {
    let si = seat as usize;
    if si >= s.n() {
        return Err("No such seat");
    }
    let w = input.widget.as_str();
    match s.phase.clone() {
        Phase::Over { .. } => Err("The game is over"),
        Phase::Setup { hut, .. } => {
            if s.setup_seat() != Some(seat) {
                return Err("Not your turn to place");
            }
            match (hut, w) {
                (None, "place_hut") => {
                    setup_place_hut(s, seat, index_of(input).ok_or("Pick a spot")?)
                }
                (Some(_), "place_causeway") => {
                    setup_place_causeway(s, seat, index_of(input).ok_or("Pick a spot")?)
                }
                _ => Err("Place your piece"),
            }
        }
        Phase::PreRoll => {
            if seat != s.current {
                return Err("Not your turn");
            }
            match w {
                "roll" => {
                    roll(s);
                    Ok(())
                }
                "play_card" => {
                    let key = str_of(input).ok_or("Pick a card")?.to_string();
                    play_card(s, seat, &key, Back::PreRoll)
                }
                _ => Err("Roll the tide dice first"),
            }
        }
        Phase::Main => {
            if seat != s.current {
                return Err("Not your turn");
            }
            main_action(s, seat, input)
        }
        Phase::Discard { .. } => {
            if w != "discard" {
                return Err("Discard first");
            }
            let picks = list_of(input).ok_or("Pick cards to discard")?;
            let counts = parse_units(&picks, None).ok_or("Pick your cards")?;
            apply_discard(s, seat, counts)
        }
        Phase::MoveSquall { back } => {
            if seat != s.current || w != "squall_hex" {
                return Err("Move the Squall");
            }
            move_squall(s, index_of(input).ok_or("Pick a hex")?, back)
        }
        Phase::Steal { from, back } => {
            if seat != s.current || w != "steal_from" {
                return Err("Pick who to rob");
            }
            let victim = index_of(input).ok_or("Pick a player")? as SeatId;
            if !from.contains(&victim) {
                return Err("You can't rob them");
            }
            s.steal(seat, victim);
            s.go_back(back);
            Ok(())
        }
        Phase::Surveyor { left, back } => {
            if seat != s.current {
                return Err("Not your turn");
            }
            match w {
                "surveyor_done" => {
                    s.go_back(back);
                    Ok(())
                }
                "surveyor_edge" => {
                    let e = index_of(input).ok_or("Pick a spot")?;
                    if !s.causeway_legal(seat, e) || s.players[si].causeways >= MAX_CAUSEWAYS {
                        return Err("A causeway can't go there");
                    }
                    place_causeway(s, seat, e);
                    let left = left - 1;
                    if left == 0
                        || s.players[si].causeways >= MAX_CAUSEWAYS
                        || s.causeway_spots(seat).is_empty()
                    {
                        s.go_back(back);
                    } else {
                        s.phase = Phase::Surveyor { left, back };
                    }
                    Ok(())
                }
                _ => Err("Lay your free causeways"),
            }
        }
        Phase::Bounty { back } => {
            if seat != s.current || w != "bounty" {
                return Err("Pick 2 resources");
            }
            let picks = list_of(input).ok_or("Pick 2 resources")?;
            let counts = parse_units(&picks, None).ok_or("Pick 2 resources")?;
            if counts.iter().map(|&c| c as u32).sum::<u32>() != 2
                || (0..5).any(|r| counts[r] > s.bank[r])
            {
                return Err("Pick 2 resources the bank has");
            }
            for (r, &n) in counts.iter().enumerate() {
                s.bank[r] -= n;
                s.players[si].res[r] += n;
            }
            let line = format!("{} took {} from the bank", s.name(seat), res_list(&counts));
            s.log(line);
            s.go_back(back);
            Ok(())
        }
        Phase::Market { back } => {
            if seat != s.current || w != "market" {
                return Err("Name a resource");
            }
            let r = str_of(input).and_then(res_index).ok_or("Name a resource")?;
            let mut got = 0u8;
            for i in 0..s.n() {
                if i != si {
                    got += s.players[i].res[r];
                    s.players[i].res[r] = 0;
                }
            }
            s.players[si].res[r] += got;
            let line = format!(
                "Market Day: {} collected {got} {} from everyone",
                s.name(seat),
                RES[r]
            );
            s.log(line);
            s.go_back(back);
            Ok(())
        }
        Phase::TradeRespond { .. } => respond(s, seat, input),
        Phase::TradeConfirm { .. } => confirm(s, seat, input),
    }
}

/// What happens when a seat runs out of time: always some progress.
fn time_out(s: &mut State, seat: SeatId) {
    let si = seat as usize;
    if si >= s.n() {
        return;
    }
    match s.phase.clone() {
        Phase::Setup { hut, .. } => {
            if s.setup_seat() != Some(seat) {
                return;
            }
            match hut {
                None => {
                    let spots: Vec<usize> = (0..s.buildings.len())
                        .filter(|&v| s.hut_spot_free(v))
                        .collect();
                    if spots.is_empty() {
                        return;
                    }
                    let v = spots[s.rng.below(spots.len() as u32) as usize];
                    let _ = setup_place_hut(s, seat, v);
                }
                Some(_) => {
                    let spots = setup_causeway_spots(s);
                    if spots.is_empty() {
                        return;
                    }
                    let e = spots[s.rng.below(spots.len() as u32) as usize];
                    let _ = setup_place_causeway(s, seat, e);
                }
            }
        }
        Phase::PreRoll if seat == s.current => roll(s),
        Phase::Main if seat == s.current => s.next_turn(),
        Phase::Discard { .. } => random_discard(s, seat),
        Phase::MoveSquall { back } if seat == s.current => {
            let hexes: Vec<usize> = (0..HEX_COUNT).filter(|&h| h != s.squall).collect();
            let h = hexes[s.rng.below(hexes.len() as u32) as usize];
            let _ = move_squall(s, h, back);
        }
        Phase::Steal { from, back } if seat == s.current => {
            if !from.is_empty() {
                let v = from[s.rng.below(from.len() as u32) as usize];
                s.steal(seat, v);
            }
            s.go_back(back);
        }
        Phase::Surveyor { back, .. } | Phase::Bounty { back } | Phase::Market { back }
            if seat == s.current =>
        {
            s.go_back(back)
        }
        Phase::TradeRespond { .. } => {
            let w = if seat == s.current {
                "trade_cancel"
            } else {
                "trade_decline"
            };
            let _ = respond(s, seat, &PhoneInput::press(w));
        }
        Phase::TradeConfirm { .. } if seat == s.current => {
            let _ = confirm(s, seat, &PhoneInput::press("trade_cancel"));
        }
        _ => {}
    }
}

/// Wins are checked for the player whose turn it is.
fn check_win(s: &mut State) {
    if matches!(s.phase, Phase::Setup { .. } | Phase::Over { .. }) {
        return;
    }
    let cur = s.current;
    if s.total_points(cur) >= WIN_POINTS {
        let lore = s.players[cur as usize].old_lore();
        let mut line = format!("{} wins with {} points!", s.name(cur), s.total_points(cur));
        if lore > 0 {
            line.push_str(&format!(" ({lore} from Old Lore)"));
        }
        s.log(line);
        s.phase = Phase::Over { winner: cur };
    }
}

fn wait(seat: SeatId, prompt: impl Into<String>, live: u64, reaction: bool) -> Wait {
    Wait {
        seat,
        prompt: prompt.into(),
        live_timeout_ms: live,
        async_timeout_ms: ASYNC_MS,
        on_timeout: OnTimeout::Rules,
        reaction,
    }
}

/// The waits for the current state. Every return path derives them from
/// here, so a rejected input never drops a timer.
pub fn waits_for(s: &State) -> Vec<Wait> {
    let cur = s.current;
    match &s.phase {
        Phase::Over { .. } => Vec::new(),
        Phase::Setup { round, hut, .. } => {
            let Some(seat) = s.setup_seat() else {
                return Vec::new();
            };
            let which = if *round == 0 { "first" } else { "second" };
            let what = if hut.is_none() { "hut" } else { "causeway" };
            vec![wait(
                seat,
                format!("Place your {which} {what}"),
                SETUP_MS,
                false,
            )]
        }
        Phase::PreRoll => vec![wait(cur, "Roll the tide dice", TURN_MS, false)],
        Phase::Main => vec![wait(cur, "Build, trade or end your turn", TURN_MS, false)],
        Phase::Discard { owed } => owed
            .iter()
            .enumerate()
            .filter(|(_, &n)| n > 0)
            .map(|(i, &n)| {
                wait(
                    i as SeatId,
                    format!("Squall! Discard {n}"),
                    DISCARD_MS,
                    true,
                )
            })
            .collect(),
        Phase::MoveSquall { .. } => vec![wait(cur, "Move the Squall", STEP_MS, false)],
        Phase::Steal { .. } => vec![wait(cur, "Pick who to rob", STEP_MS, false)],
        Phase::Surveyor { .. } => vec![wait(cur, "Lay free causeways", STEP_MS, false)],
        Phase::Bounty { .. } => vec![wait(cur, "Take 2 from the bank", STEP_MS, false)],
        Phase::Market { .. } => vec![wait(cur, "Name a resource", STEP_MS, false)],
        Phase::TradeRespond { responses, .. } => {
            let prompt = format!("Trade offer from {}", s.name(cur));
            responses
                .iter()
                .enumerate()
                .filter(|(_, r)| **r == Resp::Pending)
                .map(|(i, _)| {
                    Wait::reaction(
                        i as SeatId,
                        prompt.clone(),
                        TRADE_MS,
                        ASYNC_MS,
                        PhoneInput::press("trade_decline"),
                    )
                })
                .collect()
        }
        Phase::TradeConfirm { .. } => vec![wait(cur, "Pick a trade", STEP_MS, false)],
    }
}

fn outcome(s: State, events: Vec<Event>) -> Outcome<State> {
    let finished = match s.phase {
        Phase::Over { winner } => {
            let scores: Vec<i64> = (0..s.n())
                .map(|i| s.total_points(i as SeatId) as i64)
                .collect();
            Some(Results {
                ranking: rank_by_score(&scores),
                summary: format!(
                    "{} wins with {} points",
                    s.name(winner),
                    s.total_points(winner)
                ),
            })
        }
        _ => None,
    };
    let waiting_on = if finished.is_some() {
        Vec::new()
    } else {
        waits_for(&s)
    };
    Outcome {
        state: s,
        events,
        waiting_on,
        finished,
    }
}

fn after_change(before: &State, s: &mut State) -> Vec<Event> {
    check_win(s);
    let mut events = Vec::new();
    if s.current != before.current && !matches!(s.phase, Phase::Over { .. }) {
        events.push(Event::Feedback {
            seat: s.current,
            kind: Feedback::YourTurn,
        });
    }
    if let (Some(d), None) = (s.dice, before.dice) {
        events.push(Event::Sound {
            name: "dice".into(),
        });
        if d.0 + d.1 == 7 {
            events.push(Event::Toast {
                text: "A Squall blows in!".into(),
            });
        }
    }
    if s.trades_completed != before.trades_completed {
        events.push(Event::Sound {
            name: "trade".into(),
        });
    }
    if let Phase::Over { winner } = s.phase {
        events.push(Event::Sound {
            name: "fanfare".into(),
        });
        events.push(Event::Toast {
            text: format!("{} wins!", s.name(winner)),
        });
    }
    events
}

impl Rules for Tideholm {
    type State = State;

    fn seats(&self) -> (u32, u32) {
        (3, 4)
    }

    fn setup(&self, seats: &[Seat], seed: u64) -> RulesResult<Outcome<State>> {
        if seats.len() < 3 || seats.len() > 4 {
            return Err(RulesError::Game("Tideholm needs 3 or 4 players".into()));
        }
        let s = new_game(seats, seed);
        Ok(outcome(s, Vec::new()))
    }

    fn on_action(
        &self,
        state: State,
        seat: SeatId,
        input: &PhoneInput,
    ) -> RulesResult<Outcome<State>> {
        let mut s = state.clone();
        match step(&mut s, seat, input) {
            Ok(()) => {
                let events = after_change(&state, &mut s);
                Ok(outcome(s, events))
            }
            Err(_) => {
                let mut events = Vec::new();
                if (seat as usize) < state.players.len() {
                    events.push(Event::Feedback {
                        seat,
                        kind: Feedback::Warning,
                    });
                }
                Ok(outcome(state, events))
            }
        }
    }

    fn on_timeout(&self, state: State, seat: SeatId, _now_ms: u64) -> RulesResult<Outcome<State>> {
        let mut s = state.clone();
        time_out(&mut s, seat);
        let events = after_change(&state, &mut s);
        Ok(outcome(s, events))
    }

    fn table_view(&self, state: &State) -> RulesResult<Value> {
        Ok(view::table(state).to_value())
    }

    fn phone_view(&self, state: &State, seat: SeatId) -> RulesResult<Widget> {
        Ok(view::phone(state, seat))
    }

    fn can_respond(&self, state: &State, seat: SeatId) -> RulesResult<bool> {
        Ok(state.owes_action(seat))
    }
}
