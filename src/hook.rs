use crate::state::AgentState;
use crate::tmux::{self, Socket};
use serde::Deserialize;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Upper bound for each tmux call; the agent waits for hooks, so never hang.
const TMUX_DEADLINE: Duration = Duration::from_secs(2);
const MSG_MAX_CHARS: usize = 60;
/// How much of the transcript's end is scanned for the last assistant text.
const TRANSCRIPT_TAIL_BYTES: u64 = 512 * 1024;

/// The subset of Claude Code's hook JSON that finter needs.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct HookInput {
    #[serde(default)]
    pub(crate) hook_event_name: String,
    #[serde(default)]
    pub(crate) message: Option<String>,
    #[serde(default)]
    pub(crate) notification_type: Option<String>,
    #[serde(default)]
    pub(crate) transcript_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plan {
    Skip,
    Set(AgentState),
    Clear,
}

/// Map a hook event to a pane update, given the pane's current state.
pub(crate) fn decide(
    event: &str,
    notification_type: Option<&str>,
    current: Option<AgentState>,
) -> Plan {
    match event {
        "UserPromptSubmit" | "PreToolUse" => {
            if current == Some(AgentState::Working) {
                Plan::Skip
            } else {
                Plan::Set(AgentState::Working)
            }
        }
        // Claude's idle reminder arrives after Stop; it must not turn `done` into `asking`.
        "Notification" if notification_type == Some("idle_prompt") => Plan::Skip,
        "Notification" => Plan::Set(AgentState::Asking),
        "Stop" => Plan::Set(AgentState::Done),
        "SessionEnd" => Plan::Clear,
        _ => Plan::Skip,
    }
}

/// Collapse whitespace and cut to `max` chars (on a char boundary), marking the cut with `…`.
pub(crate) fn summarize(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        return collapsed;
    }
    let mut cut: String = collapsed.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// Last assistant text block in a JSONL transcript tail.
pub(crate) fn last_assistant_text(jsonl: &str) -> Option<String> {
    for line in jsonl.lines().rev() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(blocks) = value.pointer("/message/content").and_then(|c| c.as_array()) else {
            continue;
        };
        let text = blocks
            .iter()
            .rev()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .find_map(|b| b.get("text").and_then(|t| t.as_str()))
            .filter(|t| !t.trim().is_empty());
        if let Some(text) = text {
            return Some(text.to_string());
        }
    }
    None
}

/// Read at most the last `max_bytes` of a file as complete lines.
fn read_tail(path: &str, max_bytes: u64) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if start == 0 {
        return Ok(text);
    }
    // The first line is probably cut mid-way.
    Ok(text
        .split_once('\n')
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_default())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn current_state(pane: &str) -> Option<AgentState> {
    let out = tmux::run_deadline(
        Socket::Default,
        &["display-message", "-p", "-t", pane, "#{@agent_state}"],
        TMUX_DEADLINE,
    )?;
    AgentState::parse(String::from_utf8_lossy(&out.stdout).trim())
}

fn run_tmux_args(args: &[String]) {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    if tmux::run_deadline(Socket::Default, &refs, TMUX_DEADLINE).is_none() {
        eprintln!("finter hook: tmux did not answer within {TMUX_DEADLINE:?}");
    }
}

/// Entry point of `finter hook <agent>`. Never fails: Claude must not be blocked.
pub fn run_hook(agent: &str) {
    if let Err(e) = try_hook(agent) {
        eprintln!("finter hook: {e}");
    }
}

