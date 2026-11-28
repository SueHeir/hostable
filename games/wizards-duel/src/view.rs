//! Phone screens and the shared table.

use std::collections::{BTreeSet, HashMap};

use hostable_sdk::prelude::*;
use wizards_rules::{
    Aim, Effect, Game, Mana, Occupant, Piece, SpellDef, Status, Target, Terrain, Vertex,
};

use crate::arena::{self, mana_color, mana_name};
use crate::{claim_value, hex_value, vertex_value, State};

/// An eight-point compass direction from `from` to `to` (x right, y up).
pub fn compass(from: (f32, f32), to: (f32, f32)) -> &'static str {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    if dx.abs() < 1e-4 && dy.abs() < 1e-4 {
        return "here";
    }
    let sector = (dy.atan2(dx).to_degrees() / 45.0).round() as i32;
    ["E", "NE", "N", "NW", "W", "SW", "S", "SE"][sector.rem_euclid(8) as usize]
}

/// Lasting conditions, e.g. "shield 1, rooted 2". Empty when none.
pub fn describe_status(s: &Status) -> String {
    let mut parts = Vec::new();
    if s.shield > 0 {
        parts.push(format!("shield {}", s.shield));
    }
    if s.stunned > 0 {
        parts.push(format!("stunned {}", s.stunned));
    }
    if s.rooted > 0 {
        parts.push(format!("rooted {}", s.rooted));
    }
    if s.burn_turns > 0 && s.burn > 0 {
        parts.push(format!("burning {} for {}", s.burn, s.burn_turns));
    }
    parts.join(", ")
}

