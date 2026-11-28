//! The card pool as data, and the rules text generated from that data, so a
//! card's text can never drift from what the interpreter does.

use std::fmt::Write as _;

/// Index into [`CARDS`].
pub type CardId = u16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum School {
    /// Fire alchemists: damage and fast Allies.
    Ember,
    /// Quicksilver scholars: negation, returns and card draw.
    Quicksilver,
    /// The Lantern Watch: Resolve, wards, boosts and Lamplighters.
    Lantern,
    /// The Velvet guild of the back alleys: discard, drain and removal.
    Velvet,
}

impl School {
    pub fn name(self) -> &'static str {
        match self {
            School::Ember => "Ember",
            School::Quicksilver => "Quicksilver",
            School::Lantern => "Lantern",
            School::Velvet => "Velvet",
        }
    }

    pub fn color(self) -> &'static str {
        match self {
            School::Ember => "#d9573b",
            School::Quicksilver => "#5b8fd9",
            School::Lantern => "#d9a830",
            School::Velvet => "#9161c9",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Stays in play; strikes and blocks.
    Ally,
    /// May be played whenever you have priority, even in response.
    Feint,
    /// Only on your own turn with an empty stack.
    Gambit,
    /// A lasting effect in play.
    Charm,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Ally => "Ally",
            Kind::Feint => "Feint",
            Kind::Gambit => "Gambit",
            Kind::Charm => "Charm",
        }
    }
}

/// What a card (or ability) may target. One target per card; every
/// targeted effect on the card uses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tgt {
    None,
    /// Either player or any Ally.
    Any,
    /// Any Ally.
    Ally,
    /// An Ally the opponent controls.
    Enemy,
    /// An Ally you control.
    Own,
}

/// Which stack item a negation can answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Neg {
    /// A card or an ability.
    CardOrAbility,
    /// A Feint card.
    Feint,
    /// A Strike declaration.
    Strike,
}

/// One effect. Targeted effects use the card's [`Tgt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eff {
    /// Deal damage to the target.
    Damage(u8),
    DamageFoe(u8),
    DamageEnemyAllies(u8),
    DamageEveryAlly(u8),
    /// The opponent loses Resolve (wards don't absorb loss).
    FoeLoses(u8),
    YouLose(u8),
    Heal(u8),
    /// The opponent loses n and you gain n.
    Drain(u8),
    Draw(u8),
    Ward(u8),
    /// The target Ally gets +power/+guard until end of turn.
    Boost(u8, u8),
    /// Your Allies get +power/+guard until end of turn.
    BoostTeam(u8, u8),
    /// Ready the target Ally.
    Ready,
    /// Spend the target Ally.
    Spend,
    /// Return the target Ally to its owner's hand.
    Bounce,
    BounceEnemyAllies,
    /// Destroy the target Ally.
    Destroy,
    FoeDiscards(u8),
    /// The opponent puts the top n cards of their deck into their Gutter.
    Mill(u8),
    /// Return the top card of your Gutter to your hand.
    Recall,
    /// Summon n Lamplighter tokens.
    Summon(u8),
    /// Negate the stack item this responds to.
    Negate(Neg),
    /// Negate a card or ability unless its owner pays n Spark.
    NegateUnlessPay(u8),
}

impl Eff {
    pub fn targeted(self) -> bool {
        matches!(
            self,
            Eff::Damage(_) | Eff::Boost(..) | Eff::Ready | Eff::Spend | Eff::Bounce | Eff::Destroy
        )
    }

    pub fn negation(self) -> Option<Neg> {
        match self {
            Eff::Negate(n) => Some(n),
            Eff::NegateUnlessPay(_) => Some(Neg::CardOrAbility),
            _ => None,
        }
    }
}

/// What a Charm does while it is in play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharmEff {
    StartHeal(u8),
    StartDamage(u8),
    StartSummon(u8),
    TeamPower(u8),
    FeintDiscount(u8),
    /// Whenever the opponent plays a Feint, they lose n Resolve.
    FeintToll(u8),
}