fn try_hook(agent: &str) -> Result<(), Box<dyn std::error::Error>> {
    if agent != "claude" {
        return Err(format!("unsupported agent `{agent}`").into());
    }
    // No pane means a headless run (`claude -p`): nothing to track.
    let Ok(pane) = std::env::var("TMUX_PANE") else {
        return Ok(());
    };
    if pane.is_empty() {
        return Ok(());
    }

    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let input: HookInput = serde_json::from_str(&raw)?;

    let current = match input.hook_event_name.as_str() {
        "UserPromptSubmit" | "PreToolUse" => current_state(&pane),
        _ => None,
    };
    match decide(
        &input.hook_event_name,
        input.notification_type.as_deref(),
        current,
    ) {
        Plan::Skip => {}
        Plan::Clear => {
            run_tmux_args(&tmux::clear_args(&pane));
        }
        Plan::Set(state) => {
            let msg = match state {
                AgentState::Asking => input
                    .message
                    .as_deref()
                    .map(|m| summarize(m, MSG_MAX_CHARS)),
                AgentState::Done => input
                    .transcript_path
                    .as_deref()
                    .and_then(|p| read_tail(p, TRANSCRIPT_TAIL_BYTES).ok())
                    .and_then(|t| last_assistant_text(&t))
                    .map(|t| summarize(&t, MSG_MAX_CHARS)),
                AgentState::Working => None,
            }
            .unwrap_or_default();
            let args = tmux::set_state_args(&pane, agent, state, now_secs(), &msg);
            run_tmux_args(&args);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSCRIPT: &str = include_str!("../tests/fixtures/transcript.jsonl");

    fn parse(json: &str) -> HookInput {
        serde_json::from_str(json).expect("hook json")
    }

    #[test]
    fn hook_json_maps_to_states() {
        let ev = |json: &str, cur| {
            let i = parse(json);
            decide(&i.hook_event_name, i.notification_type.as_deref(), cur)
        };
        assert_eq!(
            ev(
                r#"{"hook_event_name":"UserPromptSubmit","prompt":"hi"}"#,
                None
            ),
            Plan::Set(AgentState::Working)
        );
        assert_eq!(
            ev(
                r#"{"hook_event_name":"PreToolUse","tool_name":"Bash"}"#,
                Some(AgentState::Asking)
            ),
            Plan::Set(AgentState::Working)
        );
        assert_eq!(
            ev(r#"{"hook_event_name":"Notification","message":"x"}"#, None),
            Plan::Set(AgentState::Asking)
        );
        assert_eq!(
            ev(r#"{"hook_event_name":"Stop"}"#, Some(AgentState::Working)),
            Plan::Set(AgentState::Done)
        );
        assert_eq!(ev(r#"{"hook_event_name":"SessionEnd"}"#, None), Plan::Clear);
        assert_eq!(ev(r#"{"hook_event_name":"Other"}"#, None), Plan::Skip);
    }

    #[test]
    fn idle_prompt_notification_keeps_done() {
        let ev = |json: &str| {
            let i = parse(json);
            decide(
                &i.hook_event_name,
                i.notification_type.as_deref(),
                Some(AgentState::Done),
            )
        };
        let idle = r#"{"hook_event_name":"Notification","notification_type":"idle_prompt"}"#;
        assert_eq!(ev(idle), Plan::Skip);
        let perm = r#"{"hook_event_name":"Notification","notification_type":"permission_prompt"}"#;
        assert_eq!(ev(perm), Plan::Set(AgentState::Asking));
        assert_eq!(
            ev(r#"{"hook_event_name":"Notification"}"#),
            Plan::Set(AgentState::Asking)
        );
    }

    #[test]
    fn working_write_is_skipped_when_already_working() {
        for event in ["UserPromptSubmit", "PreToolUse"] {
            assert_eq!(decide(event, None, Some(AgentState::Working)), Plan::Skip);
            assert_eq!(
                decide(event, None, Some(AgentState::Done)),
                Plan::Set(AgentState::Working)
            );
        }
    }

    #[test]
    fn hook_input_tolerates_missing_fields() {
        let input = parse(
            r#"{"hook_event_name":"Notification","message":"Claude needs permission","session_id":"s"}"#,
        );
        assert_eq!(input.message.as_deref(), Some("Claude needs permission"));
        assert!(input.transcript_path.is_none());
        assert_eq!(parse("{}").hook_event_name, "");
    }

    #[test]
    fn transcript_tail_yields_last_assistant_text() {
        let text = last_assistant_text(TRANSCRIPT).expect("text");
        assert!(text.starts_with("All done"), "{text}");
    }

    #[test]
    fn transcript_without_assistant_text_gives_none() {
        let only_tools = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{}}]}}
{"type":"user","message":{"role":"user","content":"hi"}}
not json at all"#;
        assert_eq!(last_assistant_text(only_tools), None);
        assert_eq!(last_assistant_text(""), None);
    }

    #[test]
    fn summarize_collapses_whitespace_and_truncates_on_char_boundary() {
        assert_eq!(summarize("  a \n\n b\tc ", 60), "a b c");
        let long = "漢".repeat(100);
        let out = summarize(&long, 60);
        assert_eq!(out.chars().count(), 60);
        assert!(out.ends_with('…'));
        assert_eq!(summarize("short", 60), "short");
    }

    #[test]
    fn read_tail_drops_partial_first_line() {
        let dir = std::env::temp_dir().join(format!("finter-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        std::fs::write(&path, "first-line-long\nsecond\nthird\n").unwrap();
        let tail = read_tail(path.to_str().unwrap(), 10).unwrap();
        assert_eq!(tail, "third\n");
        let all = read_tail(path.to_str().unwrap(), 1000).unwrap();
        assert!(all.starts_with("first-line-long"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
