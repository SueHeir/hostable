//! The duel's arena and spell list.
//!
//! Both are data for `wizards-rules`: a [`BoardDef`] and a list of
//! [`SpellDef`]s. The spells are a subset of the real Wizards library
//! (`levels/spells.ron` in `SueHeir/wizards`), copied here because that file
//! lives outside the rules crate.

use std::sync::OnceLock;

use wizards_rules::{
    BoardDef, Effect, Hex, HexDef, Mana, Occupant, SpellBook, SpellDef, Target, Terrain, Vertex,
};

/// Hexes from the centre to the rim.
pub const RADIUS: i32 = 3;

/// Steps from the centre hex, in hexes.
pub fn hex_distance(h: Hex) -> i32 {
    (h.q.abs() + h.r.abs() + (h.q + h.r).abs()) / 2
}

/// Turn a hex one sixth of a full turn about the centre hex.
pub fn rotate_hex(h: Hex) -> Hex {
    Hex::new(-h.r, h.q + h.r)
}

/// Turn a vertex the same way as [`rotate_hex`] turns its hex.
pub fn rotate_vertex(v: Vertex) -> Vertex {
    // `rotate_hex` turns positions by -60 degrees (x right, y up).
    let (x, y) = v.position();
    let s = 3f32.sqrt() / 2.0;
    let want = (0.5 * x + s * y, -s * x + 0.5 * y);
    let near = |c: &Vertex| {
        let (a, b) = c.position();
        (a - want.0).powi(2) + (b - want.1).powi(2)
    };
    let corners = rotate_hex(v.hex).corners();
    let mut best = corners[0];
    for c in corners {
        if near(&c) < near(&best) {
            best = c;
        }
    }
    best
}

/// The six turns of `v` about the centre, starting with `v` itself.
pub fn orbit(v: Vertex) -> [Vertex; 6] {
    let mut out = [v; 6];
    for i in 1..6 {
        out[i] = rotate_vertex(out[i - 1]);
    }
    out
}

/// Which mana a hex holds, by its ring and place on the ring. Every ring
/// is the same after a sixth of a turn, so every start corner sees the
/// same board.
fn hex_kind(h: Hex) -> (Terrain, Option<Mana>) {
    let reps = [
        (Hex::new(1, 0), Mana::Orange),
        (Hex::new(2, 0), Mana::Green),
        (Hex::new(1, 1), Mana::Purple),
        (Hex::new(3, 0), Mana::Yellow),
        (Hex::new(2, 1), Mana::Red),
        (Hex::new(1, 2), Mana::Blue),
    ];
    if h == Hex::new(0, 0) {
        return (Terrain::Water, None);
    }
    for (rep, mana) in reps {
        let mut x = rep;
        for _ in 0..6 {
            if x == h {
                return (Terrain::Ground, Some(mana));
            }
            x = rotate_hex(x);
        }
    }
    (Terrain::Ground, None)
}

/// The start vertex of corner 0: the outermost corner of the rim hex at
/// `(RADIUS, 0)`. The other corners are its turns.
fn base_start() -> Vertex {
    let corners = Hex::new(RADIUS, 0).corners();
    let far = |v: &Vertex| {
        let (x, y) = v.position();
        x * x + y * y
    };
    let mut best = corners[0];
    for c in corners {
        if far(&c) > far(&best) + 1e-3 || ((far(&c) - far(&best)).abs() <= 1e-3 && c > best) {
            best = c;
        }
    }
    best
}

/// The six start corners, in order around the board.
pub fn start_corners() -> [Vertex; 6] {
    orbit(base_start())
}

/// Which corners the wizards start on, for 2, 3 or 4 wizards. Opposite or
/// evenly spaced, so nobody starts closer to a rival than anyone else does
/// (with 4, the two pairs face each other across the lake).
pub fn corner_plan(wizards: usize) -> Vec<usize> {
    match wizards {
        0 | 1 => vec![0],
        2 => vec![0, 3],
        3 => vec![0, 2, 4],
        _ => vec![0, 1, 3, 4],
    }
}

