use crate::config::get_home_dir;
use skim::prelude::*;
use std::{borrow::Cow, error::Error};

#[derive(Debug, Clone)]
pub struct Project {
    pub(crate) folder: String,
    pub(crate) path: String,
    pub(crate) session_exists: bool,
}

impl SkimItem for Project {
    fn text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.folder)
    }

    fn display<'a>(&'a self, _context: DisplayContext<'a>) -> AnsiString<'a> {
        if self.session_exists {
            return AnsiString::from(format!("*{}", self.folder));
        }
        AnsiString::from(format!(" {}", self.folder))
    }
}

pub(crate) fn get_match(folder: String, projects: Vec<Project>) -> Result<Project, Box<dyn Error>> {
    for p in projects.into_iter() {
        if folder == p.folder {
            return Ok(p);
        }
    }
    Err("selected value not found in projects?".into())
}

pub(crate) fn build_projects(
    folders: Vec<(String, String)>,
    sessions: Vec<String>,
    current_session: Option<&str>,
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

    if let Some(current) = current_session {
        if let Some(idx) = projects.iter().position(|p| p.folder == current) {
            let current_project = projects.remove(idx);
            projects.insert(0, current_project);
        }
    }

    Ok(projects)
}

pub(crate) fn select_in_skim(projects: Vec<Project>) -> Result<String, Box<dyn Error>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn count_by_name(projects: &[Project], name: &str) -> usize {
        projects.iter().filter(|p| p.folder == name).count()
    }

    #[test]
    fn build_projects_prefers_existing_session_over_folder_duplicate() {
        let folders = vec![("repo1".to_string(), "/tmp/repo1".to_string())];
        let sessions = vec!["repo1".to_string()];

        let projects = build_projects(folders, sessions, None).expect("build should succeed");

        assert_eq!(count_by_name(&projects, "repo1"), 1);
        assert!(projects[0].session_exists);
    }

    #[test]
    fn build_projects_prioritizes_current_active_session() {
        let folders = vec![("repo1".to_string(), "/tmp/repo1".to_string())];
        let sessions = vec!["other".to_string(), "work".to_string()];

        let projects =
            build_projects(folders, sessions, Some("work")).expect("build should succeed");

        assert_eq!(projects[0].folder, "work");
    }
}
