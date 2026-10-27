use serde::{Deserialize, Serialize};

/// Suffix of the inner terminal sessions that accompany project session `X`:
/// `X~term` for window 2, `X~term2`, `X~term3`... for windows opened later.
pub(crate) const TERM_SUFFIX: &str = "~term";

/// What an agent in a pane is doing. Stored in the `@agent_state` pane option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AgentState {
    Asking,
    Done,
    Working,
}

impl AgentState {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "asking" => Some(Self::Asking),
            "done" => Some(Self::Done),
            "working" => Some(Self::Working),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Asking => "asking",
            Self::Done => "done",
            Self::Working => "working",
        }
    }

    /// Lower sorts first: asking > done > working.
    fn priority(self) -> u8 {
        match self {
            Self::Asking => 0,
            Self::Done => 1,
            Self::Working => 2,
        }
    }

    /// States that need a human (everything but `working`).
    pub(crate) fn is_waiting(self) -> bool {
        self != Self::Working
    }
}

/// One agent pane, local or on a remote host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentEntry {
    /// Host name from `[[hosts]]`, `None` for this machine.
    pub(crate) host: Option<String>,
    /// tmux session the pane lives in (the inner session for remote entries).
    pub(crate) session: String,
    pub(crate) window: String,
    pub(crate) pane_id: String,
    pub(crate) state: AgentState,
    /// Unix epoch seconds of the last state change.
    pub(crate) since: u64,
    pub(crate) msg: String,
    /// The pane lives on an inner (`-L finter`) server, not the default one. Pane ids of
    /// the two local servers can collide (`%3` on both), so consumers must pick the socket.
    pub(crate) inner: bool,
}

impl AgentEntry {
    /// Project name: the session without the `~term[N]` suffix.
    pub(crate) fn project(&self) -> &str {
        project_of(&self.session)
    }

    pub(crate) fn is_term(&self) -> bool {
        project_of(&self.session).len() < self.session.len()
    }

    /// Session on this machine's default server that shows the entry: the wrapper
    /// (`mini/X`, or `X` for an inner entry of this machine), else the session itself.
    pub(crate) fn local_session(&self) -> String {
        match &self.host {
            Some(host) => wrapper_session(host, self.project()),
            None if self.inner => self.project().to_string(),
            None => self.session.clone(),
        }
    }
}

pub(crate) fn project_of(session: &str) -> &str {
    term_slot(session).map_or(session, |(project, _)| project)
}

/// Split `X~term` into `("X", 1)` and `X~termN` into `("X", N)`; `None` for project sessions.
pub(crate) fn term_slot(session: &str) -> Option<(&str, u32)> {
    let (project, n) = session.rsplit_once(TERM_SUFFIX)?;
    if project.is_empty() {
        return None;
    }
    match n {
        "" => Some((project, 1)),
        // Only the spelling `inner_session_name` produces: `~term2`, never `~term1` or `~term02`.
        _ if n.bytes().all(|b| b.is_ascii_digit()) && !n.starts_with('0') => {
            n.parse().ok().filter(|&n| n >= 2).map(|n| (project, n))
        }
        _ => None,
    }
}

pub(crate) fn wrapper_session(host: &str, project: &str) -> String {
    format!("{host}/{project}")
}

pub(crate) fn age_secs(now: u64, since: u64) -> u64 {
    now.saturating_sub(since)
}

/// Compact wait time: `42s`, `3m`, `2h`, `1d`.
pub(crate) fn format_age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

/// asking first, then done, then working; oldest first within a state.
pub(crate) fn sort_entries(entries: &mut [AgentEntry]) {
    entries.sort_by_key(|e| (e.state.priority(), e.since));
}

/// The entry `finter next` jumps to: oldest asking, else oldest done.
pub(crate) fn pick_next(entries: &[AgentEntry]) -> Option<&AgentEntry> {
    entries
        .iter()
        .filter(|e| e.state.is_waiting())
        .min_by_key(|e| (e.state.priority(), e.since))
}

/// Collapse each project to its most urgent entry (asking > done > working,
/// then oldest). `X` and `X~term` therefore yield one badge.
pub(crate) fn best_per_project(entries: &[AgentEntry]) -> Vec<AgentEntry> {
    let mut sorted = entries.to_vec();
    sort_entries(&mut sorted);
    let mut seen: Vec<(Option<String>, String)> = Vec::new();
    let mut out = Vec::new();
    for entry in sorted {
        let key = (entry.host.clone(), entry.project().to_string());
        if !seen.contains(&key) {
            seen.push(key);
            out.push(entry);
        }
    }
    out
}

