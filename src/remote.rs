use crate::config::{Config, HostConfig};
use crate::state::{wrapper_session, AgentEntry, AgentState, TERM_SUFFIX};
use crate::tmux::{self, shell_quote, Socket};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs,
    io::Read,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Schema version of `finter list --json`.
pub(crate) const LIST_VERSION: u32 = 1;
/// A cache entry older than this is refreshed.
pub(crate) const CACHE_TTL_SECS: u64 = 4;
/// Offline hosts are retried less often so a dead host does not stall every refresh.
pub(crate) const OFFLINE_TTL_SECS: u64 = 30;
/// Hard stop for one ssh call; ConnectTimeout alone does not bound a hung session.
const SSH_DEADLINE: Duration = Duration::from_secs(6);

const INNER_CONF: &str = include_str!("inner.conf");

// ── list --json schema

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ListProject {
    pub(crate) name: String,
    pub(crate) path: String,
    /// Whether session `name` exists on the inner server.
    pub(crate) session_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ListPane {
    /// Inner session name (`X` or `X~term`).
    pub(crate) session: String,
    pub(crate) window: String,
    pub(crate) pane: String,
    pub(crate) state: AgentState,
    pub(crate) since: u64,
    pub(crate) msg: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ListOutput {
    pub(crate) v: u32,
    pub(crate) projects: Vec<ListProject>,
    pub(crate) panes: Vec<ListPane>,
}

pub(crate) fn build_list_output(
    folders: &[(String, String)],
    inner_sessions: &[String],
    panes: &[AgentEntry],
) -> ListOutput {
    ListOutput {
        v: LIST_VERSION,
        projects: folders
            .iter()
            .map(|(name, path)| ListProject {
                name: name.clone(),
                path: path.clone(),
                session_exists: inner_sessions.contains(name),
            })
            .collect(),
        panes: panes
            .iter()
            .map(|p| ListPane {
                session: p.session.clone(),
                window: p.window.clone(),
                pane: p.pane_id.clone(),
                state: p.state,
                since: p.since,
                msg: p.msg.clone(),
            })
            .collect(),
    }
}

pub(crate) fn entries_from_list(host: &str, list: &ListOutput) -> Vec<AgentEntry> {
    list.panes
        .iter()
        .map(|p| AgentEntry {
            host: Some(host.to_string()),
            session: p.session.clone(),
            window: p.window.clone(),
            pane_id: p.pane.clone(),
            state: p.state,
            since: p.since,
            msg: p.msg.clone(),
            inner: true,
        })
        .collect()
}

/// Reflect killed inner sessions in a host's list, so a picker opened before
/// the next refresh does not show them.
pub(crate) fn forget_sessions(list: &mut ListOutput, sessions: &[&str]) {
    for p in list.projects.iter_mut() {
        if sessions.contains(&p.name.as_str()) {
            p.session_exists = false;
        }
    }
    list.panes
        .retain(|p| !sessions.contains(&p.session.as_str()));
}

/// Parse ssh output. The JSON is the last line that starts with `{`, so login
/// banners cannot break it. Newer schema versions are accepted (fields are only
/// ever added), and panes with an unknown state are skipped.
pub(crate) fn parse_list_output(raw: &str) -> Result<ListOutput, String> {
    let line = raw
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .ok_or("no JSON in remote output")?;
    let mut value: serde_json::Value =
        serde_json::from_str(line.trim()).map_err(|e| format!("bad remote JSON: {e}"))?;
    let version = value.get("v").and_then(|v| v.as_u64()).unwrap_or(0);
    if version < u64::from(LIST_VERSION) {
        return Err(format!("unsupported list version {version}"));
    }
    if let Some(panes) = value.get_mut("panes").and_then(|p| p.as_array_mut()) {
        panes.retain(|p| {
            p.get("state")
                .and_then(|s| s.as_str())
                .and_then(AgentState::parse)
                .is_some()
        });
    }
    serde_json::from_value(value).map_err(|e| format!("bad remote JSON: {e}"))
}

/// Short, human-facing reason for a failed `ssh ... finter list --json`.
pub(crate) fn ssh_failure_reason(code: Option<i32>) -> String {
    match code {
        Some(255) => "unreachable".to_string(),
        // clap's usage error: an old finter without the `list` subcommand.
        Some(2) => "finter too old?".to_string(),
        Some(127) => "finter not found".to_string(),
        Some(c) => format!("ssh exit {c}"),
        None => "ssh killed".to_string(),
    }
}

// ── host cache

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HostCache {
    pub(crate) fetched_at: u64,
    /// True when the last refresh failed; `data` is then the last good result.
    pub(crate) offline: bool,
    pub(crate) data: Option<ListOutput>,
    /// Short reason of the last failed refresh, shown next to the host in the status bar.
    #[serde(default)]
    pub(crate) error: Option<String>,
}

pub(crate) fn is_stale(cache: &HostCache, now: u64) -> bool {
    let ttl = if cache.offline {
        OFFLINE_TTL_SECS
    } else {
        CACHE_TTL_SECS
    };
    now.saturating_sub(cache.fetched_at) >= ttl
}

/// Fold a fetch result into the previous cache: failure keeps old data but flags offline.
pub(crate) fn merge_fetch(
    prev: Option<HostCache>,
    now: u64,
    fetched: Result<ListOutput, String>,
) -> HostCache {
    match fetched {
        Ok(data) => HostCache {
            fetched_at: now,
            offline: false,
            data: Some(data),
            error: None,
        },
        Err(e) => HostCache {
            fetched_at: now,
            offline: true,
            data: prev.and_then(|p| p.data),
            error: Some(e),
        },
    }
}

/// Per-user fallback owner id: the owner of the home directory.
fn user_tag() -> String {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(crate::config::get_home_dir())
        .map(|m| m.uid().to_string())
        .unwrap_or_else(|_| "user".to_string())
}

/// `$XDG_RUNTIME_DIR/finter`, or `<tmp>/finter-<uid>` when the variable is unset.
pub(crate) fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(base) => PathBuf::from(base).join("finter"),
        None => std::env::temp_dir().join(format!("finter-{}", user_tag())),
    }
}

