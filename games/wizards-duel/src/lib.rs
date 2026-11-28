//! Wizards Duel: two to four wizards on one hex board, last one standing
//! wins.
//!
//! The rules are Liz's `wizards-rules` crate (from `SueHeir/wizards`),
//! used as is. There is no dedicated PvP API in `wizards-rules` yet, so this
//! builds on its multi-player [`Game`]: with two or more players and no
//! monsters, turns rotate among living players and `Event::Winner` fires
//! when one is left. This crate adds what Hostable needs around it: an
//! arena and spell list ([`arena`]), a serializable state, phone screens,
//! the shared table, a turn timer and a bot.
//!
//! # State
//!
//! [`Game`] isn't serializable (its `Board` keeps private lookup maps and
//! the spell book is an `Arc`), but every field of it is public. [`State`]
//! mirrors those fields as plain data: the board definition (which
//! `wizards-rules` keeps in step when terrain changes or rocks break), the
//! players, zones, whose turn it is and so on. Each call rebuilds a `Game`
//! from the mirror with a struct literal, applies one action and copies the
//! result back. That keeps every call the same cost however long the game
//! runs, unlike replaying every action from the start. `Player::board_used`
//! is a `HashSet`; the mirror keeps it as a sorted list so state JSON is
//! the same on every run.
//!
//! # Secrets
//!
//! Each wizard is dealt one spell from each of five families, and no two
//! wizards share a spell. A wizard's spells and claimed mana only ever
//! appear on their own phone. The table shows positions, HP, statuses,
//! zones and the board's mana, and names a spell only once it has been
//! cast (everyone at the table sees the spell go off).

use std::collections::BTreeSet;

use hostable_sdk::prelude::*;
use wizards_rules::{
    Action, Aim, Bestiary, Board, BoardDef, Claim, Corner, Event as WizEvent, Game, Hex, Piece,
    Player, Status, Vertex, Zone,
};

pub mod arena;
pub mod bot;
mod view;

pub use view::{compass, describe_status, hits, targets};

hostable_sdk::export!(WizardsDuel);

/// How long a live turn lasts.
pub const TURN_LIVE_MS: u64 = 60_000;
/// How long an async turn lasts.
pub const TURN_ASYNC_MS: u64 = 24 * 60 * 60 * 1000;
/// Starting HP. Wizards' own default is 3, which ends a duel before
/// claimed mana starts to matter.
pub const DUEL_HP: u32 = 5;
/// Lines of public log kept in state.
pub const LOG_LINES: usize = 12;
/// Most animation events one call sends (the host allows 64 events).
pub const MAX_ANIMATIONS: usize = 48;

/// Colours for seats that didn't pick one.
pub const SEAT_COLORS: [&str; 4] = ["#e4572e", "#29a7d8", "#f2c14e", "#76b041"];

#[derive(Debug, Default, Clone, Copy)]
pub struct WizardsDuel;

/// A [`Player`] as plain, order-stable data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wizard {
    pub at: Vertex,
    pub hp: u32,
    pub max_hp: u32,
    /// Spell ids, in the order they were dealt. Private.
    pub spells: Vec<String>,
    /// Mana claimed from board hexes. Private.
    pub claimed: Vec<Claim>,
    pub steps_left: u32,
    /// Board mana hexes spent this turn, sorted.
    pub board_used: Vec<Hex>,
    pub status: Status,
}

impl Wizard {
    pub fn from_player(p: &Player) -> Self {
        let mut board_used: Vec<Hex> = p.board_used.iter().copied().collect();
        board_used.sort();
        Self {
            at: p.at,
            hp: p.hp,
            max_hp: p.max_hp,
            spells: p.spells.clone(),
            claimed: p.claimed.clone(),
            steps_left: p.steps_left,
            board_used,
            status: p.status,
        }
    }

    pub fn to_player(&self) -> Player {
        Player {
            at: self.at,
            hp: self.hp,
            max_hp: self.max_hp,
            spells: self.spells.clone(),
            claimed: self.claimed.clone(),
            steps_left: self.steps_left,
            board_used: self.board_used.iter().copied().collect(),
            status: self.status,
        }
    }

