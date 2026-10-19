use crate::config::{Config, HostConfig};
use crate::remote::{self, HostState};
use crate::state::{
    best_per_project, pick_next, project_of, wrapper_session, AgentEntry, AgentState,
};
use crate::tmux::{self, Socket};
use std::{
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};

/// Named badges shown before the rest collapse into `+N`.
const MAX_NAMED: usize = 3;

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `#` starts a tmux format sequence, so literal text must double it.
fn escape(text: &str) -> String {
    text.replace('#', "##")
}

fn badge(entry: &AgentEntry) -> String {
    let name = escape(entry.project());
    let remote = entry.host.is_some();
    match entry.state {
        AgentState::Asking => {
            let bg = if remote { "colour125" } else { "colour160" };
            format!("#[bg={bg},fg=colour255,bold]❓{name}#[default]")
        }
        AgentState::Done => {
            let fg = if remote { "colour141" } else { "colour71" };
            format!("#[fg={fg}]✅{name}#[default]")
        }
        AgentState::Working => format!("⚙{name}"),
    }
}

/// tmux status-format string, empty when there is nothing to show.
/// `current` is the session the client looks at; it is never badged.
pub(crate) fn render_status(
    entries: &[AgentEntry],
    offline_hosts: &[String],
    current: Option<&str>,
) -> String {
    let visible: Vec<AgentEntry> = entries
        .iter()
        .filter(|e| Some(e.local_session().as_str()) != current)
        .cloned()
        .collect();
    let best = best_per_project(&visible);
    let waiting: Vec<&AgentEntry> = best.iter().filter(|e| e.state.is_waiting()).collect();
    let working = best.len() - waiting.len();

    let mut parts: Vec<String> = waiting.iter().take(MAX_NAMED).map(|e| badge(e)).collect();
    if waiting.len() > MAX_NAMED {
        parts.push(format!("+{}", waiting.len() - MAX_NAMED));
    }
    if working > 0 {
        parts.push(format!("⚙{working}"));
    }
    for host in offline_hosts {
        parts.push(format!("#[fg=colour243]{} ✕#[default]", escape(host)));
    }
    parts.join(" ")
}

/// Local agent panes plus every host's (cache-backed) panes.
fn gather(config: Option<&Config>) -> (Vec<AgentEntry>, Vec<HostState>) {
    let mut entries = tmux::list_agent_entries(Socket::Default, None);
    let hosts = config
        .map(|c| remote::load_hosts(c, now_secs()))
        .unwrap_or_default();
    for h in &hosts {
        entries.extend(h.entries());
    }
    (entries, hosts)
}

fn load_config_quiet() -> Option<Config> {
    crate::config::load_config().ok()
}

/// `finter status`: print the status-bar segment.
/// `--current` wins; otherwise ask tmux, but only when running inside it.
pub(crate) fn resolve_current(
    arg: Option<&str>,
    fallback: impl FnOnce() -> Option<String>,
) -> Option<String> {
    match arg.filter(|a| !a.is_empty()) {
        Some(name) => Some(name.to_string()),
        None => fallback(),
    }
}

pub fn run_status(current_arg: Option<&str>) {
    let config = load_config_quiet();
    let (entries, hosts) = gather(config.as_ref());
    let offline: Vec<String> = hosts
        .iter()
        .filter(|h| h.offline)
        .map(|h| h.host.name.clone())
        .collect();
    let current = resolve_current(current_arg, || {
        std::env::var_os("TMUX").and_then(|_| tmux::get_current_session())
    });
    print!("{}", render_status(&entries, &offline, current.as_deref()));
    let _ = std::io::stdout().flush();
}

// ── next

/// Window to select inside the local wrapper for a remote entry.
fn wrapper_window(wrapper: &str, host: &HostConfig, entry: &AgentEntry) -> String {
    let tag = remote::remote_tag(&host.ssh, &entry.session);
    let found = tmux::list_pane_rows(Socket::Default)
        .into_iter()
        .find(|p| p.session == wrapper && p.remote == tag)
        .map(|p| p.window);
    // base-index is 1 on every host, so claude is window 1 and the terminal window 2.
    found.unwrap_or_else(|| if entry.is_term() { "2" } else { "1" }.to_string())
}

