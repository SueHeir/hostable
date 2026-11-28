//! Riposte: a two-player duel of fencers and alchemists in a lantern-lit
//! city, played with cards that answer each other on a shared stack.
//!
//! Each player has 20 Resolve and builds a 20-card deck on their phone. On
//! your turn your Spark maximum rises by one (to 10) and refills, your Allies
//! ready, and you draw a card. Drawing from an empty deck loses the duel.
//!
//! Every card play, ability and strike goes on the stack and gives the other
//! player priority. A Feint may be played whenever you have priority, so
//! answers can be answered in turn, to any depth. When both players pass in
//! a row, the top item resolves and the active player gets priority again.

use hostable_sdk::prelude::*;

pub mod bot;
pub mod cards;
mod view;

#[cfg(test)]
mod tests;

pub use cards::{card, card_text, CardDef, CardId, Eff, Kind, Neg, School, Tgt, CARDS, POOL_SIZE};
use cards::{CharmEff, LAMPLIGHTER, STARTERS};

hostable_sdk::export!(Riposte);

pub const START_RESOLVE: i32 = 20;
pub const DECK_SIZE: usize = 20;
pub const START_HAND: usize = 5;
pub const MAX_SPARK: u8 = 10;
pub const MAX_ALLIES: usize = 6;
pub const MAX_CHARMS: usize = 4;
const LOG_LINES: usize = 40;

pub const BUILD_LIVE_MS: u64 = 180_000;
pub const TURN_LIVE_MS: u64 = 90_000;
pub const REACT_LIVE_MS: u64 = 8_000;
pub const BLOCK_LIVE_MS: u64 = 30_000;
pub const ASYNC_MS: u64 = 86_400_000;

#[derive(Debug, Default, Clone, Copy)]
pub struct Riposte;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Both players are building decks.
    Building,
    /// Normal play: the active player acts, or someone holds priority over
    /// a non-empty stack.
    Main,
    /// A Strike resolved; the defender assigns blockers.
    Blocking,
    Over,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Player {
    pub resolve: i32,
    pub spark: u8,
    pub spark_max: u8,
    pub hand: Vec<CardId>,
    /// Top of the deck is the end.
    pub deck: Vec<CardId>,
    /// Top of the Gutter is the end.
    pub gutter: Vec<CardId>,
    pub charms: Vec<CardId>,
    pub ward: u8,
    pub built: bool,
    pub decked: bool,
}