    pub fn alive(&self) -> bool {
        self.hp > 0
    }
}

/// Everything about a duel in progress. Wizard `i` sits in seat `i`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub seats: Vec<Seat>,
    /// The board as it is now (broken rocks and changed terrain included).
    /// Left out of the JSON while it is still the untouched arena, which
    /// roughly halves the cost of reading the state inside the sandbox.
    #[serde(
        default = "arena::arena",
        skip_serializing_if = "arena::is_untouched_arena"
    )]
    pub board: BoardDef,
    pub wizards: Vec<Wizard>,
    pub zones: Vec<Zone>,
    /// Whose turn it is.
    pub current: usize,
    /// Round number, counted by `wizards-rules`.
    pub turn: u32,
    pub over: bool,
    pub cleared: bool,
    pub opened: Vec<Vertex>,
    /// The spell the current wizard picked and is now aiming.
    pub aiming: Option<String>,
    /// Seats knocked out, earliest first; seats knocked out by the same
    /// action share a group.
    pub out: Vec<Vec<SeatId>>,
    pub winner: Option<SeatId>,
    /// Public log, newest last.
    pub log: Vec<String>,
    /// Public `wizards-rules` events from the last call, as JSON, for a
    /// renderer to animate.
    pub last_events: Vec<Value>,
    pub rng: Rng,
}

impl State {
    /// Deal spells and place wizards. Start corners, spell hands and who
    /// goes first come from `seed`.
    pub fn new(seats: &[Seat], seed: u64) -> Self {
        let n = seats.len().clamp(1, 4);
        let mut rng = Rng::new(seed);
        let corners = arena::start_corners();
        let mut plan = arena::corner_plan(n);
        rng.shuffle(&mut plan);
        let mut hands: Vec<Vec<String>> = vec![Vec::new(); n];
        for (_, family) in arena::FAMILIES {
            let mut f = family.to_vec();
            rng.shuffle(&mut f);
            for (i, hand) in hands.iter_mut().enumerate() {
                hand.push(f[i % f.len()].to_string());
            }
        }
        let wizards = (0..n)
            .map(|i| {
                let mut p = Player::new(corners[plan[i % plan.len()]], hands[i].clone());
                p.hp = DUEL_HP;
                p.max_hp = DUEL_HP;
                Wizard::from_player(&p)
            })
            .collect();
        let current = rng.below(n as u32) as usize;
        let mut state = Self {
            seats: seats.iter().take(n).cloned().collect(),
            board: arena::arena(),
            wizards,
            zones: Vec::new(),
            current,
            turn: 1,
            over: false,
            cleared: false,
            opened: Vec::new(),
            aiming: None,
            out: Vec::new(),
            winner: None,
            log: Vec::new(),
            last_events: Vec::new(),
            rng,
        };
        let first = state.name(current as SeatId);
        state.push_log(format!("The duel begins. {first} goes first."));
        state
    }

    /// The `wizards-rules` game this state describes.
    pub fn game(&self) -> Game {
        Game {
            board: Board::new(self.board.clone()),
            book: arena::book(),
            // The arena has no monsters, so no monster kinds are needed
            // (`Bestiary::builtin()` would parse its RON on every call).
            bestiary: Bestiary::default(),
            players: self.wizards.iter().map(Wizard::to_player).collect(),
            monsters: Vec::new(),
            zones: self.zones.clone(),
            current: self.current,
            turn: self.turn,
            cleared: self.cleared,
            over: self.over,
            opened: self.opened.clone(),
        }
    }

    /// Copy a game's state back into the mirror.
    pub fn sync(&mut self, game: &Game) {
        self.board = game.board.def.clone();
        self.wizards = game.players.iter().map(Wizard::from_player).collect();
        self.zones = game.zones.clone();
        self.current = game.current;
        self.turn = game.turn;
        self.cleared = game.cleared;
        self.over = game.over;
        self.opened = game.opened.clone();
    }