/// Create the runtime dir with mode 0700 (it can hold session names and messages).
pub(crate) fn ensure_runtime_dir() -> std::io::Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let dir = runtime_dir();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

fn cache_path(host: &str) -> PathBuf {
    runtime_dir().join(format!("{host}.json"))
}

pub(crate) fn read_cache(host: &str) -> Option<HostCache> {
    serde_json::from_str(&fs::read_to_string(cache_path(host)).ok()?).ok()
}

pub(crate) fn write_cache(host: &str, cache: &HostCache) {
    let Ok(json) = serde_json::to_string(cache) else {
        return;
    };
    let Ok(dir) = ensure_runtime_dir() else {
        return;
    };
    // Write-then-rename so a concurrent reader never sees half a file.
    let tmp = dir.join(format!("{host}.json.{}", std::process::id()));
    if fs::write(&tmp, json).is_ok() {
        let _ = fs::rename(&tmp, cache_path(host));
    }
}

// ── ssh

fn ssh_args(alias: &str, remote_cmd: &str) -> Vec<String> {
    [
        "-o",
        "ConnectTimeout=2",
        "-o",
        "BatchMode=yes",
        "-o",
        "ServerAliveInterval=2",
        "-o",
        "ServerAliveCountMax=1",
        alias,
        remote_cmd,
    ]
    .map(String::from)
    .to_vec()
}

