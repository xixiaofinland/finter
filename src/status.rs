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
    offline_hosts: &[(String, Option<String>)],
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
    for (host, reason) in offline_hosts {
        let why = reason
            .as_deref()
            .map(|r| format!(" ({})", escape(r)))
            .unwrap_or_default();
        parts.push(format!("#[fg=colour243]{} ✕{why}#[default]", escape(host)));
    }
    parts.join(" ")
}

/// Local agent panes plus every host's (cache-backed) panes.
fn gather(config: Option<&Config>, refresh: bool) -> (Vec<AgentEntry>, Vec<HostState>) {
    let mut entries = tmux::list_agent_entries(Socket::Default, None);
    let hosts = config
        .map(|c| remote::load_hosts(c, now_secs(), refresh))
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
    let (entries, hosts) = gather(config.as_ref(), true);
    let offline: Vec<(String, Option<String>)> = hosts
        .iter()
        .filter(|h| h.offline)
        .map(|h| (h.host.name.clone(), h.error.clone()))
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
    // `next` is a keypress: never wait on ssh, use whatever the status bar cached.
    let (entries, _) = gather(config.as_ref(), false);
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

/// What a focus change does to one session's agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SeenAction {
    /// The session being viewed: its `done` is acknowledged.
    View,
    /// The session just left: `done` is acknowledged as well, and an `asking`
    /// the user saw but did not answer (e.g. interrupted the prompt, which fires
    /// no hook) is demoted to `done`, so a reminder stays behind.
    Leave,
}

/// Sessions to update: the one being viewed and, only when `leave` is set
/// (session switch), the one the client came from.
pub(crate) fn seen_targets(current: &str, last: &str, leave: bool) -> Vec<(String, SeenAction)> {
    let mut out: Vec<(String, SeenAction)> = Vec::new();
    if !current.is_empty() {
        out.push((current.to_string(), SeenAction::View));
    }
    if leave && !last.is_empty() && last != current {
        out.push((last.to_string(), SeenAction::Leave));
    }
    out
}

/// States a seen action touches.
fn affected(state: AgentState, action: SeenAction) -> bool {
    state == AgentState::Done || (action == SeenAction::Leave && state == AgentState::Asking)
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

/// tmux argv (local or remote) applying `action` to each pane. Every change is
/// conditional on the pane's state at execution time, so a transition that
/// happens in between is never overwritten. `done` is cleared before `asking`
/// is demoted, so a demoted pane keeps its `done`.
pub(crate) fn seen_args(panes: &[&str], action: SeenAction) -> Vec<String> {
    let unset = tmux::AGENT_OPTIONS
        .iter()
        .map(|o| format!("set-option -pu -q {o}"))
        .collect::<Vec<_>>()
        .join(" ; ");
    let mut steps: Vec<(&str, String)> = vec![("done", unset)];
    if action == SeenAction::Leave {
        steps.push(("asking", "set-option -p -q @agent_state done".to_string()));
    }
    let mut args: Vec<String> = Vec::new();
    for pane in panes {
        for (state, cmd) in &steps {
            if !args.is_empty() {
                args.push(";".to_string());
            }
            args.extend([
                "if-shell".to_string(),
                "-F".to_string(),
                "-t".to_string(),
                pane.to_string(),
                format!("#{{==:#{{@agent_state}},{state}}}"),
                cmd.clone(),
            ]);
        }
    }
    args
}

fn seen_remote(host: &HostConfig, project: &str, action: SeenAction) {
    let Some(mut cache) = remote::read_cache(&host.name) else {
        return;
    };
    let Some(data) = cache.data.as_mut() else {
        return;
    };
    let mine =
        |p: &remote::ListPane| project_of(&p.session) == project && affected(p.state, action);
    let panes: Vec<String> = data
        .panes
        .iter()
        .filter(|p| mine(p))
        .map(|p| p.pane.clone())
        .collect();
    if panes.is_empty() {
        return;
    }
    // Reflect the change locally right away; the remote write is fire-and-forget.
    data.panes
        .retain(|p| !(mine(p) && p.state == AgentState::Done));
    for p in data
        .panes
        .iter_mut()
        .filter(|p| project_of(&p.session) == project)
    {
        if p.state == AgentState::Asking && action == SeenAction::Leave {
            p.state = AgentState::Done;
        }
    }
    remote::write_cache(&host.name, &cache);
    let refs: Vec<&str> = panes.iter().map(String::as_str).collect();
    let args = seen_args(&refs, action);
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    remote::ssh_spawn(&host.ssh, &remote::remote_tmux(&argv));
}

fn seen_local(session: &str, action: SeenAction) {
    let panes: Vec<String> = tmux::list_agent_entries(Socket::Default, None)
        .into_iter()
        .filter(|e| e.session == session && affected(e.state, action))
        .map(|e| e.pane_id)
        .collect();
    if panes.is_empty() {
        return;
    }
    let refs: Vec<&str> = panes.iter().map(String::as_str).collect();
    tmux::run_owned(Socket::Default, &seen_args(&refs, action));
}

pub fn run_seen(leave: bool) {
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
    for (session, action) in seen_targets(current, last, leave) {
        match parse_wrapper(&session, &hosts) {
            Some((host, project)) => seen_remote(host, project, action),
            None => seen_local(&session, action),
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
        let out = render_status(&[], &[("mini".to_string(), None)], None);
        assert_eq!(out, "#[fg=colour243]mini ✕#[default]");
        let why = Some("finter too old?".to_string());
        let out = render_status(&[], &[("mini".to_string(), why)], None);
        assert_eq!(out, "#[fg=colour243]mini ✕ (finter too old?)#[default]");
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
    fn seen_targets_view_current_and_leave_previous() {
        use SeenAction::*;
        let t = |c, l, leave| seen_targets(c, l, leave);
        assert_eq!(t("a", "b", true), [("a".into(), View), ("b".into(), Leave)]);
        assert_eq!(t("a", "b", false), [("a".into(), View)]);
        assert_eq!(t("a", "a", true), [("a".into(), View)]);
        assert_eq!(t("a", "", true), [("a".into(), View)]);
        assert!(t("", "", true).is_empty());
    }

    #[test]
    fn only_leave_touches_asking() {
        assert!(affected(AgentState::Done, SeenAction::View));
        assert!(!affected(AgentState::Asking, SeenAction::View));
        assert!(affected(AgentState::Asking, SeenAction::Leave));
        assert!(!affected(AgentState::Working, SeenAction::Leave));
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
    fn seen_args_clear_done_then_demote_asking() {
        let view = seen_args(&["%1", "%2"], SeenAction::View);
        assert_eq!(view.iter().filter(|a| *a == "if-shell").count(), 2);
        assert!(!view.iter().any(|a| a.contains("asking")));
        assert!(view
            .iter()
            .any(|a| a.contains("set-option -pu -q @agent_msg")));

        let leave = seen_args(&["%1"], SeenAction::Leave);
        let done_at = leave
            .iter()
            .position(|a| a == "#{==:#{@agent_state},done}")
            .unwrap();
        let asking_at = leave
            .iter()
            .position(|a| a == "#{==:#{@agent_state},asking}")
            .unwrap();
        assert!(
            done_at < asking_at,
            "done must clear before asking is demoted"
        );
        assert!(leave
            .iter()
            .any(|a| a == "set-option -p -q @agent_state done"));
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
