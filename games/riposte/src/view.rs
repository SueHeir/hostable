//! What the shared table and each phone show.

use hostable_sdk::prelude::*;

use crate::cards::{self, card, card_text, effects_text, STARTERS};
use crate::{other, Ally, ItemKind, Phase, Source, State, Target, POOL_SIZE};

fn phase_name(p: Phase) -> &'static str {
    match p {
        Phase::Building => "building",
        Phase::Main => "main",
        Phase::Blocking => "blocking",
        Phase::Over => "over",
    }
}

pub fn status(s: &State) -> String {
    match s.phase {
        Phase::Building => {
            let waiting: Vec<&str> = (0..2)
                .filter(|seat| !s.player(*seat).built)
                .map(|seat| s.name(seat))
                .collect();
            format!("Building decks: waiting for {}", waiting.join(" and "))
        }
        Phase::Main if s.stack.is_empty() => {
            format!("Turn {}: {} to act", s.turn, s.name(s.active))
        }
        Phase::Main => {
            let top = s.stack.last().map(|i| s.item_name(i)).unwrap_or_default();
            format!(
                "{} may respond to {top} (stack of {})",
                s.name(s.priority),
                s.stack.len()
            )
        }
        Phase::Blocking => format!("{} is choosing blockers", s.name(other(s.active))),
        Phase::Over => s
            .result
            .as_ref()
            .map(|r| r.summary.clone())
            .unwrap_or_default(),
    }
}

fn ally_label(s: &State, a: &Ally) -> String {
    let mut l = format!("{} {}/{}", card(a.card).name, s.power(a), s.guard(a));
    if a.damage > 0 {
        l.push_str(&format!(" ({} dmg)", a.damage));
    }
    if a.spent {
        l.push_str(" spent");
    } else if a.fresh {
        l.push_str(" new");
    }
    l
}

fn darken(hex: &str) -> String {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x808080);
    let f = |shift: u32| ((n >> shift) & 0xff) / 2;
    format!("#{:02x}{:02x}{:02x}", f(16), f(8), f(0))
}

fn player_line(s: &State, seat: crate::SeatId) -> String {
    let p = s.player(seat);
    let mut l = format!(
        "{} · Resolve {} · Spark {}/{} · Hand {} · Deck {} · Gutter {}",
        s.name(seat),
        p.resolve,
        p.spark,
        p.spark_max,
        p.hand.len(),
        p.deck.len(),
        p.gutter.len()
    );
    if p.ward > 0 {
        l.push_str(&format!(" · Ward {}", p.ward));
    }
    l
}

pub fn table(s: &State) -> TableView {
    let mut view = TableView::new("Riposte");
    view.status = status(s);
    let mut shapes = Vec::new();
    for seat in 0..2u32 {
        let y0 = if seat == 1 { 30.0 } else { 640.0 };
        let p = s.player(seat);
        shapes.push(Shape::Text {
            x: 40.0,
            y: y0 + 20.0,
            text: player_line(s, seat),
            size: 26.0,
            color: if s.acting() == Some(seat) {
                "#ffe08a".into()
            } else {
                "#e8e6f0".into()
            },
        });
        for (i, a) in s.allies_of(seat).enumerate() {
            let color = card(a.card).school.color();
            shapes.push(Shape::Rect {
                x: 40.0 + i as f32 * 185.0,
                y: y0 + 50.0,
                w: 170.0,
                h: 110.0,
                fill: if a.spent {
                    darken(color)
                } else {
                    color.to_string()
                },
                stroke: "#f4f1e8".into(),
                label: ally_label(s, a),
            });
        }
        for (i, c) in p.charms.iter().enumerate() {
            shapes.push(Shape::Rect {
                x: 1160.0 + (i % 2) as f32 * 210.0,
                y: y0 + 50.0 + (i / 2) as f32 * 60.0,
                w: 200.0,
                h: 50.0,
                fill: darken(card(*c).school.color()),
                stroke: card(*c).school.color().into(),
                label: format!("Charm: {}", card(*c).name),
            });
        }
    }
    // The stack, bottom item lowest.
    let n = s.stack.len();
    shapes.push(Shape::Text {
        x: 620.0,
        y: 230.0,
        text: if n == 0 {
            "The stack is empty".into()
        } else {
            format!("The stack ({n})")
        },
        size: 24.0,
        color: "#e8e6f0".into(),
    });
    for (i, item) in s.stack.iter().enumerate() {
        let from_top = n - 1 - i;
        let fill = match &item.kind {
            ItemKind::Card { card: c } | ItemKind::Ability { card: c, .. } => {
                card(*c).school.color().to_string()
            }
            _ => "#6b6b80".into(),
        };
        shapes.push(Shape::Rect {
            x: 560.0 + (i % 2) as f32 * 20.0,
            y: 250.0 + from_top as f32 * 44.0,
            w: 480.0,
            h: 40.0,
            fill,
            stroke: if from_top == 0 {
                "#ffffff".into()
            } else {
                "#333344".into()
            },
            label: format!("{} — {}", s.item_name(item), s.name(item.owner)),
        });
        if shapes.len() > 3000 {
            break;
        }
    }
    view.scene = Some(Scene {
        width: 1600.0,
        height: 900.0,
        background: "#1b1a2e".into(),
        shapes,
    });
    view.players = (0..2u32)
        .map(|seat| {
            let p = s.player(seat);
            let mut lines = vec![
                format!("Resolve {}", p.resolve),
                format!("Spark {}/{}", p.spark, p.spark_max),
                format!(
                    "Hand {} · Deck {} · Gutter {}",
                    p.hand.len(),
                    p.deck.len(),
                    p.gutter.len()
                ),
            ];
            if p.ward > 0 {
                lines.push(format!("Ward {}", p.ward));
            }
            PlayerPanel {
                name: s.name(seat).to_string(),
                color: s.colors.get(seat as usize).cloned().unwrap_or_default(),
                lines,
                active: s.acting() == Some(seat),
            }
        })
        .collect();
    view.log = s.log.iter().rev().take(12).rev().cloned().collect();
    view.extra = json!({
        "phase": phase_name(s.phase),
        "stack": s.stack.iter().map(|i| s.item_name(i)).collect::<Vec<_>>(),
        "deepest_chain": s.deepest_chain,
        "resolve": [s.players[0].resolve, s.players[1].resolve],
        "turn": s.turn,
        "active": s.active,
    });
    view
}