#[cfg(test)]
pub(crate) fn test_entry(
    host: Option<&str>,
    session: &str,
    state: AgentState,
    since: u64,
) -> AgentEntry {
    AgentEntry {
        host: host.map(str::to_string),
        session: session.to_string(),
        window: "1".to_string(),
        pane_id: "%1".to_string(),
        state,
        since,
        msg: String::new(),
        // Entries of other hosts always come from their inner server.
        inner: host.is_some(),
    }
}

/// An agent entry on this machine's inner server.
#[cfg(test)]
pub(crate) fn test_inner_entry(session: &str, state: AgentState, since: u64) -> AgentEntry {
    AgentEntry {
        inner: true,
        ..test_entry(None, session, state, since)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_state_roundtrip() {
        for s in [AgentState::Asking, AgentState::Done, AgentState::Working] {
            assert_eq!(AgentState::parse(s.as_str()), Some(s));
        }
        assert_eq!(AgentState::parse("bogus"), None);
        assert_eq!(AgentState::parse(""), None);
    }

    #[test]
    fn format_age_buckets() {
        assert_eq!(format_age(5), "5s");
        assert_eq!(format_age(60), "1m");
        assert_eq!(format_age(190), "3m");
        assert_eq!(format_age(7300), "2h");
        assert_eq!(format_age(200_000), "2d");
        assert_eq!(age_secs(10, 20), 0);
    }

    #[test]
    fn sort_orders_asking_done_working_oldest_first() {
        let mut v = vec![
            test_entry(None, "w", AgentState::Working, 1),
            test_entry(None, "d2", AgentState::Done, 50),
            test_entry(None, "a2", AgentState::Asking, 90),
            test_entry(None, "d1", AgentState::Done, 10),
            test_entry(None, "a1", AgentState::Asking, 80),
        ];
        sort_entries(&mut v);
        let names: Vec<&str> = v.iter().map(|e| e.session.as_str()).collect();
        assert_eq!(names, ["a1", "a2", "d1", "d2", "w"]);
    }

    #[test]
    fn next_prefers_oldest_asking_over_older_done() {
        let v = vec![
            test_entry(None, "done-old", AgentState::Done, 1),
            test_entry(Some("mini"), "ask-new", AgentState::Asking, 100),
            test_entry(None, "ask-old", AgentState::Asking, 50),
        ];
        assert_eq!(pick_next(&v).unwrap().session, "ask-old");
    }

    #[test]
    fn next_falls_back_to_oldest_done_and_ignores_working() {
        let v = vec![
            test_entry(None, "w", AgentState::Working, 1),
            test_entry(None, "d-new", AgentState::Done, 9),
            test_entry(Some("mini"), "d-old", AgentState::Done, 3),
        ];
        assert_eq!(pick_next(&v).unwrap().session, "d-old");
        assert!(pick_next(&[test_entry(None, "w", AgentState::Working, 1)]).is_none());
        assert!(pick_next(&[]).is_none());
    }

    #[test]
    fn term_sessions_map_to_project_and_wrapper() {
        let e = test_entry(Some("mini"), "recap~term", AgentState::Asking, 1);
        assert!(e.is_term());
        assert_eq!(e.project(), "recap");
        assert_eq!(e.local_session(), "mini/recap");
        let local = test_entry(None, "recap", AgentState::Asking, 1);
        assert_eq!(local.local_session(), "recap");
        let inner = test_inner_entry("recap~term", AgentState::Asking, 1);
        assert_eq!(inner.local_session(), "recap");
        assert_eq!(
            test_inner_entry("recap", AgentState::Done, 1).local_session(),
            "recap"
        );
        let third = test_entry(Some("mini"), "recap~term3", AgentState::Done, 1);
        assert!(third.is_term());
        assert_eq!(third.local_session(), "mini/recap");
    }

    #[test]
    fn term_slot_accepts_only_canonical_suffixes() {
        assert_eq!(term_slot("x~term"), Some(("x", 1)));
        assert_eq!(term_slot("x~term2"), Some(("x", 2)));
        assert_eq!(term_slot("a~term~term12"), Some(("a~term", 12)));
        for s in ["x", "x~term1", "x~term02", "x~termy", "~term", "x~term-2"] {
            assert_eq!(term_slot(s), None, "{s}");
        }
        assert_eq!(project_of("x~termy"), "x~termy");
    }

    #[test]
    fn best_per_project_collapses_term_sessions() {
        let v = vec![
            test_entry(Some("mini"), "x", AgentState::Working, 5),
            test_entry(Some("mini"), "x~term", AgentState::Asking, 9),
            test_entry(None, "x", AgentState::Done, 1),
        ];
        let best = best_per_project(&v);
        assert_eq!(best.len(), 2);
        assert_eq!(best[0].state, AgentState::Asking);
        assert_eq!(best[1].host, None);
    }
}
