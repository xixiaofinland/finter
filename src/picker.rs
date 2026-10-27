use crate::config::{get_home_dir, HostConfig};
use crate::remote::HostState;
use crate::state::{
    age_secs, best_per_project, format_age, wrapper_session, AgentEntry, AgentState,
};
use crate::status::{local_target, parse_wrapper, remote_target};
use ansi_to_tui::IntoText;
use ratatui_core::text::Line;
use skim::prelude::*;
use std::error::Error;

/// What selecting a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Local { session: String, path: String },
    Remote { host: HostConfig, project: String },
}

#[derive(Debug, Clone)]
pub(crate) struct Row {
    /// Name shown and matched: `recap` or `mini/recap`.
    pub(crate) name: String,
    pub(crate) target: Target,
    /// A tmux session for it exists (local session, wrapper or inner session).
    pub(crate) exists: bool,
    /// Most urgent agent in the project.
    pub(crate) agent: Option<AgentEntry>,
    now: u64,
    name_width: usize,
}

impl Row {
    /// Unique key; doubles as the `finter preview` target.
    pub(crate) fn key(&self) -> String {
        match &self.target {
            Target::Local { session, .. } => local_target(session),
            Target::Remote { host, project } => remote_target(&host.ssh, project),
        }
    }

    /// 0 asking, 1 done, 2 working, 3 existing session, 4 folder.
    fn group(&self) -> u8 {
        match self.agent.as_ref().map(|a| a.state) {
            Some(AgentState::Asking) => 0,
            Some(AgentState::Done) => 1,
            Some(AgentState::Working) => 2,
            None if self.exists => 3,
            None => 4,
        }
    }

    fn line(&self) -> String {
        let (icon, color) = match self.agent.as_ref().map(|a| a.state) {
            Some(AgentState::Asking) => ("❓", "\x1b[31m"),
            Some(AgentState::Done) => ("✅", "\x1b[32m"),
            Some(AgentState::Working) => ("⚙ ", "\x1b[33m"),
            None if self.exists => ("* ", ""),
            None => ("  ", ""),
        };
        let pad = " ".repeat(self.name_width.saturating_sub(self.name.chars().count()));
        let (age, msg) = match &self.agent {
            Some(a) => (format_age(age_secs(self.now, a.since)), a.msg.as_str()),
            None => (String::new(), ""),
        };
        // Same colour as the wrapper session's name in the tmux status-left.
        let name_color = match &self.target {
            Target::Remote { host, .. } => format!("\x1b[38;5;{}m", host.color()),
            Target::Local { .. } => String::new(),
        };
        format!(
            "{color}{icon}\x1b[0m {name_color}{}\x1b[0m{pad}  \x1b[2m{age:>4}  {msg}\x1b[0m",
            self.name
        )
    }
}

impl SkimItem for Row {
    fn text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.name)
    }

    fn display(&self, _context: DisplayContext) -> Line<'_> {
        self.line()
            .into_text()
            .ok()
            .and_then(|t| t.lines.into_iter().next())
            .unwrap_or_default()
    }

    fn output(&self) -> Cow<'_, str> {
        Cow::Owned(self.key())
    }
}

fn agents_of(entries: &[AgentEntry], host: Option<&str>, project: &str) -> Option<AgentEntry> {
    let matching: Vec<AgentEntry> = entries
        .iter()
        .filter(|e| e.host.as_deref() == host && e.project() == project)
        .cloned()
        .collect();
    best_per_project(&matching).into_iter().next()
}

