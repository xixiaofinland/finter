// TODO(finter-0.4): removed once every module is wired into the CLI.
#![allow(dead_code)]

mod config;
mod hook;
mod picker;
mod state;
mod tmux;

use config::{get_folders, load_config, load_project_paths};
use picker::{build_projects, get_match, select_in_skim};
use std::error::Error;
use tmux::{get_current_session, get_sessions, run_tmux_with_args};

pub use config::save_paths;
pub use hook::run_hook;
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
