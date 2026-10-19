use crate::state::{AgentEntry, AgentState};
use std::{
    error::Error,
    os::unix::process::ExitStatusExt,
    process::{self, Command, Stdio},
};

/// Which tmux server a command talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Socket {
    /// The server the environment points at (`$TMUX`, or tmux's default socket).
    Default,
    /// The dedicated `tmux -L finter` server that hosts remote-attachable sessions.
    Inner,
}

/// Socket name of the inner server. Overridable so smoke tests never touch the real one.
pub(crate) fn inner_socket_name() -> String {
    std::env::var("FINTER_INNER_SOCKET")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "finter".to_string())
}

fn base_command(socket: Socket) -> Command {
    let mut cmd = Command::new("tmux");
    if socket == Socket::Inner {
        cmd.arg("-L").arg(inner_socket_name());
    }
    cmd
}

fn failed_output(reason: &str) -> process::Output {
    process::Output {
        status: process::ExitStatus::from_raw(1 << 8),
        stdout: Vec::new(),
        stderr: reason.as_bytes().to_vec(),
    }
}

/// Run tmux; a missing binary yields a failed output instead of a panic.
pub(crate) fn run_on(socket: Socket, args: &[&str]) -> process::Output {
    base_command(socket)
        .args(args)
        .stdin(Stdio::inherit())
        .output()
        .unwrap_or_else(|e| failed_output(&format!("failed to run tmux: {e}")))
}

pub(crate) fn run_tmux_with_args(args: &[&str]) -> process::Output {
    run_on(Socket::Default, args)
}

/// Trimmed stdout of a successful tmux command.
pub(crate) fn stdout_of(socket: Socket, args: &[&str]) -> Option<String> {
    let out = run_on(socket, args);
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout)
        .ok()
        .map(|s| s.trim_end().to_string())
}

pub(crate) fn get_sessions_on(socket: Socket) -> Vec<String> {
    stdout_of(socket, &["list-sessions", "-F", "#S"])
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

pub(crate) fn get_sessions() -> Result<Vec<String>, Box<dyn Error>> {
    let out = run_tmux_with_args(&["list-sessions", "-F", "#S"]);
    Ok(String::from_utf8(out.stdout)?
        .lines()
        .map(|s| s.to_string())
        .collect())
}

pub(crate) fn get_current_session() -> Option<String> {
    stdout_of(Socket::Default, &["display-message", "-p", "#S"]).filter(|s| !s.is_empty())
}

/// Field separator for `list-panes -F`; tmux passes tabs through untouched.
const SEP: char = '\t';

const PANE_FORMAT: &str = "#{session_name}\t#{window_index}\t#{pane_id}\t#{@agent_state}\t#{@agent_since}\t#{@finter_remote}\t#{@agent_msg}";

/// A pane that carries `@agent_*` options, or `@finter_remote`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneRow {
    pub(crate) session: String,
    pub(crate) window: String,
    pub(crate) pane_id: String,
    pub(crate) state: Option<AgentState>,
    pub(crate) since: u64,
    pub(crate) remote: String,
    pub(crate) msg: String,
}

pub(crate) fn parse_pane_line(line: &str) -> Option<PaneRow> {
    let mut f = line.splitn(7, SEP);
    let session = f.next()?.to_string();
    let window = f.next()?.to_string();
    let pane_id = f.next()?.to_string();
    let state = AgentState::parse(f.next()?);
    let since = f.next()?.parse().unwrap_or(0);
    let remote = f.next()?.to_string();
    let msg = f.next().unwrap_or("").to_string();
    Some(PaneRow {
        session,
        window,
        pane_id,
        state,
        since,
        remote,
        msg,
    })
}

pub(crate) fn list_pane_rows(socket: Socket) -> Vec<PaneRow> {
    stdout_of(socket, &["list-panes", "-a", "-F", PANE_FORMAT])
        .map(|s| s.lines().filter_map(parse_pane_line).collect())
        .unwrap_or_default()
}

/// Agent panes on a server (panes without a valid `@agent_state` are skipped).
pub(crate) fn list_agent_entries(socket: Socket, host: Option<&str>) -> Vec<AgentEntry> {
    list_pane_rows(socket)
        .into_iter()
        .filter_map(|p| {
            Some(AgentEntry {
                host: host.map(str::to_string),
                session: p.session,
                window: p.window,
                pane_id: p.pane_id,
                state: p.state?,
                since: p.since,
                msg: p.msg,
            })
        })
        .collect()
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// Options that make up the agent contract, in unset order.
pub(crate) const AGENT_OPTIONS: [&str; 4] =
    ["@agent", "@agent_state", "@agent_since", "@agent_msg"];

/// tmux argv (with `;` separators) that records an agent state on `pane`.
pub(crate) fn set_state_args(
    pane: &str,
    agent: &str,
    state: AgentState,
    since: u64,
    msg: &str,
) -> Vec<String> {
    let since = since.to_string();
    let values = [agent, state.as_str(), since.as_str(), msg];
    let mut args: Vec<String> = Vec::new();
    for (i, (name, value)) in AGENT_OPTIONS.iter().zip(values).enumerate() {
        if i > 0 {
            args.push(";".to_string());
        }
        args.extend(["set-option", "-p", "-t", pane, name, value].map(String::from));
    }
    args
}

/// tmux argv that removes every `@agent_*` option from `pane`.
pub(crate) fn clear_args(pane: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    for (i, name) in AGENT_OPTIONS.iter().enumerate() {
        if i > 0 {
            args.push(";".to_string());
        }
        args.extend(["set-option", "-pu", "-q", "-t", pane, name].map(String::from));
    }
    args
}

pub(crate) fn run_owned(socket: Socket, args: &[String]) -> process::Output {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_on(socket, &refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pane_line_reads_agent_fields() {
        let row = parse_pane_line("recap\t1\t%4\tasking\t1700000000\t\tneeds you").unwrap();
        assert_eq!(row.session, "recap");
        assert_eq!(row.pane_id, "%4");
        assert_eq!(row.state, Some(AgentState::Asking));
        assert_eq!(row.since, 1_700_000_000);
        assert_eq!(row.msg, "needs you");
    }

    #[test]
    fn parse_pane_line_handles_plain_panes_and_tabs_in_msg() {
        let row = parse_pane_line("a\t2\t%1\t\t\t\t").unwrap();
        assert_eq!(row.state, None);
        assert_eq!(row.since, 0);
        let row = parse_pane_line("a\t2\t%1\tdone\t5\tmini:x\ta\tb").unwrap();
        assert_eq!(row.remote, "mini:x");
        assert_eq!(row.msg, "a\tb");
        assert!(parse_pane_line("garbage").is_none());
    }

    #[test]
    fn set_state_args_chain_four_options() {
        let args = set_state_args("%3", "claude", AgentState::Done, 42, "all good");
        assert_eq!(args.iter().filter(|a| *a == ";").count(), 3);
        assert!(args.windows(2).any(|w| w == ["@agent_state", "done"]));
        assert!(args.windows(2).any(|w| w == ["@agent_since", "42"]));
        assert!(args.windows(2).any(|w| w == ["@agent_msg", "all good"]));
    }

    #[test]
    fn clear_args_unset_all_options() {
        let args = clear_args("%3");
        for name in AGENT_OPTIONS {
            assert!(args.iter().any(|a| a == name), "{name} missing");
        }
        assert!(args.contains(&"-pu".to_string()));
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }
}
