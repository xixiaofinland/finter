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

/// Name of the inner session for project `name`.
pub(crate) fn inner_session_name(name: &str, term: bool) -> String {
    if term {
        format!("{name}{TERM_SUFFIX}")
    } else {
        name.to_string()
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
pub(crate) fn exec_attach(name: &str, term: bool, path: &str) -> Result<(), Box<dyn Error>> {
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

// ── local wrapper session

/// `@finter_remote` value for a wrapper pane: `<ssh alias>:<inner session>`.
pub(crate) fn remote_tag(alias: &str, inner_session: &str) -> String {
    format!("{alias}:{inner_session}")
}

pub(crate) fn split_remote_tag(tag: &str) -> Option<(&str, &str)> {
    tag.rsplit_once(':')
        .filter(|(a, s)| !a.is_empty() && !s.is_empty())
}

/// Window command: retry ssh only when the connection drops (exit 255).
/// Any other exit (clean detach, killed inner session, remote error) ends the loop.
/// Wrapped in `sh -c` because the tmux default-shell may be fish.
pub(crate) fn attach_command(alias: &str, project: &str, term: bool) -> String {
    let suffix = if term { " --term" } else { "" };
    // ssh joins its argv with spaces for the remote shell, so send one quoted string.
    let remote = format!("finter attach {}{suffix}", shell_quote(project));
    let script = format!(
        "while :; do ssh -t {} {}; rc=$?; [ $rc -eq 255 ] || exit $rc; echo 'finter: reconnecting...'; sleep 2; done",
        shell_quote(alias),
        shell_quote(&remote)
    );
    format!("sh -c {}", shell_quote(&script))
}

/// Create local session `host/project` with window 1 = claude, window 2 = terminal,
/// both attached to inner sessions over ssh. Returns false when tmux refused.
pub(crate) fn create_wrapper(host: &HostConfig, project: &str) -> bool {
    let session = wrapper_session(&host.name, project);
    let first = tmux::stdout_of(
        Socket::Default,
        &[
            "new-session",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-s",
            &session,
            &attach_command(&host.ssh, project, false),
        ],
    );
    let Some(first) = first else { return false };
    let target = format!("{session}:");
    let second = tmux::stdout_of(
        Socket::Default,
        &[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &target,
            &attach_command(&host.ssh, project, true),
        ],
    );
    let tag1 = remote_tag(&host.ssh, project);
    tmux::run_on(
        Socket::Default,
        &["set-option", "-p", "-t", &first, "@finter_remote", &tag1],
    );
    if let Some(second) = second {
        let tag2 = remote_tag(&host.ssh, &inner_session_name(project, true));
        tmux::run_on(
            Socket::Default,
            &["set-option", "-p", "-t", &second, "@finter_remote", &tag2],
        );
    }
    tmux::run_on(Socket::Default, &["select-window", "-t", &first]);
    true
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
    fn attach_helpers() {
        assert_eq!(inner_session_name("x", false), "x");
        assert_eq!(inner_session_name("x", true), "x~term");
        let folders = vec![("a".to_string(), "/p/a".to_string())];
        assert_eq!(find_project_path(&folders, "a"), Some("/p/a"));
        assert_eq!(find_project_path(&folders, "b"), None);
        let cmd = attach_command("mac-mini", "x", true);
        assert!(cmd.starts_with("sh -c '"));
        assert!(cmd.contains("ssh -t "));
        assert!(cmd.contains("finter attach "));
        assert!(cmd.contains("--term"));
        assert!(cmd.contains("[ $rc -eq 255 ] || exit $rc"));
        assert!(cmd.contains("finter: reconnecting..."));
        assert!(!attach_command("mac-mini", "x", false).contains("--term"));
        assert_eq!(remote_tag("mac-mini", "x~term"), "mac-mini:x~term");
        assert_eq!(
            split_remote_tag("mac-mini:x~term"),
            Some(("mac-mini", "x~term"))
        );
        assert_eq!(split_remote_tag("nocolon"), None);
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
            ("my proj", false, "<finter><attach><my proj>"),
            ("it's", true, "<finter><attach><it's><--term>"),
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
