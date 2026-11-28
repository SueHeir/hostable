//! The sandbox level: a table of objects and no rules. Players move cards,
//! roll dice and keep score themselves, like at a real table, and the host
//! only keeps hands private.
//!
//! A sandbox pack's `table.json` lists the objects at the start
//! ([`TableDef`]). Every phone gets the same generic controller: a touchpad
//! that moves the player's cursor on the shared screen, a tap that picks up
//! whatever is under the cursor, buttons for what can be done with it, and
//! the player's own hand.

use hostable_protocol::{
    ButtonStyle, CardFace, ChoiceOption, ChoiceStyle, PhoneInput, PlayerPanel, Scene, Shape, Tab,
    TableView, Widget,
};
use hostable_rules_api::{
    Event, Outcome, Results, Rng, Rules, RulesError, RulesResult, Seat, SeatId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Most objects on a table, and most cards in all.
pub const MAX_OBJECTS: usize = 400;
pub const MAX_CARDS: usize = 2000;
/// How far one full touchpad swipe moves the cursor, as a share of the table.
const CURSOR_SPEED: f32 = 0.35;
const CARD_W: f32 = 70.0;
const CARD_H: f32 = 100.0;

/// One card's faces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardDef {
    pub id: String,
    /// What the face shows, e.g. "Q♥".
    pub face: String,
    /// Text colour of the face.
    #[serde(default)]
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectDef {
    /// A background area; never moves.
    Board {
        id: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        #[serde(default)]
        color: String,
        #[serde(default)]
        label: String,
    },
    /// A stack of cards, face down unless `face_up`.
    Deck {
        id: String,
        x: f32,
        y: f32,
        cards: Vec<CardDef>,
        #[serde(default)]
        face_up: bool,
        #[serde(default)]
        label: String,
    },
    /// A marked area cards can be put into (a discard pile, a play area).
    Zone {
        id: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        #[serde(default)]
        label: String,
    },
    Token {
        id: String,
        x: f32,
        y: f32,
        #[serde(default)]
        color: String,
        #[serde(default)]
        label: String,
    },
    Die {
        id: String,
        x: f32,
        y: f32,
        sides: u32,
        #[serde(default = "one")]
        value: u32,
    },
    Counter {
        id: String,
        x: f32,
        y: f32,
        #[serde(default)]
        value: i32,
        #[serde(default)]
        label: String,
    },
}

/// `table.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableDef {
    pub title: String,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub background: String,
    pub objects: Vec<ObjectDef>,
    /// Cards dealt to each hand when the game starts, from the first deck.
    #[serde(default)]
    pub deal: u32,
}

fn one() -> u32 {
    1
}