/// The arena: a hexagon of 37 hexes with a small lake in the middle, a
/// ring of six rocks, and the same mana in every sixth.
pub fn arena() -> BoardDef {
    static ARENA: OnceLock<BoardDef> = OnceLock::new();
    ARENA.get_or_init(build_arena).clone()
}

/// Whether `def` is the arena as it starts.
pub fn is_untouched_arena(def: &BoardDef) -> bool {
    *def == arena()
}

fn build_arena() -> BoardDef {
    let mut hexes = Vec::new();
    for r in -RADIUS..=RADIUS {
        for q in -RADIUS..=RADIUS {
            let h = Hex::new(q, r);
            if hex_distance(h) <= RADIUS {
                let (terrain, mana) = hex_kind(h);
                hexes.push(HexDef {
                    hex: h,
                    mana,
                    terrain,
                });
            }
        }
    }
    // The vertex where hexes (2,0), (1,0) and (1,1) meet, and its turns.
    let rock = Hex::new(2, 0)
        .corners()
        .into_iter()
        .find(|v| {
            let t = v.touching();
            t.contains(&Hex::new(1, 0)) && t.contains(&Hex::new(1, 1))
        })
        .expect("those three hexes share a corner");
    let mut rocks = orbit(rock).to_vec();
    rocks.sort();
    BoardDef {
        id: "duel_arena".into(),
        hexes,
        doors: vec![],
        monsters: vec![],
        rocks,
        walls: vec![],
        bridges: vec![],
        goal: None,
        chests: vec![],
        spells: None,
    }
}

/// Spell families. Each wizard is dealt one spell from each family, and no
/// two wizards get the same spell, so spell lists are worth keeping secret.
/// Spells in a family do the same job at about the same price (see
/// `docs/spells.md` in the Wizards repo for how they were priced); the
/// Surge family trades a bigger cost for a bigger hit.
///
/// Spells with a `Walk` target or an `Area` effect (Flame step, Charge,
/// Flame ring...) are left out: `wizards-rules` works those out with a
/// path search per board vertex, which costs tens of millions of wasm
/// instructions per call on this board, too close to the sandbox's fuel
/// budget.
pub const FAMILIES: [(&str, [&str; 4]); 5] = [
    ("Strike", ["fire_swipe", "drain", "fireball", "vine_whip"]),
    ("Curse", ["ember", "wither", "firework", "shadow_grasp"]),
    ("Move", ["blink", "leap", "haste", "far_blink"]),
    ("Guard", ["barrier", "ice_ward", "frost_bolt", "dazzle"]),
    ("Surge", ["wildfire", "meteor", "blizzard", "shove"]),
];

fn spell(
    id: &str,
    name: &str,
    text: &str,
    tier: u32,
    cost: Vec<Mana>,
    target: Target,
    effects: Vec<Effect>,
) -> SpellDef {
    SpellDef {
        id: id.into(),
        name: name.into(),
        text: text.into(),
        tier,
        cost,
        target,
        effects,
    }
}

fn dist(min: u32, max: u32, occupant: Occupant) -> Target {
    Target::Distance { min, max, occupant }
}