impl Player {
    fn new() -> Self {
        Self {
            resolve: START_RESOLVE,
            spark: 0,
            spark_max: 0,
            hand: Vec::new(),
            deck: Vec::new(),
            gutter: Vec::new(),
            charms: Vec::new(),
            ward: 0,
            built: false,
            decked: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ally {
    pub uid: u32,
    pub card: CardId,
    pub owner: SeatId,
    pub spent: bool,
    /// Arrived since its owner's turn began.
    pub fresh: bool,
    pub damage: u8,
    pub boost_power: u8,
    pub boost_guard: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v", rename_all = "snake_case")]
pub enum Target {
    Player(SeatId),
    Ally(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ItemKind {
    Card { card: CardId },
    Ability { card: CardId, ally: u32 },
    Strike { allies: Vec<u32> },
    Clash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub owner: SeatId,
    pub kind: ItemKind,
    #[serde(default)]
    pub target: Option<Target>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Combat {
    pub strikers: Vec<u32>,
    /// (blocker, striker)
    pub blocks: Vec<(u32, u32)>,
}

/// Where a pending play came from, while its owner picks a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v", rename_all = "snake_case")]
pub enum Source {
    Hand(CardId),
    Ability(u32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub names: Vec<String>,
    pub colors: Vec<String>,
    pub rng: Rng,
    pub phase: Phase,
    pub players: Vec<Player>,
    pub allies: Vec<Ally>,
    pub next_uid: u32,
    pub stack: Vec<Item>,
    /// Who holds priority while the stack isn't empty.
    pub priority: SeatId,
    /// Passes in a row on the current top item.
    pub passes: u8,
    /// Bumped whenever priority moves, so each window gets its own deadline.
    pub window: u32,
    pub active: SeatId,
    pub turn: u32,
    pub struck: bool,
    pub combat: Option<Combat>,
    pub selecting: Option<Source>,
    pub deepest_chain: u32,
    pub log: Vec<String>,
    pub result: Option<Results>,
}

fn other(seat: SeatId) -> SeatId {
    1 - seat.min(1)
}

impl State {
    pub fn name(&self, seat: SeatId) -> &str {
        self.names.get(seat as usize).map_or("?", |s| s.as_str())
    }

    fn note(&mut self, line: String) {
        self.log.push(line);
        if self.log.len() > LOG_LINES {
            let extra = self.log.len() - LOG_LINES;
            self.log.drain(..extra);
        }
    }

    pub fn player(&self, seat: SeatId) -> &Player {
        &self.players[seat.min(1) as usize]
    }

    fn player_mut(&mut self, seat: SeatId) -> &mut Player {
        &mut self.players[seat.min(1) as usize]
    }

    pub fn ally(&self, uid: u32) -> Option<&Ally> {
        self.allies.iter().find(|a| a.uid == uid)
    }

    fn ally_mut(&mut self, uid: u32) -> Option<&mut Ally> {
        self.allies.iter_mut().find(|a| a.uid == uid)
    }

    pub fn allies_of(&self, seat: SeatId) -> impl Iterator<Item = &Ally> {
        self.allies.iter().filter(move |a| a.owner == seat)
    }

    pub fn power(&self, a: &Ally) -> u32 {
        let team: u32 = self
            .player(a.owner)
            .charms
            .iter()
            .filter_map(|c| match card(*c).charm {
                Some(CharmEff::TeamPower(n)) => Some(n as u32),
                _ => None,
            })
            .sum();
        card(a.card).power as u32 + a.boost_power as u32 + team
    }

    pub fn guard(&self, a: &Ally) -> u32 {
        card(a.card).guard as u32 + a.boost_guard as u32
    }

    /// What `card` costs `seat` to play right now.
    pub fn cost(&self, seat: SeatId, id: CardId) -> u8 {
        let c = card(id);
        if c.kind != Kind::Feint {
            return c.cost;
        }
        let discount: u8 = self
            .player(seat)
            .charms
            .iter()
            .filter_map(|c| match card(*c).charm {
                Some(CharmEff::FeintDiscount(n)) => Some(n),
                _ => None,
            })
            .fold(0u8, |a, b| a.saturating_add(b));
        c.cost.saturating_sub(discount)
    }

    /// Whether `seat` may act now (play a Feint or use an ability).
    pub fn has_priority(&self, seat: SeatId) -> bool {
        self.phase == Phase::Main
            && if self.stack.is_empty() {
                seat == self.active
            } else {
                seat == self.priority
            }
    }

    /// The seat the game is waiting on, if one.
    pub fn acting(&self) -> Option<SeatId> {
        match self.phase {
            Phase::Main if self.stack.is_empty() => Some(self.active),
            Phase::Main => Some(self.priority),
            Phase::Blocking => Some(other(self.active)),
            _ => None,
        }
    }

    pub fn item_name(&self, item: &Item) -> String {
        match &item.kind {
            ItemKind::Card { card: c } => card(*c).name.to_string(),
            ItemKind::Ability { card: c, .. } => format!("{}'s ability", card(*c).name),
            ItemKind::Strike { allies } => format!(
                "Strike with {}",
                allies
                    .iter()
                    .filter_map(|u| self.ally(*u))
                    .map(|a| card(a.card).name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            ItemKind::Clash => "Clash (blocks are set)".to_string(),
        }
    }

    // ---- legality ----

    /// Every legal target for `seat` under `t`.
    pub fn targets(&self, seat: SeatId, t: Tgt) -> Vec<Target> {
        let mut out = Vec::new();
        if t == Tgt::Any {
            out.push(Target::Player(other(seat)));
            out.push(Target::Player(seat));
        }
        for a in &self.allies {
            let ok = match t {
                Tgt::None => false,
                Tgt::Any | Tgt::Ally => true,
                Tgt::Enemy => a.owner != seat,
                Tgt::Own => a.owner == seat,
            };
            if ok {
                out.push(Target::Ally(a.uid));
            }
        }
        out
    }

    fn target_ok(&self, seat: SeatId, t: Tgt, target: Target) -> bool {
        self.targets(seat, t).contains(&target)
    }

    /// Whether a negation could answer the current top of the stack.
    fn negation_fits(&self, n: Neg) -> bool {
        let Some(top) = self.stack.last() else {
            return false;
        };
        neg_matches(n, &top.kind)
    }

    /// Whether `seat` could play this card from hand now (target aside from
    /// existing).
    pub fn can_play(&self, seat: SeatId, id: CardId) -> bool {
        let c = card(id);
        if c.token || !self.has_priority(seat) || !self.player(seat).hand.contains(&id) {
            return false;
        }
        if self.cost(seat, id) > self.player(seat).spark {
            return false;
        }
        if c.kind != Kind::Feint && !(seat == self.active && self.stack.is_empty()) {
            return false;
        }
        match c.kind {
            Kind::Ally if self.allies_of(seat).count() >= MAX_ALLIES => return false,
            Kind::Charm if self.player(seat).charms.len() >= MAX_CHARMS => return false,
            _ => {}
        }
        if let Some(n) = c.negation() {
            if !self.negation_fits(n) {
                return false;
            }
        }
        !c.needs_target() || !self.targets(seat, c.target).is_empty()
    }

    /// Whether `seat` could use this Ally's ability now.
    pub fn can_use(&self, seat: SeatId, uid: u32) -> bool {
        let Some(a) = self.ally(uid) else {
            return false;
        };
        let Some(ab) = card(a.card).ability else {
            return false;
        };
        a.owner == seat
            && !a.spent
            && self.has_priority(seat)
            && self.player(seat).spark >= ab.cost
            && (ab.target == Tgt::None || !self.targets(seat, ab.target).is_empty())
    }

    /// Allies `seat` may declare as strikers now.
    pub fn strikers(&self, seat: SeatId) -> Vec<u32> {
        if self.phase != Phase::Main || !self.stack.is_empty() || seat != self.active || self.struck
        {
            return Vec::new();
        }
        self.allies_of(seat)
            .filter(|a| !a.spent && (!a.fresh || card(a.card).swift))
            .map(|a| a.uid)
            .collect()
    }

    /// (blocker, striker) pairs the defender may still add.
    pub fn block_options(&self) -> Vec<(u32, u32)> {
        let Some(combat) = &self.combat else {
            return Vec::new();
        };
        if self.phase != Phase::Blocking {
            return Vec::new();
        }
        let defender = other(self.active);
        let mut out = Vec::new();
        for b in self.allies_of(defender) {
            if b.spent || combat.blocks.iter().any(|(x, _)| *x == b.uid) {
                continue;
            }
            for s in &combat.strikers {
                let open = self
                    .ally(*s)
                    .is_some_and(|a| !card(a.card).elusive && a.owner == self.active)
                    && !combat.blocks.iter().any(|(_, y)| y == s);
                if open {
                    out.push((b.uid, *s));
                }
            }
        }
        out
    }

    /// Feints and abilities `seat` could use right now.
    pub fn can_respond_now(&self, seat: SeatId) -> bool {
        if !self.has_priority(seat) || self.stack.is_empty() {
            return false;
        }
        let p = self.player(seat);
        p.hand.iter().any(|c| self.can_play(seat, *c))
            || self.allies_of(seat).any(|a| self.can_use(seat, a.uid))
    }
}

fn neg_matches(n: Neg, kind: &ItemKind) -> bool {
    match (n, kind) {
        (Neg::CardOrAbility, ItemKind::Card { .. } | ItemKind::Ability { .. }) => true,
        (Neg::Feint, ItemKind::Card { card: c }) => card(*c).kind == Kind::Feint,
        (Neg::Strike, ItemKind::Strike { .. }) => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Setup and turns
// ---------------------------------------------------------------------------

pub fn new_game(seats: &[Seat], seed: u64) -> State {
    State {
        names: seats.iter().map(|s| s.name.clone()).collect(),
        colors: seats.iter().map(|s| s.color.clone()).collect(),
        rng: Rng::new(seed),
        phase: Phase::Building,
        players: vec![Player::new(), Player::new()],
        allies: Vec::new(),
        next_uid: 1,
        stack: Vec::new(),
        priority: 0,
        passes: 0,
        window: 0,
        active: 0,
        turn: 0,
        struck: false,
        combat: None,
        selecting: None,
        deepest_chain: 0,
        log: vec!["Build your decks.".into()],
        result: None,
    }
}

fn set_deck(s: &mut State, seat: SeatId, deck: Vec<CardId>) {
    let p = s.player_mut(seat);
    p.deck = deck;
    p.built = true;
    // The log doesn't say which deck: that would give away its cards.
    let line = format!("{}'s deck is ready.", s.name(seat));
    s.note(line);
    if s.players.iter().all(|p| p.built) {
        begin(s);
    }
}

fn begin(s: &mut State) {
    for seat in 0..2 {
        let mut deck = std::mem::take(&mut s.player_mut(seat).deck);
        s.rng.shuffle(&mut deck);
        s.player_mut(seat).deck = deck;
        draw(s, seat, START_HAND as u8);
    }
    s.phase = Phase::Main;
    let first = s.rng.below(2);
    let line = format!("{} goes first.", s.name(first));
    s.note(line);
    start_turn(s, first);
}

fn start_turn(s: &mut State, seat: SeatId) {
    s.active = seat;
    s.turn += 1;
    s.struck = false;
    s.combat = None;
    s.selecting = None;
    s.window += 1;
    {
        let p = s.player_mut(seat);
        p.spark_max = (p.spark_max + 1).min(MAX_SPARK);
        p.spark = p.spark_max;
        p.ward = 0;
    }
    for a in s.allies.iter_mut().filter(|a| a.owner == seat) {
        a.spent = false;
        a.fresh = false;
    }
    let line = format!("Turn {}: {}.", s.turn, s.name(seat));
    s.note(line);
    let charms = s.player(seat).charms.clone();
    for c in charms {
        let foe = other(seat);
        match card(c).charm {
            Some(CharmEff::StartHeal(n)) => heal(s, seat, n),
            Some(CharmEff::StartDamage(n)) => hurt_player(s, foe, n as u32),
            Some(CharmEff::StartSummon(n)) => summon(s, seat, n),
            _ => {}
        }
    }
    if check_over(s) {
        return;
    }
    draw(s, seat, 1);
    check_over(s);
}

fn end_turn(s: &mut State) {
    for a in &mut s.allies {
        a.damage = 0;
        a.boost_power = 0;
        a.boost_guard = 0;
    }
    let line = format!("{} ends their turn.", s.name(s.active));
    s.note(line);
    let next = other(s.active);
    start_turn(s, next);
}

fn check_over(s: &mut State) -> bool {
    if s.phase == Phase::Over {
        return true;
    }
    let lost: Vec<bool> = s
        .players
        .iter()
        .map(|p| p.resolve <= 0 || p.decked)
        .collect();
    let (ranking, summary) = match (lost[0], lost[1]) {
        (false, false) => return false,
        (true, true) => (vec![vec![0, 1]], "Both duellists fall: a draw.".to_string()),
        (true, false) => (
            vec![vec![1], vec![0]],
            format!("{} wins the duel!", s.name(1)),
        ),
        (false, true) => (
            vec![vec![0], vec![1]],
            format!("{} wins the duel!", s.name(0)),
        ),
    };
    s.note(summary.clone());
    s.phase = Phase::Over;
    s.stack.clear();
    s.combat = None;
    s.selecting = None;
    s.result = Some(Results { ranking, summary });
    true
}

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

fn draw(s: &mut State, seat: SeatId, n: u8) {
    for _ in 0..n {
        let p = s.player_mut(seat);
        match p.deck.pop() {
            Some(c) => p.hand.push(c),
            None => {
                p.decked = true;
                let line = format!("{} must draw from an empty deck!", s.name(seat));
                s.note(line);
                return;
            }
        }
    }
}

fn heal(s: &mut State, seat: SeatId, n: u8) {
    let p = s.player_mut(seat);
    p.resolve = (p.resolve + n as i32).min(99);
}

fn lose(s: &mut State, seat: SeatId, n: u32) {
    s.player_mut(seat).resolve -= n as i32;
}

fn hurt_player(s: &mut State, seat: SeatId, n: u32) {
    let p = s.player_mut(seat);
    let absorbed = (p.ward as u32).min(n);
    p.ward -= absorbed as u8;
    p.resolve -= (n - absorbed) as i32;
}

fn hurt_ally(s: &mut State, uid: u32, n: u32) {
    if let Some(a) = s.ally_mut(uid) {
        a.damage = (a.damage as u32 + n).min(255) as u8;
    }
}

/// Destroy Allies whose damage reached their guard.
fn cleanup(s: &mut State) {
    let dead: Vec<u32> = s
        .allies
        .iter()
        .filter(|a| a.damage as u32 >= s.guard(a))
        .map(|a| a.uid)
        .collect();
    for uid in dead {
        destroy(s, uid);
    }
}

fn remove_ally(s: &mut State, uid: u32) -> Option<Ally> {
    let i = s.allies.iter().position(|a| a.uid == uid)?;
    Some(s.allies.remove(i))
}

fn destroy(s: &mut State, uid: u32) {
    if let Some(a) = remove_ally(s, uid) {
        let line = format!("{}'s {} falls.", s.name(a.owner), card(a.card).name);
        s.note(line);
        if !card(a.card).token {
            s.player_mut(a.owner).gutter.push(a.card);
        }
    }
}

fn bounce(s: &mut State, uid: u32) {
    if let Some(a) = remove_ally(s, uid) {
        if card(a.card).token {
            let line = format!("{}'s Lamplighter fades away.", s.name(a.owner));
            s.note(line);
        } else {
            s.player_mut(a.owner).hand.push(a.card);
            let line = format!("An Ally returns to {}'s hand.", s.name(a.owner));
            s.note(line);
        }
    }
}

fn add_ally(s: &mut State, seat: SeatId, id: CardId) -> bool {
    if s.allies_of(seat).count() >= MAX_ALLIES {
        return false;
    }
    let uid = s.next_uid;
    s.next_uid += 1;
    s.allies.push(Ally {
        uid,
        card: id,
        owner: seat,
        spent: false,
        fresh: true,
        damage: 0,
        boost_power: 0,
        boost_guard: 0,
    });
    true
}

fn summon(s: &mut State, seat: SeatId, n: u8) {
    for _ in 0..n {
        if !add_ally(s, seat, LAMPLIGHTER) {
            break;
        }
    }
}

fn mill(s: &mut State, seat: SeatId, n: u8) {
    for _ in 0..n {
        let p = s.player_mut(seat);
        match p.deck.pop() {
            Some(c) => p.gutter.push(c),
            None => break,
        }
    }
}

fn discard_random(s: &mut State, seat: SeatId, n: u8) {
    for _ in 0..n {
        let len = s.player(seat).hand.len();
        if len == 0 {
            break;
        }
        let i = s.rng.below(len as u32) as usize;
        let p = s.player_mut(seat);
        let c = p.hand.remove(i.min(len - 1));
        p.gutter.push(c);
        let line = format!("{} discards {}.", s.name(seat), card(c).name);
        s.note(line);
    }
}

/// Apply one effect for `owner`. Returns false once the game is over.
fn apply(s: &mut State, owner: SeatId, e: Eff, target: Option<Target>) -> bool {
    let foe = other(owner);
    match e {
        Eff::Damage(n) => match target {
            Some(Target::Player(p)) => hurt_player(s, p, n as u32),
            Some(Target::Ally(u)) => hurt_ally(s, u, n as u32),
            None => {}
        },
        Eff::DamageFoe(n) => hurt_player(s, foe, n as u32),
        Eff::DamageEnemyAllies(n) => {
            let uids: Vec<u32> = s.allies_of(foe).map(|a| a.uid).collect();
            for u in uids {
                hurt_ally(s, u, n as u32);
            }
        }
        Eff::DamageEveryAlly(n) => {
            for a in &mut s.allies {
                a.damage = (a.damage as u32 + n as u32).min(255) as u8;
            }
        }
        Eff::FoeLoses(n) => lose(s, foe, n as u32),
        Eff::YouLose(n) => lose(s, owner, n as u32),
        Eff::Heal(n) => heal(s, owner, n),
        Eff::Drain(n) => {
            lose(s, foe, n as u32);
            heal(s, owner, n);
        }
        Eff::Draw(n) => draw(s, owner, n),
        Eff::Ward(n) => {
            let p = s.player_mut(owner);
            p.ward = p.ward.saturating_add(n);
        }
        Eff::Boost(pw, g) => {
            if let Some(Target::Ally(u)) = target {
                if let Some(a) = s.ally_mut(u) {
                    a.boost_power = a.boost_power.saturating_add(pw);
                    a.boost_guard = a.boost_guard.saturating_add(g);
                }
            }
        }
        Eff::BoostTeam(pw, g) => {
            for a in s.allies.iter_mut().filter(|a| a.owner == owner) {
                a.boost_power = a.boost_power.saturating_add(pw);
                a.boost_guard = a.boost_guard.saturating_add(g);
            }
        }
        Eff::Ready | Eff::Spend => {
            if let Some(Target::Ally(u)) = target {
                if let Some(a) = s.ally_mut(u) {
                    a.spent = e == Eff::Spend;
                }
            }
        }
        Eff::Bounce => {
            if let Some(Target::Ally(u)) = target {
                bounce(s, u);
            }
        }
        Eff::BounceEnemyAllies => {
            let uids: Vec<u32> = s.allies_of(foe).map(|a| a.uid).collect();
            for u in uids {
                bounce(s, u);
            }
        }
        Eff::Destroy => {
            if let Some(Target::Ally(u)) = target {
                destroy(s, u);
            }
        }
        Eff::FoeDiscards(n) => discard_random(s, foe, n),
        Eff::Mill(n) => mill(s, foe, n),
        Eff::Recall => {
            let p = s.player_mut(owner);
            if let Some(c) = p.gutter.pop() {
                p.hand.push(c);
                let line = format!("{} takes a card back from the Gutter.", s.name(owner));
                s.note(line);
            }
        }
        Eff::Summon(n) => summon(s, owner, n),
        Eff::Negate(n) => {
            if s.stack.last().is_some_and(|i| neg_matches(n, &i.kind)) {
                negate_top(s);
            }
        }
        Eff::NegateUnlessPay(cost) => {
            if let Some(top) = s.stack.last().cloned() {
                if neg_matches(Neg::CardOrAbility, &top.kind) {
                    if s.player(top.owner).spark >= cost {
                        s.player_mut(top.owner).spark -= cost;
                        let line = format!("{} pays {cost} Spark to keep it.", s.name(top.owner));
                        s.note(line);
                    } else {
                        negate_top(s);
                    }
                }
            }
        }
    }
    cleanup(s);
    !check_over(s)
}

fn negate_top(s: &mut State) {
    let Some(item) = s.stack.pop() else {
        return;
    };
    let line = format!("{} is negated.", s.item_name(&item));
    s.note(line);
    if let ItemKind::Card { card: c } = item.kind {
        s.player_mut(item.owner).gutter.push(c);
    }
}

// ---------------------------------------------------------------------------
// The stack
// ---------------------------------------------------------------------------

fn push(s: &mut State, item: Item) {
    let feint_by = match &item.kind {
        ItemKind::Card { card: c } if card(*c).kind == Kind::Feint => Some(item.owner),
        _ => None,
    };
    let clash = item.kind == ItemKind::Clash;
    let line = format!("{}: {}.", s.name(item.owner), s.item_name(&item));
    s.note(line);
    let owner = item.owner;
    s.stack.push(item);
    s.deepest_chain = s.deepest_chain.max(s.stack.len() as u32);
    s.selecting = None;
    s.window += 1;
    if clash {
        // Before damage, the active player may act first.
        s.priority = s.active;
        s.passes = 0;
    } else {
        s.priority = other(owner);
        s.passes = 1;
    }
    if let Some(seat) = feint_by {
        let toll: u32 = s
            .player(other(seat))
            .charms
            .iter()
            .filter_map(|c| match card(*c).charm {
                Some(CharmEff::FeintToll(n)) => Some(n as u32),
                _ => None,
            })
            .sum();
        if toll > 0 {
            lose(s, seat, toll);
            let line = format!(
                "{} loses {toll} Resolve to the Whispering Gallery.",
                s.name(seat)
            );
            s.note(line);
            check_over(s);
        }
    }
}

fn pass(s: &mut State, seat: SeatId) {
    s.selecting = None;
    s.passes += 1;
    s.window += 1;
    if s.passes >= 2 {
        resolve_top(s);
        if s.phase == Phase::Main && !s.stack.is_empty() {
            s.priority = s.active;
            s.passes = 0;
            s.window += 1;
        }
    } else {
        s.priority = other(seat);
    }
}

fn resolve_top(s: &mut State) {
    let Some(item) = s.stack.pop() else {
        return;
    };
    let owner = item.owner;
    match item.kind.clone() {
        ItemKind::Card { card: id } => {
            let c = card(id);
            let target_ok = match item.target {
                Some(t) => s.target_ok(owner, c.target, t),
                None => !c.needs_target(),
            };
            if !target_ok {
                let line = format!("{} fizzles: its target is gone.", c.name);
                s.note(line);
                s.player_mut(owner).gutter.push(id);
                return;
            }
            let line = format!("{} resolves.", c.name);
            s.note(line);
            match c.kind {
                Kind::Ally => {
                    if !add_ally(s, owner, id) {
                        s.player_mut(owner).gutter.push(id);
                        return;
                    }
                }
                Kind::Charm => {
                    s.player_mut(owner).charms.push(id);
                    return;
                }
                Kind::Feint | Kind::Gambit => {}
            }
            for e in c.effects {
                if !apply(s, owner, *e, item.target) {
                    break;
                }
            }
            // Spent Feints and Gambits go to the Gutter after their effects,
            // so a Recall can't fetch the card doing the recalling.
            if matches!(c.kind, Kind::Feint | Kind::Gambit) {
                s.player_mut(owner).gutter.push(id);
            }
        }
        ItemKind::Ability { card: id, .. } => {
            let Some(ab) = card(id).ability else {
                return;
            };
            if let Some(t) = item.target {
                if !s.target_ok(owner, ab.target, t) {
                    let line = format!("{}'s ability fizzles.", card(id).name);
                    s.note(line);
                    return;
                }
            }
            for e in ab.effects {
                if !apply(s, owner, *e, item.target) {
                    return;
                }
            }
        }
        ItemKind::Strike { allies } => {
            let strikers: Vec<u32> = allies
                .into_iter()
                .filter(|u| s.ally(*u).is_some_and(|a| a.owner == owner))
                .collect();
            if strikers.is_empty() {
                s.note("The strike finds no one left to strike.".into());
                return;
            }
            s.combat = Some(Combat {
                strikers,
                blocks: Vec::new(),
            });
            s.phase = Phase::Blocking;
            s.window += 1;
            // Nothing can block: go straight to the Clash.
            if s.block_options().is_empty() {
                blocks_done(s);
            }
        }
        ItemKind::Clash => clash(s),
    }
}

fn clash(s: &mut State) {
    let Some(combat) = s.combat.take() else {
        return;
    };
    let attacker = s.active;
    let defender = other(attacker);
    let mut to_player = 0u32;
    let mut hits: Vec<(u32, u32)> = Vec::new();
    for su in &combat.strikers {
        let Some(striker) = s.ally(*su).filter(|a| a.owner == attacker) else {
            continue;
        };
        let sp = s.power(striker);
        match combat.blocks.iter().find(|(_, x)| x == su) {
            Some((bu, _)) => {
                if let Some(blocker) = s.ally(*bu) {
                    hits.push((*bu, sp));
                    hits.push((*su, s.power(blocker)));
                }
            }
            None => to_player += sp,
        }
    }
    for (u, n) in hits {
        hurt_ally(s, u, n);
    }
    if to_player > 0 {
        hurt_player(s, defender, to_player);
        let line = format!("{} takes {to_player} from the strike.", s.name(defender));
        s.note(line);
    }
    cleanup(s);
    check_over(s);
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// Read a target value relative to `seat`: `foe`, `me`, or `a<uid>`.
fn parse_target(seat: SeatId, v: &str) -> Option<Target> {
    match v {
        "foe" => Some(Target::Player(other(seat))),
        "me" => Some(Target::Player(seat)),
        _ => v
            .strip_prefix('a')
            .and_then(|n| n.parse().ok())
            .map(Target::Ally),
    }
}

fn parse_uid(v: &str) -> Option<u32> {
    v.strip_prefix('a').and_then(|n| n.parse().ok())
}

/// Strings from a single value or a list.
fn values(v: &Value) -> Vec<&str> {
    match v {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

type Act = Result<Vec<Event>, &'static str>;

fn act(s: &mut State, seat: SeatId, input: &PhoneInput) -> Act {
    let v = &input.value;
    match (s.phase, input.widget.as_str()) {
        (Phase::Building, "deck") => {
            if s.player(seat).built {
                return Err("already built");
            }
            let picks = values(v);
            let mut deck: Vec<CardId> = Vec::new();
            for p in picks {
                let id: CardId = p
                    .strip_prefix('c')
                    .and_then(|n| n.parse().ok())
                    .ok_or("bad card")?;
                if id as usize >= POOL_SIZE || deck.contains(&id) {
                    return Err("bad card");
                }
                deck.push(id);
            }
            if deck.len() != DECK_SIZE {
                return Err("a deck has 20 cards");
            }
            set_deck(s, seat, deck);
            Ok(vec![])
        }
        (Phase::Building, w) if w.starts_with("starter") => {
            if s.player(seat).built {
                return Err("already built");
            }
            let i: usize = w[7..].parse().map_err(|_| "bad starter")?;
            STARTERS.get(i).ok_or("bad starter")?;
            set_deck(s, seat, cards::starter(i));
            Ok(vec![])
        }
        (Phase::Main, "play") => {
            let id: CardId = v
                .as_str()
                .and_then(|x| x.strip_prefix('c'))
                .and_then(|n| n.parse().ok())
                .ok_or("bad card")?;
            if !s.can_play(seat, id) {
                return Err("can't play that now");
            }
            if card(id).needs_target() {
                s.selecting = Some(Source::Hand(id));
                return Ok(vec![]);
            }
            play_card(s, seat, id, None)
        }
        (Phase::Main, "ability") => {
            let uid = v.as_str().and_then(parse_uid).ok_or("bad ally")?;
            if !s.can_use(seat, uid) {
                return Err("can't use that now");
            }
            let needs = s
                .ally(uid)
                .and_then(|a| card(a.card).ability)
                .is_some_and(|ab| ab.target != Tgt::None);
            if needs {
                s.selecting = Some(Source::Ability(uid));
                return Ok(vec![]);
            }
            use_ability(s, seat, uid, None)
        }
        (Phase::Main, "target") => {
            let t = v
                .as_str()
                .and_then(|x| parse_target(seat, x))
                .ok_or("bad target")?;
            match s.selecting {
                Some(Source::Hand(id)) if s.can_play(seat, id) => {
                    if !s.target_ok(seat, card(id).target, t) {
                        return Err("bad target");
                    }
                    play_card(s, seat, id, Some(t))
                }
                Some(Source::Ability(uid)) if s.can_use(seat, uid) => {
                    let tg = s
                        .ally(uid)
                        .and_then(|a| card(a.card).ability)
                        .map_or(Tgt::None, |ab| ab.target);
                    if !s.target_ok(seat, tg, t) {
                        return Err("bad target");
                    }
                    use_ability(s, seat, uid, Some(t))
                }
                _ => Err("nothing to target"),
            }
        }
        (Phase::Main, "cancel") if s.selecting.is_some() && s.has_priority(seat) => {
            s.selecting = None;
            Ok(vec![])
        }
        (Phase::Main, "strike") => {
            let able = s.strikers(seat);
            let mut chosen: Vec<u32> = Vec::new();
            for x in values(v) {
                let uid = parse_uid(x).ok_or("bad ally")?;
                if !able.contains(&uid) || chosen.contains(&uid) {
                    return Err("can't strike with that");
                }
                chosen.push(uid);
            }
            if chosen.is_empty() {
                return Err("pick strikers");
            }
            for u in &chosen {
                let steadfast = s.ally(*u).is_some_and(|a| card(a.card).steadfast);
                if let Some(a) = s.ally_mut(*u) {
                    if !steadfast {
                        a.spent = true;
                    }
                }
            }
            s.struck = true;
            push(
                s,
                Item {
                    owner: seat,
                    kind: ItemKind::Strike { allies: chosen },
                    target: None,
                },
            );
            Ok(vec![Event::Sound {
                name: "strike".into(),
            }])
        }
        (Phase::Main, "end_turn") if s.stack.is_empty() && seat == s.active => {
            end_turn(s);
            Ok(vec![])
        }
        (Phase::Main, "pass") if !s.stack.is_empty() && seat == s.priority => {
            pass(s, seat);
            Ok(vec![])
        }
        (Phase::Blocking, "block") if seat == other(s.active) => {
            let x = v.as_str().ok_or("bad block")?;
            let (b, t) = x.split_once('>').ok_or("bad block")?;
            let pair = (
                parse_uid(b).ok_or("bad block")?,
                parse_uid(t).ok_or("bad block")?,
            );
            if !s.block_options().contains(&pair) {
                return Err("can't block that way");
            }
            if let Some(c) = &mut s.combat {
                c.blocks.push(pair);
            }
            Ok(vec![])
        }
        (Phase::Blocking, "unblock") if seat == other(s.active) => {
            if let Some(c) = &mut s.combat {
                c.blocks.clear();
            }
            Ok(vec![])
        }
        (Phase::Blocking, "blocks_done") if seat == other(s.active) => {
            blocks_done(s);
            Ok(vec![])
        }
        _ => Err("not now"),
    }
}

fn blocks_done(s: &mut State) {
    s.phase = Phase::Main;
    let n = s.combat.as_ref().map_or(0, |c| c.blocks.len());
    let line = format!("{} blocks with {n}.", s.name(other(s.active)));
    s.note(line);
    push(
        s,
        Item {
            owner: s.active,
            kind: ItemKind::Clash,
            target: None,
        },
    );
}

fn play_card(s: &mut State, seat: SeatId, id: CardId, target: Option<Target>) -> Act {
    let cost = s.cost(seat, id);
    let p = s.player_mut(seat);
    let Some(i) = p.hand.iter().position(|c| *c == id) else {
        return Err("not in hand");
    };
    p.hand.remove(i);
    p.spark = p.spark.saturating_sub(cost);
    push(
        s,
        Item {
            owner: seat,
            kind: ItemKind::Card { card: id },
            target,
        },
    );
    Ok(vec![Event::Toast {
        text: format!("{} plays {}", s.name(seat), card(id).name),
    }])
}

fn use_ability(s: &mut State, seat: SeatId, uid: u32, target: Option<Target>) -> Act {
    let Some(a) = s.ally(uid).cloned() else {
        return Err("no ally");
    };
    let Some(ab) = card(a.card).ability else {
        return Err("no ability");
    };
    let p = s.player_mut(seat);
    p.spark = p.spark.saturating_sub(ab.cost);
    if let Some(a) = s.ally_mut(uid) {
        a.spent = true;
    }
    push(
        s,
        Item {
            owner: seat,
            kind: ItemKind::Ability {
                card: a.card,
                ally: uid,
            },
            target,
        },
    );
    Ok(vec![])
}

fn waits(s: &State) -> Vec<Wait> {
    match s.phase {
        Phase::Over => Vec::new(),
        Phase::Building => (0..2)
            .filter(|seat| !s.player(*seat).built)
            .map(|seat| {
                let mut w = Wait::turn(seat, BUILD_LIVE_MS, ASYNC_MS);
                w.prompt = "Build your deck".into();
                w
            })
            .collect(),
        Phase::Blocking => {
            let seat = other(s.active);
            vec![Wait {
                seat,
                prompt: format!("Assign blockers (turn {})", s.turn),
                live_timeout_ms: BLOCK_LIVE_MS,
                async_timeout_ms: ASYNC_MS,
                on_timeout: OnTimeout::Input {
                    input: PhoneInput::press("blocks_done"),
                },
                reaction: false,
            }]
        }
        Phase::Main if s.stack.is_empty() => {
            let mut w = Wait::turn(s.active, TURN_LIVE_MS, ASYNC_MS);
            w.prompt = format!("Turn {}: your move", s.turn);
            vec![w]
        }
        Phase::Main => {
            let top = s.stack.last().map(|i| s.item_name(i)).unwrap_or_default();
            vec![Wait::reaction(
                s.priority,
                format!("Respond to {top}? (stack {}, #{})", s.stack.len(), s.window),
                REACT_LIVE_MS,
                ASYNC_MS,
                PhoneInput::press("pass"),
            )]
        }
    }
}

fn outcome(s: State, events: Vec<Event>) -> Outcome<State> {
    let mut o = Outcome::new(s);
    o.events = events;
    o.waiting_on = waits(&o.state);
    o.finished = o.state.result.clone();
    o
}

/// Run one input; illegal input leaves the state as it was.
pub fn step(state: State, seat: SeatId, input: &PhoneInput) -> Outcome<State> {
    if seat > 1 || state.phase == Phase::Over {
        return outcome(state, vec![]);
    }
    let mut next = state.clone();
    match act(&mut next, seat, input) {
        Ok(events) => outcome(next, events),
        Err(_) => outcome(
            state,
            vec![Event::Feedback {
                seat,
                kind: Feedback::Warning,
            }],
        ),
    }
}

/// A wait ran out for `seat`.
pub fn time_out(mut s: State, seat: SeatId) -> State {
    if seat > 1 {
        return s;
    }
    match s.phase {
        Phase::Building if !s.player(seat).built => {
            let i = s.rng.below(STARTERS.len() as u32) as usize;
            set_deck(&mut s, seat, cards::starter(i));
        }
        Phase::Main if s.stack.is_empty() && seat == s.active => end_turn(&mut s),
        Phase::Main if !s.stack.is_empty() && seat == s.priority => pass(&mut s, seat),
        Phase::Blocking if seat == other(s.active) => blocks_done(&mut s),
        _ => {}
    }
    s
}

impl Rules for Riposte {
    type State = State;

    fn seats(&self) -> (u32, u32) {
        (2, 2)
    }

    fn setup(&self, seats: &[Seat], seed: u64) -> RulesResult<Outcome<State>> {
        if seats.len() != 2 {
            return Err(RulesError::Game(
                "Riposte is for exactly two players".into(),
            ));
        }
        Ok(outcome(new_game(seats, seed), vec![]))
    }

    fn on_action(
        &self,
        state: State,
        seat: SeatId,
        input: &PhoneInput,
    ) -> RulesResult<Outcome<State>> {
        Ok(step(state, seat, input))
    }

    fn on_timeout(&self, state: State, seat: SeatId, _now_ms: u64) -> RulesResult<Outcome<State>> {
        Ok(outcome(time_out(state, seat), vec![]))
    }

    fn table_view(&self, state: &State) -> RulesResult<Value> {
        Ok(view::table(state).to_value())
    }

    fn phone_view(&self, state: &State, seat: SeatId) -> RulesResult<Widget> {
        Ok(view::phone(state, seat))
    }

    fn can_respond(&self, state: &State, seat: SeatId) -> RulesResult<bool> {
        Ok(seat <= 1 && state.can_respond_now(seat))
    }
}
