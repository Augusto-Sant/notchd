//! Canvas commands (tier 2): a flat list of vector commands in design pixels
//! and token colors.

use crate::common::Token;
use crate::num::Num;
use serde::{Deserialize, Serialize};

/// A token, or a `Theme.mix` of two tokens: `{"mix":["accent","background",0.4]}`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Color {
    Token(Token),
    Mix { mix: (Token, Token, Num) },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cap {
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Join {
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "c", rename_all = "lowercase")]
pub enum Cmd {
    /// Rounded (`r`), filled and/or stroked.
    Rect {
        x: Num,
        y: Num,
        w: Num,
        h: Num,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        r: Option<Num>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<Color>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<Color>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<Num>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opacity: Option<Num>,
    },
    Circle {
        x: Num,
        y: Num,
        r: Num,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<Color>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<Color>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<Num>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opacity: Option<Num>,
    },
    Line {
        x1: Num,
        y1: Num,
        x2: Num,
        y2: Num,
        stroke: Color,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<Num>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cap: Option<Cap>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opacity: Option<Num>,
    },
    /// SVG path data, the `M L Q C Z` subset `Outline.js` emits.
    Path {
        d: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<Color>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<Color>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<Num>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cap: Option<Cap>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        join: Option<Join>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opacity: Option<Num>,
    },
    Text {
        x: Num,
        y: Num,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<crate::common::TextSize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tone: Option<Token>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opacity: Option<Num>,
    },
    /// Everything up to the matching `unclip` is clipped to this rounded rectangle.
    Clip {
        x: Num,
        y: Num,
        w: Num,
        h: Num,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        r: Option<Num>,
    },
    Unclip {},
}

impl crate::Validate for Cmd {}
