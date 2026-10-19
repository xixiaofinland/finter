// TODO(finter-0.4): removed once every module is wired into the CLI.
#![allow(dead_code)]

mod config;
mod hook;
mod picker;
mod remote;
mod state;
mod status;
mod tmux;

use config::{get_folders, load_config, load_project_paths};
use picker::{build_projects, get_match, select_in_skim};
use std::error::Error;
use tmux::{get_current_session, get_sessions, run_tmux_with_args, Socket};

pub use config::save_paths;
pub use hook::run_hook;
pub use status::{run_copy_mode, run_next, run_preview, run_seen, run_status};
pub type MyResult<T> = Result<T, Box<dyn Error>>;

pub fn run_finter() -> Result<(), Box<dyn Error>> {
    let config = load_config()?;
    let paths = load_project_paths(&config)?;
    let folders = get_folders(paths)?;
    let sessions = get_sessions()?;
    let current_session = get_current_session();

    let projects = build_projects(folders, sessions, current_session.as_deref())?;

    let selected = select_in_skim(projects.clone())?;
    let selected_project = get_match(selected, projects)?;

    let session_name = selected_project.folder;
    let path = selected_project.path;

    if !selected_project.session_exists {
        run_tmux_with_args(&["new-session", "-ds", &session_name, "-c", &path]);
        run_tmux_with_args(&[
            "new-window",
            "-t",
            &format!("{session_name}:2"),
            "-c",
            &path,
        ]);
        run_tmux_with_args(&["select-window", "-t", &format!("{session_name}:1")]);
    }

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

/// `finter attach`: runs on the remote machine and execs into the inner tmux server.
pub fn run_attach(name: &str, term: bool) -> Result<(), Box<dyn Error>> {
    let config = load_config()?;
    let folders = get_folders(load_project_paths(&config)?)?;
    let path = remote::find_project_path(&folders, name)
        .ok_or_else(|| format!("Err: project `{name}` not found in configured roots"))?;
    remote::exec_attach(name, term, path)
}
