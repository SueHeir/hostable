//! What the shared screen and each phone show.

use hostable_sdk::prelude::*;

use crate::board::{self, topo, HEX_COUNT, LETTERS};
use crate::*;

const PALETTE: [&str; 4] = ["#d1495b", "#2e86ab", "#e0a526", "#5c9e5a"];
const TILE_FILL: [&str; 5] = ["#9fc46a", "#c0603a", "#e9e4d8", "#4f9fc9", "#c98a3a"];
const DUNES_FILL: &str = "#e6cf8f";

pub fn color(s: &State, seat: SeatId) -> String {
    match s.seats.get(seat as usize) {
        Some(seat) if !seat.color.is_empty() => seat.color.clone(),
        _ => PALETTE[seat as usize % PALETTE.len()].to_string(),
    }
}

fn pips(n: u8) -> u8 {
    if n == 0 {
        0
    } else {
        6 - (7i32 - n as i32).unsigned_abs() as u8
    }
}

fn tile_name(t: &Tile) -> &'static str {
    t.res.map_or("Dunes", |r| RES[r])
}

fn hex_name(s: &State, h: usize) -> String {
    let t = &s.tiles[h];
    if t.number == 0 {
        format!("{} · {}", LETTERS[h], tile_name(t))
    } else {
        format!("{} · {} {}", LETTERS[h], tile_name(t), t.number)
    }
}

fn status(s: &State) -> String {
    let cur = s.name(s.current);
    match &s.phase {
        Phase::Setup { round, hut, .. } => {
            let who = s.setup_seat().map_or("?", |x| s.name(x));
            let which = if *round == 0 { "first" } else { "second" };
            let what = if hut.is_none() { "hut" } else { "causeway" };
            format!("Setup: {who} places their {which} {what}")
        }
        Phase::PreRoll => format!("{cur}'s turn: roll the tide dice"),
        Phase::Main => {
            let roll = s.dice.map(|(a, b)| a + b).unwrap_or(0);
            format!("{cur}'s turn (rolled {roll})")
        }
        Phase::Discard { owed } => {
            let names: Vec<&str> = owed
                .iter()
                .enumerate()
                .filter(|(_, &n)| n > 0)
                .map(|(i, _)| s.name(i as SeatId))
                .collect();
            format!("Squall! Waiting for {} to discard", names.join(", "))
        }
        Phase::MoveSquall { .. } => format!("{cur} moves the Squall"),
        Phase::Steal { .. } => format!("{cur} picks who to rob"),
        Phase::Surveyor { .. } => format!("{cur} lays free causeways"),
        Phase::Bounty { .. } => format!("{cur} takes from the bank"),
        Phase::Market { .. } => format!("{cur} names a resource for Market Day"),
        Phase::TradeRespond { offer, .. } => {
            format!("{cur} offers {}", offer_text(offer))
        }
        Phase::TradeConfirm { .. } => format!("{cur} picks a trade"),
        Phase::Over { winner } => format!(
            "{} wins with {} points!",
            s.name(*winner),
            s.total_points(*winner)
        ),
    }
}

fn phase_key(p: &Phase) -> &'static str {
    match p {
        Phase::Setup { .. } => "setup",
        Phase::PreRoll => "pre_roll",
        Phase::Main => "main",
        Phase::Discard { .. } => "discard",
        Phase::MoveSquall { .. } => "move_squall",
        Phase::Steal { .. } => "steal",
        Phase::Surveyor { .. } => "surveyor",
        Phase::Bounty { .. } => "bounty",
        Phase::Market { .. } => "market",
        Phase::TradeRespond { .. } => "trade_respond",
        Phase::TradeConfirm { .. } => "trade_confirm",
        Phase::Over { .. } => "over",
    }
}

