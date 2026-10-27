use crate::config::{get_home_dir, HostConfig};
use crate::remote::HostState;
use crate::state::{
    age_secs, best_per_project, format_age, wrapper_session, AgentEntry, AgentState,
};
use crate::status::{local_target, parse_wrapper, remote_target};
use ratatui_core::style::{Color, Modifier, Style};
use ratatui_core::text::{Line, Span};
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
    /// State of every tracked pane (Claude, `finter watch`) in window order.
    states: Vec<AgentState>,
    now: u64,
    name_width: usize,
    /// Icons of the row with the most tracked panes, so names line up.
    icon_slots: usize,
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
}

impl SkimItem for Row {
    fn text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.name)
    }

    // Spans set only a foreground or DIM, never a background or reset colour,
    // so skim's full-row highlight shows through the whole row.
    fn display(&self, _context: DisplayContext) -> Line<'_> {
        let icon = |state: Option<AgentState>| match state {
            Some(AgentState::Asking) => Span::styled("❓", Style::new().fg(Color::Red)),
            Some(AgentState::Done) => Span::styled("✅", Style::new().fg(Color::Green)),
            Some(AgentState::Working) => Span::styled("⚙ ", Style::new().fg(Color::Yellow)),
            None if self.exists => Span::raw("* "),
            None => Span::raw("  "),
        };
        let mut spans: Vec<Span> = match self.states.as_slice() {
            [] => vec![icon(None)],
            states => states.iter().map(|s| icon(Some(*s))).collect(),
        };
        // Every icon is two cells wide.
        let icon_pad = "  ".repeat(self.icon_slots.saturating_sub(self.states.len().max(1)));
        if !icon_pad.is_empty() {
            spans.push(Span::raw(icon_pad));
        }
        spans.push(Span::raw(" "));
        // Same colour as the wrapper session's name in the tmux status-left.
        let name_style = match &self.target {
            Target::Remote { host, .. } => Style::new().fg(Color::Indexed(host.color())),
            Target::Local { .. } => Style::new(),
        };
        spans.push(Span::styled(self.name.as_str(), name_style));
        let pad = " ".repeat(self.name_width.saturating_sub(self.name.chars().count()));
        spans.push(Span::raw(format!("{pad}  ")));
        let (age, msg) = match &self.agent {
            Some(a) => (format_age(age_secs(self.now, a.since)), a.msg.as_str()),
            None => (String::new(), ""),
        };
        spans.push(Span::styled(
            format!("{age:>4}  {msg}"),
            Style::new().add_modifier(Modifier::DIM),
        ));
        Line::from(spans)
    }

    fn output(&self) -> Cow<'_, str> {
        Cow::Owned(self.key())
    }
}

/// Index for ordering `1`, `2`, `10` and `%3`, `%12` numerically; anything else last.
fn index_of(id: &str) -> u64 {
    id.trim_start_matches('%').parse().unwrap_or(u64::MAX)
}