// ---------------------------------------------------------------------------
// Phones
// ---------------------------------------------------------------------------

fn face(s: &State, seat: u32, id: cards::CardId) -> CardFace {
    let c = card(id);
    CardFace {
        value: format!("c{id}"),
        title: c.name.to_string(),
        text: card_text(c),
        corner: s.cost(seat, id).to_string(),
        color: c.school.color().to_string(),
        image: None,
        disabled: !s.can_play(seat, id),
    }
}

fn target_label(s: &State, seat: u32, t: Target) -> (String, String) {
    match t {
        Target::Player(p) if p == seat => ("me".into(), "You".into()),
        Target::Player(p) => ("foe".into(), format!("{} (opponent)", s.name(p))),
        Target::Ally(u) => {
            let label = s.ally(u).map_or(String::new(), |a| {
                let whose = if a.owner == seat { "Your" } else { "Enemy" };
                format!("{whose} {}", ally_label(s, a))
            });
            (format!("a{u}"), label)
        }
    }
}

fn header(s: &State, seat: u32) -> Vec<Widget> {
    let me = s.player(seat);
    let foe = other(seat);
    let them = s.player(foe);
    let mut out = vec![
        Widget::Meter {
            label: "Resolve".into(),
            value: me.resolve,
            max: crate::START_RESOLVE.max(me.resolve),
            style: MeterStyle::Health,
        },
        Widget::Text {
            id: Some("spark".into()),
            value: format!(
                "Spark {}/{} · Deck {} · Gutter {}{}",
                me.spark,
                me.spark_max,
                me.deck.len(),
                me.gutter.len(),
                if me.ward > 0 {
                    format!(" · Ward {}", me.ward)
                } else {
                    String::new()
                }
            ),
            style: TextStyle::Body,
        },
        Widget::muted(format!(
            "{}: Resolve {} · Spark {}/{} · Hand {} · Deck {}",
            s.name(foe),
            them.resolve,
            them.spark,
            them.spark_max,
            them.hand.len(),
            them.deck.len()
        )),
    ];
    let mine: Vec<String> = s.allies_of(seat).map(|a| ally_label(s, a)).collect();
    let theirs: Vec<String> = s.allies_of(foe).map(|a| ally_label(s, a)).collect();
    if !mine.is_empty() {
        out.push(Widget::text(format!("Your Allies: {}", mine.join(", "))));
    }
    if !theirs.is_empty() {
        out.push(Widget::muted(format!(
            "Their Allies: {}",
            theirs.join(", ")
        )));
    }
    let charms =
        |p: u32| -> Vec<&str> { s.player(p).charms.iter().map(|c| card(*c).name).collect() };
    if !charms(seat).is_empty() {
        out.push(Widget::muted(format!(
            "Your Charms: {}",
            charms(seat).join(", ")
        )));
    }
    if !charms(foe).is_empty() {
        out.push(Widget::muted(format!(
            "Their Charms: {}",
            charms(foe).join(", ")
        )));
    }
    if !s.stack.is_empty() {
        let items: Vec<String> = s
            .stack
            .iter()
            .rev()
            .map(|i| {
                let who = if i.owner == seat {
                    "you".to_string()
                } else {
                    s.name(i.owner).to_string()
                };
                format!("{} ({who})", s.item_name(i))
            })
            .collect();
        out.push(Widget::text(format!(
            "Stack, top first: {}",
            items.join(" ← ")
        )));
    }
    out
}

