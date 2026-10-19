mod config;
mod picker;
mod tmux;

use config::{get_folders, load_config, load_project_paths, SshConfig};
use picker::{build_projects, get_match, select_in_skim};
use std::error::Error;
use tmux::{get_current_session, get_sessions, run_tmux_with_args};

pub use config::save_paths;
pub type MyResult<T> = Result<T, Box<dyn std::error::Error>>;

pub fn run_finter() -> Result<(), Box<dyn Error>> {
    let config = load_config()?;
    let paths = load_project_paths(&config)?;
    let folders = get_folders(paths)?;
    let sessions = get_sessions()?;
    let current_session = get_current_session();

    let projects = build_projects(
        folders,
        sessions,
        config.ssh.as_ref().map(|s| s.session_name.as_str()),
        current_session.as_deref(),
    )?;

    let selected = select_in_skim(projects.clone())?;
    let selected_project = get_match(selected, projects)?;

    let session_name = selected_project.folder;
    let path = selected_project.path;

    if !selected_project.session_exists {
        run_tmux_with_args(&["new-session", "-ds", &session_name, "-c", &path]);

        if selected_project.is_ssh_session {
            if let Some(ssh) = &config.ssh {
                let ssh_connect_cmd = build_ssh_connect_cmd(ssh);
                run_tmux_with_args(&[
                    "rename-window",
                    "-t",
                    &format!("{session_name}:1"),
                    "Mac-mini",
                ]);
                run_tmux_with_args(&[
                    "send-keys",
                    "-t",
                    &format!("{session_name}:1"),
                    &ssh_connect_cmd,
                    "C-m",
                ]);
            }
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

fn build_ssh_connect_cmd(ssh: &SshConfig) -> String {
    let primary_cmd = build_ssh_target_cmd(&ssh.primary, ssh.remote_session_name.as_deref());
    match ssh.tailscale.as_deref() {
        Some(target) if !target.trim().is_empty() => {
            let tailscale_cmd = build_ssh_target_cmd(target, ssh.remote_session_name.as_deref());
            format!("if nc -z -w2 192.168.1.113 22; then {primary_cmd}; else {tailscale_cmd}; fi")
        }
        _ => primary_cmd,
    }
}

fn build_ssh_target_cmd(target: &str, remote_session_name: Option<&str>) -> String {
    match remote_session_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        Some(name) => {
            let remote_tmux_cmd = format!("tmux new-session -A -s {}", shell_quote(name));
            format!("ssh -t {} {}", target, shell_quote(&remote_tmux_cmd))
        }
        None => format!("ssh {}", target),
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ssh() -> SshConfig {
        SshConfig {
            session_name: "ssh_mac_mini".to_string(),
            primary: "xixiao@192.168.1.113".to_string(),
            tailscale: None,
            remote_session_name: Some("mac_mini".to_string()),
        }
    }

    #[test]
    fn build_ssh_connect_cmd_without_tailscale_target_uses_primary() {
        let ssh = test_ssh();
        assert_eq!(
            build_ssh_connect_cmd(&ssh),
            "ssh -t xixiao@192.168.1.113 'tmux new-session -A -s '\"'\"'mac_mini'\"'\"''"
        );

        let ssh_blank = SshConfig {
            tailscale: Some("   ".to_string()),
            ..test_ssh()
        };
        assert_eq!(
            build_ssh_connect_cmd(&ssh_blank),
            "ssh -t xixiao@192.168.1.113 'tmux new-session -A -s '\"'\"'mac_mini'\"'\"''"
        );
    }

    #[test]
    fn build_ssh_connect_cmd_with_tailscale_target_uses_fallback_flow() {
        let ssh = SshConfig {
            tailscale: Some("xixiao@macmini.example.ts.net".to_string()),
            ..test_ssh()
        };
        let command = build_ssh_connect_cmd(&ssh);

        assert!(command.contains("nc -z -w2 192.168.1.113 22"));
        assert!(command.contains("ssh -t xixiao@192.168.1.113"));
        assert!(command.contains("ssh -t xixiao@macmini.example.ts.net"));
        assert!(command.contains("tmux new-session -A -s "));
        assert!(command.contains("mac_mini"));
    }

    #[test]
    fn build_ssh_connect_cmd_without_remote_session_name_uses_plain_ssh() {
        let ssh = SshConfig {
            remote_session_name: None,
            ..test_ssh()
        };

        assert_eq!(build_ssh_connect_cmd(&ssh), "ssh xixiao@192.168.1.113");

        let ssh_blank = SshConfig {
            remote_session_name: Some("   ".to_string()),
            tailscale: Some("xixiao@macmini.example.ts.net".to_string()),
            ..test_ssh()
        };

        let command = build_ssh_connect_cmd(&ssh_blank);
        assert!(command.contains("ssh xixiao@192.168.1.113"));
        assert!(command.contains("ssh xixiao@macmini.example.ts.net"));
        assert!(!command.contains("ssh -t"));
    }
}