    pub fn name(&self, seat: SeatId) -> String {
        self.seats
            .get(seat as usize)
            .map_or_else(|| format!("Wizard {}", seat + 1), |s| s.name.clone())
    }

    pub fn color(&self, seat: SeatId) -> String {
        match self.seats.get(seat as usize) {
            Some(s) if !s.color.is_empty() => s.color.clone(),
            _ => SEAT_COLORS[seat as usize % SEAT_COLORS.len()].to_string(),
        }
    }

    fn push_log(&mut self, line: String) {
        self.log.push(line);
        if self.log.len() > LOG_LINES {
            let extra = self.log.len() - LOG_LINES;
            self.log.drain(..extra);
        }
    }

    /// Final places: the winner, then the others from last out to first.
    pub fn ranking(&self) -> Vec<Vec<SeatId>> {
        let mut ranking = Vec::new();
        if let Some(w) = self.winner {
            ranking.push(vec![w]);
        }
        for group in self.out.iter().rev() {
            ranking.push(group.clone());
        }
        let placed: BTreeSet<SeatId> = ranking.iter().flatten().copied().collect();
        let rest: Vec<SeatId> = (0..self.wizards.len() as SeatId)
            .filter(|s| !placed.contains(s))
            .collect();
        if !rest.is_empty() {
            ranking.insert(usize::from(self.winner.is_some()), rest);
        }
        ranking
    }
}

/// What an input asks for, before the rules check it.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// A `wizards-rules` action.
    Act(Action),
    /// Pick a spell to aim.
    Aim(String),
    /// Stop aiming.
    Cancel,
}

/// Choice values on phone screens.
pub fn vertex_value(v: Vertex) -> String {
    let c = match v.corner {
        Corner::N => "N",
        Corner::S => "S",
    };
    format!("v:{}:{}:{c}", v.hex.q, v.hex.r)
}

pub fn hex_value(h: Hex) -> String {
    format!("h:{}:{}", h.q, h.r)
}

pub fn claim_value(h: Hex) -> String {
    format!("claim:{}:{}", h.q, h.r)
}

fn parse_ints(parts: &[&str]) -> Option<(i32, i32)> {
    match parts {
        [q, r] => Some((q.parse().ok()?, r.parse().ok()?)),
        _ => None,
    }
}

pub fn parse_vertex(s: &str) -> Option<Vertex> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.as_slice() {
        ["v", q, r, "N"] => parse_ints(&[q, r]).map(|(q, r)| Vertex::n(q, r)),
        ["v", q, r, "S"] => parse_ints(&[q, r]).map(|(q, r)| Vertex::s(q, r)),
        _ => None,
    }
}

fn parse_hex(s: &str, tag: &str) -> Option<Hex> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.as_slice() {
        [t, q, r] if *t == tag => parse_ints(&[q, r]).map(|(q, r)| Hex::new(q, r)),
        _ => None,
    }
}

pub fn parse_aim(s: &str) -> Option<Aim> {
    parse_vertex(s)
        .map(Aim::Vertex)
        .or_else(|| parse_hex(s, "h").map(Aim::Hex))
}

/// Whether the current wizard can cast `spell` right now: known, not
/// stunned, payable and with somewhere to aim it.
pub fn castable(game: &Game, spell: &str) -> bool {
    let p = &game.players[game.current];
    p.spells.iter().any(|s| s == spell)
        && p.status.stunned == 0
        && game.auto_pay(spell).is_some()
        && game.book.get(spell).is_some_and(|def| {
            !view::targets(game, def, &game.board.distances_from(p.at)).is_empty()
        })
}

