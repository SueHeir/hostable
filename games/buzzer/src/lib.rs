//! Buzzer: the first phone to tap wins the round. First to five points wins.
//!
//! The demo game for phase 1. It exercises joining, a phone screen that
//! changes with the game, and input from many phones at once.

use hostable_sdk::prelude::*;

hostable_sdk::export!(Buzzer);

pub const WINNING_SCORE: u32 = 5;

/// The seat that can start the next round. Seat 0 is whoever joined first.
const LEADER: SeatId = 0;

#[derive(Debug, Default, Clone, Copy)]
pub struct Buzzer;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub seats: Vec<Seat>,
    pub scores: Vec<u32>,
    pub round: u32,
    pub phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Phase {
    /// Waiting for someone to buzz.
    Open,
    /// Someone buzzed; waiting for the leader to start the next round.
    Buzzed {
        by: SeatId,
    },
    Finished {
        winner: SeatId,
    },
}

impl State {
    fn name(&self, seat: SeatId) -> &str {
        self.seats
            .iter()
            .find(|s| s.id == seat)
            .map_or("?", |s| s.name.as_str())
    }
}

impl Rules for Buzzer {
    type State = State;

    fn seats(&self) -> (u32, u32) {
        (2, 12)
    }

    fn setup(&self, seats: &[Seat], _seed: u64) -> RulesResult<Outcome<State>> {
        Ok(Outcome::new(State {
            seats: seats.to_vec(),
            scores: vec![0; seats.len()],
            round: 1,
            phase: Phase::Open,
        }))
    }

    fn on_action(
        &self,
        state: State,
        seat: SeatId,
        input: &PhoneInput,
    ) -> RulesResult<Outcome<State>> {
        Ok(buzz_or_next(state, seat, input))
    }

    fn table_view(&self, state: &State) -> RulesResult<Value> {
        let mut view = TableView::new("Buzzer");
        view.status = match state.phase {
            Phase::Open => format!("Round {}: buzz!", state.round),
            Phase::Buzzed { by } => format!("{} buzzed first", state.name(by)),
            Phase::Finished { winner } => format!("{} wins!", state.name(winner)),
        };
        view.players = state
            .seats
            .iter()
            .zip(&state.scores)
            .map(|(seat, score)| PlayerPanel {
                name: seat.name.clone(),
                color: seat.color.clone(),
                lines: vec![format!("{score} points")],
                active: matches!(state.phase, Phase::Buzzed { by } if by == seat.id),
            })
            .collect();
        view.extra = json!({ "round": state.round, "phase": state.phase });
        Ok(view.to_value())
    }

    fn phone_view(&self, state: &State, seat: SeatId) -> RulesResult<Widget> {
        Ok(phone(state, seat))
    }
}

fn buzz_or_next(mut state: State, seat: SeatId, input: &PhoneInput) -> Outcome<State> {
    match (state.phase, input.widget.as_str()) {
        (Phase::Open, "buzz") => {
            let Some(score) = state.scores.get_mut(seat as usize) else {
                return Outcome::new(state);
            };
            *score += 1;
            let won = *score >= WINNING_SCORE;
            let name = state.name(seat).to_owned();
            if won {
                state.phase = Phase::Finished { winner: seat };
                let finished = Results {
                    ranking: ranking(&state.scores),
                    summary: format!("{name} wins!"),
                };
                let mut out = Outcome::new(state)
                    .with_event(Event::Sound {
                        name: "fanfare".into(),
                    })
                    .with_event(Event::Feedback {
                        seat,
                        kind: Feedback::Success,
                    });
                out.finished = Some(finished);
                out
            } else {
                state.phase = Phase::Buzzed { by: seat };
                Outcome::new(state)
                    .with_event(Event::Sound {
                        name: "buzz".into(),
                    })
                    .with_event(Event::Toast {
                        text: format!("{name} buzzed first"),
                    })
                    .with_event(Event::Feedback {
                        seat,
                        kind: Feedback::Success,
                    })
            }
        }
        (Phase::Buzzed { .. }, "next_round") if seat == LEADER => {
            state.round += 1;
            state.phase = Phase::Open;
            Outcome::new(state)
        }
        // Late buzzes and stray input change nothing.
        _ => Outcome::new(state),
    }
}

fn phone(state: &State, seat: SeatId) -> Widget {
    let score = state.scores.get(seat as usize).copied().unwrap_or(0);
    let header = Widget::text(format!("Round {} · you have {score}", state.round));
    let body = match state.phase {
        Phase::Open => vec![Widget::button("buzz", "Buzz!", ButtonStyle::Huge)],
        Phase::Buzzed { by } => {
            let mut items = vec![Widget::title(if by == seat {
                "You were first!".to_owned()
            } else {
                format!("{} was first", state.name(by))
            })];
            if seat == LEADER {
                items.push(Widget::button(
                    "next_round",
                    "Next round",
                    ButtonStyle::Primary,
                ));
            } else {
                items.push(Widget::text("Waiting for the next round"));
            }
            items
        }
        Phase::Finished { winner } => vec![Widget::title(if winner == seat {
            "You win!".to_owned()
        } else {
            format!("{} wins", state.name(winner))
        })],
    };
    let mut children = vec![header];
    children.extend(body);
    Widget::Column { children }
}

/// Seats from highest score to lowest, ties grouped.
fn ranking(scores: &[u32]) -> Vec<Vec<SeatId>> {
    rank_by_score(&scores.iter().map(|&s| s as i64).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seats(names: &[&str]) -> Vec<Seat> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| Seat::new(i as SeatId, *n))
            .collect()
    }

    fn setup(names: &[&str]) -> State {
        Buzzer.setup(&seats(names), 1).unwrap().state
    }

    fn act(state: State, seat: SeatId, widget: &str) -> Outcome<State> {
        Buzzer
            .on_action(state, seat, &PhoneInput::press(widget))
            .unwrap()
    }

    fn buzz(state: State, seat: SeatId) -> Outcome<State> {
        act(state, seat, "buzz")
    }

    #[test]
    fn first_buzz_wins_and_late_buzzes_do_nothing() {
        let state = setup(&["Ana", "Bo", "Cy"]);
        let state = buzz(state, 1).state;
        assert_eq!(state.phase, Phase::Buzzed { by: 1 });
        let state = buzz(state, 2).state;
        assert_eq!(state.scores, vec![0, 1, 0]);
    }

    #[test]
    fn only_the_leader_starts_the_next_round() {
        let state = buzz(setup(&["Ana", "Bo"]), 1).state;
        let state = act(state, 1, "next_round").state;
        assert_eq!(state.phase, Phase::Buzzed { by: 1 });
        let state = act(state, 0, "next_round").state;
        assert_eq!((state.phase, state.round), (Phase::Open, 2));
        assert!(Buzzer.phone_view(&state, 1).unwrap().has_input("buzz"));
    }

    #[test]
    fn five_points_finishes_the_game() {
        let mut state = setup(&["Ana", "Bo"]);
        for _ in 0..WINNING_SCORE - 1 {
            state = buzz(state, 1).state;
            state = act(state, 0, "next_round").state;
        }
        let out = buzz(state, 1);
        assert_eq!(out.state.phase, Phase::Finished { winner: 1 });
        assert_eq!(out.finished.unwrap().ranking, vec![vec![1], vec![0]]);
    }
}