fn rules_sheet() -> Widget {
    Widget::Sheet {
        label: "How to play".into(),
        children: vec![
            Widget::text("Bring the opponent from 20 Resolve to 0. Drawing from an empty deck loses."),
            Widget::text("Each of your turns: Spark maximum +1 (to 10) and refills, your Allies ready, you draw a card."),
            Widget::text("Allies, Gambits and Charms: your turn, empty stack. Feints and abilities: whenever you have priority."),
            Widget::text("Everything goes on the stack and the other player may respond. When both pass in a row, the top item resolves."),
            Widget::text("Strike once a turn with ready Allies that arrived before this turn. The defender blocks with ready Allies, one blocker per striker; then a Clash resolves the damage."),
            Widget::text("An Ally is destroyed when its damage reaches its guard. Damage and boosts wear off at end of turn. Wards absorb damage, not lost Resolve."),
            Widget::text(format!(
                "If a target is gone when a card resolves, the card does nothing. At most {} Allies and {} Charms each; extra Lamplighters are lost.",
                crate::MAX_ALLIES,
                crate::MAX_CHARMS
            )),
        ],
    }
}

fn building(s: &State, seat: u32) -> Vec<Widget> {
    if s.player(seat).built {
        return vec![
            Widget::title("Deck ready"),
            Widget::text(format!("Waiting for {} to build.", s.name(other(seat)))),
        ];
    }
    let mut out = vec![
        Widget::title("Build your deck"),
        Widget::text("Pick 20 different cards, or use a ready-made deck."),
    ];
    for (i, (name, _)) in STARTERS.iter().enumerate() {
        out.push(Widget::button(
            format!("starter{i}"),
            format!("Use the {name} deck"),
            ButtonStyle::Normal,
        ));
    }
    let options = (0..POOL_SIZE as u16)
        .map(|id| {
            let c = card(id);
            ChoiceOption::new(format!("c{id}"), c.name).detail(format!(
                "{} · {} Spark · {}",
                c.school.name(),
                c.cost,
                card_text(c)
            ))
        })
        .collect();
    out.push(Widget::Choice {
        id: "deck".into(),
        style: ChoiceStyle::Cards,
        options,
        max: crate::DECK_SIZE as u32,
        min: crate::DECK_SIZE as u32,
        label: "Your 20 cards".into(),
    });
    out
}

fn hand(s: &State, seat: u32, enabled: bool) -> Widget {
    Widget::Hand {
        id: "play".into(),
        cards: s
            .player(seat)
            .hand
            .iter()
            .map(|c| {
                let mut f = face(s, seat, *c);
                f.disabled |= !enabled;
                f
            })
            .collect(),
        max: 1,
        min: 1,
    }
}

fn abilities(s: &State, seat: u32) -> Option<Widget> {
    let options: Vec<ChoiceOption> = s
        .allies_of(seat)
        .filter(|a| s.can_use(seat, a.uid))
        .filter_map(|a| {
            let ab = card(a.card).ability?;
            Some(
                ChoiceOption::new(
                    format!("a{}", a.uid),
                    format!("{} ({} Spark)", card(a.card).name, ab.cost),
                )
                .detail(effects_text(ab.effects, ab.target)),
            )
        })
        .collect();
    (!options.is_empty()).then(|| Widget::Choice {
        id: "ability".into(),
        style: ChoiceStyle::List,
        options,
        max: 1,
        min: 1,
        label: "Use an Ally's ability".into(),
    })
}

fn selecting(s: &State, seat: u32, src: Source) -> Vec<Widget> {
    let (name, text, tgt) = match src {
        Source::Hand(id) => {
            let c = card(id);
            (
                c.name.to_string(),
                effects_text(c.effects, c.target),
                c.target,
            )
        }
        Source::Ability(uid) => {
            let Some(a) = s.ally(uid) else {
                return vec![Widget::button("cancel", "Cancel", ButtonStyle::Normal)];
            };
            let c = card(a.card);
            match c.ability {
                Some(ab) => (
                    format!("{}'s ability", c.name),
                    effects_text(ab.effects, ab.target),
                    ab.target,
                ),
                None => (c.name.to_string(), String::new(), cards::Tgt::None),
            }
        }
    };
    let options = s
        .targets(seat, tgt)
        .into_iter()
        .map(|t| {
            let (v, l) = target_label(s, seat, t);
            ChoiceOption::new(v, l)
        })
        .collect();
    vec![
        Widget::title(format!("Target for {name}")),
        Widget::Choice {
            id: "target".into(),
            style: ChoiceStyle::List,
            options,
            max: 1,
            min: 1,
            label: text,
        },
        Widget::button("cancel", "Cancel", ButtonStyle::Normal),
    ]
}

