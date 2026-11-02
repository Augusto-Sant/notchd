//! Small enums shared by several messages.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// `State.mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Rest,
    Compact,
    Peek,
    Expanded,
}

/// A module's view slots (`NotchModule`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Slot {
    Compact,
    VerticalCompact,
    Peek,
    VerticalPeek,
    Expanded,
}

impl Slot {
    pub const ALL: [Slot; 5] = [
        Slot::Compact,
        Slot::VerticalCompact,
        Slot::Peek,
        Slot::VerticalPeek,
        Slot::Expanded,
    ];
}

/// The `tone` of a node: the semantic colors a primitive may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Foreground,
    Muted,
    Faint,
    Accent,
    Danger,
    Success,
    Line,
}

impl Tone {
    pub const ALL: [Tone; 7] = [
        Tone::Foreground,
        Tone::Muted,
        Tone::Faint,
        Tone::Accent,
        Tone::Danger,
        Tone::Success,
        Tone::Line,
    ];
}

/// Every theme token (`welcome.tokens`); a canvas may use all of them, a
/// node's `tone` only the [`Tone`] subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Token {
    Foreground,
    Background,
    Accent,
    Danger,
    Success,
    Muted,
    Faint,
    Line,
    Hover,
    Selected,
    Surface,
}

impl Token {
    pub const ALL: [Token; 11] = [
        Token::Foreground,
        Token::Background,
        Token::Accent,
        Token::Danger,
        Token::Success,
        Token::Muted,
        Token::Faint,
        Token::Line,
        Token::Hover,
        Token::Selected,
        Token::Surface,
    ];
}

/// theming.md's five text sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextSize {
    Caption,
    BodySmall,
    Body,
    Heading,
    Display,
}

impl TextSize {
    pub const ALL: [TextSize; 5] = [
        TextSize::Caption,
        TextSize::BodySmall,
        TextSize::Body,
        TextSize::Heading,
        TextSize::Display,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Start,
    Center,
    End,
}