/// Assemble picker rows: local sessions and folders plus every host's projects.
/// `sessions` are the default server's; a folder also counts as running when its
/// project has an inner session (`inner_sessions`, never shown as rows of their own).
pub(crate) fn build_rows(
    folders: &[(String, String)],
    sessions: &[String],
    inner_sessions: &[String],
    local_agents: &[AgentEntry],
    hosts: &[HostState],
    current_session: Option<&str>,
    now: u64,
) -> Vec<Row> {
    let configured: Vec<HostConfig> = hosts.iter().map(|h| h.host.clone()).collect();
    let mut rows: Vec<Row> = Vec::new();
    let mk = |name: String, target: Target, exists: bool, agent: Option<AgentEntry>| Row {
        name,
        target,
        exists,
        agent,
        now,
        name_width: 0,
    };

    // Wrapper sessions (`mini/X`) are shown through the remote rows below.
    let is_wrapper = |s: &str| parse_wrapper(s, &configured).is_some();

    for s in sessions.iter().filter(|s| !is_wrapper(s)) {
        rows.push(mk(
            s.clone(),
            Target::Local {
                session: s.clone(),
                path: get_home_dir(),
            },
            true,
            agents_of(local_agents, None, s),
        ));
    }
    for (name, path) in folders.iter().filter(|f| !sessions.contains(&f.0)) {
        rows.push(mk(
            name.clone(),
            Target::Local {
                session: name.clone(),
                path: path.clone(),
            },
            inner_sessions.contains(name),
            agents_of(local_agents, None, name),
        ));
    }

    let remote_entries: Vec<AgentEntry> = hosts.iter().flat_map(|h| h.entries()).collect();
    for h in hosts {
        let mut seen: Vec<String> = Vec::new();
        let projects = h.data.iter().flat_map(|d| d.projects.iter());
        for p in projects {
            let wrapper = wrapper_session(&h.host.name, &p.name);
            seen.push(p.name.clone());
            rows.push(mk(
                wrapper.clone(),
                Target::Remote {
                    host: h.host.clone(),
                    project: p.name.clone(),
                },
                sessions.contains(&wrapper) || p.session_exists,
                agents_of(&remote_entries, Some(&h.host.name), &p.name),
            ));
        }
        // Wrappers whose project the host no longer lists (e.g. host offline) stay reachable.
        for s in sessions {
            if let Some((host, project)) = parse_wrapper(s, &configured) {
                if host.name == h.host.name && !seen.iter().any(|p| p == project) {
                    rows.push(mk(
                        s.clone(),
                        Target::Remote {
                            host: host.clone(),
                            project: project.to_string(),
                        },
                        true,
                        agents_of(&remote_entries, Some(&host.name), project),
                    ));
                }
            }
        }
    }

    // Waiting agents first (oldest wait on top), then everything else in discovery order.
    rows.sort_by_key(|r| {
        let since = match r.agent.as_ref() {
            Some(a) if a.state.is_waiting() => a.since,
            _ => 0,
        };
        (r.group(), since)
    });

    if let Some(current) = current_session {
        if let Some(idx) = rows.iter().position(|r| r.name == current) {
            let row = rows.remove(idx);
            rows.insert(0, row);
        }
    }

    let width = rows
        .iter()
        .map(|r| r.name.chars().count())
        .max()
        .unwrap_or(0);
    rows.iter_mut().for_each(|r| r.name_width = width);
    rows
}

pub(crate) fn get_match(key: &str, rows: Vec<Row>) -> Result<Row, Box<dyn Error>> {
    rows.into_iter()
        .find(|r| r.key() == key)
        .ok_or_else(|| "selected value not found in projects?".into())
}

/// One line naming each unreachable host and why, so a host with no rows
/// reads as "offline" rather than "not configured".
pub(crate) fn offline_header(hosts: &[HostState]) -> Option<String> {
    let down: Vec<String> = hosts
        .iter()
        .filter(|h| h.offline)
        .map(|h| match &h.error {
            Some(e) => format!("{} ✕ ({e})", h.host.name),
            None => format!("{} ✕", h.host.name),
        })
        .collect();
    (!down.is_empty()).then(|| down.join("  "))
}