fn main_controls(s: &State, seat: u32) -> Vec<Widget> {
    let mut out = Vec::new();
    if let Some(src) = s.selecting.filter(|_| s.has_priority(seat)) {
        out.extend(selecting(s, seat, src));
        out.push(hand(s, seat, false));
        return out;
    }
    if s.has_priority(seat) && s.stack.is_empty() {
        out.push(Widget::title("Your turn"));
        let able = s.strikers(seat);
        if !able.is_empty() {
            out.push(Widget::Choice {
                id: "strike".into(),
                style: ChoiceStyle::List,
                options: able
                    .iter()
                    .filter_map(|u| s.ally(*u))
                    .map(|a| ChoiceOption::new(format!("a{}", a.uid), ally_label(s, a)))
                    .collect(),
                max: able.len() as u32,
                min: 1,
                label: "Strike with…".into(),
            });
        }
        out.extend(abilities(s, seat));
        out.push(hand(s, seat, true));
        out.push(Widget::button("end_turn", "End turn", ButtonStyle::Primary));
    } else if s.has_priority(seat) {
        let top = s.stack.last().map(|i| s.item_name(i)).unwrap_or_default();
        out.push(Widget::title(format!("Respond to {top}?")));
        out.push(Widget::button("pass", "Pass", ButtonStyle::Primary));
        out.extend(abilities(s, seat));
        out.push(hand(s, seat, true));
    } else {
        out.push(Widget::muted(status(s)));
        out.push(hand(s, seat, false));
    }
    out
}

fn blocking(s: &State, seat: u32) -> Vec<Widget> {
    let Some(combat) = &s.combat else {
        return vec![];
    };
    let name = |u: u32| s.ally(u).map_or("?".to_string(), |a| ally_label(s, a));
    let strikers: Vec<String> = combat.strikers.iter().map(|u| name(*u)).collect();
    if seat == s.active {
        return vec![
            Widget::muted(format!("Striking with {}", strikers.join(", "))),
            Widget::muted(status(s)),
            hand(s, seat, false),
        ];
    }
    let mut out = vec![
        Widget::title("Choose blockers"),
        Widget::text(format!("Striking you: {}", strikers.join(", "))),
    ];
    for (b, t) in &combat.blocks {
        out.push(Widget::muted(format!("{} blocks {}", name(*b), name(*t))));
    }
    let options: Vec<ChoiceOption> = s
        .block_options()
        .into_iter()
        .map(|(b, t)| {
            ChoiceOption::new(
                format!("a{b}>a{t}"),
                format!("Block {} with {}", name(t), name(b)),
            )
        })
        .collect();
    if !options.is_empty() {
        out.push(Widget::Choice {
            id: "block".into(),
            style: ChoiceStyle::List,
            options,
            max: 1,
            min: 1,
            label: "Add a block".into(),
        });
    }
    if !combat.blocks.is_empty() {
        out.push(Widget::button(
            "unblock",
            "Clear blocks",
            ButtonStyle::Normal,
        ));
    }
    out.push(Widget::button(
        "blocks_done",
        "Done blocking",
        ButtonStyle::Primary,
    ));
    out.push(hand(s, seat, false));
    out
}

pub fn phone(s: &State, seat: u32) -> Widget {
    if seat > 1 {
        return Widget::text("Watching");
    }
    let mut children = Vec::new();
    match s.phase {
        Phase::Building => children.extend(building(s, seat)),
        Phase::Over => {
            let won = s
                .result
                .as_ref()
                .and_then(|r| r.ranking.first())
                .map(|first| (first.contains(&seat), first.len() > 1));
            children.push(Widget::title(match won {
                Some((_, true)) => "A draw",
                Some((true, false)) => "You win the duel!",
                _ => "You lose the duel",
            }));
            children.push(Widget::text(status(s)));
            children.extend(header(s, seat));
        }
        Phase::Main => {
            children.extend(header(s, seat));
            children.extend(main_controls(s, seat));
        }
        Phase::Blocking => {
            children.extend(header(s, seat));
            children.extend(blocking(s, seat));
        }
    }
    if s.phase != Phase::Building || s.player(seat).built {
        children.push(rules_sheet());
    }
    Widget::column(children)
}