/// Turn a phone input into what it asks for. `Err` holds why it was
/// refused; the state is never changed by a refused input.
pub fn read_input(
    state: &State,
    game: &Game,
    seat: SeatId,
    input: &PhoneInput,
) -> Result<Step, &'static str> {
    if state.over {
        return Err("the duel is over");
    }
    if seat as usize != state.current {
        return Err("it isn't your turn");
    }
    let me = &state.wizards[state.current];
    if !me.alive() {
        return Err("you are out of the duel");
    }
    let value = input.value.as_str();
    if let Some(spell) = &state.aiming {
        return match (input.widget.as_str(), value) {
            ("aim", Some(v)) => {
                let target = parse_aim(v).ok_or("not a target")?;
                let pay = game.auto_pay(spell).ok_or("not enough mana")?;
                Ok(Step::Act(Action::Cast {
                    spell: spell.clone(),
                    target,
                    pay,
                }))
            }
            ("cancel", _) => Ok(Step::Cancel),
            _ => Err("finish aiming first"),
        };
    }
    match (input.widget.as_str(), value) {
        ("move", Some(v)) => parse_vertex(v)
            .map(|v| Step::Act(Action::Move(v)))
            .ok_or("not a vertex"),
        ("cast", Some(id)) => {
            if me.spells.iter().any(|s| s == id) {
                Ok(Step::Aim(id.to_string()))
            } else {
                Err("you don't know that spell")
            }
        }
        ("end", Some("end")) => Ok(Step::Act(Action::EndTurn { claim: vec![] })),
        ("end", Some(v)) => parse_hex(v, "claim")
            .map(|h| Step::Act(Action::EndTurn { claim: vec![h] }))
            .ok_or("not a hex"),
        _ => Err("unknown input"),
    }
}

impl Rules for WizardsDuel {
    type State = State;

    fn seats(&self) -> (u32, u32) {
        (2, 4)
    }

    fn setup(&self, seats: &[Seat], seed: u64) -> RulesResult<Outcome<State>> {
        Ok(outcome(State::new(seats, seed), Vec::new()))
    }

    fn on_action(
        &self,
        state: State,
        seat: SeatId,
        input: &PhoneInput,
    ) -> RulesResult<Outcome<State>> {
        Ok(handle(state, seat, input))
    }

    fn on_timeout(&self, state: State, seat: SeatId, _now_ms: u64) -> RulesResult<Outcome<State>> {
        if state.over || seat as usize != state.current {
            return Ok(outcome(state, Vec::new()));
        }
        let game = state.game();
        let mut next = state.clone();
        next.push_log(format!("{} ran out of time.", state.name(seat)));
        Ok(
            apply(next, game, Action::EndTurn { claim: vec![] }).unwrap_or_else(|| {
                // Ending a turn without claiming can't fail while the game runs.
                outcome(state, Vec::new())
            }),
        )
    }

    fn table_view(&self, state: &State) -> RulesResult<Value> {
        Ok(view::table(state).to_value())
    }

    fn phone_view(&self, state: &State, seat: SeatId) -> RulesResult<Widget> {
        Ok(view::phone(state, seat))
    }
}

fn handle(state: State, seat: SeatId, input: &PhoneInput) -> Outcome<State> {
    let game = state.game();
    match read_input(&state, &game, seat, input) {
        Err(_) => reject(state, seat),
        Ok(Step::Cancel) => {
            let mut next = state;
            next.aiming = None;
            outcome(next, Vec::new())
        }
        Ok(Step::Aim(spell)) => {
            if castable(&game, &spell) {
                let mut next = state;
                next.aiming = Some(spell);
                outcome(next, Vec::new())
            } else {
                reject(state, seat)
            }
        }
        Ok(Step::Act(action)) => match apply(state.clone(), game, action) {
            Some(out) => out,
            None => reject(state, seat),
        },
    }
}

/// Unchanged state, a buzz on the phone and the turn timer kept.
fn reject(state: State, seat: SeatId) -> Outcome<State> {
    let events = if (seat as usize) < state.wizards.len() {
        vec![Event::Feedback {
            seat,
            kind: Feedback::Warning,
        }]
    } else {
        Vec::new()
    };
    outcome(state, events)
}

