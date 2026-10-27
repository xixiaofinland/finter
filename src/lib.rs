mod config;
mod hook;
mod picker;
mod remote;
mod state;
mod status;
mod tmux;
mod watch;

use config::{get_folders, load_config, load_project_paths};
use picker::{build_rows, get_match, offline_header, select_in_skim, Target};
use std::error::Error;
use tmux::{get_current_session, get_sessions, run_tmux_with_args, Socket};

pub use config::save_paths;
pub use hook::run_hook;
pub use status::{run_kill, run_next, run_preview, run_seen, run_status};
pub use watch::run_watch;
pub type MyResult<T> = Result<T, Box<dyn Error>>;

pub fn run_finter() -> Result<(), Box<dyn Error>> {
    let config = load_config()?;
    let paths = load_project_paths(&config)?;
    let folders = get_folders(paths)?;
    let sessions = get_sessions()?;
    let current_session = get_current_session();
    let now = status::now_secs();

    let inner_sessions = tmux::get_sessions_on(Socket::Inner);
    let local_agents = tmux::list_local_agent_entries();
    let hosts = remote::load_hosts(&config, now, true);
    let rows = build_rows(
        &folders,
        &sessions,
        &inner_sessions,
        &local_agents,
        &hosts,
        current_session.as_deref(),
        now,
    );

    let header = offline_header(&hosts);
    let selected = select_in_skim(rows.clone(), header.as_deref())?;
    let row = get_match(&selected, rows)?;

    let session_name = match row.target {
        Target::Local { session, path } => {
            // An existing default-server session (legacy plain session or a wrapper) is
            // used as is; a project folder gets a wrapper around its inner sessions.
            if !tmux::has_session(&session) && !remote::create_local_wrapper(&session, Some(&path))
            {
                return Err(format!("Err: could not create session `{session}`").into());
            }
            session
        }
        Target::Remote { host, project } => {
            let wrapper = state::wrapper_session(&host.name, &project);
            if !tmux::has_session(&wrapper) && !remote::create_remote_wrapper(&host, &project) {
                return Err(format!("Err: could not create session `{wrapper}`").into());
            }
            wrapper
        }
    };

    let result = run_tmux_with_args(&["switch-client", "-t", &session_name]);
    if !result.status.success() {
        run_tmux_with_args(&["attach", "-t", &session_name]);
    }
    Ok(())
}

/// `finter list`: this machine's projects and agent panes on the inner server.
pub fn run_list(json: bool) -> Result<(), Box<dyn Error>> {
    let config = load_config()?;
    let folders = load_project_paths(&config)
        .and_then(get_folders)
        .unwrap_or_default();
    let sessions = tmux::get_sessions_on(Socket::Inner);
    let panes = tmux::list_agent_entries(Socket::Inner, None);
    let list = remote::build_list_output(&folders, &sessions, &panes);
    if json {
        println!("{}", serde_json::to_string(&list)?);
    } else {
        for p in &list.projects {
            let mark = if p.session_exists { "*" } else { " " };
            println!("{mark}{}\t{}", p.name, p.path);
        }
        for p in &list.panes {
            println!("{}\t{}\t{}", p.session, p.state.as_str(), p.msg);
        }
    }
    Ok(())
}

/// `finter attach`: runs on the machine owning the project and execs into its inner tmux server.
pub fn run_attach(name: &str) -> Result<(), Box<dyn Error>> {
    let config = load_config()?;
    let folders = get_folders(load_project_paths(&config)?)?;
    let path = remote::find_project_path(&folders, name)
        .ok_or_else(|| format!("Err: project `{name}` not found in configured roots"))?;
    remote::exec_attach(name, path)
}