/// Run `remote_cmd` on `alias` with a hard deadline; Ok carries stdout.
pub(crate) fn ssh_run(alias: &str, remote_cmd: &str) -> Result<String, String> {
    let mut child = Command::new("ssh")
        .args(ssh_args(alias, remote_cmd))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "ssh not found".to_string())?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let reader = thread::spawn(move || {
        let mut buf = String::new();
        let _ = stdout.read_to_string(&mut buf);
        buf
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None if started.elapsed() > SSH_DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err("timed out".to_string());
            }
            None => thread::sleep(Duration::from_millis(20)),
        }
    };
    let out = reader.join().unwrap_or_default();
    if status.success() {
        Ok(out)
    } else {
        Err(ssh_failure_reason(status.code()))
    }
}

/// Fire-and-forget ssh; used where the caller must not wait (focus hooks).
pub(crate) fn ssh_spawn(alias: &str, remote_cmd: &str) {
    let _ = Command::new("ssh")
        .args(ssh_args(alias, remote_cmd))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// The inner-server tmux command prefix for a remote shell.
pub(crate) fn remote_tmux(args: &[&str]) -> String {
    let quoted: Vec<String> = args.iter().map(|a| shell_quote(a)).collect();
    format!(
        "tmux -L {} {}",
        shell_quote(&tmux::inner_socket_name()),
        quoted.join(" ")
    )
}

// ── host state

#[derive(Debug, Clone)]
pub(crate) struct HostState {
    pub(crate) host: HostConfig,
    pub(crate) offline: bool,
    pub(crate) error: Option<String>,
    pub(crate) data: Option<ListOutput>,
}

impl HostState {
    pub(crate) fn entries(&self) -> Vec<AgentEntry> {
        self.data
            .as_ref()
            .map(|d| entries_from_list(&self.host.name, d))
            .unwrap_or_default()
    }
}

/// Load a host's state, refreshing the cache through `fetch` when it is stale
/// (or missing). With `refresh` false the cache is used as is, however old, and
/// nothing blocks. `fetch` is the test seam for ssh.
pub(crate) fn load_host_with(
    host: &HostConfig,
    now: u64,
    refresh: bool,
    fetch: &dyn Fn(&HostConfig) -> Result<ListOutput, String>,
) -> HostState {
    let cached = read_cache(&host.name);
    let cache = match cached {
        Some(c) if !refresh || !is_stale(&c, now) => c,
        None if !refresh => HostCache {
            fetched_at: 0,
            offline: false,
            data: None,
            error: None,
        },
        prev => {
            let fresh = merge_fetch(prev, now, fetch(host));
            write_cache(&host.name, &fresh);
            fresh
        }
    };
    HostState {
        host: host.clone(),
        offline: cache.offline,
        error: cache.error,
        data: cache.data,
    }
}

pub(crate) fn fetch_list(host: &HostConfig) -> Result<ListOutput, String> {
    parse_list_output(&ssh_run(&host.ssh, "finter list --json")?)
}

pub(crate) fn load_hosts(config: &Config, now: u64, refresh: bool) -> Vec<HostState> {
    // Hosts are independent, so refresh them in parallel.
    let handles: Vec<_> = config
        .hosts
        .iter()
        .cloned()
        .map(|h| thread::spawn(move || load_host_with(&h, now, refresh, &fetch_list)))
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok()).collect()
}

// ── attach (runs on the remote machine)

/// Name of the inner session for project `name`: the project session for `None`,
/// `X~term` for terminal slot 1 and `X~termN` for slot N (see `state::term_slot`).
pub(crate) fn inner_session_name(name: &str, term: Option<u32>) -> String {
    match term {
        None => name.to_string(),
        Some(1) => format!("{name}{TERM_SUFFIX}"),
        Some(n) => format!("{name}{TERM_SUFFIX}{n}"),
    }
}

pub(crate) fn find_project_path<'a>(
    folders: &'a [(String, String)],
    name: &str,
) -> Option<&'a str> {
    folders
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, p)| p.as_str())
}