/// Make sure the wrapper for a remote project exists, then switch to it.
pub(crate) fn go_to_remote(host: &HostConfig, project: &str) -> bool {
    let wrapper = wrapper_session(&host.name, project);
    if !tmux::has_session(&wrapper) && !remote::create_wrapper(host, project) {
        return false;
    }
    tmux::run_on(Socket::Default, &["switch-client", "-t", &wrapper])
        .status
        .success()
}

pub fn run_next() {
    let config = load_config_quiet();
    let (entries, _) = gather(config.as_ref());
    let Some(entry) = pick_next(&entries) else {
        tmux::run_on(Socket::Default, &["display-message", "nothing waiting"]);
        return;
    };
    match &entry.host {
        None => {
            tmux::run_on(Socket::Default, &["switch-client", "-t", &entry.session]);
            let window = format!("{}:{}", entry.session, entry.window);
            tmux::run_on(Socket::Default, &["select-window", "-t", &window]);
            tmux::run_on(Socket::Default, &["select-pane", "-t", &entry.pane_id]);
        }
        Some(name) => {
            let Some(host) = config
                .as_ref()
                .and_then(|c| c.hosts.iter().find(|h| &h.name == name))
            else {
                return;
            };
            if go_to_remote(host, entry.project()) {
                let wrapper = entry.local_session();
                let window = wrapper_window(&wrapper, host, entry);
                tmux::run_on(
                    Socket::Default,
                    &["select-window", "-t", &format!("{wrapper}:{window}")],
                );
            }
        }
    }
}

// ── seen

/// Sessions whose `done` badge should clear: where the client is and where it came from.
pub(crate) fn sessions_to_clear(current: &str, last: &str) -> Vec<String> {
    let mut out = Vec::new();
    for s in [current, last] {
        if !s.is_empty() && !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    }
    out
}

/// Split a wrapper session name into its configured host and project.
pub(crate) fn parse_wrapper<'a>(
    session: &'a str,
    hosts: &'a [HostConfig],
) -> Option<(&'a HostConfig, &'a str)> {
    let (name, project) = session.split_once('/')?;
    let host = hosts.iter().find(|h| h.name == name)?;
    Some((host, project))
}

/// Remote tmux argv that unsets `@agent_*` on each pane, but only while it is still `done`.
pub(crate) fn remote_clear_args(panes: &[&str]) -> Vec<String> {
    let unset = tmux::AGENT_OPTIONS
        .iter()
        .map(|o| format!("set-option -pu -q {o}"))
        .collect::<Vec<_>>()
        .join(" ; ");
    let mut args: Vec<String> = Vec::new();
    for (i, pane) in panes.iter().enumerate() {
        if i > 0 {
            args.push(";".to_string());
        }
        args.extend(
            [
                "if-shell",
                "-F",
                "-t",
                pane,
                "#{==:#{@agent_state},done}",
                &unset,
            ]
            .map(String::from),
        );
    }
    args
}

fn clear_remote_done(host: &HostConfig, project: &str) {
    let Some(mut cache) = remote::read_cache(&host.name) else {
        return;
    };
    let Some(data) = cache.data.as_mut() else {
        return;
    };
    let done: Vec<String> = data
        .panes
        .iter()
        .filter(|p| p.state == AgentState::Done && project_of(&p.session) == project)
        .map(|p| p.pane.clone())
        .collect();
    if done.is_empty() {
        return;
    }
    data.panes
        .retain(|p| !(p.state == AgentState::Done && project_of(&p.session) == project));
    // Reflect the clear locally right away; the remote write is fire-and-forget.
    remote::write_cache(&host.name, &cache);
    let refs: Vec<&str> = done.iter().map(String::as_str).collect();
    let args = remote_clear_args(&refs);
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    remote::ssh_spawn(&host.ssh, &remote::remote_tmux(&argv));
}

fn clear_local_done(session: &str) {
    for e in tmux::list_agent_entries(Socket::Default, None) {
        if e.session == session && e.state == AgentState::Done {
            tmux::run_owned(Socket::Default, &tmux::clear_args(&e.pane_id));
        }
    }
}

