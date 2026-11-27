//! Schema checks on what the rules return, so a buggy or hostile game can't
//! send the host something it would choke on.

use hostable_protocol::{Shape, TableView};
use hostable_rules_api::{Event, Outcome, RulesError};
use serde_json::Value;

/// Most events one call may emit.
pub const MAX_EVENTS: usize = 64;
/// Most shapes a table scene may hold.
pub const MAX_TABLE_SHAPES: usize = 4000;
/// Largest table view, as JSON.
pub const MAX_TABLE_BYTES: usize = 512 * 1024;
const MAX_TEXT: usize = 2000;

/// Check an outcome against the number of seats in the game.
pub fn check_outcome<S>(outcome: &Outcome<S>, seats: u32) -> Result<(), RulesError> {
    let bad = |m: String| Err(RulesError::Malformed(m));
    if outcome.events.len() > MAX_EVENTS {
        return bad(format!("{} events in one call", outcome.events.len()));
    }
    for event in &outcome.events {
        match event {
            Event::Feedback { seat, .. } if *seat >= seats => {
                return bad(format!("feedback for seat {seat}, which doesn't exist"))
            }
            Event::Sound { name } if name.len() > 64 => return bad("sound name too long".into()),
            Event::Toast { text } if text.len() > MAX_TEXT => return bad("toast too long".into()),
            Event::Animate { what } if what.to_string().len() > 16 * 1024 => {
                return bad("animation too large".into())
            }
            _ => {}
        }
    }
    if outcome.waiting_on.len() > seats as usize * 4 {
        return bad(format!("{} waits", outcome.waiting_on.len()));
    }
    for wait in &outcome.waiting_on {
        if wait.seat >= seats {
            return bad(format!(
                "waiting on seat {}, which doesn't exist",
                wait.seat
            ));
        }
        if wait.prompt.len() > MAX_TEXT {
            return bad("wait prompt too long".into());
        }
    }
    if let Some(results) = &outcome.finished {
        let mut seen = Vec::new();
        for seat in results.ranking.iter().flatten() {
            if *seat >= seats || seen.contains(seat) {
                return bad(format!("ranking lists seat {seat} wrongly"));
            }
            seen.push(*seat);
        }
        if results.summary.len() > MAX_TEXT {
            return bad("summary too long".into());
        }
    }
    Ok(())
}

/// Check a table view: it must be a [`TableView`] of bounded size.
pub fn check_table(table: &Value) -> Result<TableView, RulesError> {
    let size = table.to_string().len();
    if size > MAX_TABLE_BYTES {
        return Err(RulesError::Malformed(format!("table view is {size} bytes")));
    }
    let view: TableView = serde_json::from_value(table.clone())
        .map_err(|e| RulesError::Malformed(format!("table view: {e}")))?;
    if let Some(scene) = &view.scene {
        if scene.shapes.len() > MAX_TABLE_SHAPES {
            return Err(RulesError::Malformed(format!(
                "{} shapes on the table",
                scene.shapes.len()
            )));
        }
        let finite = |v: f32| v.is_finite();
        let ok = scene.shapes.iter().all(|s| match s {
            Shape::Hex { x, y, r, .. } | Shape::Circle { x, y, r, .. } => {
                finite(*x) && finite(*y) && finite(*r)
            }
            Shape::Rect { x, y, w, h, .. } => finite(*x) && finite(*y) && finite(*w) && finite(*h),
            Shape::Line { x1, y1, x2, y2, .. } => {
                finite(*x1) && finite(*y1) && finite(*x2) && finite(*y2)
            }
            Shape::Text { x, y, size, .. } => finite(*x) && finite(*y) && finite(*size),
        });
        if !ok {
            return Err(RulesError::Malformed(
                "table shape at a bad position".into(),
            ));
        }
    }
    Ok(view)
}
