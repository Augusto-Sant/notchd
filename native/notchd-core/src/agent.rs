//! The rules of the coding agents' modules (ported from `core/Agent.js`, gone
//! with 3.5; its tests live on as `tests/vectors/rules/agent.json`).
//!
//! Coding agents in the notch, each in its own terminal UI. What the closed
//! notch shows comes from the terminal's title, which the agent sets itself.
//! Where the JS took `undefined`/`null` for "no title", pass `""`.

use serde::{Deserialize, Serialize};

use crate::js;

/// What an agent's terminal title says (`{ working, title }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTitle {
    pub working: bool,
    pub title: String,
}

fn trim(raw: &str) -> &str {
    raw.trim_matches(js::is_ws)
}

/// The `ID` regexp: a lowercase or uppercase hex UUID, hyphenated.
fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Drops the first character and trims, as `t.slice(1).trim()`. Only used
/// for the BMP marker characters, where a UTF-16 unit is one `char`.
fn after_marker(t: &str) -> &str {
    let mut chars = t.chars();
    chars.next();
    trim(chars.as_str())
}

fn is_braille(c: char) -> bool {
    ('\u{2800}'..='\u{28FF}').contains(&c)
}

/// `codexTitle`: Codex with its default `tui.terminal_title` (spinner,
/// thread, project): a braille spinner and a space while it works, then
/// "<thread> | <project>", or just the project until Codex has named the
/// thread. An id as the thread is no title.
pub fn codex_title(raw: &str) -> AgentTitle {
    let mut t = trim(raw);
    let mut working = false;
    if t.chars().next().is_some_and(is_braille) {
        working = true;
        t = after_marker(t);
    }
    let parts: Vec<&str> = t.split(" | ").collect();
    let mut title = if parts.len() > 1 {
        trim(&parts[..parts.len() - 1].join(" | ")).to_string()
    } else {
        String::new()
    };
    if is_uuid(&title) {
        title.clear();
    }
    AgentTitle { working, title }
}

/// `claudeTitle`: Claude Code: "✳ <title>" while idle, a turning half
/// circle (◐◑◒◓) in place of the ✳ while it works. The title is "Claude
/// Code" until Claude Code has named the conversation.
pub fn claude_title(raw: &str) -> AgentTitle {
    let mut t = trim(raw);
    let mut working = false;
    match t.chars().next() {
        Some('\u{2733}') => t = after_marker(t),
        Some(c) if ('\u{25D0}'..='\u{25D3}').contains(&c) || is_braille(c) => {
            working = true;
            t = after_marker(t);
        }
        _ => {}
    }
    AgentTitle {
        working,
        title: if t == "Claude Code" {
            String::new()
        } else {
            t.to_string()
        },
    }
}

/// The inputs of [`done_peek`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PeekState {
    pub was_working: bool,
    pub working: bool,
    pub open: bool,
    pub typed: bool,
}

/// `donePeek`: whether to say the agent is done (a peek): it stopped
/// working while no view was open, after you had typed to it.
pub fn done_peek(s: &PeekState) -> bool {
    s.was_working && !s.working && !s.open && s.typed
}

/// An IPC command's action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Toggle,
    New,
    Ask,
    Usage,
}

/// A parsed IPC command (`{ action, text }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub action: Action,
    pub text: String,
}

