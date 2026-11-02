//! The primitive tree, v1 (docs/architecture.md, "Primitive tree").

use crate::Validate;
use crate::canvas::Cmd;
use crate::common::{Align, TextSize, Tone};
use crate::num::Num;
use crate::surface::SurfaceKind;
use crate::version::{MAX_CANVAS_FPS, MAX_TREE_DEPTH, MAX_TREE_NODES};
use serde::{Deserialize, Serialize};

/// What every node may carry.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Common {
    /// Stable across updates, so a UI may animate a changed node in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Default true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    /// Fires an `event` on press or tap; absent means not interactive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    /// In a `row` or `column`: this child takes the rest of the space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<bool>,
    /// A fixed width in design pixels, for a node whose natural one would
    /// shift (a number that changes) or that a design fixes (a bar's length).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Num>,
    /// A fixed height in design pixels (a vertical bar's length).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<Num>,
    /// At least this wide, in design pixels.
    #[serde(rename = "minWidth", default, skip_serializing_if = "Option::is_none")]
    pub min_width: Option<Num>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Node {
    Row(Layout),
    Column(Layout),
    Stack(Stack),
    Spacer(Spacer),
    Text(Text),
    Icon(Icon),
    Image(Image),
    Progress(Progress),
    Meter(Meter),
    Equalizer(Equalizer),
    List(List),
    Button(Button),
    Chip(Chip),
    Canvas(Canvas),
    Surface(SurfaceNode),
}

/// `row` and `column`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    #[serde(flatten)]
    pub common: Common,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    /// At most this wide, in design pixels: the natural width up to it.
    #[serde(rename = "maxWidth", default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<Num>,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stack {
    #[serde(flatten)]
    pub common: Common,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spacer {
    #[serde(flatten)]
    pub common: Common,
    /// A fixed gap; absent with `fill: true` is a flexible one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Num>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Elide {
    End,
    None,
}

/// Always plain text, never rich.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Text {
    #[serde(flatten)]
    pub common: Common,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<TextSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabular: Option<bool>,
    /// A heavier weight: a track's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elide: Option<Elide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Icon {
    #[serde(flatten)]
    pub common: Common,
    /// A codepoint string (Nerd Font): the fallback for a `name` the UI does
    /// not know. An icon needs a `glyph` or a `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,
    /// A named icon (`icons::NAMES`); an unknown one is not an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<IconSize>,
}