/// Every spell in the duel, with the names, costs and effects they have
/// in Wizards.
pub fn spells() -> Vec<SpellDef> {
    use Effect::*;
    use Mana::*;
    use Occupant::*;
    vec![
        // Strike
        spell(
            "fire_swipe",
            "Fire swipe",
            "1 damage to a piece next to you.",
            1,
            vec![Red],
            dist(1, 1, Occupied),
            vec![Damage(1)],
        ),
        spell(
            "drain",
            "Drain",
            "1 damage to a piece next to you, and heal 1.",
            1,
            vec![Purple],
            dist(1, 1, Occupied),
            vec![Damage(1), OnCaster(vec![Heal(1)])],
        ),
        spell(
            "fireball",
            "Fireball",
            "1 damage to a piece exactly 2 away, over anything in between.",
            1,
            vec![Orange],
            dist(2, 2, Occupied),
            vec![Damage(1)],
        ),
        spell(
            "vine_whip",
            "Vine whip",
            "1 damage to a piece 2 away, and pull it 1 step toward you.",
            1,
            vec![Green],
            dist(2, 2, Occupied),
            vec![Damage(1), Pull(1)],
        ),
        // Curse
        spell(
            "ember",
            "Ember",
            "Set a piece up to 2 away burning: 1 damage at the end of its next 2 turns.",
            1,
            vec![Red],
            dist(1, 2, Occupied),
            vec![Burn {
                damage: 1,
                turns: 2,
            }],
        ),
        spell(
            "wither",
            "Wither",
            "A piece up to 2 away burns for 3 turns: 1 damage at the end of each.",
            2,
            vec![Purple],
            dist(1, 2, Occupied),
            vec![Burn {
                damage: 1,
                turns: 3,
            }],
        ),
        spell(
            "firework",
            "Firework",
            "1 damage to a piece exactly 3 away, and knock it back 1 step.",
            2,
            vec![Orange],
            dist(3, 3, Occupied),
            vec![Damage(1), Push(1)],
        ),
        spell(
            "shadow_grasp",
            "Shadow grasp",
            "Pull a piece 2 or 3 away up to 2 steps toward you, then 1 damage to it.",
            2,
            vec![Red, Blue],
            dist(2, 3, Occupied),
            vec![Pull(2), Damage(1)],
        ),
        // Move
        spell(
            "blink",
            "Blink",
            "Teleport to the far corner of a hex you touch.",
            1,
            vec![Blue],
            Target::AcrossHex { occupant: Empty },
            vec![MoveCaster],
        ),
        spell(
            "leap",
            "Leap",
            "Jump to an empty spot exactly 2 edges away, over anything.",
            1,
            vec![Yellow],
            dist(2, 2, Empty),
            vec![MoveCaster],
        ),
        spell(
            "haste",
            "Haste",
            "Take 2 extra steps this turn.",
            1,
            vec![Yellow],
            Target::Caster,
            vec![GainSteps(2)],
        ),
        spell(
            "far_blink",
            "Far blink",
            "Teleport to an empty spot 2 to 4 edges away, over anything.",
            2,
            vec![Blue, Blue],
            dist(2, 4, Empty),
            vec![MoveCaster],
        ),
        // Guard
        spell(
            "barrier",
            "Barrier",
            "Shield yourself from the next 2 damage.",
            2,
            vec![Purple],
            Target::Caster,
            vec![Shield(2)],
        ),
        spell(
            "ice_ward",
            "Ice ward",
            "Shield yourself from the next 1 damage.",
            1,
            vec![Blue],
            Target::Caster,
            vec![Shield(1)],
        ),
        spell(
            "frost_bolt",
            "Frost bolt",
            "Root a piece up to 2 away for 2 turns: it can't walk.",
            1,
            vec![Blue],
            dist(1, 2, Occupied),
            vec![Root(2)],
        ),
        spell(
            "dazzle",
            "Dazzle",
            "Stun a piece up to 2 away for 1 turn: it can't move or cast.",
            2,
            vec![Yellow],
            dist(1, 2, Occupied),
            vec![Stun(1)],
        ),
        // Surge
        spell(
            "wildfire",
            "Wildfire",
            "1 damage to every enemy on a hex up to 2 away, now and on your next 3 turns.",
            3,
            vec![Orange, Red],
            Target::Hex { min: 0, max: 2 },
            vec![Repeat {
                turns: 4,
                effects: vec![Damage(1)],
            }],
        ),
        spell(
            "meteor",
            "Meteor",
            "2 damage to every enemy on a hex 1 to 3 away.",
            3,
            vec![Red, Red, Red],
            Target::Hex { min: 1, max: 3 },
            vec![Damage(2)],
        ),
        spell(
            "blizzard",
            "Blizzard",
            "1 damage to every enemy on a hex up to 1 away, now and on your next 2 turns.",
            2,
            vec![Blue, Blue],
            Target::Hex { min: 0, max: 1 },
            vec![Repeat {
                turns: 3,
                effects: vec![Damage(1)],
            }],
        ),
        spell(
            "shove",
            "Shove",
            "Push a piece next to you 2 steps, then 1 damage to it.",
            1,
            vec![Red, Yellow],
            dist(1, 1, Occupied),
            vec![Push(2), Damage(1)],
        ),
    ]
}

