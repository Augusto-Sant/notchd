//! `agent.json` (from the removed `core/Agent.js`) against `notchd_core::agent`. A missing title is `""`.
use super::*;
use notchd_core::agent as g;

pub fn call(f: &str, a: &[Value]) -> Option<Value> {
    Some(match f {
        "codexTitle" => to_json(&g::codex_title(&jstext(a, 0))),
        "claudeTitle" => to_json(&g::claude_title(&jstext(a, 0))),
        "donePeek" => json!(g::done_peek(&from_json(arg(a, 0)))),
        "parseCommand" => to_json(&g::parse_command(&jstext(a, 0))),
        _ => return None,
    })
}