/// A card somewhere on the table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardOnTable {
    pub card: usize,
    pub face_up: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Thing {
    Board {
        w: f32,
        h: f32,
        color: String,
        label: String,
    },
    /// Decks and zones are both piles; a zone is drawn as an outline.
    Pile {
        cards: Vec<CardOnTable>,
        zone: Option<(f32, f32)>,
        label: String,
    },
    /// A single card lying on the table.
    Card(CardOnTable),
    Token {
        color: String,
        label: String,
    },
    Die {
        sides: u32,
        value: u32,
    },
    Counter {
        value: i32,
        label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub thing: Thing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Player {
    pub seat: Seat,
    pub hand: Vec<usize>,
    pub cursor: (f32, f32),
    /// Index of the object this player picked up.
    pub holding: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub title: String,
    pub width: f32,
    pub height: f32,
    pub background: String,
    pub cards: Vec<CardDef>,
    pub objects: Vec<Object>,
    pub players: Vec<Player>,
    pub rng: Rng,
    pub log: Vec<String>,
    pub next_id: u32,
    pub over: bool,
}

/// The sandbox engine for one table.
#[derive(Debug, Clone)]
pub struct Tabletop {
    def: TableDef,
    seats: (u32, u32),
}

impl Tabletop {
    pub fn new(def: TableDef, seats: (u32, u32)) -> Result<Self, String> {
        let cards: usize = def
            .objects
            .iter()
            .map(|o| match o {
                ObjectDef::Deck { cards, .. } => cards.len(),
                _ => 0,
            })
            .sum();
        if def.objects.len() > MAX_OBJECTS || cards > MAX_CARDS {
            return Err("too many objects or cards".into());
        }
        if !(def.width > 0.0 && def.height > 0.0 && def.width <= 10_000.0 && def.height <= 10_000.0)
        {
            return Err("table size must be between 0 and 10000".into());
        }
        let mut ids: Vec<&str> = def.objects.iter().map(object_id).collect();
        ids.sort_unstable();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err("object ids must be unique".into());
        }
        for o in &def.objects {
            if let ObjectDef::Die { sides, .. } = o {
                if !(2..=100).contains(sides) {
                    return Err("dice have 2 to 100 sides".into());
                }
            }
        }
        Ok(Self { def, seats })
    }

    /// From `table.json` bytes and the manifest's player counts.
    pub fn from_json(bytes: &[u8], players: impl Into<(u32, u32)>) -> Result<Self, String> {
        let def: TableDef = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        Self::new(def, players.into())
    }
}

fn object_id(o: &ObjectDef) -> &str {
    match o {
        ObjectDef::Board { id, .. }
        | ObjectDef::Deck { id, .. }
        | ObjectDef::Zone { id, .. }
        | ObjectDef::Token { id, .. }
        | ObjectDef::Die { id, .. }
        | ObjectDef::Counter { id, .. } => id,
    }
}

const SEAT_COLOURS: [&str; 8] = [
    "#e4572e", "#29335c", "#f3a712", "#669bbc", "#a8c686", "#8e44ad", "#16a085", "#d35400",
];

impl State {
    fn name(&self, seat: SeatId) -> String {
        self.players
            .get(seat as usize)
            .map(|p| p.seat.name.clone())
            .unwrap_or_default()
    }

    fn note(&mut self, text: String) {
        self.log.push(text);
        if self.log.len() > 12 {
            self.log.remove(0);
        }
    }

    /// The topmost movable object under a point.
    fn object_at(&self, x: f32, y: f32) -> Option<usize> {
        self.objects
            .iter()
            .enumerate()
            .rev()
            .find(|(_, o)| {
                let (w, h) = size(&o.thing);
                !matches!(o.thing, Thing::Board { .. })
                    && x >= o.x
                    && x <= o.x + w
                    && y >= o.y
                    && y <= o.y + h
            })
            .map(|(i, _)| i)
    }

    fn pile_at(&self, x: f32, y: f32, except: Option<usize>) -> Option<usize> {
        self.objects
            .iter()
            .enumerate()
            .rev()
            .find(|(i, o)| {
                let (w, h) = size(&o.thing);
                Some(*i) != except
                    && matches!(o.thing, Thing::Pile { .. })
                    && x >= o.x
                    && x <= o.x + w
                    && y >= o.y
                    && y <= o.y + h
            })
            .map(|(i, _)| i)
    }

    fn describe(&self, i: usize) -> String {
        let o = &self.objects[i];
        match &o.thing {
            Thing::Board { label, .. } => label.clone(),
            Thing::Pile { cards, label, zone } => {
                let what = if label.is_empty() {
                    if zone.is_some() {
                        "zone"
                    } else {
                        "deck"
                    }
                } else {
                    label
                };
                format!("{what} ({} cards)", cards.len())
            }
            Thing::Card(c) => {
                if c.face_up {
                    self.cards[c.card].face.clone()
                } else {
                    "a face-down card".into()
                }
            }
            Thing::Token { label, .. } => {
                if label.is_empty() {
                    "a token".into()
                } else {
                    label.clone()
                }
            }
            Thing::Die { sides, value } => format!("d{sides} showing {value}"),
            Thing::Counter { value, label } => format!("{label} {value}"),
        }
    }

    fn new_card_object(&mut self, card: CardOnTable, x: f32, y: f32) {
        self.next_id += 1;
        self.objects.push(Object {
            id: format!("card-{}", self.next_id),
            x: x.clamp(0.0, self.width - CARD_W),
            y: y.clamp(0.0, self.height - CARD_H),
            thing: Thing::Card(card),
        });
    }

    /// Take object `i` out, fixing up everyone's `holding`.
    fn remove_object(&mut self, i: usize) -> Object {
        let o = self.objects.remove(i);
        for p in &mut self.players {
            p.holding = match p.holding {
                Some(h) if h == i => None,
                Some(h) if h > i => Some(h - 1),
                other => other,
            };
        }
        o
    }
}

fn size(thing: &Thing) -> (f32, f32) {
    match thing {
        Thing::Board { w, h, .. } => (*w, *h),
        Thing::Pile {
            zone: Some((w, h)), ..
        } => (*w, *h),
        Thing::Pile { zone: None, .. } | Thing::Card(_) => (CARD_W, CARD_H),
        Thing::Token { .. } | Thing::Die { .. } => (40.0, 40.0),
        Thing::Counter { .. } => (90.0, 40.0),
    }
}

impl Rules for Tabletop {
    type State = State;

    fn seats(&self) -> (u32, u32) {
        self.seats
    }

    fn setup(&self, seats: &[Seat], seed: u64) -> RulesResult<Outcome<State>> {
        let def = &self.def;
        let mut cards = Vec::new();
        let mut objects = Vec::new();
        for o in &def.objects {
            let (id, x, y, thing) = match o {
                ObjectDef::Board {
                    id,
                    x,
                    y,
                    w,
                    h,
                    color,
                    label,
                } => (
                    id,
                    *x,
                    *y,
                    Thing::Board {
                        w: *w,
                        h: *h,
                        color: color.clone(),
                        label: label.clone(),
                    },
                ),
                ObjectDef::Deck {
                    id,
                    x,
                    y,
                    cards: defs,
                    face_up,
                    label,
                } => {
                    let start = cards.len();
                    cards.extend(defs.iter().cloned());
                    let pile = (start..cards.len())
                        .map(|card| CardOnTable {
                            card,
                            face_up: *face_up,
                        })
                        .collect();
                    (
                        id,
                        *x,
                        *y,
                        Thing::Pile {
                            cards: pile,
                            zone: None,
                            label: label.clone(),
                        },
                    )
                }
                ObjectDef::Zone {
                    id,
                    x,
                    y,
                    w,
                    h,
                    label,
                } => (
                    id,
                    *x,
                    *y,
                    Thing::Pile {
                        cards: vec![],
                        zone: Some((*w, *h)),
                        label: label.clone(),
                    },
                ),
                ObjectDef::Token {
                    id,
                    x,
                    y,
                    color,
                    label,
                } => (
                    id,
                    *x,
                    *y,
                    Thing::Token {
                        color: color.clone(),
                        label: label.clone(),
                    },
                ),
                ObjectDef::Die {
                    id,
                    x,
                    y,
                    sides,
                    value,
                } => (
                    id,
                    *x,
                    *y,
                    Thing::Die {
                        sides: *sides,
                        value: (*value).clamp(1, *sides),
                    },
                ),
                ObjectDef::Counter {
                    id,
                    x,
                    y,
                    value,
                    label,
                } => (
                    id,
                    *x,
                    *y,
                    Thing::Counter {
                        value: *value,
                        label: label.clone(),
                    },
                ),
            };
            objects.push(Object {
                id: id.clone(),
                x,
                y,
                thing,
            });
        }
        let mut state = State {
            title: def.title.clone(),
            width: def.width,
            height: def.height,
            background: def.background.clone(),
            cards,
            objects,
            players: seats
                .iter()
                .enumerate()
                .map(|(i, s)| Player {
                    seat: s.clone(),
                    hand: vec![],
                    cursor: (
                        def.width * (i as f32 + 1.0) / (seats.len() as f32 + 1.0),
                        def.height * 0.85,
                    ),
                    holding: None,
                })
                .collect(),
            rng: Rng::new(seed),
            log: vec![],
            next_id: 0,
            over: false,
        };
        // Shuffle face-down decks, then deal.
        for i in 0..state.objects.len() {
            let mut rng = state.rng.clone();
            if let Thing::Pile {
                cards, zone: None, ..
            } = &mut state.objects[i].thing
            {
                if cards.iter().all(|c| !c.face_up) {
                    rng.shuffle(cards);
                }
            }
            state.rng = rng;
        }
        if def.deal > 0 {
            if let Some(deck) = state
                .objects
                .iter()
                .position(|o| matches!(o.thing, Thing::Pile { zone: None, .. }))
            {
                for _ in 0..def.deal {
                    for p in 0..state.players.len() {
                        if let Thing::Pile { cards, .. } = &mut state.objects[deck].thing {
                            if let Some(c) = cards.pop() {
                                state.players[p].hand.push(c.card);
                            }
                        }
                    }
                }
            }
        }
        state.note("The table is set.".into());
        Ok(Outcome::new(state))
    }

    fn on_action(
        &self,
        mut state: State,
        seat: SeatId,
        input: &PhoneInput,
    ) -> RulesResult<Outcome<State>> {
        let s = seat as usize;
        if s >= state.players.len() {
            return Err(RulesError::Game(format!("no seat {seat}")));
        }
        if state.over {
            return Ok(Outcome::new(state));
        }
        let who = state.name(seat);
        let mut out_events = Vec::new();
        match input.widget.as_str() {
            "pad" => {
                if input.value.get("tap") == Some(&Value::Bool(true)) {
                    let (x, y) = state.players[s].cursor;
                    let held = state.object_at(x, y);
                    state.players[s].holding = held;
                } else {
                    let dx = input.value.get("dx").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                    let dy = input.value.get("dy").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                    let (w, h) = (state.width, state.height);
                    let c = &mut state.players[s].cursor;
                    c.0 = (c.0 + dx * w * CURSOR_SPEED).clamp(0.0, w);
                    c.1 = (c.1 + dy * h * CURSOR_SPEED).clamp(0.0, h);
                }
            }
            "pick" => {
                let i = input
                    .value
                    .as_str()
                    .and_then(|v| state.objects.iter().position(|o| o.id == v));
                state.players[s].holding = i;
                if let Some(i) = i {
                    let o = &state.objects[i];
                    state.players[s].cursor = (o.x + 10.0, o.y + 10.0);
                }
            }
            "drop" => state.players[s].holding = None,
            "move_here" => {
                if let Some(i) = state.players[s].holding {
                    let (x, y) = state.players[s].cursor;
                    let o = &state.objects[i];
                    let is_card = matches!(o.thing, Thing::Card(_));
                    let target = if is_card {
                        state.pile_at(x, y, Some(i))
                    } else {
                        None
                    };
                    match target {
                        Some(pile) => {
                            let Thing::Card(card) = state.remove_object(i).thing else {
                                return Err(RulesError::Game("held object is not a card".into()));
                            };
                            let pile = if pile > i { pile - 1 } else { pile };
                            if let Thing::Pile { cards, .. } = &mut state.objects[pile].thing {
                                cards.push(card);
                            }
                            let d = state.describe(pile);
                            state.note(format!("{who} put a card on {d}"));
                        }
                        None => {
                            let (w, h) = size(&state.objects[i].thing);
                            let (tw, th) = (state.width, state.height);
                            let o = &mut state.objects[i];
                            o.x = (x - w / 2.0).clamp(0.0, (tw - w).max(0.0));
                            o.y = (y - h / 2.0).clamp(0.0, (th - h).max(0.0));
                            // Moved things go on top.
                            let o = state.remove_object(i);
                            state.objects.push(o);
                            state.players[s].holding = Some(state.objects.len() - 1);
                        }
                    }
                }
            }
            "flip" => {
                if let Some(i) = state.players[s].holding {
                    match &mut state.objects[i].thing {
                        Thing::Card(c) => c.face_up = !c.face_up,
                        Thing::Pile { cards, .. } => {
                            cards.reverse();
                            for c in cards.iter_mut() {
                                c.face_up = !c.face_up;
                            }
                        }
                        _ => {}
                    }
                    let d = state.describe(i);
                    state.note(format!("{who} flipped {d}"));
                }
            }
            "roll" => {
                if let Some(i) = state.players[s].holding {
                    let mut rng = state.rng.clone();
                    if let Thing::Die { sides, value } = &mut state.objects[i].thing {
                        *value = rng.die(*sides);
                        out_events.push(Event::Sound {
                            name: "dice".into(),
                        });
                    }
                    state.rng = rng;
                    let d = state.describe(i);
                    state.note(format!("{who} rolled {d}"));
                }
            }
            "shuffle" => {
                if let Some(i) = state.players[s].holding {
                    let mut rng = state.rng.clone();
                    if let Thing::Pile { cards, .. } = &mut state.objects[i].thing {
                        rng.shuffle(cards);
                        for c in cards.iter_mut() {
                            c.face_up = false;
                        }
                    }
                    state.rng = rng;
                    state.note(format!("{who} shuffled"));
                }
            }
            "draw" => {
                if let Some(i) = state.players[s].holding {
                    if let Thing::Pile { cards, .. } = &mut state.objects[i].thing {
                        if let Some(c) = cards.pop() {
                            state.players[s].hand.push(c.card);
                            state.note(format!("{who} drew a card"));
                        }
                    }
                }
            }
            "take_top" => {
                if let Some(i) = state.players[s].holding {
                    let (x, y) = state.players[s].cursor;
                    let top = match &mut state.objects[i].thing {
                        Thing::Pile { cards, .. } => cards.pop(),
                        _ => None,
                    };
                    if let Some(c) = top {
                        state.new_card_object(c, x + 20.0, y + 20.0);
                        state.players[s].holding = Some(state.objects.len() - 1);
                    }
                }
            }
            "deal" => {
                if let Some(i) = state.players[s].holding {
                    for p in 0..state.players.len() {
                        if let Thing::Pile { cards, .. } = &mut state.objects[i].thing {
                            if let Some(c) = cards.pop() {
                                state.players[p].hand.push(c.card);
                            }
                        }
                    }
                    state.note(format!("{who} dealt a card to everyone"));
                }
            }
            "plus" | "minus" => {
                if let Some(i) = state.players[s].holding {
                    if let Thing::Counter { value, .. } = &mut state.objects[i].thing {
                        let step = if input.widget == "plus" { 1 } else { -1 };
                        *value = value.saturating_add(step).clamp(-9999, 9999);
                    }
                    let d = state.describe(i);
                    state.note(format!("{who} set {d}"));
                }
            }
            "play_up" | "play_down" => {
                let pos = input
                    .value
                    .as_str()
                    .and_then(|v| v.parse::<usize>().ok())
                    .and_then(|card| state.players[s].hand.iter().position(|&c| c == card));
                if let Some(pos) = pos {
                    let card = state.players[s].hand.remove(pos);
                    let face_up = input.widget == "play_up";
                    let (x, y) = state.players[s].cursor;
                    match state.pile_at(x, y, None) {
                        Some(pile) => {
                            if let Thing::Pile { cards, .. } = &mut state.objects[pile].thing {
                                cards.push(CardOnTable { card, face_up });
                            }
                        }
                        None => state.new_card_object(
                            CardOnTable { card, face_up },
                            x - CARD_W / 2.0,
                            y - CARD_H / 2.0,
                        ),
                    }
                    let what = if face_up {
                        state.cards[card].face.clone()
                    } else {
                        "a card face down".into()
                    };
                    state.note(format!("{who} played {what}"));
                }
            }
            "end_game" => {
                state.over = true;
                state.note(format!("{who} ended the game"));
                let all: Vec<SeatId> = (0..state.players.len() as SeatId).collect();
                let mut out = Outcome::new(state);
                out.finished = Some(Results {
                    ranking: vec![all],
                    summary: "Game over".into(),
                });
                return Ok(out);
            }
            _ => {}
        }
        let mut out = Outcome::new(state);
        out.events = out_events;
        Ok(out)
    }

    fn table_view(&self, state: &State) -> RulesResult<Value> {
        let mut shapes = Vec::new();
        for (i, o) in state.objects.iter().enumerate() {
            let (w, h) = size(&o.thing);
            match &o.thing {
                Thing::Board { color, label, .. } => shapes.push(Shape::Rect {
                    x: o.x,
                    y: o.y,
                    w,
                    h,
                    fill: or(color, "#2d6a4f"),
                    stroke: String::new(),
                    label: label.clone(),
                }),
                Thing::Pile { cards, zone, label } => {
                    let top = cards.last().map(|c| {
                        if c.face_up {
                            state.cards[c.card].face.clone()
                        } else {
                            String::new()
                        }
                    });
                    let fill = match (zone, &top) {
                        (_, Some(t)) if !t.is_empty() => "#fdfdf8",
                        (None, Some(_)) => "#7a2e2e",
                        _ => "#00000022",
                    };
                    shapes.push(Shape::Rect {
                        x: o.x,
                        y: o.y,
                        w,
                        h,
                        fill: fill.into(),
                        stroke: "#ffffff".into(),
                        label: top.unwrap_or_default(),
                    });
                    shapes.push(Shape::Text {
                        x: o.x,
                        y: o.y + h + 14.0,
                        text: format!(
                            "{}{}",
                            if label.is_empty() { "" } else { label },
                            if cards.is_empty() {
                                String::new()
                            } else {
                                format!(" ({})", cards.len())
                            }
                        ),
                        size: 13.0,
                        color: "#ffffff".into(),
                    });
                }
                Thing::Card(c) => shapes.push(Shape::Rect {
                    x: o.x,
                    y: o.y,
                    w,
                    h,
                    fill: if c.face_up { "#fdfdf8" } else { "#7a2e2e" }.into(),
                    stroke: "#222222".into(),
                    label: if c.face_up {
                        state.cards[c.card].face.clone()
                    } else {
                        String::new()
                    },
                }),
                Thing::Token { color, label } => shapes.push(Shape::Circle {
                    x: o.x + w / 2.0,
                    y: o.y + h / 2.0,
                    r: w / 2.0,
                    fill: or(color, "#f3a712"),
                    stroke: "#222222".into(),
                    label: label.clone(),
                }),
                Thing::Die { value, .. } => shapes.push(Shape::Rect {
                    x: o.x,
                    y: o.y,
                    w,
                    h,
                    fill: "#ffffff".into(),
                    stroke: "#222222".into(),
                    label: value.to_string(),
                }),
                Thing::Counter { value, label } => shapes.push(Shape::Rect {
                    x: o.x,
                    y: o.y,
                    w,
                    h,
                    fill: "#1b263b".into(),
                    stroke: "#ffffff".into(),
                    label: format!("{label} {value}"),
                }),
            }
            for p in &state.players {
                if p.holding == Some(i) {
                    shapes.push(Shape::Rect {
                        x: o.x - 3.0,
                        y: o.y - 3.0,
                        w: w + 6.0,
                        h: h + 6.0,
                        fill: "#00000000".into(),
                        stroke: seat_colour(&p.seat),
                        label: String::new(),
                    });
                }
            }
        }
        for p in &state.players {
            shapes.push(Shape::Circle {
                x: p.cursor.0,
                y: p.cursor.1,
                r: 9.0,
                fill: seat_colour(&p.seat),
                stroke: "#ffffff".into(),
                label: String::new(),
            });
            shapes.push(Shape::Text {
                x: p.cursor.0 + 12.0,
                y: p.cursor.1 - 10.0,
                text: p.seat.name.clone(),
                size: 14.0,
                color: "#ffffff".into(),
            });
        }
        let mut view = TableView::new(state.title.clone());
        view.status = if state.over {
            "Game over".into()
        } else {
            "Free play".into()
        };
        view.scene = Some(Scene {
            width: state.width,
            height: state.height,
            background: or(&state.background, "#1e5133"),
            shapes,
        });
        view.players = state
            .players
            .iter()
            .map(|p| PlayerPanel {
                name: p.seat.name.clone(),
                color: seat_colour(&p.seat),
                lines: vec![format!("{} cards in hand", p.hand.len())],
                active: p.holding.is_some(),
            })
            .collect();
        view.log = state.log.clone();
        Ok(view.to_value())
    }

    fn phone_view(&self, state: &State, seat: SeatId) -> RulesResult<Widget> {
        let Some(p) = state.players.get(seat as usize) else {
            return Ok(Widget::text("You're watching."));
        };
        if state.over {
            return Ok(Widget::title("Game over"));
        }
        let mut table = vec![Widget::Touchpad { id: "pad".into() }];
        match p.holding {
            Some(i) => {
                table.push(Widget::text(format!("Holding {}", state.describe(i))));
                let mut buttons = vec![
                    Widget::button("move_here", "Move to cursor", ButtonStyle::Primary),
                    Widget::button("drop", "Let go", ButtonStyle::Normal),
                ];
                match &state.objects[i].thing {
                    Thing::Card(_) => {
                        buttons.push(Widget::button("flip", "Flip", ButtonStyle::Normal))
                    }
                    Thing::Pile { cards, .. } if !cards.is_empty() => {
                        buttons.push(Widget::button("draw", "Draw to hand", ButtonStyle::Normal));
                        buttons.push(Widget::button(
                            "take_top",
                            "Take top card",
                            ButtonStyle::Normal,
                        ));
                        buttons.push(Widget::button("shuffle", "Shuffle", ButtonStyle::Normal));
                        buttons.push(Widget::button("deal", "Deal one each", ButtonStyle::Normal));
                        buttons.push(Widget::button("flip", "Flip pile", ButtonStyle::Normal));
                    }
                    Thing::Die { .. } => {
                        buttons.push(Widget::button("roll", "Roll", ButtonStyle::Primary))
                    }
                    Thing::Counter { .. } => {
                        buttons.push(Widget::button("plus", "+1", ButtonStyle::Normal));
                        buttons.push(Widget::button("minus", "−1", ButtonStyle::Normal));
                    }
                    _ => {}
                }
                table.push(Widget::Grid {
                    columns: 2,
                    children: buttons,
                });
            }
            None => table.push(Widget::muted(
                "Move your cursor with the pad and tap to pick something up.",
            )),
        }
        let pickable: Vec<ChoiceOption> = state
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| !matches!(o.thing, Thing::Board { .. }))
            .take(60)
            .map(|(i, o)| ChoiceOption::new(o.id.clone(), state.describe(i)))
            .collect();
        if !pickable.is_empty() {
            table.push(Widget::Sheet {
                label: "Pick from a list".into(),
                children: vec![Widget::choice("pick", ChoiceStyle::List, pickable)],
            });
        }
        let hand_cards: Vec<CardFace> = p
            .hand
            .iter()
            .take(100)
            .map(|&c| CardFace {
                value: c.to_string(),
                title: state.cards[c].face.clone(),
                text: String::new(),
                corner: String::new(),
                color: state.cards[c].color.clone(),
                image: None,
                disabled: false,
            })
            .collect();
        let hand = if hand_cards.is_empty() {
            vec![Widget::muted("Your hand is empty.")]
        } else {
            vec![
                Widget::muted("Play a card to where your cursor is:"),
                Widget::Hand {
                    id: "play_up".into(),
                    cards: hand_cards.clone(),
                    max: 1,
                    min: 1,
                },
                Widget::Sheet {
                    label: "Play face down".into(),
                    children: vec![Widget::Hand {
                        id: "play_down".into(),
                        cards: hand_cards,
                        max: 1,
                        min: 1,
                    }],
                },
            ]
        };
        Ok(Widget::Tabs {
            tabs: vec![
                Tab {
                    label: "Table".into(),
                    children: table,
                },
                Tab {
                    label: format!("Hand ({})", p.hand.len()),
                    children: hand,
                },
                Tab {
                    label: "More".into(),
                    children: vec![Widget::button(
                        "end_game",
                        "End the game for everyone",
                        ButtonStyle::Danger,
                    )],
                },
            ],
        })
    }
}

fn or(s: &str, default: &str) -> String {
    if s.is_empty() {
        default.into()
    } else {
        s.into()
    }
}

fn seat_colour(seat: &Seat) -> String {
    if seat.color.is_empty() {
        SEAT_COLOURS[seat.id as usize % SEAT_COLOURS.len()].into()
    } else {
        seat.color.clone()
    }
}

#[cfg(test)]
mod tests;