pub(crate) fn select_in_skim(
    rows: Vec<Row>,
    header: Option<&str>,
) -> Result<String, Box<dyn Error>> {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "finter".to_string());
    let preview_cmd = format!("{} preview {{}}", crate::tmux::shell_quote(&exe));

    let mut builder = SkimOptionsBuilder::default();
    builder
        .height("100%")
        .multi(false)
        .color("dark")
        .preview(preview_cmd)
        .preview_window("right:50%");
    if let Some(h) = header {
        builder.header(h);
    }
    let options = builder.build()?;

    let (tx_item, rx_item): (SkimItemSender, SkimItemReceiver) = unbounded();
    let items: Vec<Arc<dyn SkimItem>> = rows
        .into_iter()
        .map(|r| Arc::new(r) as Arc<dyn SkimItem>)
        .collect();
    let _ = tx_item.send(items);
    drop(tx_item);

    let skim_result = Skim::run_with(options, Some(rx_item))?;

    if skim_result.is_abort {
        return Err("No selection made.".into());
    }

    Ok(skim_result
        .selected_items
        .first()
        .ok_or("no item in Skim is selected")?
        .output()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{ListOutput, ListPane, ListProject, LIST_VERSION};
    use crate::state::test_entry;

    fn host() -> HostConfig {
        HostConfig {
            name: "mini".into(),
            ssh: "mac-mini".into(),
            color: None,
        }
    }

    fn host_state(projects: &[(&str, bool)], panes: Vec<ListPane>, offline: bool) -> HostState {
        HostState {
            host: host(),
            offline,
            error: None,
            data: Some(ListOutput {
                v: LIST_VERSION,
                projects: projects
                    .iter()
                    .map(|(n, e)| ListProject {
                        name: n.to_string(),
                        path: format!("/p/{n}"),
                        session_exists: *e,
                    })
                    .collect(),
                panes,
            }),
        }
    }

    fn pane(session: &str, state: AgentState, since: u64) -> ListPane {
        ListPane {
            session: session.into(),
            window: "1".into(),
            pane: "%1".into(),
            state,
            since,
            msg: "hello".into(),
        }
    }

    fn folders(names: &[&str]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|n| (n.to_string(), format!("/tmp/{n}")))
            .collect()
    }

    fn names(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn prefers_existing_session_over_folder_duplicate() {
        let rows = build_rows(
            &folders(&["repo1"]),
            &["repo1".into()],
            &[],
            &[],
            &[],
            None,
            0,
        );
        assert_eq!(rows.len(), 1);
        assert!(rows[0].exists);
    }

    #[test]
    fn folder_exists_when_its_project_has_an_inner_session() {
        let agents = vec![crate::state::test_inner_entry("repo1", AgentState::Done, 5)];
        let inner: Vec<String> = vec!["repo1".into()];
        let rows = build_rows(
            &folders(&["repo1", "repo2"]),
            &[],
            &inner,
            &agents,
            &[],
            None,
            10,
        );
        // Inner sessions never get rows of their own.
        assert_eq!(names(&rows), ["repo1", "repo2"]);
        assert!(rows[0].exists && !rows[1].exists);
        assert_eq!(rows[0].agent.as_ref().unwrap().state, AgentState::Done);
        assert_eq!(rows[0].key(), "local:repo1");
    }

    #[test]
    fn current_session_goes_first() {
        let rows = build_rows(
            &folders(&["repo1"]),
            &["other".into(), "work".into()],
            &[],
            &[],
            &[],
            Some("work"),
            0,
        );
        assert_eq!(rows[0].name, "work");
    }

    #[test]
    fn sorts_asking_done_working_sessions_folders() {
        let agents = vec![
            test_entry(None, "w", AgentState::Working, 5),
            test_entry(None, "d", AgentState::Done, 5),
            test_entry(None, "a", AgentState::Asking, 5),
        ];
        let sessions: Vec<String> = ["plain", "w", "d", "a"].map(String::from).to_vec();
        let rows = build_rows(
            &folders(&["folder"]),
            &sessions,
            &[],
            &agents,
            &[],
            None,
            100,
        );
        assert_eq!(names(&rows), ["a", "d", "w", "plain", "folder"]);
    }

    #[test]
    fn oldest_wait_comes_first_within_a_state() {
        let agents = vec![
            test_entry(None, "new", AgentState::Asking, 90),
            test_entry(None, "old", AgentState::Asking, 10),
        ];
        let sessions: Vec<String> = ["new", "old"].map(String::from).to_vec();
        let rows = build_rows(&[], &sessions, &[], &agents, &[], None, 100);
        assert_eq!(names(&rows), ["old", "new"]);
    }

    #[test]
    fn remote_projects_become_host_prefixed_rows() {
        let hs = host_state(
            &[("recap", true), ("finter", false)],
            vec![pane("recap", AgentState::Done, 40)],
            false,
        );
        let sessions: Vec<String> = vec!["mini/recap".into(), "local".into()];
        let rows = build_rows(&folders(&[]), &sessions, &[], &[], &[hs], None, 100);
        assert_eq!(names(&rows), ["mini/recap", "local", "mini/finter"]);
        assert_eq!(rows[0].agent.as_ref().unwrap().state, AgentState::Done);
        assert!(rows[0].exists);
        assert!(!rows[2].exists);
        assert_eq!(rows[0].key(), "remote:mac-mini:recap");
        assert_eq!(rows.iter().filter(|r| r.name == "mini/recap").count(), 1);
    }

    #[test]
    fn wrapper_without_remote_listing_stays_reachable() {
        let hs = HostState {
            host: host(),
            offline: true,
            error: None,
            data: None,
        };
        let sessions: Vec<String> = vec!["mini/gone".into()];
        let rows = build_rows(&[], &sessions, &[], &[], &[hs], None, 0);
        assert_eq!(names(&rows), ["mini/gone"]);
        assert!(matches!(rows[0].target, Target::Remote { .. }));
    }

    #[test]
    fn row_line_shows_icon_age_and_message() {
        let agents = vec![{
            let mut e = test_entry(None, "a", AgentState::Asking, 40);
            e.msg = "needs ok".into();
            e
        }];
        let rows = build_rows(&[], &["a".into()], &[], &agents, &[], None, 220);
        let line = rows[0].line();
        assert!(line.contains('❓') && line.contains("3m") && line.contains("needs ok"));
        assert_eq!(get_match("local:a", rows).unwrap().name, "a");
    }

    #[test]
    fn display_shows_the_row_line_with_its_colours() {
        let agents = vec![test_entry(None, "a", AgentState::Asking, 40)];
        let rows = build_rows(&[], &["a".into()], &[], &agents, &[], None, 220);
        let line = rows[0].display(DisplayContext::default());
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("❓ a"), "{text:?}");
        let icon = line
            .spans
            .iter()
            .find(|s| s.content.contains('❓'))
            .unwrap();
        assert_eq!(icon.style.fg, Some(ratatui_core::style::Color::Red));
    }

    #[test]
    fn remote_row_names_use_host_color() {
        let hs = host_state(&[("recap", false)], vec![], false);
        let rows = build_rows(&[], &[], &[], &[], &[hs], None, 0);
        assert!(rows[0].line().contains("\x1b[38;5;208mmini/recap\x1b[0m"));
        let mut hs = host_state(&[("recap", false)], vec![], false);
        hs.host.color = Some(228);
        let rows = build_rows(&[], &[], &[], &[], &[hs], None, 0);
        assert!(rows[0].line().contains("\x1b[38;5;228mmini/recap"));
        let rows = build_rows(&[], &["a".into()], &[], &[], &[], None, 0);
        assert!(!rows[0].line().contains("38;5;208"));
    }

    #[test]
    fn offline_header_names_down_hosts_with_reason() {
        let up = host_state(&[], vec![], false);
        assert_eq!(offline_header(std::slice::from_ref(&up)), None);

        let mut down = host_state(&[], vec![], true);
        down.error = Some("finter too old?".to_string());
        let header = offline_header(&[up, down]).expect("header");
        assert!(header.ends_with("✕ (finter too old?)"), "{header}");
    }
}