/// An Ally's ability: pay Spark and spend the Ally (it must be ready).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ability {
    pub cost: u8,
    pub target: Tgt,
    pub effects: &'static [Eff],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardDef {
    pub name: &'static str,
    pub school: School,
    pub kind: Kind,
    pub cost: u8,
    pub power: u8,
    pub guard: u8,
    /// May strike the turn it arrives.
    pub swift: bool,
    /// Striking doesn't spend it.
    pub steadfast: bool,
    /// Can't be blocked.
    pub elusive: bool,
    pub target: Tgt,
    /// For Feints and Gambits: what happens on resolution. For Allies: the
    /// Arrival effects (never targeted).
    pub effects: &'static [Eff],
    pub ability: Option<Ability>,
    pub charm: Option<CharmEff>,
    /// Not in the pool; made by Summon.
    pub token: bool,
}

const fn base(name: &'static str, school: School, kind: Kind, cost: u8) -> CardDef {
    CardDef {
        name,
        school,
        kind,
        cost,
        power: 0,
        guard: 0,
        swift: false,
        steadfast: false,
        elusive: false,
        target: Tgt::None,
        effects: &[],
        ability: None,
        charm: None,
        token: false,
    }
}

const fn ally(name: &'static str, school: School, cost: u8, power: u8, guard: u8) -> CardDef {
    let mut c = base(name, school, Kind::Ally, cost);
    c.power = power;
    c.guard = guard;
    c
}

const fn feint(
    name: &'static str,
    school: School,
    cost: u8,
    target: Tgt,
    effects: &'static [Eff],
) -> CardDef {
    let mut c = base(name, school, Kind::Feint, cost);
    c.target = target;
    c.effects = effects;
    c
}

const fn gambit(
    name: &'static str,
    school: School,
    cost: u8,
    target: Tgt,
    effects: &'static [Eff],
) -> CardDef {
    let mut c = base(name, school, Kind::Gambit, cost);
    c.target = target;
    c.effects = effects;
    c
}

const fn charm(name: &'static str, school: School, cost: u8, effect: CharmEff) -> CardDef {
    let mut c = base(name, school, Kind::Charm, cost);
    c.charm = Some(effect);
    c
}

impl CardDef {
    const fn swift(mut self) -> Self {
        self.swift = true;
        self
    }
    const fn steadfast(mut self) -> Self {
        self.steadfast = true;
        self
    }
    const fn elusive(mut self) -> Self {
        self.elusive = true;
        self
    }
    const fn arrive(mut self, effects: &'static [Eff]) -> Self {
        self.effects = effects;
        self
    }
    const fn ability(mut self, cost: u8, target: Tgt, effects: &'static [Eff]) -> Self {
        self.ability = Some(Ability {
            cost,
            target,
            effects,
        });
        self
    }
    const fn token(mut self) -> Self {
        self.token = true;
        self
    }

    /// Whether playing it needs a target picked.
    pub fn needs_target(&self) -> bool {
        self.target != Tgt::None && self.effects.iter().any(|e| e.targeted())
    }

    /// The negation this card performs, if any.
    pub fn negation(&self) -> Option<Neg> {
        self.effects.iter().find_map(|e| e.negation())
    }
}

use Eff::*;
use School::*;

