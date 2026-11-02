//! The terminal's rules (ported from `core/Terminal.js`, which keeps only the
//! widget's side; tests: `tests/vectors/rules/terminal.json`, plus a few marked "(added)").
//!
//! The terminal a module shows in its expanded view (`core/terminal/
//! TerminalView.qml`, used by `modules/codex.qml`): whether the widget is
//! installed, the command line it runs, its colors.
//!
//! Probably UI-side: [`PROBE`] (QML source for `Qt.createQmlObject`) and
//! [`SCHEME`] (a qmltermwidget color scheme name) are presentation for the QML
//! widget; they are ported so the strings live in one place.

use crate::js;

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

/// `"~"` and `"~/x"` under `home`; empty is the home itself; anything else as
/// given (`expandHome`). A home with trailing slashes loses them; no home at
/// all is `/`. `path` of `None` (JS `undefined`/`null`) is empty.
pub fn expand_home(path: Option<&str>, home: &str) -> String {
    let p = js::trim(path.unwrap_or(""));
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
    let cmd = js::trim(command);
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
        title: js::trim(raw.unwrap_or("")).to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    /// Runs `/bin/sh` with `args` in a clean environment (hermetic: only
    /// what the test gives, plus PATH), returning stdout.
    fn run_sh(args: &[String], cwd: Option<&Path>, env: &[(&str, &str)]) -> String {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(args).env_clear();
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
        cmd.envs(env.iter().copied());
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        let out = cmd.output().expect("run /bin/sh");
        String::from_utf8(out.stdout).expect("utf8 output")
    }

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("notchd-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp home");
        dir
    }

    // expandHome: ~ and ~/x under home, the rest as given
    #[test]
    fn expand_home_tilde_under_home_the_rest_as_given() {
        assert_eq!(expand_home(Some("~"), "/home/u"), "/home/u");
        assert_eq!(expand_home(Some(""), "/home/u/"), "/home/u");
        assert_eq!(
            expand_home(Some("  ~/Projects/x "), "/home/u"),
            "/home/u/Projects/x"
        );
        assert_eq!(expand_home(Some("/srv/work"), "/home/u"), "/srv/work");
        assert_eq!(expand_home(Some("~other"), "/home/u"), "~other");
        assert_eq!(expand_home(None, ""), "/");
    }

    // (added) expandHome: trailing slashes of home go; a missing path is home
    #[test]
    fn expand_home_trailing_slashes_go_a_missing_path_is_home() {
        assert_eq!(expand_home(Some("~/x"), "/home/u//"), "/home/u/x");
        assert_eq!(expand_home(Some("~/x"), ""), "/x");
        assert_eq!(expand_home(None, "/home/u"), "/home/u");
    }

    // shellArgs: the command is parsed by sh, the extra arguments are not
    // (run through a real sh: they arrive as given, quotes and brackets included)
    #[test]
    fn shell_args_the_command_is_parsed_by_sh_the_extra_arguments_are_not() {
        let argv = shell_args(
            "printf '%s|' first",
            &s(&["-c", "a=[\"x\",\"y\"]", "it's \"x\" $HOME"]),
            false,
            false,
        );
        assert_eq!(
            run_sh(&argv, None, &[]),
            "first|-c|a=[\"x\",\"y\"]|it's \"x\" $HOME|"
        );
    }

    // shellArgs: TERM says 256 colors
    #[test]
    fn shell_args_term_says_256_colors() {
        let argv = shell_args("printenv TERM", &[], false, false);
        assert_eq!(
            run_sh(&argv, None, &[("TERM", "xterm")]).trim(),
            "xterm-256color"
        );
    }

    // SCHEME: Tango, whose bright black is gray
    #[test]
    fn scheme_tango_whose_bright_black_is_gray() {
        assert_eq!(SCHEME, "Tango");
    }

    // PROBE imports only the widget
    #[test]
    fn probe_imports_only_the_widget() {
        assert!(PROBE.contains("import QMLTermWidget"));
    }

    // isLeaveKey: Ctrl+Escape alone; plain Escape is the agent's
    #[test]
    fn is_leave_key_ctrl_escape_alone_plain_escape_is_the_agents() {
        const SHIFT: u32 = 0x0200_0000;
        assert!(is_leave_key(KEY_ESCAPE, MOD_CONTROL));
        assert!(is_leave_key(KEY_ESCAPE, MOD_CONTROL | MOD_KEYPAD));
        assert!(!is_leave_key(KEY_ESCAPE, 0));
        assert!(!is_leave_key(KEY_ESCAPE, MOD_CONTROL | SHIFT));
        assert!(!is_leave_key(0x43, MOD_CONTROL)); // Ctrl+C goes to the agent
    }

    // (added) isLeaveKey: Alt and a lone keypad flag do not leave
    #[test]
    fn is_leave_key_alt_and_a_lone_keypad_flag_do_not_leave() {
        assert!(!is_leave_key(KEY_ESCAPE, MOD_CONTROL | 0x0800_0000)); // Ctrl+Alt
        assert!(!is_leave_key(KEY_ESCAPE, MOD_KEYPAD));
    }

    // shellArgs: own folder in the documents folder, ~/Documents when there is none
    // (no folder set: notchd's own, notchd in the documents folder, made first)
    #[test]
    fn shell_args_own_folder_in_the_documents_folder_documents_when_there_is_none() {
        let home = temp_home("own-dir");
        let home_s = home.to_str().expect("utf8 path");
        let config = home.join(".config");
        let env = [
            ("HOME", home_s),
            ("XDG_CONFIG_HOME", config.to_str().unwrap()),
        ];
        let pwd = || {
            run_sh(&shell_args("pwd", &[], true, false), Some(&home), &env)
                .trim()
                .to_owned()
        };
        assert_eq!(pwd(), home.join("Documents/notchd").to_str().unwrap());
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("user-dirs.dirs"),
            "XDG_DOCUMENTS_DIR=\"$HOME/Documentos\"\n",
        )
        .unwrap();
        let has_xdg = Command::new("/bin/sh")
            .args(["-c", "command -v xdg-user-dir"])
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .output()
            .is_ok_and(|o| o.status.success());
        if has_xdg {
            assert_eq!(pwd(), home.join("Documentos/notchd").to_str().unwrap());
        }
        assert!(home.join("Documents/notchd").is_dir());
        // A folder of its own: the command runs where it is started.
        let here = run_sh(&shell_args("pwd", &[], false, false), Some(&home), &env);
        assert_eq!(here.trim(), home_s);
        let _ = std::fs::remove_dir_all(&home);
    }

    // plainTitle: a terminal of your own shows its title as it is, never working
    #[test]
    fn plain_title_a_terminal_of_your_own_shows_its_title_as_it_is_never_working() {
        let check = |t: Option<&str>, title: &str| {
            assert_eq!(
                plain_title(t),
                PlainTitle {
                    working: false,
                    title: title.into()
                }
            );
        };
        check(Some(" eryk@box:~ "), "eryk@box:~");
        check(Some("⠋ spinner"), "⠋ spinner");
        check(None, "");
    }

    // shellArgs: a script runs every command in it, its extra arguments as "$@"
    // (bug: exec in front of "a; b" ran `a` in the shell's place and never `b`)
    #[test]
    fn shell_args_a_script_runs_every_command_in_it_its_extra_arguments_as_args() {
        let argv = shell_args(
            "printf a; printf '%s' \"$1\" && echo $TERM",
            &s(&["b"]),
            false,
            true,
        );
        assert_eq!(
            run_sh(&argv, None, &[("TERM", "xterm")]),
            "abxterm-256color\n"
        );
    }

    // (added) shellArgs: the exact line for a command and for a script, own folder first
    #[test]
    fn shell_args_the_exact_line_for_a_command_and_for_a_script() {
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
        let own = shell_args("claude", &[], true, false);
        assert_eq!(
            own[1],
            format!("{OWN_DIR}TERM=xterm-256color; export TERM; exec claude \"$@\"")
        );
        let script = shell_args("cd ~/app && npm run dev", &s(&["--x"]), false, true);
        assert!(!script[1].contains("exec"));
        assert_eq!(script.len(), 4);
    }
}
