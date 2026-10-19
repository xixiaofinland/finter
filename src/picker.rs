use crate::config::get_home_dir;
use skim::prelude::*;
use std::{borrow::Cow, error::Error};

#[derive(Debug, Clone)]
pub struct Project {
    pub(crate) folder: String,
    pub(crate) path: String,
    pub(crate) session_exists: bool,
    pub(crate) is_ssh_session: bool,
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
    ssh_session_name: Option<&str>,
    current_session: Option<&str>,
) -> Result<Vec<Project>, Box<dyn Error>> {
    let mut projects = Vec::new();

    if let Some(ssh_name) = ssh_session_name {
        let ssh_session_exists = sessions.iter().any(|s| s == ssh_name);

        sessions.iter().for_each(|s| {
            if s != ssh_name {
                projects.push(Project {
                    folder: s.to_string(),
                    path: get_home_dir(),
                    session_exists: true,
                    is_ssh_session: false,
                })
            }
        });

        projects.push(Project {
            folder: ssh_name.to_string(),
            path: get_home_dir(),
            session_exists: ssh_session_exists,
            is_ssh_session: true,
        });

        folders
            .iter()
            .filter(|f| f.0 != ssh_name)
            .filter(|f| !sessions.contains(&f.0))
            .for_each(|(folder, path)| {
                projects.push(Project {
                    folder: folder.clone(),
                    path: path.clone(),
                    session_exists: false,
                    is_ssh_session: false,
                })
            });
    } else {
        sessions.iter().for_each(|s| {
            projects.push(Project {
                folder: s.to_string(),
                path: get_home_dir(),
                session_exists: true,
                is_ssh_session: false,
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
                    is_ssh_session: false,
                })
            });
    }

    if let Some(current) = current_session {
        if let Some(idx) = projects.iter().position(|p| p.folder == current) {
            let current_project = projects.remove(idx);
            projects.insert(0, current_project);
        }
    }

    Ok(projects)
}

/// Load paths from config. Supports both old `roots` (Vec<String>) and new
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
    fn build_projects_includes_ssh_when_configured() {
        let projects = build_projects(vec![], vec![], Some("ssh_mac_mini"), None)
            .expect("build should succeed");

        assert_eq!(count_by_name(&projects, "ssh_mac_mini"), 1);
        let ssh = projects
            .iter()
            .find(|p| p.folder == "ssh_mac_mini")
            .expect("ssh item should exist");
        assert!(!ssh.session_exists);
        assert!(ssh.is_ssh_session);
    }

    #[test]
    fn build_projects_no_ssh_when_not_configured() {
        let projects = build_projects(vec![], vec![], None, None).expect("build should succeed");

        assert!(!projects.iter().any(|p| p.folder == "ssh_mac_mini"));
        assert!(!projects.iter().any(|p| p.is_ssh_session));
    }

    #[test]
    fn build_projects_marks_existing_ssh_session() {
        let projects = build_projects(
            vec![],
            vec!["ssh_mac_mini".to_string(), "work".to_string()],
            Some("ssh_mac_mini"),
            None,
        )
        .expect("build should succeed");

        let ssh = projects
            .iter()
            .find(|p| p.folder == "ssh_mac_mini")
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
            ("ssh_mac_mini".to_string(), "/tmp/ssh_mac_mini".to_string()),
            ("repo1".to_string(), "/tmp/repo1".to_string()),
        ];

        let projects = build_projects(folders, vec![], Some("ssh_mac_mini"), None)
            .expect("build should succeed");

        assert_eq!(count_by_name(&projects, "ssh_mac_mini"), 1);
        assert_eq!(count_by_name(&projects, "repo1"), 1);
    }

    #[test]
    fn build_projects_prefers_existing_session_over_folder_duplicate() {
        let folders = vec![("repo1".to_string(), "/tmp/repo1".to_string())];
        let sessions = vec!["repo1".to_string()];

        let projects = build_projects(folders, sessions, Some("ssh_mac_mini"), None)
            .expect("build should succeed");

        assert_eq!(count_by_name(&projects, "repo1"), 1);
        let repo = projects
            .iter()
            .find(|p| p.folder == "repo1")
            .expect("repo1 should exist");
        assert!(repo.session_exists);
        assert!(!repo.is_ssh_session);
    }

    #[test]
    fn build_projects_prioritizes_current_active_session() {
        let folders = vec![("repo1".to_string(), "/tmp/repo1".to_string())];
        let sessions = vec!["work".to_string(), "ssh_mac_mini".to_string()];

        let projects = build_projects(folders, sessions, Some("ssh_mac_mini"), Some("work"))
            .expect("build should succeed");

        assert_eq!(projects[0].folder, "work");
    }

    #[test]
    fn build_projects_prioritizes_ssh_when_it_is_current_session() {
        let projects = build_projects(
            vec![],
            vec!["ssh_mac_mini".to_string(), "work".to_string()],
            Some("ssh_mac_mini"),
            Some("ssh_mac_mini"),
        )
        .expect("build should succeed");

        assert_eq!(projects[0].folder, "ssh_mac_mini");
    }
}