/// Write `inner.conf` unless it already matches; windows 1 and 2 attach at the same
/// time, and a plain truncate+write would let one tmux read a half-written file.
fn write_inner_conf() -> Result<PathBuf, Box<dyn Error>> {
    let dir = ensure_runtime_dir()?;
    let path = dir.join("inner.conf");
    if fs::read_to_string(&path).is_ok_and(|c| c == INNER_CONF) {
        return Ok(path);
    }
    let tmp = dir.join(format!("inner.conf.{}", std::process::id()));
    fs::write(&tmp, INNER_CONF)?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Replace this process with `tmux -L finter new-session -A` for project `name`.
pub(crate) fn exec_attach(name: &str, term: Option<u32>, path: &str) -> Result<(), Box<dyn Error>> {
    let conf = write_inner_conf()?;
    let session = inner_session_name(name, term);
    let err = Command::new("tmux")
        .arg("-L")
        .arg(tmux::inner_socket_name())
        .arg("-f")
        .arg(conf)
        .args(["new-session", "-A", "-s", &session, "-c", path])
        // A nested $TMUX would make tmux refuse to attach.
        .env_remove("TMUX")
        .exec();
    Err(format!("failed to exec tmux: {err}").into())
}

// ── wrapper session

/// `@finter_remote` value for a wrapper pane: `<ssh alias>:<inner session>`. An empty
/// alias means the inner server of this machine (`:X`), so the tag is non-empty for every
/// wrapper pane, which the tmux bindings rely on.
pub(crate) fn remote_tag(alias: &str, inner_session: &str) -> String {
    format!("{alias}:{inner_session}")
}

/// Split a `@finter_remote` value into `(alias, inner session)`; the alias is empty for
/// this machine. `None` for anything that is not a tag (including an empty session).
pub(crate) fn split_remote_tag(tag: &str) -> Option<(&str, &str)> {
    tag.rsplit_once(':').filter(|(_, s)| !s.is_empty())
}

/// Run inner-server tmux argv on `alias`'s machine and return stdout. An empty alias is
/// this machine: the command runs on `Socket::Inner` directly, never over ssh.
pub(crate) fn inner_run(alias: &str, args: &[&str]) -> Result<String, String> {
    if alias.is_empty() {
        let out = tmux::run_on(Socket::Inner, args);
        return if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(err.trim().to_string())
        };
    }
    ssh_run(alias, &remote_tmux(args))
}

/// Window command that attaches to inner session `project` (`--term[=N]` for terminals).
/// Remote (non-empty alias): retry ssh only when the connection drops (exit 255); any
/// other exit (clean detach, killed inner session, remote error) ends the loop.
/// Local (empty alias): `exec finter attach`, no ssh.
/// Wrapped in `sh -c` because the tmux default-shell may be fish.
pub(crate) fn attach_command(alias: &str, project: &str, term: Option<u32>) -> String {
    let suffix = match term {
        None => String::new(),
        Some(1) => " --term".to_string(),
        Some(n) => format!(" --term={n}"),
    };
    if alias.is_empty() {
        let script = format!("exec finter attach {}{suffix}", shell_quote(project));
        return format!("sh -c {}", shell_quote(&script));
    }
    // ssh joins its argv with spaces for the remote shell, so send one quoted string.
    let remote = format!("finter attach {}{suffix}", shell_quote(project));
    let script = format!(
        "while :; do ssh -t {} {}; rc=$?; [ $rc -eq 255 ] || exit $rc; echo 'finter: reconnecting...'; sleep 2; done",
        shell_quote(alias),
        shell_quote(&remote)
    );
    format!("sh -c {}", shell_quote(&script))
}