/// Collapses every whitespace run holding a line break into one space
/// (`replace(/\s*[\r\n]+\s*/g, " ")`), so the text is one line.
fn one_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.contains(['\r', '\n']) {
            out.push(' ');
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in s.chars() {
        if js::is_ws(c) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// `parseCommand`: "toggle" opens the agent's notch on it, or closes it if
/// it shows it; "new" starts a new session; "ask <text>" opens the notch and
/// types the text in ("ask" alone only opens it); else usage. The text is
/// one line: a line break would send it early.
pub fn parse_command(raw: &str) -> Command {
    let cmd = trim(raw);
    let (action, text) = match cmd {
        "toggle" => (Action::Toggle, String::new()),
        "new" => (Action::New, String::new()),
        "ask" => (Action::Ask, String::new()),
        _ if cmd.starts_with("ask ") => (Action::Ask, trim(&one_line(&cmd[3..])).to_string()),
        _ => (Action::Usage, String::new()),
    };
    Command { action, text }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(working: bool, title: &str) -> AgentTitle {
        AgentTitle {
            working,
            title: title.to_string(),
        }
    }

    fn command(action: Action, text: &str) -> Command {
        Command {
            action,
            text: text.to_string(),
        }
    }

    // codexTitle: the spinner means working
    #[test]
    fn codex_title_the_spinner_means_working() {
        assert_eq!(codex_title("⠋ job-stuff"), title(true, ""));
        assert_eq!(
            codex_title("⠼ Fix the login bug | job-stuff"),
            title(true, "Fix the login bug")
        );
        assert_eq!(
            codex_title("Fix the login bug | job-stuff"),
            title(false, "Fix the login bug")
        );
    }

    // codexTitle: the project alone is no title
    #[test]
    fn codex_title_the_project_alone_is_no_title() {
        assert_eq!(codex_title("job-stuff"), title(false, ""));
        assert_eq!(codex_title(""), title(false, ""));
    }

    // codexTitle: a thread named with the separator keeps it; an id is no title
    #[test]
    fn codex_title_a_thread_named_with_the_separator_keeps_it_an_id_is_no_title() {
        assert_eq!(codex_title("A | B | proj"), title(false, "A | B"));
        assert_eq!(
            codex_title("⠙ 01a194a2-ebb9-7443-ad26-3577def90666 | proj"),
            title(true, "")
        );
    }

    // claudeTitle: ✳ is idle, a half circle is working
    #[test]
    fn claude_title_idle_star_is_idle_a_half_circle_is_working() {
        assert_eq!(claude_title("✳ Claude Code"), title(false, ""));
        assert_eq!(claude_title("◐ Claude Code"), title(true, ""));
        assert_eq!(
            claude_title("◑ Simple acknowledgment reply"),
            title(true, "Simple acknowledgment reply")
        );
        assert_eq!(
            claude_title("✳ Simple acknowledgment reply"),
            title(false, "Simple acknowledgment reply")
        );
    }

    // claudeTitle: no title before it starts
    #[test]
    fn claude_title_no_title_before_it_starts() {
        assert_eq!(claude_title(""), title(false, ""));
    }

    // donePeek: only when work ends with no view open, after typing
    #[test]
    fn done_peek_only_when_work_ends_with_no_view_open_after_typing() {
        let base = PeekState {
            was_working: true,
            working: false,
            open: false,
            typed: true,
        };
        assert!(done_peek(&base));
        assert!(
            !done_peek(&PeekState { open: true, ..base }),
            "you are looking at it"
        );
        assert!(
            !done_peek(&PeekState {
                typed: false,
                ..base
            }),
            "the spinner while it starts"
        );
        assert!(!done_peek(&PeekState {
            working: true,
            ..base
        }));
        assert!(!done_peek(&PeekState {
            was_working: false,
            ..base
        }));
    }

    // parseCommand: new, ask with or without text, else usage
    #[test]
    fn parse_command_new_ask_with_or_without_text_else_usage() {
        assert_eq!(parse_command(" new "), command(Action::New, ""));
        assert_eq!(parse_command("ask"), command(Action::Ask, ""));
        assert_eq!(
            parse_command("ask  why is the sky blue? "),
            command(Action::Ask, "why is the sky blue?")
        );
        assert_eq!(parse_command("asking"), command(Action::Usage, ""));
        assert_eq!(parse_command(""), command(Action::Usage, ""));
    }

    // parseCommand: a line break in the question would send it early
    #[test]
    fn parse_command_a_line_break_in_the_question_would_send_it_early() {
        assert_eq!(
            parse_command("ask first\nsecond\r\n third"),
            command(Action::Ask, "first second third")
        );
    }

    // parseCommand: toggle
    #[test]
    fn parse_command_toggle() {
        assert_eq!(parse_command(" toggle "), command(Action::Toggle, ""));
        assert_eq!(parse_command("toggle now").action, Action::Usage);
    }
}
