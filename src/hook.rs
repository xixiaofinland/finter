use crate::state::AgentState;
use crate::tmux::{self, Socket};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Upper bound for each tmux call; the agent waits for hooks, so never hang.
pub(crate) const TMUX_DEADLINE: Duration = Duration::from_secs(2);
pub(crate) const MSG_MAX_CHARS: usize = 60;
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
    #[serde(default)]
    pub(crate) last_assistant_message: Option<String>,
    #[serde(default)]
    pub(crate) tool_name: Option<String>,
    #[serde(default)]
    pub(crate) tool_input: Option<serde_json::Value>,
}

/// What a permission dialog asks about, e.g. `Bash: rm -rf build/`: the tool
/// plus its most telling input field.
fn permission_text(input: &HookInput) -> Option<String> {
    let tool = input.tool_name.as_deref()?;
    let detail = input.tool_input.as_ref().and_then(|v| {
        ["command", "file_path", "url", "pattern", "path", "prompt"]
            .iter()
            .find_map(|k| v.get(k).and_then(|x| x.as_str()))
    });
    Some(match detail {
        Some(d) => format!("{tool}: {d}"),
        None => tool.to_string(),
    })
}

/// Claude's final reply for a Stop event. Prefer the field Claude Code sends,
/// since the transcript may not be flushed yet when the hook runs; fall back to
/// the transcript tail for versions that lack it.
fn done_text(input: &HookInput, tail: Option<&str>) -> Option<String> {
    input
        .last_assistant_message
        .clone()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| tail.and_then(last_assistant_text))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plan {
    Skip,
    Set(AgentState),
    Clear,
}

/// Map a hook event to a pane update, given the pane's current state.
/// `background` is whether Claude still waits for background work at Stop;
/// `watched` whether the user is looking at the pane then.
pub(crate) fn decide(
    event: &str,
    notification_type: Option<&str>,
    current: Option<AgentState>,
    background: bool,
    watched: bool,
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
        // PermissionRequest fires as the dialog opens; Notification only after it has waited a while.
        "PermissionRequest" | "Notification" => Plan::Set(AgentState::Asking),
        // Claude ends its turn while a background task (e.g. a CI watch) runs and is woken when
        // it finishes, so the turn is not the end of the work: stay `working`.
        "Stop" if background => {
            if current == Some(AgentState::Working) {
                Plan::Skip
            } else {
                Plan::Set(AgentState::Working)
            }
        }
        // The user saw the turn end, so there is nothing to come back to: no `done` badge
        // (`finter seen` would never clear one, since no focus change follows).
        "Stop" if watched => Plan::Clear,
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

/// Whether a transcript tail has background work that has not reported back: a
/// `run_in_background` command, an async agent or a Monitor. Each ends (completed, failed,
/// killed) with a `<task-notification>` naming its id. A launch older than the tail is missed.
pub(crate) fn has_pending_background(jsonl: &str) -> bool {
    let mut pending = HashSet::new();
    for line in jsonl.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(id) = value.get("toolUseResult").and_then(background_task_id) {
            pending.insert(id.to_string());
        }
        for text in notification_texts(&value) {
            for id in finished_task_ids(text) {
                pending.remove(id);
            }
        }
    }
    !pending.is_empty()
}

/// The task id in a tool result that started background work.
fn background_task_id(result: &Value) -> Option<&str> {
    let is_async = result.get("isAsync").and_then(|a| a.as_bool()) == Some(true);
    result
        .get("backgroundTaskId")
        .or_else(|| result.get("taskId"))
        .or_else(|| is_async.then(|| result.get("agentId")).flatten())
        .and_then(|id| id.as_str())
}

