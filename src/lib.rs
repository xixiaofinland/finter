use skim::prelude::*;
use std::{error::Error, fs, path::Path, process};

pub type MyResult<T> = Result<T, Box<dyn Error>>;

const SSH_SESSION_NAME: &str = "ssh_mac_mini";
const SSH_PRIMARY_CMD: &str = "ssh xixiao@192.168.1.200";
const SSH_TAILSCALE_TARGET_ENV: &str = "FINTER_SSH_TAILSCALE_TARGET";

#[derive(Debug, Clone)]
pub struct Project {
    folder: String,
    path: String,
    session_exists: bool,
    is_ssh_session: bool,
}

impl SkimItem for Project {
    fn text(&self) -> Cow<str> {
        Cow::Borrowed(&self.folder)
    }

    fn display<'a>(&'a self, _context: DisplayContext<'a>) -> AnsiString<'a> {
        if self.session_exists {
            return AnsiString::from(format!("*{}", self.folder));
        }
        AnsiString::from(format!(" {}", self.folder))
    }
}

pub fn save_paths(args: &[String]) -> Result<(), Box<dyn Error>> {
    let paths = args.join("\n");
    fs::write(get_config_file_path(), paths).expect("Unable to write file");
    Ok(())
}

pub fn run_finter() -> Result<(), Box<dyn Error>> {
    let paths = load_project_paths()?;
    let folders = get_folders(paths)?;
    let sessions = get_sessions()?;

    let projects = build_projects(folders, sessions)?;

    let selected = select_in_skim(projects.clone())?;
    let selected_project = get_match(selected, projects)?;

    let session_name = selected_project.folder;
    let path = selected_project.path;

    if !selected_project.session_exists {
        run_tmux_with_args(&["new-session", "-ds", &session_name, "-c", &path]);

        if selected_project.is_ssh_session {
            let ssh_connect_cmd = build_ssh_connect_cmd();
            run_tmux_with_args(&[
                "send-keys",
                "-t",
                &format!("{session_name}:1"),
                &ssh_connect_cmd,
                "C-m",
            ]);
        } else {
            run_tmux_with_args(&[
                "new-window",
                "-t",
                &format!("{session_name}:2"),
                "-c",
                &path,
            ]);
            run_tmux_with_args(&["select-window", "-t", &format!("{session_name}:1")]);
        }
    }

    let result = run_tmux_with_args(&["switch-client", "-t", &session_name]);
    if !result.status.success() {
        run_tmux_with_args(&["attach", "-t", &session_name]);
    }
    Ok(())
}

fn get_match(folder: String, projects: Vec<Project>) -> Result<Project, Box<dyn Error>> {
    for p in projects.into_iter() {
        if folder == p.folder {
            return Ok(p);
        }
    }
    Err("selected value not found in projects?".into())
}

fn build_projects(
    folders: Vec<(String, String)>,
    sessions: Vec<String>,
) -> Result<Vec<Project>, Box<dyn Error>> {
    let mut projects = Vec::new();
    let ssh_session_exists = sessions.iter().any(|s| s == SSH_SESSION_NAME);

    sessions.iter().for_each(|s| {
        if s != SSH_SESSION_NAME {
            projects.push(Project {
                folder: s.to_string(),
                path: get_home_dir(),
                session_exists: true,
                is_ssh_session: false,
            })
        }
    });

    projects.insert(
        0,
        Project {
            folder: SSH_SESSION_NAME.to_string(),
            path: get_home_dir(),
            session_exists: ssh_session_exists,
            is_ssh_session: true,
        },
    );

    folders
        .iter()
        .filter(|f| f.0 != SSH_SESSION_NAME)
        .filter(|f| !sessions.contains(&f.0))
        .for_each(|(folder, path)| {
            projects.push(Project {
                folder: folder.clone(),
                path: path.clone(),
                session_exists: false,
                is_ssh_session: false,
            })
        });

    Ok(projects)
}

fn load_project_paths() -> Result<Vec<String>, Box<dyn Error>> {
    let config_file_location = get_config_file_path();
    let file_content =
        fs::read_to_string(config_file_location).expect("Should be able to read config file.");
    let paths: Vec<_> = file_content.lines().collect();

    let folders: Vec<String> = paths
        .iter()
        .filter(|p| Path::new(p).is_dir())
        .map(|&s| s.into())
        .collect();

    match folders.len() {
        0 => Err("Err: no valid path is configured.".into()),
        _ => Ok(folders),
    }
}

fn get_config_file_path() -> String {
    let mut config_file_location = get_home_dir();
    config_file_location.push_str("/.finter");
    config_file_location
}

fn get_folders(paths: Vec<String>) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut folders = Vec::new();

    for path in paths {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                let folder = p
                    .file_name()
                    .ok_or("expect a file_name()")?
                    .to_str()
                    .ok_or("file_name can't turn to a string")?
                    .replace('.', "_") // tmux session name doesn't accept "." or ":"
                    .replace(":", "_")
                    .to_string();

                let full_path = p
                    .clone()
                    .into_os_string()
                    .into_string()
                    .expect("path is not parsed correctly.");
                folders.push((folder, full_path));
            }
        }
    }
    match folders.len() {
        0 => Err("Err: no folder is found in the configured paths.".into()),
        _ => Ok(folders),
    }
}

