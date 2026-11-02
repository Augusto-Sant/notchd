//! Port of `core/Terminal.js` (no Node test file exists: the tests below are
//! written from reading the JS).
//!
//! The terminal a module shows in its expanded view (`core/terminal/
//! TerminalView.qml`, used by `modules/codex.qml`): whether the widget is
//! installed, the command line it runs, its colors.
//!
//! Probably UI-side: [`PROBE`] (QML source for `Qt.createQmlObject`) and
//! [`SCHEME`] (a qmltermwidget color scheme name) are presentation for the QML
//! widget; they are ported so the strings live in one place.

/// QML that imports only `QMLTermWidget`, from the optional `qmltermwidget`
/// package; `Qt.createQmlObject` throws on it quietly when it is missing
/// (`PROBE`). Probably UI-side.
pub const PROBE: &str = "import QtQuick\nimport QMLTermWidget\nItem {}\n";

/// The shell prelude that moves to notchd's own folder for agents (`OWN_DIR`):
/// `notchd` in the documents folder (`xdg-user-dir`, which says the home when
/// there is none: then `~/Documents`), made first. If it can't be made, the
/// agent starts where the shell is.
pub const OWN_DIR: &str = concat!(
    "d=$(xdg-user-dir DOCUMENTS 2>/dev/null); ",
    "if [ -z \"$d\" ] || [ \"$d\" = \"$HOME\" ]; then d=\"$HOME/Documents\"; fi; ",
    "d=\"$d/notchd\"; mkdir -p -- \"$d\" && cd -- \"$d\"; "
);

/// The built-in qmltermwidget color scheme behind the ANSI colors (`SCHEME`):
/// Tango, whose bright black is a dark gray (Breeze's is orange, and Codex
/// paints your messages with it). Probably UI-side.
pub const SCHEME: &str = "Tango";

/// Qt's `Qt.Key_Escape`.
pub const KEY_ESCAPE: u32 = 0x0100_0000;
/// Qt's `Qt.ControlModifier`.
pub const MOD_CONTROL: u32 = 0x0400_0000;
/// Qt's `Qt.KeypadModifier`, which doesn't count.
pub const MOD_KEYPAD: u32 = 0x2000_0000;

/// JS `String.prototype.trim`: Unicode white space plus the BOM.
fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// `"~"` and `"~/x"` under `home`; empty is the home itself; anything else as
/// given (`expandHome`). A home with trailing slashes loses them; no home at
/// all is `/`. `path` of `None` (JS `undefined`/`null`) is empty.
pub fn expand_home(path: Option<&str>, home: &str) -> String {
    let p = js_trim(path.unwrap_or(""));
    let h = home.trim_end_matches('/');
    if p.is_empty() || p == "~" {
        return if h.is_empty() {
            "/".to_owned()
        } else {
            h.to_owned()
        };
    }
    match p.strip_prefix("~/") {
        Some(rest) => format!("{h}/{rest}"),
        None => p.to_owned(),
    }
}

/// The arguments of `/bin/sh` that run `command` (a command line as typed in
/// the settings, `codex --search`) with `extra` after its own arguments
/// (`shellArgs`), in notchd's own folder when `own_dir` is set. The shell
/// parses the command, so it can be anything a shell takes; `extra` reaches
/// the program as given, never parsed again. `TERM` says what the widget
/// emulates. With `script`, the command is a script of its own
/// (`cd ~/app && npm run dev`, `a; b`), run as is with `extra` as its `"$@"`:
/// `exec` in front of it would run its first command alone.
pub fn shell_args(command: &str, extra: &[String], own_dir: bool, script: bool) -> Vec<String> {
    let cmd = js_trim(command);
    let run = if script {
        cmd.to_owned()
    } else {
        format!("exec {cmd} \"$@\"")
    };
    let prelude = if own_dir { OWN_DIR } else { "" };
    let mut args = vec![
        "-c".to_owned(),
        format!("{prelude}TERM=xterm-256color; export TERM; {run}"),
        "sh".to_owned(),
    ];
    args.extend_from_slice(extra);
    args
}

/// Whether a key press leaves the terminal (`isLeaveKey`; the notch closes,
/// the program goes on): Ctrl+Escape, which neither Claude Code nor Codex
/// binds. `key` and `modifiers` are as Qt gives them; Shift, Alt or Super
/// with it make another key, the keypad flag doesn't count.
pub fn is_leave_key(key: u32, modifiers: u32) -> bool {
    key == KEY_ESCAPE && (modifiers & !MOD_KEYPAD) == MOD_CONTROL
}

