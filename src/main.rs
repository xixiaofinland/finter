extern crate home;
extern crate skim;

use skim::prelude::*;
use std::{env, error::Error, fs, path::Path, process};

#[derive(Debug, Clone)]
struct Project {
    folder: String,
    path: String,
    session_exists: bool,
}

impl SkimItem for Project {
    fn text(&self) -> Cow<str> {
        Cow::Borrowed(&self.folder)
    }
    // fn preview(&self, _context: PreviewContext) -> ItemPreview {
    //     ItemPreview::Text(self.folder)
    // }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let args = &args[1..];

    if let 0 = args.len() {
        run_finter()
    } else {
        save_paths(args)
    }
    // Some(("kill", _)) => {
    //     let defaults = confy::load::<Config>("tms", None)
    //         .into_report()
    //         .change_context(ConfigError::LoadError)
    //         .attach_printable("Failed to load the config file")
    //         .change_context(TmsError::ConfigError)?;
    //     let mut current_session =
    //         String::from_utf8(execute_tmux_command("tmux display-message -p '#S'").stdout)
    //             .expect("The tmux command static string should always be valid utf-9");
    //     current_session.retain(|x| x != '\'' && x != '\n');
    //
    //     let sessions =
    //         String::from_utf8(execute_tmux_command("tmux list-sessions -F #S").stdout)
    //             .expect("The tmux command static string should always be valid utf-9");
    //     let sessions: Vec<&str> = sessions.lines().collect();
    //
    //     let to_session = if defaults.default_session.is_some()
    //         && sessions.contains(&defaults.default_session.clone().unwrap().as_str())
    //         && current_session != defaults.default_session.clone().unwrap()
    //     {
    //         defaults.default_session.unwrap()
    //     } else if current_session != sessions[0] {
    //         sessions[0].to_string()
    //     } else {
    //         sessions.get(1).unwrap_or_else(|| &sessions[0]).to_string()
    //     };
    //     execute_tmux_command(&format!("tmux switch-client -t {to_session}"));
    //     execute_tmux_command(&format!("tmux kill-session -t {current_session}"));
    //     Ok(SubCommandGiven::Yes)
    // }
}

fn run_finter() -> Result<(), Box<dyn Error>> {
    let paths = load_project_paths()?;
    let folders = get_folders(paths)?;
    let sessions = get_sessions()?;

    let projects = build_projects(folders, sessions)?;

    let selected = select_in_skim(projects.clone())?;
    let selected_project = get_match(selected, projects)?;

    let session_name = selected_project.folder;
    let path = selected_project.path;

    if !selected_project.session_exists {
        let params = &format!("new-session -ds {session_name} -c {path}");
        run_tmux_with_params(params);
    }

    let result = run_tmux_with_params(&format!("switch-client -t {session_name}"));
    if !result.status.success() {
        run_tmux_with_params(&format!("attach -t {session_name}"));
    }
    Ok(())
}

fn get_match(folder: String, projects: Vec<Project>) -> Result<Project, Box<dyn Error>> {
    for p in projects.into_iter() {
        if folder == p.folder {
            return Ok(p);
        }
    }
    return Err("selected value not found in projects?".into());
}

fn build_projects(
    folders: Vec<(String, String)>,
    sessions: Vec<String>,
) -> Result<Vec<Project>, Box<dyn Error>> {
    let mut projects = Vec::new();

    sessions.iter().for_each(|s| {
        projects.push(Project {
            folder: s.to_string(),
            path: get_home_dir(),
            session_exists: true,
        })
    });

    folders
        .iter()
        .filter(|f| !sessions.contains(&f.0))
        .for_each(|(folder, path)| {
            projects.push(Project {
                folder: folder.clone(),
                path: path.clone(),
                session_exists: false,
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
                    .replace(".", "_") // tmux session name doesn't accept "." or ":"
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
        String::from_utf8(run_tmux_with_params("list-sessions -F #S").stdout)?
            .lines()
            .map(|s| s.to_string())
            .collect();
    Ok(sessions)
}

fn select_in_skim(projects: Vec<Project>) -> Result<String, Box<dyn Error>> {
    let options = SkimOptionsBuilder::default()
        .height(Some("50%"))
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

    let skim_result;
    match skim_output {
        Some(value) => skim_result = value,
        None => return Err("Skim internal error.".into()),
    }
    if skim_result.is_abort {
        return Err("No selection made.".into());
    }

    Ok(skim_result
        .selected_items
        .get(0)
        .ok_or("no item in Skim is selected")?
        .output()
        .to_string())
}

fn save_paths(args: &[String]) -> Result<(), Box<dyn Error>> {
    let paths = args.join("\n");
    fs::write(get_config_file_path(), paths).expect("Unable to write file");
    Ok(())
}

fn run_tmux_with_params(command: &str) -> process::Output {
    let args: Vec<&str> = command.split(' ').collect();
    process::Command::new("tmux")
        .args(args)
        .stdin(process::Stdio::inherit())
        .output()
        .unwrap_or_else(|_| panic!("Failed to run tmux with params: `{command}`"))
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

    #[test]
    fn it_saves_paths() {
        let paths = ["whatever path".to_owned()];
        let x = save_paths(&paths).is_err();
        assert_eq!(x, false);
    }

    #[test]
    #[should_panic]
    fn greater_than_100() {
        panic!("hehe");
    }
}