/// Every card. The pool is everything but tokens; the Lamplighter token is
/// last.
pub static CARDS: &[CardDef] = &[
    // ---- Ember (16) ----
    ally("Cinder Whelp", Ember, 1, 1, 1).swift(),
    feint("Ember Dart", Ember, 1, Tgt::Any, &[Damage(2)]),
    feint("Flashpowder Pinch", Ember, 0, Tgt::Ally, &[Boost(2, 0)]),
    ally("Kiln Brawler", Ember, 2, 3, 1),
    ally("Spark Juggler", Ember, 2, 1, 2).ability(1, Tgt::Any, &[Damage(1)]),
    ally("Furnace Duellist", Ember, 3, 3, 2).swift(),
    gambit(
        "Scorch the Rooftops",
        Ember,
        3,
        Tgt::None,
        &[DamageEnemyAllies(1), DamageFoe(1)],
    ),
    feint("Alembic Burst", Ember, 2, Tgt::Ally, &[Damage(3)]),
    gambit("Blazing Lunge", Ember, 2, Tgt::None, &[DamageFoe(3)]),
    ally("Crucible Knight", Ember, 4, 4, 3).arrive(&[DamageFoe(1)]),
    charm("Smouldering Brazier", Ember, 3, CharmEff::StartDamage(1)),
    gambit("Fury of the Kilns", Ember, 2, Tgt::None, &[BoostTeam(2, 0)]),
    gambit("Ashfall Volley", Ember, 4, Tgt::None, &[DamageEveryAlly(2)]),
    ally("Forge Colossus", Ember, 6, 6, 5).arrive(&[DamageEnemyAllies(2)]),
    feint(
        "Flare Riposte",
        Ember,
        2,
        Tgt::None,
        &[Negate(Neg::Strike), DamageFoe(2)],
    ),
    gambit(
        "Inferno Sermon",
        Ember,
        7,
        Tgt::None,
        &[DamageFoe(5), DamageEnemyAllies(2)],
    ),
    // ---- Quicksilver (16) ----
    ally("Bead of Mercury", Quicksilver, 1, 1, 2).elusive(),
    feint(
        "Recant the Formula",
        Quicksilver,
        2,
        Tgt::None,
        &[Negate(Neg::CardOrAbility)],
    ),
    feint(
        "Thought Better Of",
        Quicksilver,
        1,
        Tgt::None,
        &[Negate(Neg::Feint)],
    ),
    feint(
        "Quicksilver Levy",
        Quicksilver,
        1,
        Tgt::None,
        &[NegateUnlessPay(2)],
    ),
    feint("Reflux Tincture", Quicksilver, 2, Tgt::Ally, &[Bounce]),
    gambit("Distilled Insight", Quicksilver, 2, Tgt::None, &[Draw(2)]),
    ally("Glass Familiar", Quicksilver, 2, 1, 3).arrive(&[Draw(1)]),
    feint(
        "Glimmer of Foresight",
        Quicksilver,
        1,
        Tgt::None,
        &[Draw(1)],
    ),
    ally("Tidewater Scholar", Quicksilver, 3, 2, 3).ability(1, Tgt::None, &[Draw(1)]),
    feint("Silver Hush", Quicksilver, 1, Tgt::Ally, &[Spend]),
    ally("Retort Archivist", Quicksilver, 4, 3, 4).arrive(&[Recall]),
    feint(
        "Unmake the Recipe",
        Quicksilver,
        3,
        Tgt::None,
        &[Negate(Neg::CardOrAbility), Draw(1)],
    ),
    gambit(
        "Volatile Solution",
        Quicksilver,
        3,
        Tgt::Ally,
        &[Bounce, Draw(1)],
    ),
    ally("Brass-Feathered Owl", Quicksilver, 5, 3, 5)
        .elusive()
        .arrive(&[Draw(1)]),
    charm(
        "Sublimation Coil",
        Quicksilver,
        2,
        CharmEff::FeintDiscount(1),
    ),
    gambit(
        "Grand Transmutation",
        Quicksilver,
        7,
        Tgt::None,
        &[BounceEnemyAllies, Draw(2)],
    ),
    // ---- Lantern (16) ----
    ally("Lamplighter Recruit", Lantern, 1, 1, 2).arrive(&[Heal(1)]),
    feint("Kindly Glow", Lantern, 0, Tgt::None, &[Heal(2)]),
    feint("Watchman's Shield", Lantern, 1, Tgt::None, &[Ward(3)]),
    feint("Lamppost Stand", Lantern, 1, Tgt::Ally, &[Boost(0, 3)]),
    ally("Night Watch Captain", Lantern, 3, 2, 3).arrive(&[Summon(1)]),
    gambit("Call the Watch", Lantern, 2, Tgt::None, &[Summon(2)]),
    charm("Lantern Over the Door", Lantern, 2, CharmEff::StartHeal(2)),
    feint(
        "Rally the Lanterns",
        Lantern,
        2,
        Tgt::None,
        &[BoostTeam(1, 1)],
    ),
    ally("Lamp-post Sentinel", Lantern, 2, 1, 4).steadfast(),
    ally("Hearth-Lamp Keeper", Lantern, 4, 3, 4).ability(2, Tgt::None, &[Heal(2)]),
    gambit("Sanctuary Bell", Lantern, 3, Tgt::None, &[Heal(4), Ward(2)]),
    charm("Lantern Parade", Lantern, 4, CharmEff::StartSummon(1)),
    ally("Gilded Fencer", Lantern, 3, 3, 3).steadfast(),
    feint(
        "Rekindled Stride",
        Lantern,
        2,
        Tgt::Own,
        &[Ready, Boost(1, 1)],
    ),
    ally("Watchtower Warden", Lantern, 5, 4, 6),
    charm("Oiled Blades", Lantern, 3, CharmEff::TeamPower(1)),
    // ---- Velvet (16) ----
    ally("Alley Cutpurse", Velvet, 2, 1, 1).arrive(&[FoeDiscards(1)]),
    feint("Siphon Kiss", Velvet, 2, Tgt::None, &[Drain(2)]),
    gambit("Alley Whispers", Velvet, 1, Tgt::None, &[Mill(3)]),
    feint(
        "Pickpocket's Trick",
        Velvet,
        2,
        Tgt::None,
        &[FoeDiscards(1)],
    ),
    ally("Velvet Assassin", Velvet, 4, 3, 2)
        .elusive()
        .arrive(&[FoeDiscards(1)]),
    feint("Smoke Behind the Lantern", Velvet, 0, Tgt::None, &[Ward(2)]),
    ally("Night Market Fence", Velvet, 2, 2, 2).ability(1, Tgt::None, &[Mill(1)]),
    ally("Gutter Rat Pack", Velvet, 1, 2, 1),
    gambit("Dusk Decree", Velvet, 4, Tgt::Ally, &[Destroy]),
    feint("Kiss of Nightshade", Velvet, 3, Tgt::Ally, &[Destroy]),
    gambit(
        "Hollow Bargain",
        Velvet,
        0,
        Tgt::None,
        &[YouLose(2), Draw(2)],
    ),
    charm("Whispering Gallery", Velvet, 3, CharmEff::FeintToll(1)),
    gambit("Moonless Heist", Velvet, 3, Tgt::None, &[FoeDiscards(2)]),
    feint("Echo from the Gutter", Velvet, 2, Tgt::None, &[Recall]),
    ally("Velvet Duchess", Velvet, 6, 5, 5).arrive(&[Drain(3)]),
    gambit(
        "Curtain at Midnight",
        Velvet,
        5,
        Tgt::None,
        &[FoeLoses(4), FoeDiscards(1)],
    ),
    // ---- Tokens ----
    ally("Lamplighter", Lantern, 0, 1, 1).token(),
];