pub fn run_seen() {
    let Some(info) = tmux::stdout_of(
        Socket::Default,
        &[
            "display-message",
            "-p",
            "#{session_name}\t#{client_last_session}",
        ],
    ) else {
        return;
    };
    let (current, last) = info.split_once('\t').unwrap_or((info.as_str(), ""));
    let hosts = load_config_quiet().map(|c| c.hosts).unwrap_or_default();
    for session in sessions_to_clear(current, last) {
        match parse_wrapper(&session, &hosts) {
            Some((host, project)) => clear_remote_done(host, project),
            None => clear_local_done(&session),
        }
    }
}

// ── preview

/// Preview target understood by `finter preview`.
pub(crate) fn local_target(session: &str) -> String {
    format!("local:{session}")
}

pub(crate) fn remote_target(alias: &str, session: &str) -> String {
    format!("remote:{alias}:{session}")
}

pub(crate) fn parse_target(target: &str) -> Option<(Option<&str>, &str)> {
    if let Some(session) = target.strip_prefix("local:") {
        return Some((None, session));
    }
    let rest = target.strip_prefix("remote:")?;
    let (alias, session) = rest.rsplit_once(':')?;
    Some((Some(alias), session))
}

fn is_blank_line(line: &str) -> bool {
    let mut plain = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip a CSI sequence: ESC [ ... final byte in @..~
            for n in chars.by_ref() {
                if ('@'..='~').contains(&n) && n != '[' {
                    break;
                }
            }
        } else {
            plain.push(c);
        }
    }
    plain.trim().is_empty()
}

/// Drop trailing blank lines and keep the last `n` lines, so the newest output stays visible.
pub(crate) fn tail_lines(text: &str, n: usize) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    while lines.last().is_some_and(|l| is_blank_line(l)) {
        lines.pop();
    }
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

pub fn run_preview(target: &str) {
    let Some((alias, session)) = parse_target(target) else {
        println!("(bad preview target)");
        return;
    };
    let pane = format!("={session}:");
    let capture = ["capture-pane", "-p", "-e", "-S", "-30", "-t", &pane];
    let text = match alias {
        None => tmux::stdout_of(Socket::Default, &capture),
        Some(alias) => remote::ssh_run(alias, &remote::remote_tmux(&capture)).ok(),
    };
    let rows = std::env::var("LINES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    match text {
        Some(t) => println!("{}", tail_lines(&t, rows)),
        None => println!("(no live session)"),
    }
}

// ── copy-mode