/// What a wrapper session is made of.
pub(crate) struct WrapperSpec<'a> {
    /// Session name on the default server: `host/X`, or `X` for this machine.
    pub(crate) session: &'a str,
    /// ssh alias of the owning machine; empty for this machine.
    pub(crate) alias: &'a str,
    pub(crate) project: &'a str,
    /// `@finter_colour` value; `None` leaves it unset (shown white in the status bar).
    pub(crate) colour: Option<String>,
    /// Start directory of the wrapper windows, so `pane_current_path` is the project.
    pub(crate) cwd: Option<&'a str>,
}

/// Create the wrapper with window 1 = claude, window 2 = terminal, both attached to
/// inner sessions (over ssh, or directly for an empty alias). False when tmux refused.
pub(crate) fn create_wrapper(spec: &WrapperSpec) -> bool {
    let mut args = vec![
        "new-session",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-s",
        spec.session,
    ];
    if let Some(cwd) = spec.cwd {
        args.extend(["-c", cwd]);
    }
    let cmd = attach_command(spec.alias, spec.project, None);
    args.push(&cmd);
    let Some(first) = tmux::stdout_of(Socket::Default, &args) else {
        return false;
    };
    tmux::run_on(
        Socket::Default,
        &[
            "set-option",
            "-p",
            "-t",
            &first,
            "@finter_remote",
            &remote_tag(spec.alias, spec.project),
        ],
    );
    add_term_window(spec.session, spec.alias, spec.project, 1, spec.cwd);
    if let Some(colour) = &spec.colour {
        // Read by the status-left so the session name shows in the host's colour.
        tmux::run_on(
            Socket::Default,
            &["set-option", "-t", spec.session, "@finter_colour", colour],
        );
    }
    tmux::run_on(Socket::Default, &["select-window", "-t", &first]);
    true
}

/// Wrapper `host/X` for a remote project.
pub(crate) fn create_remote_wrapper(host: &HostConfig, project: &str) -> bool {
    create_wrapper(&WrapperSpec {
        session: &wrapper_session(&host.name, project),
        alias: &host.ssh,
        project,
        colour: Some(format!("colour{}", host.color())),
        cwd: None,
    })
}

/// Wrapper `X` on the default server for a project of this machine.
pub(crate) fn create_local_wrapper(project: &str, path: Option<&str>) -> bool {
    create_wrapper(&WrapperSpec {
        session: project,
        alias: "",
        project,
        colour: None,
        cwd: path,
    })
}

/// Open a detached window in wrapper `session` attached to terminal slot `slot` of
/// `project`, tagged with `@finter_remote`. Returns the new pane id.
pub(crate) fn add_term_window(
    session: &str,
    alias: &str,
    project: &str,
    slot: u32,
    cwd: Option<&str>,
) -> Option<String> {
    let target = format!("{session}:");
    let cmd = attach_command(alias, project, Some(slot));
    let mut args = vec!["new-window", "-d", "-P", "-F", "#{pane_id}", "-t", &target];
    if let Some(cwd) = cwd {
        args.extend(["-c", cwd]);
    }
    args.push(&cmd);
    let pane = tmux::stdout_of(Socket::Default, &args)?;
    let tag = remote_tag(alias, &inner_session_name(project, Some(slot)));
    tmux::run_on(
        Socket::Default,
        &["set-option", "-p", "-t", &pane, "@finter_remote", &tag],
    );
    Some(pane)
}