pub fn table(s: &State) -> TableView {
    let t = topo();
    let mut shapes = Vec::new();
    for (h, tile) in s.tiles.iter().enumerate().take(HEX_COUNT) {
        let (x, y) = board::hex_pos(h);
        let fill = tile.res.map_or(DUNES_FILL, |r| TILE_FILL[r]);
        shapes.push(Shape::Hex {
            x,
            y,
            r: board::SIZE - 2.0,
            fill: fill.into(),
            stroke: "#2b4c5e".into(),
            label: String::new(),
        });
        shapes.push(Shape::Text {
            x,
            y: y - 26.0,
            text: LETTERS[h].to_string(),
            size: 16.0,
            color: "#1d2b33".into(),
        });
        shapes.push(Shape::Text {
            x,
            y: y + 30.0,
            text: tile_name(tile).into(),
            size: 10.0,
            color: "#1d2b33".into(),
        });
        if tile.number > 0 {
            let red = tile.number == 6 || tile.number == 8;
            shapes.push(Shape::Circle {
                x,
                y: y + 4.0,
                r: 14.0,
                fill: if red { "#f6c9c2" } else { "#f7f1e3" }.into(),
                stroke: if red { "#c0392b" } else { "#555" }.into(),
                label: tile.number.to_string(),
            });
        }
    }
    for (e, owner) in s.causeways.iter().enumerate() {
        if let Some(seat) = owner {
            let (a, b) = t.edges[e];
            let (x1, y1) = board::vert_pos(a);
            let (x2, y2) = board::vert_pos(b);
            // Pull the ends in so neighbouring causeways stay distinct.
            let k = 0.15;
            shapes.push(Shape::Line {
                x1: x1 + (x2 - x1) * k,
                y1: y1 + (y2 - y1) * k,
                x2: x2 - (x2 - x1) * k,
                y2: y2 - (y2 - y1) * k,
                color: color(s, *seat),
                width: 7.0,
            });
        }
    }
    for (v, b) in s.buildings.iter().enumerate() {
        if let Some(b) = b {
            let (x, y) = board::vert_pos(v);
            if b.lodge {
                shapes.push(Shape::Rect {
                    x: x - 11.0,
                    y: y - 11.0,
                    w: 22.0,
                    h: 22.0,
                    fill: color(s, b.seat),
                    stroke: "#111".into(),
                    label: "L".into(),
                });
            } else {
                shapes.push(Shape::Circle {
                    x,
                    y,
                    r: 9.0,
                    fill: color(s, b.seat),
                    stroke: "#111".into(),
                    label: String::new(),
                });
            }
        }
    }
    let (sx, sy) = board::hex_pos(s.squall);
    shapes.push(Shape::Circle {
        x: sx + 24.0,
        y: sy - 6.0,
        r: 16.0,
        fill: "#37474f".into(),
        stroke: "#eceff1".into(),
        label: "Squall".into(),
    });
    let mut view = TableView::new("Tideholm");
    view.status = status(s);
    view.scene = Some(Scene {
        width: board::SCENE_W,
        height: board::SCENE_H,
        background: "#2f7fa3".into(),
        shapes,
    });
    view.players = (0..s.players.len())
        .map(|i| {
            let seat = i as SeatId;
            let p = &s.players[i];
            let mut lines = vec![
                format!("{} points", s.visible_points(seat)),
                format!(
                    "{} resource cards · {} Almanac",
                    p.total(),
                    p.cards.len() + p.fresh.len()
                ),
                format!(
                    "Longest causeway {} · Ferrymen {}",
                    p.causeway_len, p.ferrymen
                ),
            ];
            if s.longest == Some(seat) {
                lines.push("Longest Causeway +2".into());
            }
            if s.fleet == Some(seat) {
                lines.push("Largest Fleet +2".into());
            }
            PlayerPanel {
                name: s.name(seat).to_string(),
                color: color(s, seat),
                lines,
                active: s.owes_action(seat),
            }
        })
        .collect();
    view.log = s.log.iter().rev().take(12).rev().cloned().collect();
    let points: Vec<u32> = (0..s.players.len())
        .map(|i| s.visible_points(i as SeatId))
        .collect();
    view.extra = json!({
        "turn": s.turn,
        "phase": phase_key(&s.phase),
        "current": s.current,
        "points": points,
        "dice": s.dice.map(|(a, b)| vec![a, b]),
        "squall": LETTERS[s.squall].to_string(),
        "trades_completed": s.trades_completed,
        "longest": s.longest,
        "fleet": s.fleet,
        "deck_left": s.deck.len(),
        "winner": match s.phase { Phase::Over { winner } => Some(winner), _ => None },
    });
    view
}

