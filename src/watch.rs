use crate::hook::{summarize, MSG_MAX_CHARS, TMUX_DEADLINE};
use crate::state::AgentState;
use crate::status::now_secs;
use crate::tmux::{self, Socket};
use std::{
    os::unix::process::{CommandExt, ExitStatusExt},
    process::{Command, ExitStatus},
};

/// `@agent` of a watched command; the picker and status bar never look at it.
const AGENT: &str = "watch";

/// Signals the terminal sends to the whole foreground process group.
const TERMINAL_SIGNALS: [libc::c_int; 2] = [libc::SIGINT, libc::SIGQUIT];

/// Exit code a shell would report: the child's own, or 128+signal when it was killed.
pub(crate) fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .or_else(|| status.signal().map(|s| 128 + s))
        .unwrap_or(1)
}

fn mark(pane: Option<&str>, state: AgentState, msg: &str) {
    let Some(pane) = pane else {
        return;
    };
    let args = tmux::set_state_args(pane, AGENT, state, now_secs(), msg);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    if tmux::run_deadline(Socket::Default, &refs, TMUX_DEADLINE).is_none() {
        eprintln!("finter watch: tmux did not answer within {TMUX_DEADLINE:?}");
    }
}

fn set_disposition(handler: libc::sighandler_t) {
    for sig in TERMINAL_SIGNALS {
        // SAFETY: `signal` is async-signal-safe and only swaps the disposition.
        unsafe { libc::signal(sig, handler) };
    }
}

/// Entry point of `finter watch -- <command>`: run it, marking the pane `working`
/// meanwhile and `done` afterwards. Returns the exit code to exit with; tmux trouble
/// never changes it.
pub fn run_watch(command: &[String]) -> i32 {
    let Some((program, args)) = command.split_first() else {
        eprintln!("finter watch: no command given");
        return 2;
    };
    let pane = std::env::var("TMUX_PANE").ok().filter(|p| !p.is_empty());
    let msg = summarize(&command.join(" "), MSG_MAX_CHARS);

    mark(pane.as_deref(), AgentState::Working, &msg);
    // Ctrl-C goes to the child (same process group); finter outlives it to mark `done`.
    // Ignoring before the spawn leaves no window where a Ctrl-C kills finter, and the
    // child gets the default handlers back before exec (SIG_IGN would be inherited).
    set_disposition(libc::SIG_IGN);
    let mut cmd = Command::new(program);
    cmd.args(args);
    // SAFETY: the closure only calls `signal`, which is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            set_disposition(libc::SIG_DFL);
            Ok(())
        });
    }
    let code = match cmd.status() {
        Ok(status) => exit_code(status),
        Err(e) => {
            eprintln!("finter watch: {program}: {e}");
            // What a shell reports for a command it cannot find or run.
            if e.kind() == std::io::ErrorKind::NotFound {
                127
            } else {
                126
            }
        }
    };
    mark(pane.as_deref(), AgentState::Done, &msg);
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_follows_shell_convention() {
        assert_eq!(exit_code(ExitStatus::from_raw(3 << 8)), 3);
        assert_eq!(exit_code(ExitStatus::from_raw(0)), 0);
        // Killed by SIGINT (2).
        assert_eq!(exit_code(ExitStatus::from_raw(2)), 130);
    }
}