pub fn run_copy_mode() {
    let mut args = vec!["display-message", "-p"];
    let pane = std::env::var("TMUX_PANE").ok();
    if let Some(p) = pane.as_deref() {
        args.extend(["-t", p]);
    }
    args.push("#{@finter_remote}");
    let tag = tmux::stdout_of(Socket::Default, &args).unwrap_or_default();
    match remote::split_remote_tag(&tag) {
        Some((alias, session)) => {
            let target = format!("={session}:");
            let cmd = remote::remote_tmux(&["copy-mode", "-t", &target]);
            if let Err(e) = remote::ssh_run(alias, &cmd) {
                eprintln!("finter copy-mode: {e}");
            }
        }
        None => {
            tmux::run_on(Socket::Default, &["copy-mode"]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_entry;

    fn e(host: Option<&str>, session: &str, state: AgentState, since: u64) -> AgentEntry {
        test_entry(host, session, state, since)
    }

    #[test]
    fn empty_input_renders_nothing() {
        assert_eq!(render_status(&[], &[], None), "");
    }

    #[test]
    fn renders_asking_done_and_working_count() {
        let v = vec![
            e(None, "finter", AgentState::Done, 5),
            e(None, "recap", AgentState::Asking, 9),
            e(None, "w1", AgentState::Working, 1),
            e(None, "w2", AgentState::Working, 1),
            e(None, "w3", AgentState::Working, 1),
        ];
        let out = render_status(&v, &[], None);
        assert_eq!(
            out,
            "#[bg=colour160,fg=colour255,bold]❓recap#[default] #[fg=colour71]✅finter#[default] ⚙3"
        );
    }

    #[test]
    fn remote_entries_use_distinct_colours() {
        let v = vec![
            e(Some("mini"), "a", AgentState::Asking, 1),
            e(Some("mini"), "b~term", AgentState::Done, 2),
        ];
        let out = render_status(&v, &[], None);
        assert!(out.contains("bg=colour125"));
        assert!(out.contains("fg=colour141]✅b"));
    }

    #[test]
    fn truncates_after_three_named_badges() {
        let v: Vec<AgentEntry> = (0..6)
            .map(|i| e(None, &format!("p{i}"), AgentState::Done, i))
            .collect();
        let out = render_status(&v, &[], None);
        assert_eq!(out.matches('✅').count(), 3);
        assert!(out.ends_with("+3"));
        assert!(out.contains("p0") && !out.contains("p3"));
    }

    #[test]
    fn current_session_is_excluded_for_local_and_remote() {
        let v = vec![
            e(None, "mine", AgentState::Asking, 1),
            e(Some("mini"), "far", AgentState::Asking, 2),
            e(Some("mini"), "far~term", AgentState::Done, 3),
            e(None, "other", AgentState::Done, 4),
        ];
        let out = render_status(&v, &[], Some("mine"));
        assert!(!out.contains("mine"));
        assert!(out.contains("far"));
        let out = render_status(&v, &[], Some("mini/far"));
        assert!(!out.contains("far"));
        assert!(out.contains("mine") && out.contains("other"));
    }

    #[test]
    fn offline_hosts_are_listed_and_names_are_escaped() {
        let out = render_status(&[], &["mini".to_string()], None);
        assert_eq!(out, "#[fg=colour243]mini ✕#[default]");
        let v = vec![e(None, "c#sharp", AgentState::Done, 1)];
        assert!(render_status(&v, &[], None).contains("c##sharp"));
    }

    #[test]
    fn x_and_term_collapse_to_one_badge() {
        let v = vec![
            e(None, "x", AgentState::Done, 1),
            e(None, "x~term", AgentState::Done, 2),
        ];
        assert_eq!(render_status(&v, &[], None).matches('✅').count(), 1);
    }

    #[test]
    fn current_arg_overrides_tmux_lookup() {
        let never = || -> Option<String> { panic!("must not ask tmux") };
        assert_eq!(resolve_current(Some("a"), never).as_deref(), Some("a"));
        let from_tmux = || Some("t".to_string());
        assert_eq!(resolve_current(None, from_tmux).as_deref(), Some("t"));
        assert_eq!(resolve_current(Some(""), from_tmux).as_deref(), Some("t"));
        assert_eq!(resolve_current(None, || None), None);
    }

    #[test]
    fn seen_targets_current_and_previous_once() {
        assert_eq!(sessions_to_clear("a", "b"), ["a", "b"]);
        assert_eq!(sessions_to_clear("a", "a"), ["a"]);
        assert_eq!(sessions_to_clear("a", ""), ["a"]);
        assert!(sessions_to_clear("", "").is_empty());
    }

    #[test]
    fn wrapper_names_resolve_to_hosts() {
        let hosts = vec![HostConfig {
            name: "mini".into(),
            ssh: "mac-mini".into(),
        }];
        let (h, p) = parse_wrapper("mini/recap", &hosts).unwrap();
        assert_eq!((h.ssh.as_str(), p), ("mac-mini", "recap"));
        assert!(parse_wrapper("recap", &hosts).is_none());
        assert!(parse_wrapper("other/recap", &hosts).is_none());
    }

    #[test]
    fn remote_clear_checks_done_per_pane() {
        let args = remote_clear_args(&["%1", "%2"]);
        assert_eq!(args.iter().filter(|a| *a == ";").count(), 1);
        assert_eq!(args.iter().filter(|a| *a == "if-shell").count(), 2);
        assert!(args
            .iter()
            .any(|a| a.contains("set-option -pu -q @agent_msg")));
    }

    #[test]
    fn preview_targets_round_trip() {
        assert_eq!(parse_target(&local_target("a/b")), Some((None, "a/b")));
        assert_eq!(
            parse_target(&remote_target("mac-mini", "x~term")),
            Some((Some("mac-mini"), "x~term"))
        );
        assert_eq!(parse_target("junk"), None);
    }

    #[test]
    fn tail_lines_trims_blank_tail_and_keeps_newest() {
        let text = "a\nb\nc\n\n\x1b[0m\n  \n";
        assert_eq!(tail_lines(text, 2), "b\nc");
        assert_eq!(tail_lines(text, 10), "a\nb\nc");
        assert_eq!(tail_lines("", 5), "");
    }
}
