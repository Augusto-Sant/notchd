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
    Data { b64: String },
    Input { b64: String },
    Resize { cols: u32, rows: u32 },
    Title { text: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceMsg {
    pub r#ref: String,
    #[serde(flatten)]
    pub ev: SurfaceEv,
}
