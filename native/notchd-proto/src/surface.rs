//! Free surfaces (tier 3): a rectangle the UI hands to something that draws itself.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceKind {
    Terminal,
    Webview,
}

/// Core to UI: a surface came into use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceOpen {
    pub r#ref: String,
    pub kind: SurfaceKind,
    pub cols: u32,
    pub rows: u32,
}

/// `surface-close` (core to UI) and `attach` (UI to core).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceRef {
    pub r#ref: String,
}

/// `attach` (UI to core), the first line of a surface connection: which
/// surface, and the size of the UI's view of it, so the program starts at it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attach {
    pub r#ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u32>,
    /// How the view draws: absent, the program's bytes (`data`) for a widget
    /// that emulates a terminal itself; `grid`, the daemon emulates it and
    /// sends the screen as styled rows (`grid`), and takes keys as `key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<String>,
    /// `render: grid`: the view's default foreground and background
    /// (`#rrggbb`, its theme's), what the terminal answers when the program
    /// asks for them (OSC 10/11) and draws text without colors of its own in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
    /// `render: grid`: the view's theme's ANSI colors, `#rrggbb`, from
    /// color 0 on (16 at most; the rest of the 256 are the terminal's own).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<Vec<String>>,
    /// `render: grid`: the size of a cell in the view's pixels, for what
    /// the program sizes in pixels (kitty images, size reports).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cw: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ch: Option<u32>,
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding, the encoding of a surface's bytes.
pub fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The bytes of standard base64 text; `None` for anything else.
pub fn b64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    let mut pad = 0;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                pad += 1;
                continue;
            }
            b'\r' | b'\n' => continue,
            _ => return None,
        };
        if pad > 0 {
            return None;
        }
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_every_length_and_matches_the_standard() {
        assert_eq!(b64_encode(b""), "");
        assert_eq!(b64_encode(b"f"), "Zg==");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
        assert_eq!(b64_encode(b"foo"), "Zm9v");
        assert_eq!(b64_encode(b"ls\n"), "bHMK");
        for n in 0..70usize {
            let bytes: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(b64_decode(&b64_encode(&bytes)), Some(bytes));
        }
        assert_eq!(b64_decode("a$b"), None);
        assert_eq!(b64_decode("Zg==Zg"), None, "data after the padding");
    }
}

/// What happened on a surface. `data` and `title` come from the core; `input`
/// and `resize` from the UI. Bytes are base64.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "lowercase")]
pub enum SurfaceEv {
    Data {
        b64: String,
    },
    Input {
        b64: String,
    },
    /// The view's size in cells; `cw`, `ch` a cell's in pixels (a grid's).
    Resize {
        cols: u32,
        rows: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cw: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ch: Option<u32>,
    },
    Title {
        text: String,
    },
    /// Core to UI, `render: grid`: the screen, or the rows of it that changed.
    Grid(Grid),
    /// UI to core, `render: grid`: a key pressed. The core encodes it for the
    /// program (legacy, or the kitty keyboard protocol when it asked for it).
    Key(KeyPress),
    /// UI to core, `render: grid`: the wheel, in rows (negative: up), at
    /// the cell the pointer is on (`x`, `y`, in cells; for a program that
    /// asked for the mouse).
    Wheel {
        rows: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
    },
    /// UI to core, `render: grid`: the pointer. The core decides what it
    /// does: the program's, when it asked for the mouse (the grid's `mouse`)
    /// and Shift is up; else it selects (a drag; two clicks a word, three a
    /// line). A motion with no button held is only sent while `mouse`.
    Mouse(MouseEv),
    /// Core to UI, `render: grid`: put this text on the clipboard (a copy,
    /// or the program's OSC 52).
    Copy {
        text: String,
    },
    /// Core to UI, `render: grid`: send the clipboard's text as `paste`
    /// (the paste key was pressed).
    Clipboard {},
    /// UI to core, `render: grid`: the clipboard's text, to paste (the core
    /// encodes it: bracketed when the program asked).
    Paste {
        text: String,
    },
    /// Core to UI, `render: grid`: an image a program sent (the kitty
    /// graphics protocol), as a PNG in base64: what a grid's `images` with
    /// this `id` draw. Sent again when the image changes.
    Image {
        id: u32,
        png: String,
    },
}

/// Where an image is drawn: from cell `x`, `y` of the screen (negative when
/// it starts above it), `xo`, `yo` pixels into that cell, `w` by `h` pixels,
/// of the image's `sx`, `sy`, `sw`, `sh` (pixels). `z` below 0 is under the
/// text, else over it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridImage {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub xo: u32,
    pub yo: u32,
    pub w: u32,
    pub h: u32,
    pub sx: u32,
    pub sy: u32,
    pub sw: u32,
    pub sh: u32,
    pub z: i32,
}

/// `action` is `press`, `release` or `motion`; `button` `left`, `middle`
/// or `right` (absent for a motion with none held); `x`, `y` where, in
/// cells from the top left (fractions within a cell); `mods` as a key's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MouseEv {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<String>,
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub mods: u32,
}

/// A frame of a grid surface. `full`: every row is in `lines` (the first
/// frame, a resize, a switch of screens); otherwise only the rows that
/// changed. Colors are `#rrggbb`; `fg`/`bg` are the defaults a run without
/// its own uses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    pub cols: u32,
    pub rows: u32,
    pub full: bool,
    pub fg: String,
    pub bg: String,
    /// The program asked for the mouse: the view sends it `mouse` events
    /// (clicks, drags, the wheel) instead of using them itself.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mouse: bool,
    /// The images on the screen, when they changed since the last frame
    /// (`[]`: none any more); absent, the same as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<GridImage>>,
    /// The cursor, absent when hidden or off the screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<GridCursor>,
    pub lines: Vec<GridLine>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridCursor {
    pub x: u32,
    pub y: u32,
    /// `block`, `bar`, `underline` or `hollow`.
    pub style: String,
}

/// A row as runs of one style; cells not covered are blank.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridLine {
    pub y: u32,
    pub runs: Vec<GridRun>,
    /// The selected columns of the row, first and last (inclusive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sel: Option<[u32; 2]>,
}

/// `w` cells from column `x`. `a` is a bit set: 1 bold, 2 italic,
/// 4 underline, 8 strikethrough, 16 faint, 32 inverse (already applied to
/// `fg`/`bg`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridRun {
    pub x: u32,
    pub w: u32,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub a: u32,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// A key, named as the W3C `KeyboardEvent.code` names it (`KeyA`,
/// `Enter`, `ArrowUp`...); `mods` a bit set: 1 shift, 2 alt, 4 ctrl,
/// 8 super; `text` what it types, when it types something.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyPress {
    pub key: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub mods: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceMsg {
    pub r#ref: String,
    #[serde(flatten)]
    pub ev: SurfaceEv,
}