fn cost_text(def: &SpellDef) -> String {
    def.cost
        .iter()
        .map(|m| mana_name(*m))
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Enemies of the current wizard that `def` aimed at `aim` would hurt.
/// Follows the spell's effects the way `wizards-rules` resolves them: a
/// single target takes harmful effects directly, an `Area` hurts enemies
/// within its radius, and a `MoveCaster` before an `Area` centres it where
/// the caster lands.
pub fn hits(game: &Game, def: &SpellDef, aim: Aim) -> Vec<usize> {
    let me = game.current;
    let verts = match aim {
        Aim::Vertex(v) => vec![v],
        Aim::Hex(h) => h.corners().to_vec(),
    };
    let mut out = BTreeSet::new();
    collect_hits(game, me, &def.effects, &verts, &mut out);
    out.into_iter().collect()
}

fn collect_hits(
    game: &Game,
    me: usize,
    effects: &[Effect],
    verts: &[Vertex],
    out: &mut BTreeSet<usize>,
) {
    for e in effects {
        match e {
            Effect::Area { radius, effects } => {
                let around: Vec<Vertex> = game
                    .players
                    .iter()
                    .enumerate()
                    .filter(|(i, p)| *i != me && p.hp > 0)
                    .map(|(_, p)| p.at)
                    .filter(|&at| {
                        verts
                            .iter()
                            .any(|&v| game.board.distance(v, at).is_some_and(|d| d <= *radius))
                    })
                    .collect();
                collect_hits(game, me, effects, &around, out);
            }
            Effect::Repeat { effects, .. } => collect_hits(game, me, effects, verts, out),
            Effect::OnCaster(_) => {}
            e if e.is_harmful() => {
                for &v in verts {
                    if let Some(Piece::Player(i)) = game.piece_at(v) {
                        if i != me {
                            out.insert(i);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Where the current wizard could aim `def`: the same list as
/// `Game::spell_targets`, but sharing one search of walking distances from
/// the caster (`dist`, from `distances_from`) between every spell on the
/// screen. In the sandbox each search costs millions of instructions, so
/// one per screen instead of one per spell matters. Target kinds this
/// doesn't cover go to `wizards-rules`. A test checks the two agree.
pub fn targets(game: &Game, def: &SpellDef, dist: &HashMap<Vertex, u32>) -> Vec<Aim> {
    let at = game.players[game.current].at;
    let mut out: Vec<Aim> = match &def.target {
        Target::Caster => vec![Aim::Vertex(at)],
        Target::Distance { min, max, occupant } => dist
            .iter()
            .filter(|(_, d)| (*min..=*max).contains(*d))
            .map(|(v, _)| *v)
            .filter(|&v| game.board.has_vertex(v))
            .filter(|&v| match occupant {
                Occupant::Empty => !game.board.is_rock(v) && game.piece_at(v).is_none(),
                Occupant::Occupied => game.piece_at(v).is_some(),
                Occupant::Any => true,
                Occupant::Rock => game.board.is_rock(v),
            })
            .map(Aim::Vertex)
            .collect(),
        Target::Hex { min, max } => game
            .board
            .def
            .hexes
            .iter()
            .filter(|h| {
                h.hex
                    .corners()
                    .into_iter()
                    .filter_map(|c| dist.get(&c).copied())
                    .min()
                    .is_some_and(|d| (*min..=*max).contains(&d))
            })
            .map(|h| Aim::Hex(h.hex))
            .collect(),
        _ => return game.spell_targets(&def.id),
    };
    out.sort();
    out
}

fn names(state: &State, seats: &[usize]) -> String {
    seats
        .iter()
        .map(|&i| state.name(i as u32))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Labels that are the same get a number, so every option reads apart.
fn number_duplicates(options: &mut [ChoiceOption]) {
    let mut seen: HashMap<String, u32> = HashMap::new();
    let counts: HashMap<String, u32> = options.iter().fold(HashMap::new(), |mut m, o| {
        *m.entry(o.label.clone()).or_default() += 1;
        m
    });
    for o in options.iter_mut() {
        if counts[&o.label] > 1 {
            let n = seen.entry(o.label.clone()).or_default();
            *n += 1;
            o.label = format!("{} ({n})", o.label);
        }
    }
}

fn hp_meter(hp: u32, max: u32) -> Widget {
    Widget::Meter {
        label: "HP".into(),
        value: hp as i32,
        max: max as i32,
        style: MeterStyle::Health,
    }
}

/// The wizard's own mana, for their phone only.
fn mana_lines(game: &Game) -> Vec<Widget> {
    let p = &game.players[game.current];
    let at = p.at.position();
    let here: Vec<String> = game
        .board
        .mana_near(p.at)
        .into_iter()
        .map(|(h, m)| {
            let spent = if p.board_used.contains(&h) {
                ", spent"
            } else {
                ""
            };
            format!("{} ({}{spent})", mana_name(m), compass(at, h.center()))
        })
        .collect();
    let claimed: Vec<String> = p
        .claimed
        .iter()
        .map(|c| {
            if c.used {
                format!("{} (spent)", mana_name(c.mana))
            } else {
                mana_name(c.mana).to_string()
            }
        })
        .collect();
    vec![
        Widget::text(if here.is_empty() {
            "No board mana here.".to_string()
        } else {
            format!("Board mana here: {}", here.join(", "))
        }),
        Widget::text(if claimed.is_empty() {
            "Your mana: none claimed yet.".to_string()
        } else {
            format!("Your mana: {}", claimed.join(", "))
        }),
    ]
}

fn own_lines(state: &State, seat: usize) -> Vec<Widget> {
    let w = &state.wizards[seat];
    let mut out = vec![hp_meter(w.hp, w.max_hp)];
    let status = describe_status(&w.status);
    if !status.is_empty() {
        out.push(Widget::text(format!("You are {status}.")));
    }
    let spells: Vec<String> = w.spells.iter().map(|s| arena::spell_name(s)).collect();
    out.push(Widget::muted(format!("Your spells: {}", spells.join(", "))));
    let mana: Vec<&str> = w.claimed.iter().map(|c| mana_name(c.mana)).collect();
    out.push(Widget::muted(if mana.is_empty() {
        "Your mana: none claimed yet.".to_string()
    } else {
        format!("Your mana: {}", mana.join(", "))
    }));
    out
}

/// What one seat's phone shows.
pub fn phone(state: &State, seat: SeatId) -> Widget {
    let i = seat as usize;
    let Some(me) = state.wizards.get(i) else {
        return Widget::column(vec![Widget::title("Watching the duel")]);
    };
    if state.over {
        let title = match state.winner {
            Some(w) if w == seat => "You win the duel!".to_string(),
            Some(w) => format!("{} wins the duel", state.name(w)),
            None => "A draw".to_string(),
        };
        let mut children = vec![Widget::title(title)];
        children.extend(own_lines(state, i));
        return Widget::column(children);
    }
    if !me.alive() {
        return Widget::column(vec![
            Widget::title("You were defeated"),
            Widget::text(format!("{}'s turn.", state.name(state.current as u32))),
        ]);
    }
    if i != state.current {
        let mut children = vec![Widget::title(format!(
            "{}'s turn",
            state.name(state.current as u32)
        ))];
        children.extend(own_lines(state, i));
        return Widget::column(children);
    }
    your_turn(state)
}

fn your_turn(state: &State) -> Widget {
    let game = state.game();
    let me = &game.players[game.current];
    let mut children = vec![
        Widget::title(format!("Your turn · round {}", state.turn)),
        hp_meter(me.hp, me.max_hp),
    ];
    let status = describe_status(&me.status);
    children.push(Widget::text(format!(
        "Steps left: {}{}",
        me.steps_left,
        if status.is_empty() {
            String::new()
        } else {
            format!(" · you are {status}")
        }
    )));
    children.extend(mana_lines(&game));
    if let Some(spell) = &state.aiming {
        children.extend(aim_screen(state, &game, spell));
        return Widget::column(children);
    }
    if me.status.stunned > 0 {
        children.push(Widget::styled(
            "You are stunned: you can only end your turn.",
            TextStyle::Warning,
        ));
    }
    if me.steps_left > 0 && me.status.stunned == 0 && me.status.rooted == 0 {
        if let Some(w) = move_choice(state, &game) {
            children.push(w);
        }
    }
    children.push(cast_choice(state, &game));
    children.push(end_choice(&game));
    Widget::column(children)
}

fn choice(id: &str, label: &str, options: Vec<ChoiceOption>) -> Widget {
    Widget::Choice {
        id: id.into(),
        style: ChoiceStyle::List,
        options,
        max: 1,
        min: 1,
        label: label.into(),
    }
}

/// Walking distances from every living rival.
fn rival_distances(game: &Game) -> Vec<(usize, HashMap<Vertex, u32>)> {
    game.players
        .iter()
        .enumerate()
        .filter(|(i, p)| *i != game.current && p.hp > 0)
        .map(|(i, p)| (i, game.board.distances_from(p.at)))
        .collect()
}

/// The nearest rival to `v` and how far away they are.
fn nearest(rivals: &[(usize, HashMap<Vertex, u32>)], v: Vertex) -> Option<(usize, u32)> {
    rivals
        .iter()
        .filter_map(|(i, d)| d.get(&v).map(|&d| (*i, d)))
        .min_by_key(|&(i, d)| (d, i))
}

fn move_choice(state: &State, game: &Game) -> Option<Widget> {
    let me = &game.players[game.current];
    let rivals = rival_distances(game);
    let now = nearest(&rivals, me.at);
    let mut steps: Vec<Vertex> = game
        .board
        .neighbors(me.at)
        .into_iter()
        .filter(|&v| !game.is_blocked(v))
        .collect();
    steps.sort();
    if steps.is_empty() {
        return None;
    }
    let from = me.at.position();
    let options = steps
        .into_iter()
        .map(|v| {
            let mut detail = Vec::new();
            if let (Some((_, before)), Some((who, after))) = (now, nearest(&rivals, v)) {
                let word = match after.cmp(&before) {
                    std::cmp::Ordering::Less => "closer to",
                    std::cmp::Ordering::Greater => "farther from",
                    std::cmp::Ordering::Equal => "same distance from",
                };
                detail.push(format!("{word} {} ({after} away)", state.name(who as u32)));
            }
            let fresh: Vec<&str> = game
                .board
                .mana_near(v)
                .into_iter()
                .filter(|(h, _)| !me.claimed.iter().any(|c| c.from == *h))
                .map(|(_, m)| mana_name(m))
                .collect();
            if !fresh.is_empty() {
                detail.push(format!("new mana: {}", fresh.join(", ")));
            }
            ChoiceOption::new(
                vertex_value(v),
                format!("Step {}", compass(from, v.position())),
            )
            .detail(detail.join(" · "))
        })
        .collect();
    let label = format!("Move ({} steps left)", me.steps_left);
    Some(choice("move", &label, options))
}

fn cast_choice(state: &State, game: &Game) -> Widget {
    let me = &game.players[game.current];
    let dist = game.board.distances_from(me.at);
    let book = &game.book;
    let options = me
        .spells
        .iter()
        .filter_map(|id| book.get(id).map(|def| (id, def)))
        .map(|(id, def)| {
            // `castable` without working out the targets twice.
            let stunned = me.status.stunned > 0;
            let payable = !stunned && game.auto_pay(id).is_some();
            let targets = if payable {
                targets(game, def, &dist)
            } else {
                Vec::new()
            };
            let can = payable && !targets.is_empty();
            let mut detail = format!("{} · {}", cost_text(def), def.text);
            if can {
                let mut hit = BTreeSet::new();
                for aim in targets {
                    hit.extend(hits(game, def, aim));
                }
                if !hit.is_empty() {
                    let hit: Vec<usize> = hit.into_iter().collect();
                    detail.push_str(&format!(" · can hit {}", names(state, &hit)));
                }
            } else if stunned {
                detail.push_str(" · stunned");
            } else if !payable {
                detail.push_str(" · not enough mana");
            } else {
                detail.push_str(" · nothing in reach");
            }
            ChoiceOption::new(id.clone(), def.name.clone())
                .detail(detail)
                .disabled(!can)
        })
        .collect();
    choice("cast", "Cast a spell", options)
}

fn end_choice(game: &Game) -> Widget {
    let me = &game.players[game.current];
    let at = me.at.position();
    let mut options = vec![ChoiceOption::new("end", "End turn")];
    for (h, m) in game.board.mana_near(me.at) {
        if !me.claimed.iter().any(|c| c.from == h) {
            options.push(
                ChoiceOption::new(
                    claim_value(h),
                    format!(
                        "Claim {} ({}) and end turn",
                        mana_name(m),
                        compass(at, h.center())
                    ),
                )
                .detail("Claimed mana is yours to spend once every turn"),
            );
        }
    }
    choice("end", "End turn", options)
}

fn aim_screen(state: &State, game: &Game, spell: &str) -> Vec<Widget> {
    let Some(def) = game.book.get(spell) else {
        return vec![Widget::button("cancel", "Back", ButtonStyle::Normal)];
    };
    let me = &game.players[game.current];
    let from = me.at.position();
    let dist = game.board.distances_from(me.at);
    let mut options: Vec<ChoiceOption> = targets(game, def, &dist)
        .into_iter()
        .map(|aim| {
            let (value, label) = match aim {
                Aim::Vertex(v) if v == me.at => (vertex_value(v), "Yourself".to_string()),
                Aim::Vertex(v) => {
                    let d = dist.get(&v).copied().unwrap_or(0);
                    let dir = compass(from, v.position());
                    let who = match game.piece_at(v) {
                        Some(Piece::Player(i)) => state.name(i as u32),
                        Some(Piece::Monster(_)) => "Monster".into(),
                        None => "Empty spot".into(),
                    };
                    (vertex_value(v), format!("{who} — {d} away, {dir}"))
                }
                Aim::Hex(h) => {
                    let d = h
                        .corners()
                        .into_iter()
                        .filter_map(|c| dist.get(&c).copied())
                        .min()
                        .unwrap_or(0);
                    (
                        hex_value(h),
                        format!("Hex {d} away, {}", compass(from, h.center())),
                    )
                }
            };
            let hit = hits(game, def, aim);
            let detail = if hit.is_empty() {
                String::new()
            } else {
                format!("hits {}", names(state, &hit))
            };
            ChoiceOption::new(value, label).detail(detail)
        })
        .collect();
    number_duplicates(&mut options);
    vec![
        Widget::title(format!("Aim {}", def.name)),
        Widget::muted(def.text.clone()),
        choice("aim", "Pick a target", options),
        Widget::button("cancel", "Back", ButtonStyle::Normal),
    ]
}

// ---------------------------------------------------------------- table

/// Scene units per board unit (one hex edge).
const SCALE: f32 = 36.0;
const PAD: f32 = 24.0;

fn hex_fill(terrain: Terrain, mana: Option<Mana>) -> &'static str {
    match (terrain, mana) {
        (_, Some(m)) => mana_color(m),
        (Terrain::Water, None) => "#3b6ea5",
        (Terrain::Lava, None) => "#8a2a12",
        (Terrain::Hole, None) => "#1b1b1b",
        (Terrain::Ice, None) => "#cfe8f5",
        (Terrain::Ground, None) => "#c8bfa8",
    }
}

/// The public table: board, wizards, zones, HP and statuses. No spell
/// lists and no claimed mana.
pub fn table(state: &State) -> TableView {
    let mut view = TableView::new("Wizards Duel");
    view.status = match (state.over, state.winner) {
        (true, Some(w)) => format!("{} wins the duel!", state.name(w)),
        (true, None) => "A draw: nobody is left standing.".into(),
        _ => format!(
            "{}'s turn · round {}",
            state.name(state.current as u32),
            state.turn
        ),
    };
    // Bounds of the board in board units.
    let (mut minx, mut maxx, mut miny, mut maxy) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for h in &state.board.hexes {
        let (x, y) = h.hex.center();
        minx = minx.min(x - 1.0);
        maxx = maxx.max(x + 1.0);
        miny = miny.min(y - 1.0);
        maxy = maxy.max(y + 1.0);
    }
    let to_scene = |(x, y): (f32, f32)| ((x - minx) * SCALE + PAD, (maxy - y) * SCALE + PAD);
    let mut shapes = Vec::new();
    for h in &state.board.hexes {
        let (x, y) = to_scene(h.hex.center());
        shapes.push(Shape::Hex {
            x,
            y,
            r: SCALE * 0.98,
            fill: hex_fill(h.terrain, h.mana).into(),
            stroke: "#4a3f30".into(),
            label: h.mana.map(|m| mana_name(m).to_string()).unwrap_or_default(),
        });
    }
    // A defeated wizard's zones never tick again, so they aren't drawn.
    let live_zones: Vec<&wizards_rules::Zone> = state
        .zones
        .iter()
        .filter(|z| state.wizards.get(z.owner).is_some_and(|w| w.alive()))
        .collect();
    for z in &live_zones {
        for v in &z.vertices {
            let (x, y) = to_scene(v.position());
            shapes.push(Shape::Circle {
                x,
                y,
                r: SCALE * 0.5,
                fill: "#bfe3ff66".into(),
                stroke: state.color(z.owner as u32),
                label: String::new(),
            });
        }
    }
    for r in &state.board.rocks {
        let (x, y) = to_scene(r.position());
        shapes.push(Shape::Circle {
            x,
            y,
            r: SCALE * 0.3,
            fill: "#6b6b6b".into(),
            stroke: "#333333".into(),
            label: String::new(),
        });
    }
    for (i, w) in state.wizards.iter().enumerate() {
        if !w.alive() {
            continue;
        }
        let (x, y) = to_scene(w.at.position());
        let active = i == state.current && !state.over;
        shapes.push(Shape::Circle {
            x,
            y,
            r: SCALE * 0.42,
            fill: state.color(i as u32),
            stroke: if active { "#ffffff" } else { "#222222" }.into(),
            label: state.name(i as u32),
        });
    }
    view.scene = Some(Scene {
        width: (maxx - minx) * SCALE + 2.0 * PAD,
        height: (maxy - miny) * SCALE + 2.0 * PAD,
        background: "#2b2620".into(),
        shapes,
    });
    view.players = state
        .wizards
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let mut lines = Vec::new();
            if w.alive() {
                lines.push(format!("HP {}/{}", w.hp, w.max_hp));
                let s = describe_status(&w.status);
                if !s.is_empty() {
                    lines.push(s);
                }
            } else {
                lines.push("Defeated".into());
            }
            PlayerPanel {
                name: state.name(i as u32),
                color: state.color(i as u32),
                lines,
                active: i == state.current && !state.over,
            }
        })
        .collect();
    view.log = state.log.iter().rev().take(8).rev().cloned().collect();
    view.extra = json!({
        "board": state.board,
        "wizards": state.wizards.iter().enumerate().map(|(i, w)| json!({
            "seat": i,
            "name": state.name(i as u32),
            "color": state.color(i as u32),
            "at": w.at,
            "hp": w.hp,
            "max_hp": w.max_hp,
            "alive": w.alive(),
            "status": w.status,
        })).collect::<Vec<_>>(),
        "zones": live_zones.iter().map(|z| json!({
            "owner": z.owner,
            "vertices": z.vertices,
            "turns_left": z.turns_left,
        })).collect::<Vec<_>>(),
        "current": state.current,
        "turn": state.turn,
        "over": state.over,
        "winner": state.winner,
        "last_events": state.last_events,
    });
    view
}