// ---------------------------------------------------------------------------
// Phones

fn choice(
    id: &str,
    label: impl Into<String>,
    options: Vec<ChoiceOption>,
    min: u32,
    max: u32,
) -> Widget {
    Widget::Choice {
        id: id.into(),
        style: ChoiceStyle::List,
        options,
        max,
        min,
        label: label.into(),
    }
}

fn button(id: &str, label: impl Into<String>, style: ButtonStyle, disabled: bool) -> Widget {
    Widget::Button {
        id: id.into(),
        label: label.into(),
        style,
        disabled,
    }
}

pub fn hand_counts_text(p: &Player) -> String {
    (0..5)
        .map(|r| format!("{} {}", RES[r], p.res[r]))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn vert_detail(s: &State, v: usize) -> String {
    let t = topo();
    let mut total = 0u32;
    let parts: Vec<String> = t.vert_hexes[v]
        .iter()
        .map(|&h| {
            let tile = &s.tiles[h];
            total += pips(tile.number) as u32;
            if tile.number == 0 {
                tile_name(tile).to_string()
            } else {
                format!("{} {}", tile_name(tile), tile.number)
            }
        })
        .collect();
    format!("{total} pips: {}", parts.join(", "))
}

fn hut_options(s: &State, spots: &[usize], disabled: bool) -> Vec<ChoiceOption> {
    let t = topo();
    spots
        .iter()
        .map(|&v| {
            ChoiceOption::new(v.to_string(), format!("Hut at {}", t.vert_labels[v]))
                .detail(vert_detail(s, v))
                .disabled(disabled)
        })
        .collect()
}

fn causeway_options(
    s: &State,
    _seat: SeatId,
    spots: &[usize],
    disabled: bool,
) -> Vec<ChoiceOption> {
    let t = topo();
    spots
        .iter()
        .map(|&e| {
            let (a, b) = t.edges[e];
            let opens = [a, b].into_iter().any(|v| s.hut_spot_free(v));
            let detail = if opens {
                "leads to a free hut spot"
            } else {
                ""
            };
            ChoiceOption::new(e.to_string(), format!("Causeway {}", t.edge_labels[e]))
                .detail(detail)
                .disabled(disabled)
        })
        .collect()
}

fn cost_text(cost: &[u8; 5]) -> String {
    res_list(cost)
}

fn playable(s: &State, seat: SeatId) -> Vec<ChoiceOption> {
    if s.played_card || s.current != seat {
        return Vec::new();
    }
    let p = &s.players[seat as usize];
    let mut kinds: Vec<Card> = p
        .cards
        .iter()
        .copied()
        .filter(|c| *c != Card::OldLore)
        .collect();
    kinds.sort_unstable();
    kinds.dedup();
    kinds
        .into_iter()
        .map(|c| ChoiceOption::new(c.key(), c.name()).detail(c.text()))
        .collect()
}

fn play_widget(s: &State, seat: SeatId) -> Option<Widget> {
    let opts = playable(s, seat);
    (!opts.is_empty()).then(|| choice("play_card", "Play an Almanac card", opts, 1, 1))
}

fn offer_options(p: &Player) -> Vec<ChoiceOption> {
    let mut opts = Vec::new();
    for r in 0..5 {
        for k in 1..=p.res[r].min(3) {
            opts.push(ChoiceOption::new(
                format!("give:{}:{k}", RES_KEYS[r]),
                format!("You give {} #{k}", RES[r]),
            ));
        }
    }
    for r in 0..5 {
        for k in 1..=3 {
            opts.push(ChoiceOption::new(
                format!("want:{}:{k}", RES_KEYS[r]),
                format!("You get {} #{k}", RES[r]),
            ));
        }
    }
    opts
}

fn main_controls(s: &State, seat: SeatId) -> Vec<Widget> {
    let p = &s.players[seat as usize];
    let t = topo();
    let mut build = Vec::new();
    let huts = s.hut_spots(seat);
    if huts.is_empty() {
        build.push(Widget::muted(
            "No free spot for a hut yet: extend a causeway.",
        ));
    } else {
        let off = !p.has(&COST_HUT) || p.huts >= MAX_HUTS;
        build.push(choice(
            "build_hut",
            format!("Raise a hut ({})", cost_text(&COST_HUT)),
            hut_options(s, &huts, off),
            1,
            1,
        ));
    }
    let mine: Vec<usize> = (0..s.buildings.len())
        .filter(|&v| s.buildings[v] == Some(Building { seat, lodge: false }))
        .collect();
    if !mine.is_empty() {
        let off = !p.has(&COST_LODGE) || p.lodges >= MAX_LODGES;
        build.push(choice(
            "build_lodge",
            format!("Upgrade a hut to a lodge ({})", cost_text(&COST_LODGE)),
            mine.iter()
                .map(|&v| {
                    ChoiceOption::new(v.to_string(), format!("Lodge at {}", t.vert_labels[v]))
                        .detail(vert_detail(s, v))
                        .disabled(off)
                })
                .collect(),
            1,
            1,
        ));
    }
    let edges = s.causeway_spots(seat);
    if !edges.is_empty() {
        let off = !p.has(&COST_CAUSEWAY) || p.causeways >= MAX_CAUSEWAYS;
        build.push(choice(
            "build_causeway",
            format!("Lay a causeway ({})", cost_text(&COST_CAUSEWAY)),
            causeway_options(s, seat, &edges, off),
            1,
            1,
        ));
    }
    build.push(button(
        "buy_almanac",
        format!(
            "Buy an Almanac card ({}) · {} left",
            cost_text(&COST_ALMANAC),
            s.deck.len()
        ),
        ButtonStyle::Normal,
        s.deck.is_empty() || !p.has(&COST_ALMANAC),
    ));

    let mut trade = Vec::new();
    let mut bank_opts = Vec::new();
    for a in 0..5 {
        if p.res[a] < 4 {
            continue;
        }
        for b in 0..5 {
            if a != b && s.bank[b] > 0 {
                bank_opts.push(ChoiceOption::new(
                    format!("{}>{}", RES_KEYS[a], RES_KEYS[b]),
                    format!("4 {} → 1 {}", RES[a], RES[b]),
                ));
            }
        }
    }
    if bank_opts.is_empty() {
        trade.push(Widget::muted("Bank trades need 4 of one resource."));
    } else {
        trade.push(choice(
            "bank_trade",
            "Trade with the bank (4:1)",
            bank_opts,
            1,
            1,
        ));
    }
    if s.proposals < MAX_PROPOSALS && p.total() > 0 {
        trade.push(choice(
            "offer",
            format!(
                "Offer a trade to everyone ({} of {MAX_PROPOSALS} offers left). Tick what you give and what you get.",
                MAX_PROPOSALS - s.proposals
            ),
            offer_options(p),
            2,
            6,
        ));
    } else {
        trade.push(Widget::muted("No more offers this turn."));
    }

    let mut almanac = Vec::new();
    match play_widget(s, seat) {
        Some(w) => almanac.push(w),
        None => almanac.push(Widget::muted(if s.played_card {
            "You already played a card this turn."
        } else {
            "No card you can play now (new cards wait a turn)."
        })),
    }

    vec![
        Widget::Tabs {
            tabs: vec![
                Tab {
                    label: "Build".into(),
                    children: build,
                },
                Tab {
                    label: "Trade".into(),
                    children: trade,
                },
                Tab {
                    label: "Almanac".into(),
                    children: almanac,
                },
            ],
        },
        button("end_turn", "End turn", ButtonStyle::Primary, false),
    ]
}

fn response_lines(s: &State, responses: &[Resp]) -> Vec<Widget> {
    responses
        .iter()
        .enumerate()
        .filter(|(_, r)| **r != Resp::Proposer)
        .map(|(i, r)| {
            let who = s.name(i as SeatId);
            Widget::text(match r {
                Resp::Pending => format!("{who}: thinking…"),
                Resp::Accept => format!("{who}: accepts"),
                Resp::Decline => format!("{who}: declines"),
                Resp::Counter(o) => format!("{who}: counters, you give {}", offer_text(o)),
                Resp::Proposer => String::new(),
            })
        })
        .collect()
}

fn body(s: &State, seat: SeatId) -> Vec<Widget> {
    let si = seat as usize;
    let p = &s.players[si];
    let mine = s.current == seat;
    let t = topo();
    match &s.phase {
        Phase::Over { winner } => vec![Widget::title(if *winner == seat {
            "You win!".to_string()
        } else {
            format!("{} wins", s.name(*winner))
        })],
        Phase::Setup { hut, .. } => {
            if s.setup_seat() != Some(seat) {
                return vec![Widget::muted("Others are placing. Watch the table.")];
            }
            match hut {
                None => {
                    let spots: Vec<usize> = (0..s.buildings.len())
                        .filter(|&v| s.hut_spot_free(v))
                        .collect();
                    vec![choice(
                        "place_hut",
                        "Place a hut (free)",
                        hut_options(s, &spots, false),
                        1,
                        1,
                    )]
                }
                Some(v) => {
                    let opts = topo().vert_edges[*v]
                        .iter()
                        .filter(|&&e| s.causeways[e].is_none())
                        .map(|&e| {
                            ChoiceOption::new(
                                e.to_string(),
                                format!("Causeway {}", t.edge_labels[e]),
                            )
                        })
                        .collect();
                    vec![choice(
                        "place_causeway",
                        format!("Lay a causeway from your hut at {}", t.vert_labels[*v]),
                        opts,
                        1,
                        1,
                    )]
                }
            }
        }
        Phase::PreRoll => {
            if !mine {
                return vec![Widget::muted(format!(
                    "{} is about to roll.",
                    s.name(s.current)
                ))];
            }
            let mut out = vec![button(
                "roll",
                "Roll the tide dice",
                ButtonStyle::Huge,
                false,
            )];
            if let Some(w) = play_widget(s, seat) {
                out.push(w);
            }
            out
        }
        Phase::Main => {
            if mine {
                main_controls(s, seat)
            } else {
                vec![Widget::muted(format!("{}'s turn.", s.name(s.current)))]
            }
        }
        Phase::Discard { owed } => {
            let n = owed.get(si).copied().unwrap_or(0);
            if n == 0 {
                return vec![Widget::muted("Waiting for others to discard.")];
            }
            let mut cards = Vec::new();
            for r in 0..5 {
                for k in 0..p.res[r] {
                    cards.push(CardFace {
                        value: format!("{}:{k}", RES_KEYS[r]),
                        title: RES[r].into(),
                        text: String::new(),
                        corner: String::new(),
                        color: TILE_FILL[r].into(),
                        image: None,
                        disabled: false,
                    });
                }
            }
            vec![
                Widget::title(format!("Squall! Discard {n} cards")),
                Widget::Hand {
                    id: "discard".into(),
                    cards,
                    max: n as u32,
                    min: n as u32,
                },
            ]
        }
        Phase::MoveSquall { .. } => {
            if !mine {
                return vec![Widget::muted(format!(
                    "{} is moving the Squall.",
                    s.name(s.current)
                ))];
            }
            let opts = (0..HEX_COUNT)
                .filter(|&h| h != s.squall)
                .map(|h| {
                    let mut owners: Vec<SeatId> = t.hex_verts[h]
                        .iter()
                        .filter_map(|&v| s.buildings[v].map(|b| b.seat))
                        .collect();
                    owners.sort_unstable();
                    owners.dedup();
                    let detail = if owners.contains(&seat) {
                        "(yours) beside: ".to_string()
                    } else {
                        "beside: ".to_string()
                    } + &if owners.is_empty() {
                        "nobody".to_string()
                    } else {
                        owners
                            .iter()
                            .map(|&o| s.name(o))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    ChoiceOption::new(h.to_string(), hex_name(s, h)).detail(detail)
                })
                .collect();
            vec![choice("squall_hex", "Move the Squall to", opts, 1, 1)]
        }
        Phase::Steal { from, .. } => {
            if !mine {
                return vec![Widget::muted(format!(
                    "{} is picking who to rob.",
                    s.name(s.current)
                ))];
            }
            let opts = from
                .iter()
                .map(|&o| {
                    ChoiceOption::new(
                        o.to_string(),
                        format!("{} ({} cards)", s.name(o), s.players[o as usize].total()),
                    )
                })
                .collect();
            vec![choice("steal_from", "Take a random card from", opts, 1, 1)]
        }
        Phase::Surveyor { left, .. } => {
            if !mine {
                return vec![Widget::muted("Surveyor at work.")];
            }
            let edges = s.causeway_spots(seat);
            vec![
                choice(
                    "surveyor_edge",
                    format!("Lay a free causeway ({left} left)"),
                    causeway_options(s, seat, &edges, false),
                    1,
                    1,
                ),
                button("surveyor_done", "Done", ButtonStyle::Normal, false),
            ]
        }
        Phase::Bounty { .. } => {
            if !mine {
                return vec![Widget::muted("Bountiful Tide!")];
            }
            let mut opts = Vec::new();
            for r in 0..5 {
                for k in 1..=s.bank[r].min(2) {
                    opts.push(ChoiceOption::new(
                        format!("{}:{k}", RES_KEYS[r]),
                        format!("{} #{k}", RES[r]),
                    ));
                }
            }
            vec![choice("bounty", "Take any 2 from the bank", opts, 2, 2)]
        }
        Phase::Market { .. } => {
            if !mine {
                return vec![Widget::muted("Market Day! Hold on to your cards…")];
            }
            let opts = (0..5)
                .map(|r| ChoiceOption::new(RES_KEYS[r], RES[r]))
                .collect();
            vec![choice(
                "market",
                "Name a resource: everyone hands you theirs",
                opts,
                1,
                1,
            )]
        }
        Phase::TradeRespond { offer, responses } => {
            if mine {
                let mut out = vec![Widget::title(format!("You offer {}", offer_text(offer)))];
                out.extend(response_lines(s, responses));
                out.push(button(
                    "trade_cancel",
                    "Withdraw offer",
                    ButtonStyle::Danger,
                    false,
                ));
                return out;
            }
            let from = s.name(s.current);
            match responses.get(si) {
                Some(Resp::Pending) => {
                    let can = p.has(&offer.get);
                    vec![
                        Widget::title(format!("{from} offers a trade")),
                        Widget::text(format!(
                            "You get {}, you give {}",
                            res_list(&offer.give),
                            res_list(&offer.get)
                        )),
                        button(
                            "trade_accept",
                            if can {
                                "Accept"
                            } else {
                                "Accept (you lack the cards)"
                            },
                            ButtonStyle::Primary,
                            !can,
                        ),
                        button("trade_decline", "Decline", ButtonStyle::Normal, false),
                        Widget::Sheet {
                            label: "Counter-offer".into(),
                            children: vec![choice(
                                "counter",
                                "Tick what you give and what you get",
                                offer_options(p),
                                2,
                                6,
                            )],
                        },
                    ]
                }
                _ => vec![Widget::muted(format!(
                    "You answered. Waiting for {from} to pick."
                ))],
            }
        }
        Phase::TradeConfirm { responses, .. } => {
            if !mine {
                return vec![Widget::muted(format!(
                    "{} is picking a trade.",
                    s.name(s.current)
                ))];
            }
            let opts = responses
                .iter()
                .enumerate()
                .filter_map(|(i, r)| {
                    let deal = match r {
                        Resp::Accept => {
                            let Phase::TradeConfirm { offer, .. } = &s.phase else {
                                return None;
                            };
                            *offer
                        }
                        Resp::Counter(o) => *o,
                        _ => return None,
                    };
                    let ok = trade_with(s, i).is_some();
                    let kind = if matches!(r, Resp::Accept) {
                        "accepts"
                    } else {
                        "counters"
                    };
                    Some(
                        ChoiceOption::new(i.to_string(), format!("{} {kind}", s.name(i as SeatId)))
                            .detail(format!(
                                "You give {}, you get {}",
                                res_list(&deal.give),
                                res_list(&deal.get)
                            ))
                            .disabled(!ok),
                    )
                })
                .collect();
            vec![
                choice("trade_pick", "Confirm one trade", opts, 1, 1),
                button("trade_cancel", "Cancel", ButtonStyle::Normal, false),
            ]
        }
    }
}

pub fn phone(s: &State, seat: SeatId) -> Widget {
    let si = seat as usize;
    let Some(p) = s.players.get(si) else {
        return Widget::text("You're not seated in this game.");
    };
    let mut children = vec![
        Widget::title(format!("Tideholm · {}", s.name(seat))),
        Widget::text(status(s)),
        Widget::Text {
            id: Some("hand_counts".into()),
            value: hand_counts_text(p),
            style: TextStyle::Body,
        },
    ];
    let lore = p.old_lore();
    children.push(Widget::muted(if lore > 0 {
        format!(
            "{} points ({} shown + {lore} hidden Old Lore)",
            s.total_points(seat),
            s.visible_points(seat)
        )
    } else {
        format!("{} points", s.visible_points(seat))
    }));
    for n in &p.notes {
        children.push(Widget::muted(n.clone()));
    }
    children.extend(body(s, seat));
    let mut cards: Vec<Widget> = p
        .cards
        .iter()
        .map(|c| Widget::text(format!("{}: {}", c.name(), c.text())))
        .collect();
    cards.extend(p.fresh.iter().map(|c| {
        Widget::text(format!(
            "{} (new, playable next turn): {}",
            c.name(),
            c.text()
        ))
    }));
    if cards.is_empty() {
        cards.push(Widget::muted("No Almanac cards yet."));
    }
    children.push(Widget::Sheet {
        label: format!("My Almanac ({})", p.cards.len() + p.fresh.len()),
        children: cards,
    });
    children.push(Widget::Sheet {
        label: "Costs".into(),
        children: vec![
            Widget::text(format!("Causeway: {}", cost_text(&COST_CAUSEWAY))),
            Widget::text(format!("Hut (1 point): {}", cost_text(&COST_HUT))),
            Widget::text(format!(
                "Lodge (2 points, produces 2): {}",
                cost_text(&COST_LODGE)
            )),
            Widget::text(format!("Almanac card: {}", cost_text(&COST_ALMANAC))),
            Widget::muted(format!(
                "Longest Causeway ({LONGEST_MIN}+) and Largest Fleet ({FLEET_MIN}+ Ferrymen) are worth 2 each. First to {WIN_POINTS} wins."
            )),
        ],
    });
    Widget::column(children)
}