/// Lowest terminal slot no wrapper window uses yet; slot 1 is window 2's.
pub(crate) fn free_term_slot(used: &[u32]) -> u32 {
    (1..).find(|n| !used.contains(n)).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_entry;
    use std::sync::Mutex;

    // Tests that touch XDG_RUNTIME_DIR must not run concurrently.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn sample_list() -> ListOutput {
        let folders = vec![
            ("recap".to_string(), "/p/recap".to_string()),
            ("finter".to_string(), "/p/finter".to_string()),
        ];
        let sessions = vec!["recap".to_string(), "recap~term".to_string()];
        let mut asking = test_entry(None, "recap", AgentState::Asking, 1_700_000_000);
        asking.msg = "needs you".to_string();
        build_list_output(&folders, &sessions, &[asking])
    }

    #[test]
    fn list_json_round_trip() {
        let list = sample_list();
        let json = serde_json::to_string(&list).unwrap();
        assert!(json.contains("\"v\":1"));
        assert!(json.contains("\"state\":\"asking\""));
        let back = parse_list_output(&json).unwrap();
        assert_eq!(back, list);
        assert!(back.projects[0].session_exists);
        assert!(!back.projects[1].session_exists);
    }

    #[test]
    fn parse_list_output_uses_last_json_line_and_tolerates_newer_schemas() {
        let json = serde_json::to_string(&sample_list()).unwrap();
        assert!(parse_list_output(&format!("Welcome {{brace}}\n{json}\n")).is_ok());
        // A trailing banner line that is not JSON-looking is fine; a decoy `{` line before is ignored.
        assert!(parse_list_output(&format!("{{decoy\n{json}\n")).is_ok());
        assert!(parse_list_output("nothing").is_err());
        let newer = json.replace("\"v\":1", "\"v\":2");
        assert_eq!(parse_list_output(&newer).unwrap().v, 2);
        let zero = json.replace("\"v\":1", "\"v\":0");
        assert!(parse_list_output(&zero).unwrap_err().contains("version"));
    }

    #[test]
    fn parse_list_output_skips_panes_with_unknown_state() {
        let raw = r#"{"v":1,"projects":[],"panes":[
            {"session":"a","window":"1","pane":"%1","state":"sleeping","since":1,"msg":""},
            {"session":"b","window":"1","pane":"%2","state":"done","since":2,"msg":"ok"}]}"#
            .replace('\n', "");
        let list = parse_list_output(&raw).unwrap();
        assert_eq!(list.panes.len(), 1);
        assert_eq!(list.panes[0].session, "b");
    }

    #[test]
    fn ssh_failures_get_short_reasons() {
        assert_eq!(ssh_failure_reason(Some(255)), "unreachable");
        assert_eq!(ssh_failure_reason(Some(2)), "finter too old?");
        assert_eq!(ssh_failure_reason(Some(127)), "finter not found");
        assert_eq!(ssh_failure_reason(Some(1)), "ssh exit 1");
    }

    #[test]
    fn entries_carry_host_name() {
        let entries = entries_from_list("mini", &sample_list());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].host.as_deref(), Some("mini"));
        assert_eq!(entries[0].local_session(), "mini/recap");
    }

    #[test]
    fn cache_staleness_uses_four_second_ttl() {
        let c = HostCache {
            fetched_at: 100,
            offline: false,
            data: None,
            error: None,
        };
        assert!(!is_stale(&c, 103));
        assert!(is_stale(&c, 104));
        assert!(!is_stale(&c, 50));
        let off = HostCache { offline: true, ..c };
        assert!(!is_stale(&off, 120));
        assert!(is_stale(&off, 130));
    }

    #[test]
    fn failed_fetch_keeps_last_data_and_marks_offline() {
        let good = merge_fetch(None, 10, Ok(sample_list()));
        assert!(!good.offline);
        let bad = merge_fetch(Some(good.clone()), 20, Err("timeout".into()));
        assert!(bad.offline);
        assert_eq!(bad.data, good.data);
        assert_eq!(bad.fetched_at, 20);
        assert_eq!(bad.error.as_deref(), Some("timeout"));
        let cold = merge_fetch(None, 30, Err("timeout".into()));
        assert!(cold.offline && cold.data.is_none());
        let recovered = merge_fetch(Some(bad), 40, Ok(sample_list()));
        assert!(!recovered.offline && recovered.error.is_none());
    }

    #[test]
    fn load_host_uses_cache_until_stale_then_fetches() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("finter-rt-{}", std::process::id()));
        // Safe: only this test touches XDG_RUNTIME_DIR.
        std::env::set_var("XDG_RUNTIME_DIR", &dir);
        let host = HostConfig {
            name: "mock".to_string(),
            ssh: "mock-alias".to_string(),
            color: None,
        };
        let calls = std::cell::Cell::new(0);
        let ok = |_: &HostConfig| {
            calls.set(calls.get() + 1);
            Ok(sample_list())
        };
        let fail = |_: &HostConfig| {
            calls.set(calls.get() + 1);
            Err("down".to_string())
        };
        let first = load_host_with(&host, 1000, true, &ok);
        assert!(!first.offline);
        let second = load_host_with(&host, 1002, true, &ok);
        assert_eq!(calls.get(), 1, "fresh cache must not refetch");
        assert_eq!(second.entries().len(), 1);
        let third = load_host_with(&host, 1010, true, &fail);
        assert_eq!(calls.get(), 2);
        assert!(third.offline);
        assert_eq!(third.entries().len(), 1, "stale data survives offline");
        // Offline caches are retried only after the longer TTL.
        load_host_with(&host, 1020, true, &ok);
        assert_eq!(calls.get(), 2);
        let back = load_host_with(&host, 1041, true, &ok);
        assert_eq!(calls.get(), 3);
        assert!(!back.offline);
        // Without refresh the cache is used even when stale, and ssh is never called.
        load_host_with(&host, 9999, false, &fail);
        assert_eq!(calls.get(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fallback_runtime_dir_is_private_and_per_user() {
        use std::os::unix::fs::PermissionsExt;
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let base = std::env::temp_dir().join(format!("finter-xdg-{}", std::process::id()));
        std::env::set_var("XDG_RUNTIME_DIR", &base);
        let dir = ensure_runtime_dir().unwrap();
        assert_eq!(dir, base.join("finter"));
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        // inner.conf is written once and left alone when it already matches.
        let conf = write_inner_conf().unwrap();
        assert_eq!(fs::read_to_string(&conf).unwrap(), INNER_CONF);
        assert_eq!(write_inner_conf().unwrap(), conf);
        let _ = fs::remove_dir_all(&base);
        std::env::remove_var("XDG_RUNTIME_DIR");
        let fallback = runtime_dir();
        assert!(fallback.starts_with(std::env::temp_dir()));
        assert!(fallback.to_string_lossy().contains("finter-"));
    }

    #[test]
    fn free_term_slot_fills_gaps() {
        assert_eq!(free_term_slot(&[]), 1);
        assert_eq!(free_term_slot(&[1]), 2);
        assert_eq!(free_term_slot(&[1, 3]), 2);
        assert_eq!(free_term_slot(&[2, 1, 3]), 4);
    }

    #[test]
    fn attach_helpers() {
        assert_eq!(inner_session_name("x", None), "x");
        assert_eq!(inner_session_name("x", Some(1)), "x~term");
        assert_eq!(inner_session_name("x", Some(3)), "x~term3");
        for slot in [1, 2, 10] {
            let name = inner_session_name("x", Some(slot));
            assert_eq!(crate::state::term_slot(&name), Some(("x", slot)));
        }
        let folders = vec![("a".to_string(), "/p/a".to_string())];
        assert_eq!(find_project_path(&folders, "a"), Some("/p/a"));
        assert_eq!(find_project_path(&folders, "b"), None);
        let cmd = attach_command("mac-mini", "x", Some(1));
        assert!(cmd.starts_with("sh -c '"));
        assert!(cmd.contains("ssh -t "));
        assert!(cmd.contains("finter attach "));
        assert!(cmd.contains("--term"));
        assert!(cmd.contains("[ $rc -eq 255 ] || exit $rc"));
        assert!(cmd.contains("finter: reconnecting..."));
        assert!(!attach_command("mac-mini", "x", None).contains("--term"));
        assert_eq!(remote_tag("mac-mini", "x~term"), "mac-mini:x~term");
        assert_eq!(
            split_remote_tag("mac-mini:x~term"),
            Some(("mac-mini", "x~term"))
        );
        assert_eq!(split_remote_tag("nocolon"), None);
    }

    #[test]
    fn empty_alias_tags_mean_this_machine() {
        assert_eq!(remote_tag("", "x"), ":x");
        assert_eq!(remote_tag("", "x~term2"), ":x~term2");
        assert_eq!(split_remote_tag(":x"), Some(("", "x")));
        assert_eq!(split_remote_tag(":x~term"), Some(("", "x~term")));
        // An empty session is never a tag, whatever the alias.
        assert_eq!(split_remote_tag(":"), None);
        assert_eq!(split_remote_tag("mini:"), None);
        assert_eq!(split_remote_tag(""), None);
    }

    #[test]
    fn local_attach_command_execs_finter_without_ssh() {
        let plain = attach_command("", "x", None);
        let term = attach_command("", "x", Some(1));
        assert!(plain.starts_with("sh -c '") && plain.contains("exec finter attach "));
        assert!(!plain.contains("--term"));
        assert!(term.contains("exec finter attach ") && term.contains(" --term"));
        assert!(attach_command("", "x", Some(3)).contains("--term=3"));
        for cmd in [&plain, &term] {
            assert!(!cmd.contains("ssh") && !cmd.contains("255"));
        }
    }

    #[test]
    fn local_attach_command_survives_hostile_project_names() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("finter-fakefinter-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("finter");
        fs::write(
            &bin,
            "#!/bin/sh\nfor a in \"$@\"; do printf '<%s>' \"$a\"; done\n",
        )
        .unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        for (name, term, expect) in [
            ("my proj", None, "<attach><my proj>"),
            ("it's", Some(1), "<attach><it's><--term>"),
            ("x", Some(2), "<attach><x><--term=2>"),
        ] {
            let cmd = attach_command("", name, term);
            let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap());
            let out = Command::new("sh")
                .args(["-c", &cmd])
                .env("PATH", path)
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), expect);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn attach_command_survives_hostile_project_names() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("finter-fakessh-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        // Fake ssh: like the real one it hands argument 3 to a remote shell.
        let ssh = dir.join("ssh");
        fs::write(&ssh, "#!/bin/sh\nsh -c \"printf '<%s>' $3\"\n").unwrap();
        fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
        for (name, term, expect) in [
            ("my proj", None, "<finter><attach><my proj>"),
            ("it's", Some(1), "<finter><attach><it's><--term>"),
            ("x", Some(2), "<finter><attach><x><--term=2>"),
        ] {
            let cmd = attach_command("alias", name, term);
            let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap());
            let out = Command::new("sh")
                .args(["-c", &cmd])
                .env("PATH", path)
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), expect);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_tmux_quotes_each_argument() {
        std::env::remove_var("FINTER_INNER_SOCKET");
        assert_eq!(
            remote_tmux(&["capture-pane", "-t", "a b"]),
            "tmux -L 'finter' 'capture-pane' '-t' 'a b'"
        );
    }

    #[test]
    fn inner_conf_has_required_settings() {
        for needle in [
            "status off",
            "prefix None",
            "mouse on",
            "set-clipboard on",
            "allow-passthrough on",
            "focus-events on",
            "extended-keys on",
            "extended-keys-format csi-u",
            "escape-time 1",
            "history-limit 10000",
            "mode-keys vi",
            "copy-selection-and-cancel",
        ] {
            assert!(INNER_CONF.contains(needle), "{needle} missing");
        }
    }

    #[test]
    fn forget_sessions_clears_exists_and_drops_panes() {
        let mut list = sample_list();
        forget_sessions(&mut list, &["recap", "recap~term"]);
        assert!(list.projects.iter().all(|p| !p.session_exists));
        assert!(list.panes.is_empty());
    }
}