/// Where a transcript line can deliver a `<task-notification>`: a user message when Claude was
/// idle, or the queue (and its `queued_command` attachment) when it arrived mid-turn. Tool
/// output and assistant text are left out, since they may only quote one.
fn notification_texts(value: &Value) -> Vec<&str> {
    match value.get("type").and_then(|t| t.as_str()) {
        Some("user") => match value.pointer("/message/content") {
            Some(Value::String(s)) => vec![s.as_str()],
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect(),
            _ => Vec::new(),
        },
        Some("queue-operation") => value
            .get("content")
            .and_then(|c| c.as_str())
            .into_iter()
            .collect(),
        Some("attachment") => value
            .pointer("/attachment/prompt")
            .and_then(|c| c.as_str())
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// Ids of the tasks a `<task-notification>` text reports as ended.
fn finished_task_ids(text: &str) -> impl Iterator<Item = &str> {
    text.split("<task-notification>").skip(1).filter_map(|n| {
        let n = n.split("</task-notification>").next()?;
        if !n.contains("<status>") {
            return None;
        }
        let (_, rest) = n.split_once("<task-id>")?;
        rest.split_once("</task-id>").map(|(id, _)| id.trim())
    })
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

/// Whether a `display-message` of `#{window_active}#{pane_active}` and the `#{client_flags}`
/// lines of the clients on the pane's session mean someone is looking at the pane.
pub(crate) fn is_watched(active: &str, client_flags: &str) -> bool {
    active.trim() == "11"
        && client_flags
            .lines()
            .any(|l| l.split(',').any(|f| f == "focused"))
}

/// Whether the pane is on screen in a focused terminal: the active pane of its session's
/// current window, with a client of that session holding focus. A wrapper's client on the
/// inner server gets focus from the outer tmux (`focus-events`), so leaving the wrapper
/// session, or the terminal, counts as looking away. Unknown means not watched.
fn pane_watched(pane: &str) -> bool {
    let Some(out) = tmux::run_deadline(
        Socket::Default,
        &[
            "display-message",
            "-p",
            "-t",
            pane,
            "#{window_active}#{pane_active}\t#{session_id}",
        ],
        TMUX_DEADLINE,
    ) else {
        return false;
    };
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let Some((active, session)) = line.split_once('\t') else {
        return false;
    };
    let Some(clients) = tmux::run_deadline(
        Socket::Default,
        &["list-clients", "-t", session, "-F", "#{client_flags}"],
        TMUX_DEADLINE,
    ) else {
        return false;
    };
    is_watched(active, &String::from_utf8_lossy(&clients.stdout))
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

    let tail = match input.hook_event_name.as_str() {
        "Stop" => input
            .transcript_path
            .as_deref()
            .and_then(|p| read_tail(p, TRANSCRIPT_TAIL_BYTES).ok()),
        _ => None,
    };
    let background = tail.as_deref().is_some_and(has_pending_background);
    let watched = input.hook_event_name == "Stop" && !background && pane_watched(&pane);
    let current = match input.hook_event_name.as_str() {
        "UserPromptSubmit" | "PreToolUse" => current_state(&pane),
        "Stop" if background => current_state(&pane),
        _ => None,
    };
    match decide(
        &input.hook_event_name,
        input.notification_type.as_deref(),
        current,
        background,
        watched,
    ) {
        Plan::Skip => {}
        Plan::Clear => {
            run_tmux_args(&tmux::clear_args(&pane));
        }
        Plan::Set(state) => {
            let msg = match state {
                AgentState::Asking => permission_text(&input)
                    .or_else(|| input.message.clone())
                    .map(|m| summarize(&m, MSG_MAX_CHARS)),
                AgentState::Done => {
                    done_text(&input, tail.as_deref()).map(|t| summarize(&t, MSG_MAX_CHARS))
                }
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
            decide(
                &i.hook_event_name,
                i.notification_type.as_deref(),
                cur,
                false,
                false,
            )
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
                false,
                false,
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
            assert_eq!(
                decide(event, None, Some(AgentState::Working), false, false),
                Plan::Skip
            );
            assert_eq!(
                decide(event, None, Some(AgentState::Done), false, false),
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
    fn permission_request_asks_with_tool_and_command() {
        assert_eq!(
            decide(
                "PermissionRequest",
                None,
                Some(AgentState::Working),
                false,
                false
            ),
            Plan::Set(AgentState::Asking)
        );
        let input = parse(
            r#"{"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"rm -rf build/","description":"clean"}}"#,
        );
        assert_eq!(
            permission_text(&input).as_deref(),
            Some("Bash: rm -rf build/")
        );
        let input = parse(
            r#"{"hook_event_name":"PermissionRequest","tool_name":"Edit","tool_input":{"file_path":"/a/b.rs"}}"#,
        );
        assert_eq!(permission_text(&input).as_deref(), Some("Edit: /a/b.rs"));
        let input = parse(r#"{"hook_event_name":"Notification","message":"needs permission"}"#);
        assert_eq!(permission_text(&input), None);
    }

    #[test]
    fn done_text_prefers_last_assistant_message() {
        let input = parse(
            r#"{"hook_event_name":"Stop","last_assistant_message":"hello there","transcript_path":"/nonexistent"}"#,
        );
        assert_eq!(done_text(&input, None).as_deref(), Some("hello there"));

        let input = parse(r#"{"hook_event_name":"Stop","last_assistant_message":"  "}"#);
        assert_eq!(done_text(&input, None), None);
        assert_eq!(
            done_text(&input, Some(TRANSCRIPT))
                .as_deref()
                .map(|t| &t[..8]),
            Some("All done")
        );
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
    fn stop_with_background_work_stays_working() {
        assert_eq!(
            decide("Stop", None, Some(AgentState::Working), true, false),
            Plan::Skip
        );
        assert_eq!(
            decide("Stop", None, Some(AgentState::Asking), true, false),
            Plan::Set(AgentState::Working)
        );
        assert_eq!(
            decide("Stop", None, None, true, false),
            Plan::Set(AgentState::Working)
        );
    }

    #[test]
    fn stop_while_watched_clears_instead_of_done() {
        assert_eq!(
            decide("Stop", None, Some(AgentState::Working), false, true),
            Plan::Clear
        );
        // Background work still wins: the turn is not the end of the work.
        assert_eq!(
            decide("Stop", None, Some(AgentState::Working), true, true),
            Plan::Skip
        );
        // Only Stop looks at it; asking must stay visible.
        assert_eq!(
            decide("PermissionRequest", None, None, false, true),
            Plan::Set(AgentState::Asking)
        );
    }

    #[test]
    fn watched_needs_active_pane_and_focused_client() {
        assert!(is_watched("11", "attached,focused,UTF-8\n"));
        assert!(is_watched(
            "11\n",
            "attached,UTF-8\nattached,focused,UTF-8\n"
        ));
        // Another window or pane of the session is on screen.
        assert!(!is_watched("01", "attached,focused,UTF-8"));
        assert!(!is_watched("10", "attached,focused,UTF-8"));
        // The session is attached but its clients look elsewhere (wrapper left, terminal blurred).
        assert!(!is_watched("11", "attached,UTF-8"));
        assert!(!is_watched("11", ""));
    }

    // Lines shaped like Claude Code's: the launch result carries the id in `toolUseResult`, the
    // wake-up is a user message holding a `<task-notification>`.
    const BASH_BG: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"Command running in background with ID: bj8pg9fzu."}]},"toolUseResult":{"stdout":"","backgroundTaskId":"bj8pg9fzu"}}"#;
    const BASH_DONE: &str = r#"{"type":"user","message":{"role":"user","content":"<task-notification>\n<task-id>bj8pg9fzu</task-id>\n<status>completed</status>\n</task-notification>"}}"#;
    const AGENT_BG: &str = r#"{"type":"user","message":{"role":"user","content":[]},"toolUseResult":{"isAsync":true,"status":"async_launched","agentId":"aadc"}}"#;
    const MONITOR_BG: &str = r#"{"type":"user","message":{"role":"user","content":[]},"toolUseResult":{"taskId":"b0t2","timeoutMs":900000,"persistent":false}}"#;

    #[test]
    fn background_task_pending_until_notified() {
        assert!(!has_pending_background(TRANSCRIPT));
        assert!(has_pending_background(BASH_BG));
        assert!(!has_pending_background(&format!("{BASH_BG}\n{BASH_DONE}")));
        assert!(has_pending_background(AGENT_BG));
        assert!(has_pending_background(MONITOR_BG));

        let killed = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<task-notification><task-id>aadc</task-id><status>killed</status></task-notification> <task-notification><task-id>b0t2</task-id><status>completed</status></task-notification>"}]}}"#;
        assert!(!has_pending_background(&format!(
            "{AGENT_BG}\n{MONITOR_BG}\n{killed}"
        )));
        // Only one of two finished.
        assert!(has_pending_background(&format!(
            "{BASH_BG}\n{AGENT_BG}\n{BASH_DONE}"
        )));
    }

    #[test]
    fn notification_queued_mid_turn_finishes_the_task() {
        let queued = r#"{"type":"queue-operation","operation":"enqueue","content":"<task-notification>\n<task-id>bj8pg9fzu</task-id>\n<status>completed</status>\n</task-notification>"}"#;
        assert!(!has_pending_background(&format!("{BASH_BG}\n{queued}")));
        let attached = r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":"<task-notification>\n<task-id>bj8pg9fzu</task-id>\n<status>completed</status>\n</task-notification>"}}"#;
        assert!(!has_pending_background(&format!("{BASH_BG}\n{attached}")));
    }

    #[test]
    fn notification_text_outside_a_user_message_does_not_finish_a_task() {
        // An assistant quoting a notification, or a tool printing one, is not a wake-up.
        let quoted = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"<task-notification><task-id>bj8pg9fzu</task-id><status>completed</status></task-notification>"}]}}"#;
        assert!(has_pending_background(&format!("{BASH_BG}\n{quoted}")));
        // A synchronous agent result has an agentId but is not async.
        let sync_agent = r#"{"type":"user","toolUseResult":{"agentId":"x1","status":"completed"}}"#;
        assert!(!has_pending_background(sync_agent));
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
