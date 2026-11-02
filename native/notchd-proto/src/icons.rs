//! The named icons a view may carry in `name` (an `icon` node, a `button`):
//! the one table of what the official modules send. A UI maps a name to its own
//! symbol (a Nerd Font glyph, an Adwaita symbolic icon, an SF Symbol) and falls
//! back to the node's `glyph` for a name it does not know.
//!
//! Scheme: the freedesktop icon naming specification's name where a standard
//! icon exists (`media-playback-start`), without the `-symbolic` suffix (a GNOME
//! UI appends it); otherwise a short lowercase description. The list grows with
//! the modules; an unknown name is never an error, so a user module may send
//! one a UI has not heard of.

/// Every name the official modules send, in the order the docs list them.
pub const NAMES: &[&str] = &[
    "media-playback-start",
    "media-playback-pause",
    "media-playback-stop",
    "media-skip-forward",
    "media-skip-backward",
    "audio-x-generic",
    "audio-input-microphone",
    "microphone-disabled",
    "list-add",
    "view-refresh",
    "window-close",
    "alarm",
    "notification",
    "dialog-warning",
    "utilities-terminal",
    "drive-harddisk",
    "cpu",
    "memory",
    "focus",
    "coffee",
];

#[cfg(test)]
mod tests {
    use super::NAMES;

    #[test]
    fn names_are_unique_lowercase_words() {
        let mut seen = std::collections::HashSet::new();
        for n in NAMES {
            assert!(seen.insert(*n), "{n} twice");
            assert!(
                !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{n}"
            );
        }
    }
}
