use skim::prelude::*;
use std::{error::Error, fs, path::Path, process};

pub type MyResult<T> = Result<T, Box<dyn Error>>;

#[derive(Debug, Clone)]
pub struct Project {
    folder: String,
    path: String,
    session_exists: bool,
}

impl SkimItem for Project {
    fn text(&self) -> Cow<str> {
        Cow::Borrowed(&self.folder)
    }

    fn display<'a>(&'a self, _context: DisplayContext<'a>) -> AnsiString<'a> {
        if self.session_exists {
            return AnsiString::from(format!("*{}", self.folder));
        }
        return AnsiString::from(format!(" {}", self.folder));
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