/// The duel's spell book, built once.
pub fn book() -> SpellBook {
    static BOOK: OnceLock<SpellBook> = OnceLock::new();
    BOOK.get_or_init(|| SpellBook::new(spells())).clone()
}

/// A spell's display name, or its id if it isn't in the book.
pub fn spell_name(id: &str) -> String {
    book()
        .get(id)
        .map_or_else(|| id.to_string(), |s| s.name.clone())
}

pub fn mana_name(m: Mana) -> &'static str {
    match m {
        Mana::Red => "Red",
        Mana::Yellow => "Yellow",
        Mana::Blue => "Blue",
        Mana::Orange => "Orange",
        Mana::Purple => "Purple",
        Mana::Green => "Green",
    }
}

/// A CSS colour for drawing mana.
pub fn mana_color(m: Mana) -> &'static str {
    match m {
        Mana::Red => "#d64545",
        Mana::Yellow => "#e8c547",
        Mana::Blue => "#4a7fd6",
        Mana::Orange => "#e88a3a",
        Mana::Purple => "#9a5bd0",
        Mana::Green => "#4caf50",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wizards_rules::Board;

    #[test]
    fn spell_list_is_clean_and_every_family_spell_exists() {
        let list = spells();
        assert!(
            SpellBook::lint(&list).is_empty(),
            "{:?}",
            SpellBook::lint(&list)
        );
        let book = book();
        let mut all: Vec<&str> = FAMILIES
            .iter()
            .flat_map(|(_, s)| s.iter().copied())
            .collect();
        for id in &all {
            assert!(book.get(id).is_some(), "{id}");
        }
        all.sort();
        all.dedup();
        assert_eq!(
            all.len(),
            list.len(),
            "every spell is in exactly one family"
        );
        // No spell name is part of another's name, so a name on screen
        // always means that spell.
        for a in &list {
            for b in &list {
                if a.id != b.id {
                    assert!(!b.name.contains(&a.name), "{} in {}", a.name, b.name);
                }
            }
        }
    }

    #[test]
    fn the_arena_is_the_same_from_every_corner() {
        let def = arena();
        assert_eq!(def.hexes.len(), 37);
        let board = Board::new(def.clone());
        for h in &def.hexes {
            let turned = rotate_hex(h.hex);
            let other = def.hexes.iter().find(|d| d.hex == turned).unwrap();
            assert_eq!((h.mana, h.terrain), (other.mana, other.terrain));
        }
        for r in &def.rocks {
            assert!(def.rocks.contains(&rotate_vertex(*r)));
        }
        let starts = start_corners();
        let mut seen = starts.to_vec();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 6);
        // Every free vertex can be walked to from every start, and each
        // start is as far from the next as the others are.
        let free: Vec<Vertex> = {
            let mut v: Vec<Vertex> = board
                .vertices()
                .into_iter()
                .filter(|v| !board.is_rock(*v))
                .collect();
            v.sort();
            v
        };
        for s in starts {
            assert!(board.has_vertex(s) && !board.is_rock(s));
            let reach = board.path(starts[0], s, |v| !board.is_rock(v));
            assert!(reach.is_some());
        }
        for v in &free {
            assert!(board.path(starts[0], *v, |x| !board.is_rock(x)).is_some());
        }
        let d01 = board.distance(starts[0], starts[1]);
        for i in 0..6 {
            assert_eq!(board.distance(starts[i], starts[(i + 1) % 6]), d01);
        }
        // All six colours are on the board.
        for m in Mana::ALL {
            assert!(def.hexes.iter().any(|h| h.mana == Some(m)), "{m:?}");
        }
    }
}