/// Apply one action. `None` if the rules refused it.
fn apply(mut state: State, mut game: Game, action: Action) -> Option<Outcome<State>> {
    let mut events = game.apply(action).ok()?;
    // A wizard can't take a turn once they are out. Nothing in the duel's
    // spell list hurts its caster, but end such a turn rather than stall.
    if !game.over && game.players[game.current].hp == 0 {
        if let Ok(more) = game.apply(Action::EndTurn { claim: vec![] }) {
            events.extend(more);
        }
    }
    state.sync(&game);
    state.aiming = None;
    let host_events = record(&mut state, &events);
    if !state.over && state.wizards.iter().all(|w| !w.alive()) {
        state.over = true;
        state.push_log("Nobody is left standing: a draw.".into());
    }
    Some(outcome(state, host_events))
}

/// Log the public part of what happened and turn it into host events.
/// Claims are private and never leave the claimer's phone.
fn record(state: &mut State, events: &[WizEvent]) -> Vec<Event> {
    let mut out = Vec::new();
    let mut public = Vec::new();
    let mut defeated = Vec::new();
    for e in events {
        let line = match e {
            WizEvent::Claimed { .. } => continue,
            WizEvent::Cast { player, spell, .. } => Some(format!(
                "{} casts {}.",
                state.name(*player as SeatId),
                arena::spell_name(spell)
            )),
            WizEvent::Damaged {
                piece: Piece::Player(i),
                amount,
                hp_left,
            } => Some(format!(
                "{} takes {amount} ({hp_left} HP left).",
                state.name(*i as SeatId)
            )),
            WizEvent::Blocked {
                piece: Piece::Player(i),
                amount,
            } => Some(format!(
                "{}'s shield blocks {amount}.",
                state.name(*i as SeatId)
            )),
            WizEvent::Healed {
                piece: Piece::Player(i),
                amount,
            } if *amount > 0 => Some(format!("{} heals {amount}.", state.name(*i as SeatId))),
            WizEvent::Pushed {
                piece: Piece::Player(i),
                ..
            } => Some(format!("{} is knocked aside.", state.name(*i as SeatId))),
            WizEvent::PlayerDefeated { player } => {
                defeated.push(*player as SeatId);
                Some(format!("{} is defeated!", state.name(*player as SeatId)))
            }
            WizEvent::Winner { player } => {
                state.winner = Some(*player as SeatId);
                Some(format!("{} wins the duel!", state.name(*player as SeatId)))
            }
            WizEvent::TurnStarted { player, .. } => {
                Some(format!("{}'s turn.", state.name(*player as SeatId)))
            }
            _ => None,
        };
        if let Some(line) = line {
            state.push_log(line);
        }
        if let Ok(v) = serde_json::to_value(e) {
            public.push(v);
        }
    }
    if !defeated.is_empty() {
        state.out.push(defeated);
    }
    for what in public.iter().take(MAX_ANIMATIONS) {
        out.push(Event::Animate { what: what.clone() });
    }
    if events
        .iter()
        .any(|e| matches!(e, WizEvent::Damaged { amount, .. } if *amount > 0))
    {
        out.push(Event::Sound { name: "hit".into() });
    }
    if state.winner.is_some() && state.over {
        out.push(Event::Sound {
            name: "fanfare".into(),
        });
    }
    public.truncate(MAX_ANIMATIONS);
    state.last_events = public;
    out
}

/// Wrap a state with the turn timer, or the results once it's over.
fn outcome(state: State, events: Vec<Event>) -> Outcome<State> {
    let mut out = Outcome::new(state);
    out.events = events;
    if out.state.over {
        let summary = match out.state.winner {
            Some(w) => format!("{} wins the duel!", out.state.name(w)),
            None => "A draw: nobody is left standing.".into(),
        };
        out.finished = Some(Results {
            ranking: out.state.ranking(),
            summary,
        });
    } else {
        let mut wait = Wait::turn(out.state.current as SeatId, TURN_LIVE_MS, TURN_ASYNC_MS);
        wait.prompt = "Your turn".into();
        out.waiting_on.push(wait);
    }
    out
}
