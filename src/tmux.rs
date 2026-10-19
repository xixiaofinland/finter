use std::{error::Error, process};

pub(crate) fn get_sessions() -> Result<Vec<String>, Box<dyn Error>> {
    let sessions: Vec<String> =
        String::from_utf8(run_tmux_with_args(&["list-sessions", "-F", "#S"]).stdout)?
            .lines()
            .map(|s| s.to_string())
            .collect();
    Ok(sessions)
}

pub(crate) fn get_current_session() -> Option<String> {
    let output = run_tmux_with_args(&["display-message", "-p", "#S"]);
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

pub(crate) fn run_tmux_with_args(args: &[&str]) -> process::Output {
    process::Command::new("tmux")
        .args(args)
        .stdin(process::Stdio::inherit())
        .output()
        .unwrap_or_else(|_| panic!("Failed to run tmux with args: `{}`", args.join(" ")))
}