/// The title of a terminal (`plainTitle`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PlainTitle {
    /// Always false: a program of your own is never "working".
    pub working: bool,
    /// The title as the program set it, trimmed.
    pub title: String,
}

/// A program of your own (the config's `terminals`): the terminal's title as
/// it is, never working (`plainTitle`). A shell sets it to where it is, or
/// the command. `None` is an empty title.
pub fn plain_title(raw: Option<&str>) -> PlainTitle {
    PlainTitle {
        working: false,
        title: js_trim(raw.unwrap_or("")).to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    // (written from the JS) expandHome: "~" and "~/x" under home, "" is home, else as given
    #[test]
    fn expand_home_tilde_under_home_empty_is_home_else_as_given() {
        assert_eq!(expand_home(Some("~/code"), "/home/u"), "/home/u/code");
        assert_eq!(expand_home(Some("~"), "/home/u"), "/home/u");
        assert_eq!(expand_home(Some("  "), "/home/u"), "/home/u");
        assert_eq!(expand_home(None, "/home/u"), "/home/u");
        assert_eq!(expand_home(Some("/srv/app"), "/home/u"), "/srv/app");
        assert_eq!(expand_home(Some(" ~/x "), "/home/u"), "/home/u/x");
        assert_eq!(expand_home(Some("~user/x"), "/home/u"), "~user/x");
    }

    // (written from the JS) expandHome: trailing slashes of home go; no home is the root
    #[test]
    fn expand_home_trailing_slashes_go_no_home_is_the_root() {
        assert_eq!(expand_home(Some("~/x"), "/home/u//"), "/home/u/x");
        assert_eq!(expand_home(Some("~"), "/home/u/"), "/home/u");
        assert_eq!(expand_home(Some(""), ""), "/");
        assert_eq!(expand_home(Some("~/x"), ""), "/x");
    }

    // (written from the JS) shellArgs: exec the command with "$@", extra after "sh"
    #[test]
    fn shell_args_exec_the_command_with_extra_after_sh() {
        assert_eq!(
            shell_args(" codex --search ", &s(&["resume", "a b"]), false, false),
            s(&[
                "-c",
                "TERM=xterm-256color; export TERM; exec codex --search \"$@\"",
                "sh",
                "resume",
                "a b"
            ])
        );
        assert_eq!(shell_args("x", &[], false, false).len(), 3);
    }

    // (written from the JS) shellArgs: ownDir puts notchd's folder first; script runs as is
    #[test]
    fn shell_args_own_dir_first_script_runs_as_is() {
        let args = shell_args("claude", &[], true, false);
        assert_eq!(
            args[1],
            format!("{OWN_DIR}TERM=xterm-256color; export TERM; exec claude \"$@\"")
        );
        let script = shell_args("cd ~/app && npm run dev", &s(&["--x"]), false, true);
        assert_eq!(
            script,
            s(&[
                "-c",
                "TERM=xterm-256color; export TERM; cd ~/app && npm run dev",
                "sh",
                "--x"
            ])
        );
        assert!(!script[1].contains("exec"));
    }

    // (written from the JS) isLeaveKey: Ctrl+Escape only; the keypad flag doesn't count
    #[test]
    fn is_leave_key_ctrl_escape_only_keypad_flag_ignored() {
        assert!(is_leave_key(KEY_ESCAPE, MOD_CONTROL));
        assert!(is_leave_key(KEY_ESCAPE, MOD_CONTROL | MOD_KEYPAD));
        assert!(!is_leave_key(KEY_ESCAPE, 0));
        assert!(!is_leave_key(KEY_ESCAPE, MOD_KEYPAD));
        assert!(!is_leave_key(KEY_ESCAPE, MOD_CONTROL | 0x0200_0000)); // Ctrl+Shift
        assert!(!is_leave_key(KEY_ESCAPE, MOD_CONTROL | 0x0800_0000)); // Ctrl+Alt
        assert!(!is_leave_key(0x41, MOD_CONTROL)); // Ctrl+A
    }

    // (written from the JS) plainTitle: the title as it is, trimmed, never working
    #[test]
    fn plain_title_as_it_is_never_working() {
        assert_eq!(
            plain_title(Some("  ~/code ")),
            PlainTitle {
                working: false,
                title: "~/code".into()
            }
        );
        assert_eq!(
            plain_title(None),
            PlainTitle {
                working: false,
                title: String::new()
            }
        );
    }

    // (written from the JS) the constants the QML side reads
    #[test]
    fn constants_the_view_reads() {
        assert!(PROBE.contains("import QMLTermWidget"));
        assert_eq!(SCHEME, "Tango");
        assert!(OWN_DIR.ends_with("cd -- \"$d\"; "));
    }
}