fn get_sessions() -> Result<Vec<String>, Box<dyn Error>> {
    let sessions: Vec<String> =
        String::from_utf8(run_tmux_with_args(&["list-sessions", "-F", "#S"]).stdout)?
            .lines()
            .map(|s| s.to_string())
            .collect();
    Ok(sessions)
}

fn select_in_skim(projects: Vec<Project>) -> Result<String, Box<dyn Error>> {
    let options = SkimOptionsBuilder::default()
        .height(Some("100%"))
        .multi(false)
        .color(Some("dark"))
        .preview(None)
        .build()?;

    let (tx_item, rx_item): (SkimItemSender, SkimItemReceiver) = unbounded();
    for p in projects.into_iter() {
        let _ = tx_item.send(Arc::new(p));
    }
    drop(tx_item);

    let skim_output = Skim::run_with(&options, Some(rx_item));

    let skim_result = match skim_output {
        Some(value) => value,
        None => return Err("Skim internal error.".into()),
    };

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

fn run_tmux_with_args(args: &[&str]) -> process::Output {
    process::Command::new("tmux")
        .args(args)
        .stdin(process::Stdio::inherit())
        .output()
        .unwrap_or_else(|_| panic!("Failed to run tmux with args: `{}`", args.join(" ")))
}

fn build_ssh_connect_cmd() -> String {
    build_ssh_connect_cmd_with_target(std::env::var(SSH_TAILSCALE_TARGET_ENV).ok().as_deref())
}

fn build_ssh_connect_cmd_with_target(tailscale_target: Option<&str>) -> String {
    match tailscale_target {
        Some(target) if !target.trim().is_empty() => {
            format!("if nc -z -w2 192.168.1.200 22; then {SSH_PRIMARY_CMD}; else ssh {target}; fi")
        }
        _ => SSH_PRIMARY_CMD.to_string(),
    }
}

fn get_home_dir() -> String {
    let home_dir: String = match home::home_dir() {
        Some(path) => path
            .into_os_string()
            .into_string()
            .expect("should be able to retrieve home_dir value"),
        None => panic!("can not get home_dir value"),
    };
    home_dir
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_by_name(projects: &[Project], name: &str) -> usize {
        projects.iter().filter(|p| p.folder == name).count()
    }

    #[test]
    fn build_projects_always_includes_ssh_item() {
        let projects = build_projects(vec![], vec![]).expect("build should succeed");

        assert_eq!(projects[0].folder, SSH_SESSION_NAME);
        assert_eq!(count_by_name(&projects, SSH_SESSION_NAME), 1);
        assert!(!projects[0].session_exists);
        assert!(projects[0].is_ssh_session);
    }

    #[test]
    fn build_projects_marks_existing_ssh_session() {
        let projects = build_projects(
            vec![],
            vec![SSH_SESSION_NAME.to_string(), "work".to_string()],
        )
        .expect("build should succeed");

        let ssh = projects
            .iter()
            .find(|p| p.folder == SSH_SESSION_NAME)
            .expect("ssh item should exist");
        assert!(ssh.session_exists);
        assert!(ssh.is_ssh_session);

        let work = projects
            .iter()
            .find(|p| p.folder == "work")
            .expect("work session should exist");
        assert!(work.session_exists);
        assert!(!work.is_ssh_session);
    }

    #[test]
    fn build_projects_does_not_duplicate_ssh_from_folders() {
        let folders = vec![
            (
                SSH_SESSION_NAME.to_string(),
                "/tmp/ssh_mac_mini".to_string(),
            ),
            ("repo1".to_string(), "/tmp/repo1".to_string()),
        ];

        let projects = build_projects(folders, vec![]).expect("build should succeed");

        assert_eq!(count_by_name(&projects, SSH_SESSION_NAME), 1);
        assert_eq!(count_by_name(&projects, "repo1"), 1);
    }

    #[test]
    fn build_projects_prefers_existing_session_over_folder_duplicate() {
        let folders = vec![("repo1".to_string(), "/tmp/repo1".to_string())];
        let sessions = vec!["repo1".to_string()];

        let projects = build_projects(folders, sessions).expect("build should succeed");

        assert_eq!(count_by_name(&projects, "repo1"), 1);
        let repo = projects
            .iter()
            .find(|p| p.folder == "repo1")
            .expect("repo1 should exist");
        assert!(repo.session_exists);
        assert!(!repo.is_ssh_session);
    }

    #[test]
    fn build_ssh_connect_cmd_without_tailscale_target_uses_primary() {
        assert_eq!(build_ssh_connect_cmd_with_target(None), SSH_PRIMARY_CMD);
        assert_eq!(
            build_ssh_connect_cmd_with_target(Some("   ")),
            SSH_PRIMARY_CMD
        );
    }

    #[test]
    fn build_ssh_connect_cmd_with_tailscale_target_uses_fallback_flow() {
        let command = build_ssh_connect_cmd_with_target(Some("xixiao@macmini.example.ts.net"));

        assert!(command.contains("nc -z -w2 192.168.1.200 22"));
        assert!(command.contains(SSH_PRIMARY_CMD));
        assert!(command.contains("ssh xixiao@macmini.example.ts.net"));
    }
}