/// The project's most urgent agent, and every tracked pane's state in window order.
fn agents_of(
    entries: &[AgentEntry],
    host: Option<&str>,
    project: &str,
) -> (Option<AgentEntry>, Vec<AgentState>) {
    let mut matching: Vec<AgentEntry> = entries
        .iter()
        .filter(|e| e.host.as_deref() == host && e.project() == project)
        .cloned()
        .collect();
    matching.sort_by_key(|e| (index_of(&e.window), index_of(&e.pane_id)));
    let states = matching.iter().map(|e| e.state).collect();
    (best_per_project(&matching).into_iter().next(), states)
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
    let mk = |name: String,
              target: Target,
              exists: bool,
              (agent, states): (Option<AgentEntry>, Vec<AgentState>)| Row {
        name,
        target,
        exists,
        agent,
        states,
        now,
        name_width: 0,
        icon_slots: 0,
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
    let slots = rows.iter().map(|r| r.states.len()).max().unwrap_or(0);
    rows.iter_mut().for_each(|r| {
        r.name_width = width;
        r.icon_slots = slots;
    });
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

fn skim_options(header: Option<&str>, preview_cmd: String) -> Result<SkimOptions, Box<dyn Error>> {
    let mut builder = SkimOptionsBuilder::default();
    builder
        .height("100%")
        .multi(false)
        .color("dark")
        .highlight_line(true)
        .cycle(true)
        .preview(preview_cmd)
        .preview_window("right:50%");
    if let Some(h) = header {
        builder.header(h);
    }
    Ok(builder.build()?)
}

pub(crate) fn select_in_skim(
    rows: Vec<Row>,
    header: Option<&str>,
) -> Result<String, Box<dyn Error>> {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "finter".to_string());
    let preview_cmd = format!("{} preview {{}}", crate::tmux::shell_quote(&exe));
    let options = skim_options(header, preview_cmd)?;

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
        let line = plain(&rows[0]);
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
    fn display_leaves_the_background_to_the_highlighted_row() {
        use ratatui_core::{buffer::Buffer, layout::Rect, style::Style, widgets::Widget};
        let hs = host_state(
            &[("recap", true)],
            vec![pane("recap", AgentState::Done, 40)],
            false,
        );
        let rows = build_rows(&[], &[], &[], &[], &[hs], None, 220);
        let line = rows[0].display(DisplayContext::default());
        // No span may set a background or reset colour, or the full-row
        // highlight would show gaps behind the name, age and message.
        for span in &line.spans {
            assert_eq!(span.style.bg, None, "{span:?}");
            assert_ne!(span.style.fg, Some(Color::Reset), "{span:?}");
        }
        let find = |s: &str| line.spans.iter().find(|sp| sp.content.contains(s)).unwrap();
        assert_eq!(find("✅").style.fg, Some(Color::Green));
        assert_eq!(find("mini/recap").style.fg, Some(Color::Indexed(208)));
        let age = find("3m");
        assert!(age.style.add_modifier.contains(Modifier::DIM), "{age:?}");

        // Render it the way skim draws the current row (dark theme: bg 236 on
        // the whole line): every cell keeps the highlight and its own colour.
        let width = 40;
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 1));
        let current = Style::default().bg(Color::Indexed(236));
        line.clone().style(current).render(buf.area, &mut buf);
        let mut x = 0;
        while x < width {
            assert_eq!(buf[(x, 0)].bg, Color::Indexed(236), "cell {x}");
            // The emoji's second cell is hidden behind it, so ratatui leaves it blank.
            x += if buf[(x, 0)].symbol() == "✅" { 2 } else { 1 };
        }
        let cell_of = |s: &str| {
            let text: String = (0..width).map(|x| buf[(x, 0)].symbol()).collect();
            let col = text.find(s).unwrap();
            &buf[(text[..col].chars().count() as u16, 0)]
        };
        assert_eq!(cell_of("mini").fg, Color::Indexed(208));
        assert!(cell_of("3m").modifier.contains(Modifier::DIM));
    }

    fn spans(row: &Row) -> Vec<(String, ratatui_core::style::Style)> {
        let line = row.display(DisplayContext::default());
        line.spans
            .iter()
            .map(|s| (s.content.to_string(), s.style))
            .collect()
    }

    #[test]
    fn display_builds_styled_spans_per_column() {
        use ratatui_core::style::Style;
        let dim = Style::new().add_modifier(Modifier::DIM);
        let hs = host_state(
            &[("recap", true), ("finter", false)],
            vec![pane("recap", AgentState::Done, 40)],
            false,
        );
        let mut asking = test_entry(None, "a", AgentState::Asking, 100);
        asking.msg = "needs ok".into();
        let mut working = test_entry(None, "w", AgentState::Working, 160);
        working.msg = "busy".into();
        let sessions: Vec<String> = ["a", "w", "plain"].map(String::from).to_vec();
        let rows = build_rows(
            &folders(&["folder"]),
            &sessions,
            &[],
            &[asking, working],
            &[hs],
            None,
            220,
        );
        assert_eq!(
            names(&rows),
            ["a", "mini/recap", "w", "plain", "folder", "mini/finter"]
        );
        let s = |text: &str, style: Style| (text.to_string(), style);
        let red = Style::new().fg(Color::Red);
        let green = Style::new().fg(Color::Green);
        let yellow = Style::new().fg(Color::Yellow);
        let host = Style::new().fg(Color::Indexed(208));
        let none = Style::new();
        // Names are padded to the widest, `mini/recap` and `mini/finter` (11).
        assert_eq!(
            spans(&rows[0]),
            [
                s("❓", red),
                s(" ", none),
                s("a", none),
                s("            ", none),
                s("  2m  needs ok", dim),
            ]
        );
        assert_eq!(
            spans(&rows[1]),
            [
                s("✅", green),
                s(" ", none),
                s("mini/recap", host),
                s("   ", none),
                s("  3m  hello", dim),
            ]
        );
        assert_eq!(spans(&rows[2])[0], s("⚙ ", yellow));
        assert_eq!(spans(&rows[2])[4], s("  1m  busy", dim));
        assert_eq!(spans(&rows[3])[0], s("* ", none));
        assert_eq!(
            spans(&rows[4]),
            [
                s("  ", none),
                s(" ", none),
                s("folder", none),
                s("       ", none),
                s("      ", dim),
            ]
        );
        assert_eq!(spans(&rows[5])[2], s("mini/finter", host));
    }

    #[test]
    fn skim_options_highlight_the_full_row_and_cycle() {
        let options = skim_options(Some("mini ✕"), "finter preview {}".into()).unwrap();
        assert!(options.highlight_line);
        assert!(options.cycle);
        assert_eq!(options.header.as_deref(), Some("mini ✕"));
        assert_eq!(options.preview.as_deref(), Some("finter preview {}"));
        assert!(!options.multi);
        assert!(skim_options(None, String::new()).unwrap().header.is_none());
    }

    fn plain(row: &Row) -> String {
        let line = row.display(DisplayContext::default());
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn row_shows_one_icon_per_tracked_pane_in_window_order() {
        let pane_in = |window: &str, state| AgentEntry {
            window: window.into(),
            ..crate::state::test_inner_entry("finter", state, 50)
        };
        let agents = vec![
            pane_in("10", AgentState::Asking),
            pane_in("2", AgentState::Done),
            pane_in("1", AgentState::Working),
        ];
        let rows = build_rows(
            &folders(&["finter", "solo"]),
            &[],
            &["finter".into()],
            &agents,
            &[],
            None,
            100,
        );
        assert_eq!(names(&rows), ["finter", "solo"]);
        assert!(
            plain(&rows[0]).starts_with("⚙ ✅❓ finter"),
            "{:?}",
            plain(&rows[0])
        );
        // The most urgent pane still drives the row's group.
        assert_eq!(rows[0].agent.as_ref().unwrap().state, AgentState::Asking);
        // Other rows pad the icon column so names stay aligned.
        assert!(
            plain(&rows[1]).starts_with("       solo"),
            "{:?}",
            plain(&rows[1])
        );
    }

    #[test]
    fn remote_row_shows_its_host_panes_icons() {
        let mut second = pane("recap", AgentState::Working, 40);
        second.window = "2".into();
        second.pane = "%2".into();
        let hs = host_state(
            &[("recap", true)],
            vec![second, pane("recap", AgentState::Done, 40)],
            false,
        );
        let rows = build_rows(&[], &[], &[], &[], &[hs], None, 100);
        assert!(
            plain(&rows[0]).starts_with("✅⚙  mini/recap"),
            "{:?}",
            plain(&rows[0])
        );
    }

    #[test]
    fn remote_row_names_use_host_color() {
        let hs = host_state(&[("recap", false)], vec![], false);
        let rows = build_rows(&[], &[], &[], &[], &[hs], None, 0);
        let name_fg = |row: &Row, name: &str| {
            let (_, style) = spans(row).into_iter().find(|(t, _)| t == name).unwrap();
            style.fg
        };
        assert_eq!(name_fg(&rows[0], "mini/recap"), Some(Color::Indexed(208)));
        let mut hs = host_state(&[("recap", false)], vec![], false);
        hs.host.color = Some(228);
        let rows = build_rows(&[], &[], &[], &[], &[hs], None, 0);
        assert_eq!(name_fg(&rows[0], "mini/recap"), Some(Color::Indexed(228)));
        let rows = build_rows(&[], &["a".into()], &[], &[], &[], None, 0);
        assert_eq!(name_fg(&rows[0], "a"), None);
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