/// Number of cards players can build with.
pub const POOL_SIZE: usize = 64;
/// The Lamplighter token.
pub const LAMPLIGHTER: CardId = 64;

/// A card's definition. Unknown ids get the token, so lookups never panic.
pub fn card(id: CardId) -> &'static CardDef {
    CARDS
        .get(id as usize)
        .unwrap_or(&CARDS[LAMPLIGHTER as usize])
}

pub fn find(name: &str) -> Option<CardId> {
    CARDS
        .iter()
        .take(POOL_SIZE)
        .position(|c| c.name == name)
        .map(|i| i as CardId)
}

/// Ready-made decks offered in the deck builder.
pub static STARTERS: &[(&str, [&str; 20])] = &[
    (
        "Lamplit Blaze",
        [
            "Cinder Whelp",
            "Ember Dart",
            "Flashpowder Pinch",
            "Kiln Brawler",
            "Spark Juggler",
            "Furnace Duellist",
            "Alembic Burst",
            "Blazing Lunge",
            "Crucible Knight",
            "Fury of the Kilns",
            "Flare Riposte",
            "Lamplighter Recruit",
            "Kindly Glow",
            "Watchman's Shield",
            "Lamppost Stand",
            "Night Watch Captain",
            "Call the Watch",
            "Gilded Fencer",
            "Rekindled Stride",
            "Watchtower Warden",
        ],
    ),
    (
        "Silver Shadows",
        [
            "Bead of Mercury",
            "Recant the Formula",
            "Thought Better Of",
            "Quicksilver Levy",
            "Reflux Tincture",
            "Glass Familiar",
            "Glimmer of Foresight",
            "Tidewater Scholar",
            "Silver Hush",
            "Unmake the Recipe",
            "Brass-Feathered Owl",
            "Alley Cutpurse",
            "Siphon Kiss",
            "Pickpocket's Trick",
            "Velvet Assassin",
            "Smoke Behind the Lantern",
            "Night Market Fence",
            "Gutter Rat Pack",
            "Kiss of Nightshade",
            "Velvet Duchess",
        ],
    ),
    (
        "Quick Lanterns",
        [
            "Bead of Mercury",
            "Recant the Formula",
            "Thought Better Of",
            "Quicksilver Levy",
            "Reflux Tincture",
            "Distilled Insight",
            "Glass Familiar",
            "Glimmer of Foresight",
            "Silver Hush",
            "Sublimation Coil",
            "Lamplighter Recruit",
            "Kindly Glow",
            "Watchman's Shield",
            "Lamppost Stand",
            "Lamp-post Sentinel",
            "Hearth-Lamp Keeper",
            "Rally the Lanterns",
            "Lantern Over the Door",
            "Gilded Fencer",
            "Rekindled Stride",
        ],
    ),
    (
        "Velvet Embers",
        [
            "Cinder Whelp",
            "Ember Dart",
            "Flashpowder Pinch",
            "Kiln Brawler",
            "Furnace Duellist",
            "Alembic Burst",
            "Scorch the Rooftops",
            "Smouldering Brazier",
            "Flare Riposte",
            "Forge Colossus",
            "Alley Cutpurse",
            "Siphon Kiss",
            "Alley Whispers",
            "Pickpocket's Trick",
            "Smoke Behind the Lantern",
            "Gutter Rat Pack",
            "Dusk Decree",
            "Kiss of Nightshade",
            "Whispering Gallery",
            "Echo from the Gutter",
        ],
    ),
];

