//! The shared table as a game describes it: a 2D scene any screen can draw,
//! plus player panels and a log. The desktop host draws it with Bevy, the web
//! client with a canvas. A native game (like the wizards duel) may also put
//! extra data in `extra` for its own renderer.
//!
//! Everything here is public: it goes to every screen, so games must never
//! put a seat's private information in it.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TableView {
    pub title: String,
    /// One line about what is happening, e.g. "Ana's turn".
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub scene: Option<Scene>,
    #[serde(default)]
    pub players: Vec<PlayerPanel>,
    /// Recent happenings, newest last.
    #[serde(default)]
    pub log: Vec<String>,
    /// Data for a game's own renderer. Ignored by generic renderers.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    #[ts(optional, type = "unknown")]
    pub extra: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlayerPanel {
    pub name: String,
    /// A CSS colour.
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub lines: Vec<String>,
    /// Whose turn it is, or who is being waited on.
    #[serde(default)]
    pub active: bool,
}

/// A drawing in scene units, `width` by `height`, y down.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub background: String,
    pub shapes: Vec<Shape>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum Shape {
    /// A pointy-top hexagon centred on `(x, y)` with circumradius `r`.
    Hex {
        x: f32,
        y: f32,
        r: f32,
        fill: String,
        #[serde(default)]
        stroke: String,
        #[serde(default)]
        label: String,
    },
    Circle {
        x: f32,
        y: f32,
        r: f32,
        fill: String,
        #[serde(default)]
        stroke: String,
        #[serde(default)]
        label: String,
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: String,
        #[serde(default)]
        stroke: String,
        #[serde(default)]
        label: String,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: String,
        width: f32,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        size: f32,
        #[serde(default)]
        color: String,
    },
}

impl Shape {
    /// Number of shapes is capped by the host, like phone screens.
    pub fn label(&self) -> &str {
        match self {
            Shape::Hex { label, .. } | Shape::Circle { label, .. } | Shape::Rect { label, .. } => {
                label
            }
            Shape::Text { text, .. } => text,
            Shape::Line { .. } => "",
        }
    }
}

impl TableView {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("a table view always serializes")
    }
}