/// Design pixels, or a text size (the icon beside a text of that size).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IconSize {
    Px(Num),
    Text(TextSize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Radius {
    None,
    Small,
    Round,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    Crop,
    Contain,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Image {
    #[serde(flatten)]
    pub common: Common,
    /// An absolute path, `file://` or `https://`.
    pub src: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<Radius>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<Fit>,
    /// An `icon` node, drawn while loading and on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Box<Node>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    H,
    V,
}

/// `value + rate * (now - at)`, interpolated by the UI.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    /// Epoch milliseconds.
    pub at: u64,
    /// Fraction per second.
    pub rate: Num,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    #[serde(flatten)]
    pub common: Common,
    /// 0..1; absent (or `null`) draws no bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<Orientation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    /// An action id; fires `event` with the fraction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seek: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeterStyle {
    Disc,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Meter {
    #[serde(flatten)]
    pub common: Common,
    /// The normalized level, 0..1.
    pub value: Num,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<MeterStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Num>,
    /// A node drawn inside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child: Option<Box<Node>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EqMode {
    Smooth,
    Light,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Equalizer {
    #[serde(flatten)]
    pub common: Common,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<EqMode>,
    /// Default 4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bars: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<Num>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stick {
    Start,
    End,
}

/// A scrolling column.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct List {
    #[serde(flatten)]
    pub common: Common,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stick: Option<Stick>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<Num>,
    pub children: Vec<Node>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonVariant {
    Icon,
    Outline,
}

/// Its press is `common.action`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Button {
    #[serde(flatten)]
    pub common: Common,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,
    /// A named icon, as an `icon`'s; `glyph` is the fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    /// Default true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<ButtonVariant>,
    /// A larger round button (36 px, a larger glyph): the main control of a row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub big: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chip {
    #[serde(flatten)]
    pub common: Common,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
}

/// `animate: false` (default) or `animate: {"fps": n}`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Animate {
    Flag(bool),
    Fps { fps: u32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    #[serde(flatten)]
    pub common: Common,
    pub w: Num,
    pub h: Num,
    pub cmds: Vec<Cmd>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animate: Option<Animate>,
}

/// The `surface` node (tier 3). Its wire name is `surface`, like the message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceNode {
    #[serde(flatten)]
    pub common: Common,
    pub kind: SurfaceKind,
    /// Names the surface: `term:claude`.
    pub r#ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<Num>,
}

impl Node {
    /// The node's `t`.
    pub fn kind(&self) -> &'static str {
        match self {
            Node::Row(_) => "row",
            Node::Column(_) => "column",
            Node::Stack(_) => "stack",
            Node::Spacer(_) => "spacer",
            Node::Text(_) => "text",
            Node::Icon(_) => "icon",
            Node::Image(_) => "image",
            Node::Progress(_) => "progress",
            Node::Meter(_) => "meter",
            Node::Equalizer(_) => "equalizer",
            Node::List(_) => "list",
            Node::Button(_) => "button",
            Node::Chip(_) => "chip",
            Node::Canvas(_) => "canvas",
            Node::Surface(_) => "surface",
        }
    }

    fn children(&self) -> Vec<&Node> {
        match self {
            Node::Row(l) | Node::Column(l) => l.children.iter().collect(),
            Node::Stack(s) => s.children.iter().collect(),
            Node::List(l) => l.children.iter().collect(),
            Node::Image(i) => i.fallback.iter().map(|b| &**b).collect(),
            Node::Meter(m) => m.child.iter().map(|b| &**b).collect(),
            _ => Vec::new(),
        }
    }

    fn check_own(&self) -> Result<(), String> {
        let fraction = |v: Num, what: &str| {
            if (0.0..=1.0).contains(&v.0) {
                Ok(())
            } else {
                Err(format!("{what} must be within 0..1, got {}", v.0))
            }
        };
        match self {
            Node::Progress(p) => p.value.map_or(Ok(()), |v| fraction(v, "progress.value")),
            Node::Meter(m) => fraction(m.value, "meter.value"),
            Node::Image(i) => match i.fallback.as_deref() {
                Some(Node::Icon(_)) | None => Ok(()),
                Some(other) => Err(format!(
                    "image.fallback must be an icon, got {}",
                    other.kind()
                )),
            },
            Node::Icon(i) if i.glyph.is_none() && i.name.is_none() => {
                Err("icon needs a glyph or a name".into())
            }
            Node::Text(t) if t.lines == Some(0) => Err("text.lines must be at least 1".into()),
            Node::Equalizer(e) if e.bars == Some(0) => {
                Err("equalizer.bars must be at least 1".into())
            }
            Node::Canvas(c) => {
                if c.w.0 <= 0.0 || c.h.0 <= 0.0 {
                    return Err("canvas.w and canvas.h must be positive".into());
                }
                if let Some(Animate::Fps { fps }) = c.animate
                    && !(1..=MAX_CANVAS_FPS).contains(&fps)
                {
                    return Err(format!("canvas.animate.fps must be 1..={MAX_CANVAS_FPS}"));
                }
                Ok(())
            }
            Node::Surface(s) if s.r#ref.is_empty() => Err("surface.ref must not be empty".into()),
            _ => Ok(()),
        }
    }

    fn walk(&self, depth: usize, count: &mut usize) -> Result<(), String> {
        *count += 1;
        if depth > MAX_TREE_DEPTH {
            return Err(format!("tree deeper than {MAX_TREE_DEPTH}"));
        }
        if *count > MAX_TREE_NODES {
            return Err(format!("tree has more than {MAX_TREE_NODES} nodes"));
        }
        self.check_own()?;
        for child in self.children() {
            child.walk(depth + 1, count)?;
        }
        Ok(())
    }
}

impl Validate for Node {
    /// Nesting and node-count limits, ranges, the image fallback being an icon.
    fn validate(&self) -> Result<(), String> {
        self.walk(1, &mut 0)
    }
}