/// A starter deck's cards. Out-of-range indexes wrap.
pub fn starter(index: usize) -> Vec<CardId> {
    let (_, names) = &STARTERS[index % STARTERS.len()];
    names.iter().filter_map(|n| find(n)).collect()
}

fn plural(n: u8, one: &str, many: &str) -> String {
    if n == 1 {
        format!("a {one}")
    } else {
        format!("{n} {many}")
    }
}

fn target_phrase(t: Tgt) -> &'static str {
    match t {
        Tgt::None | Tgt::Any => "any target",
        Tgt::Ally => "target Ally",
        Tgt::Enemy => "target enemy Ally",
        Tgt::Own => "target Ally of yours",
    }
}

fn ally_phrase(t: Tgt) -> &'static str {
    match t {
        Tgt::Enemy => "target enemy Ally",
        Tgt::Own => "target Ally of yours",
        _ => "target Ally",
    }
}

/// One sentence for one effect.
pub fn describe(e: Eff, t: Tgt) -> String {
    match e {
        Damage(n) => format!("Deal {n} damage to {}.", target_phrase(t)),
        DamageFoe(n) => format!("Deal {n} damage to the opponent."),
        DamageEnemyAllies(n) => format!("Deal {n} damage to each enemy Ally."),
        DamageEveryAlly(n) => format!("Deal {n} damage to each Ally."),
        FoeLoses(n) => format!("The opponent loses {n} Resolve."),
        YouLose(n) => format!("You lose {n} Resolve."),
        Heal(n) => format!("Gain {n} Resolve."),
        Drain(n) => format!("The opponent loses {n} Resolve and you gain {n}."),
        Draw(n) => format!("Draw {}.", plural(n, "card", "cards")),
        Ward(n) => format!("Gain a {n} ward (it absorbs damage to you until your next turn)."),
        Boost(p, g) => format!("{} gets +{p}/+{g} until end of turn.", cap(ally_phrase(t))),
        BoostTeam(p, g) => format!("Your Allies get +{p}/+{g} until end of turn."),
        Ready => format!("Ready {}.", ally_phrase(t)),
        Spend => format!("Spend {}.", ally_phrase(t)),
        Bounce => format!("Return {} to its owner's hand.", ally_phrase(t)),
        BounceEnemyAllies => "Return each enemy Ally to its owner's hand.".into(),
        Destroy => format!("Destroy {}.", ally_phrase(t)),
        FoeDiscards(n) => format!(
            "The opponent discards {} at random.",
            plural(n, "card", "cards")
        ),
        Mill(n) => format!(
            "The opponent puts the top {} of their deck into their Gutter.",
            if n == 1 {
                "card".to_string()
            } else {
                format!("{n} cards")
            }
        ),
        Recall => "Return the top card of your Gutter to your hand.".into(),
        Summon(n) => format!(
            "Summon {}.",
            plural(n, "Lamplighter (a 1/1 Ally)", "Lamplighters (1/1 Allies)")
        ),
        Negate(Neg::CardOrAbility) => "Negate the card or ability this responds to.".into(),
        Negate(Neg::Feint) => "Negate the Feint this responds to.".into(),
        Negate(Neg::Strike) => {
            "Negate the Strike this responds to (the strikers stay spent).".into()
        }
        NegateUnlessPay(n) => format!(
            "Negate the card or ability this responds to unless its owner pays {n} Spark (they pay if they can)."
        ),
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn effects_text(effects: &[Eff], t: Tgt) -> String {
    effects
        .iter()
        .map(|e| describe(*e, t))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn charm_text(c: CharmEff) -> String {
    match c {
        CharmEff::StartHeal(n) => format!("At the start of your turn, gain {n} Resolve."),
        CharmEff::StartDamage(n) => {
            format!("At the start of your turn, deal {n} damage to the opponent.")
        }
        CharmEff::StartSummon(n) => format!(
            "At the start of your turn, summon {}.",
            plural(n, "Lamplighter (a 1/1 Ally)", "Lamplighters (1/1 Allies)")
        ),
        CharmEff::TeamPower(n) => format!("Your Allies have +{n} power."),
        CharmEff::FeintDiscount(n) => format!("Your Feints cost {n} less (to a minimum of 0)."),
        CharmEff::FeintToll(n) => {
            format!("Whenever the opponent plays a Feint, they lose {n} Resolve.")
        }
    }
}

/// The full rules text of a card, as printed on its face.
pub fn card_text(c: &CardDef) -> String {
    let mut out = String::new();
    match c.kind {
        Kind::Ally => {
            let _ = write!(out, "Ally {}/{}.", c.power, c.guard);
            if c.swift {
                out.push_str(" Swift (can strike the turn it arrives).");
            }
            if c.steadfast {
                out.push_str(" Steadfast (striking doesn't spend it).");
            }
            if c.elusive {
                out.push_str(" Shadowed (can't be blocked).");
            }
            if !c.effects.is_empty() {
                let _ = write!(out, " Arrival: {}", effects_text(c.effects, Tgt::None));
            }
            if let Some(a) = c.ability {
                let _ = write!(
                    out,
                    " Ability ({} Spark, spend this ready Ally; any time you have priority): {}",
                    a.cost,
                    effects_text(a.effects, a.target)
                );
            }
        }
        Kind::Feint | Kind::Gambit => {
            let _ = write!(
                out,
                "{}. {}",
                c.kind.name(),
                effects_text(c.effects, c.target)
            );
        }
        Kind::Charm => {
            let _ = write!(
                out,
                "Charm. {}",
                c.charm.map(charm_text).unwrap_or_default()
            );
        }
    }
    out
}
