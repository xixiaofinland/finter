use serde::{Deserialize, Serialize};
use std::{error::Error, fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Config {
    #[serde(default)]
    pub(crate) roots: Vec<String>,
    #[serde(default)]
    pub(crate) roots_config: Vec<RootConfig>,
    #[serde(default)]
    pub(crate) hosts: Vec<HostConfig>,
    // Legacy `[ssh]` table: kept only so parsing can reject it with a migration hint.
    #[serde(default, skip_serializing)]
    pub(crate) ssh: Option<toml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RootConfig {
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) mode: Option<RootMode>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RootMode {
    #[default]
    Children,
    Direct,
}

impl RootConfig {
    pub(crate) fn mode(&self) -> RootMode {
        self.mode.clone().unwrap_or_default()
    }
}

/// A remote machine whose finter sessions are listed and attached over ssh.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct HostConfig {
    /// Short label shown in the picker and used as local session prefix (`mini/X`).
    pub(crate) name: String,
    /// ssh alias or target passed verbatim to `ssh`.
    pub(crate) ssh: String,
}

pub fn save_paths(args: &[String]) -> Result<(), Box<dyn Error>> {
    let config_file_path = get_config_file_path();
    let mut config = if Path::new(&config_file_path).exists() {
        load_config()?
    } else {
        Config {
            roots: Vec::new(),
            roots_config: Vec::new(),
            hosts: Vec::new(),
            ssh: None,
        }
    };

    config.roots_config = args
        .iter()
        .cloned()
        .map(|path| RootConfig {
            path,
            mode: None, // defaults to children
        })
        .collect();
    let content = toml::to_string_pretty(&config)?;
    fs::write(config_file_path, content).expect("Unable to write file");
    Ok(())
}

/// `roots_config` (Vec<RootConfig>) formats. Returns (path, mode) tuples.
pub(crate) fn load_project_paths(
    config: &Config,
) -> Result<Vec<(String, RootMode)>, Box<dyn Error>> {
    let mut paths: Vec<(String, RootMode)> = Vec::new();

    // New format: roots_config
    for rc in &config.roots_config {
        let mode = rc.mode();
        if Path::new(&rc.path).is_dir() {
            paths.push((rc.path.clone(), mode));
        }
    }

    // Legacy format: roots (defaults to children mode)
    for root in &config.roots {
        if Path::new(root).is_dir() {
            paths.push((root.clone(), RootMode::Children));
        }
    }

    match paths.len() {
        0 => Err("Err: no valid path is configured.".into()),
        _ => Ok(paths),
    }
}

pub(crate) fn get_config_file_path() -> String {
    let mut config_file_location = get_home_dir();
    config_file_location.push_str("/.finter.toml");
    config_file_location
}

pub(crate) fn parse_config(content: &str) -> Result<Config, Box<dyn Error>> {
    let config: Config = toml::from_str(content)?;
    if config.ssh.is_some() {
        return Err("the `[ssh]` table is no longer supported; migrate to `[[hosts]]` with `name = \"mini\"` and `ssh = \"<ssh alias>\"`".into());
    }
    for host in &config.hosts {
        validate_host(host)?;
    }
    Ok(config)
}

fn validate_host(host: &HostConfig) -> Result<(), Box<dyn Error>> {
    let bad_name = host.name.is_empty()
        || host
            .name
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | ':' | '.'));
    if bad_name {
        return Err(format!(
            "invalid host name `{}`: use a non-empty name without whitespace, `/`, `:` or `.`",
            host.name
        )
        .into());
    }
    if host.ssh.trim().is_empty() {
        return Err(format!("host `{}` has an empty `ssh` value", host.name).into());
    }
    Ok(())
}

pub(crate) fn load_config() -> Result<Config, Box<dyn Error>> {
    let config_file_location = get_config_file_path();
    let file_content = fs::read_to_string(&config_file_location).map_err(|_| {
        format!(
            "Err: config file is missing. create `{}` with roots/hosts settings or run `finter -d <ABS_PATH> ...` first.",
            config_file_location
        )
    })?;

    parse_config(&file_content).map_err(|e| {
        format!(
            "Err: invalid TOML config at `{}`: {}",
            config_file_location, e
        )
        .into()
    })
}

/// Collect folders from paths. The `mode` determines behavior:
/// - `Children`: scan immediate subdirectories of the path
/// - `Direct`: use the path itself as a project folder
pub(crate) fn get_folders(
    paths: Vec<(String, RootMode)>,
) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut folders = Vec::new();

    for (path, mode) in paths {
        match mode {
            RootMode::Children => {
                // Scan immediate subdirectories
                for entry in fs::read_dir(&path)? {
                    let entry = entry?;
                    let p = entry.path();
                    if p.is_dir() {
                        let folder = p
                            .file_name()
                            .ok_or("expect a file_name()")?
                            .to_str()
                            .ok_or("file_name can't turn to a string")?
                            .replace('.', "_")
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
            RootMode::Direct => {
                // Use this exact path as a project folder
                let folder = Path::new(&path)
                    .file_name()
                    .ok_or("expect a file_name()")?
                    .to_str()
                    .ok_or("file_name can't turn to a string")?
                    .replace('.', "_")
                    .replace(":", "_")
                    .to_string();

                folders.push((folder, path));
            }
        }
    }
    match folders.len() {
        0 => Err("Err: no folder is found in the configured paths.".into()),
        _ => Ok(folders),
    }
}

pub(crate) fn get_home_dir() -> String {
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
    fn parse_config_reads_hosts() {
        let content = r#"
roots = ["/tmp"]

[[hosts]]
name = "mini"
ssh = "mac-mini"
"#;
        let cfg = parse_config(content).expect("config should parse");
        assert_eq!(
            cfg.hosts,
            vec![HostConfig {
                name: "mini".to_string(),
                ssh: "mac-mini".to_string()
            }]
        );
    }

    #[test]
    fn parse_config_allows_no_hosts() {
        let cfg = parse_config(r#"roots = ["/tmp"]"#).expect("config should parse");
        assert!(cfg.hosts.is_empty());
    }

    #[test]
    fn parse_config_rejects_legacy_ssh_table() {
        let content = r#"
roots = ["/tmp"]

[ssh]
session_name = "ssh_mac_mini"
primary = "xixiao@192.168.1.113"
"#;
        let err = parse_config(content).expect_err("legacy [ssh] must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("[ssh]"), "{msg}");
        assert!(msg.contains("[[hosts]]"), "{msg}");
    }

    #[test]
    fn parse_config_rejects_bad_host_names() {
        for name in ["", "a/b", "a b", "a:b", "a.b"] {
            let content = format!("[[hosts]]\nname = \"{name}\"\nssh = \"x\"\n");
            assert!(parse_config(&content).is_err(), "name {name:?} accepted");
        }
        let content = "[[hosts]]\nname = \"mini\"\nssh = \" \"\n";
        assert!(parse_config(content).is_err());
    }

    #[test]
    fn parse_config_reads_roots_config_with_modes() {
        let content = r#"
[[roots_config]]
path = "/home/finxxi/Projects"
mode = "children"

[[roots_config]]
path = "/home/finxxi/notes"
mode = "direct"
"#;

        let cfg = parse_config(content).expect("config should parse");
        assert_eq!(cfg.roots_config.len(), 2);
        assert_eq!(cfg.roots_config[0].path, "/home/finxxi/Projects");
        assert_eq!(cfg.roots_config[0].mode(), RootMode::Children);
        assert_eq!(cfg.roots_config[1].path, "/home/finxxi/notes");
        assert_eq!(cfg.roots_config[1].mode(), RootMode::Direct);
        assert_eq!(cfg.roots.len(), 0);
    }

    #[test]
    fn parse_config_reads_roots_config() {
        let content = r#"
[[roots_config]]
path = "/home/finxxi/Projects"
mode = "children"
"#;

        let cfg = parse_config(content).expect("config should parse");
        assert_eq!(cfg.roots_config.len(), 1);
    }

    #[test]
    fn parse_config_defaults_mode_to_children_when_missing() {
        let content = r#"
[[roots_config]]
path = "/home/finxxi/Projects"
"#;

        let cfg = parse_config(content).expect("config should parse");
        assert_eq!(cfg.roots_config.len(), 1);
        assert_eq!(cfg.roots_config[0].mode(), RootMode::Children);
    }

    #[test]
    fn parse_config_merges_legacy_roots_and_roots_config() {
        let content = r#"
roots = ["/home/finxxi/legacy"]

[[roots_config]]
path = "/home/finxxi/notes"
mode = "direct"
"#;

        let cfg = parse_config(content).expect("config should parse");
        assert_eq!(cfg.roots.len(), 1);
        assert_eq!(cfg.roots_config.len(), 1);
        assert_eq!(cfg.roots_config[0].path, "/home/finxxi/notes");
        assert_eq!(cfg.roots_config[0].mode(), RootMode::Direct);
    }

    #[test]
    fn load_project_paths_handles_children_mode() {
        let content = r#"
[[roots_config]]
path = "/tmp"
mode = "children"
"#;

        let cfg: Config = toml::from_str(content).expect("config should parse");
        let paths = load_project_paths(&cfg).expect("paths should load");
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].0, "/tmp");
        assert_eq!(paths[0].1, RootMode::Children);
    }

    #[test]
    fn load_project_paths_handles_direct_mode() {
        let content = r#"
[[roots_config]]
path = "/tmp"
mode = "direct"
"#;

        let cfg: Config = toml::from_str(content).expect("config should parse");
        let paths = load_project_paths(&cfg).expect("paths should load");
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].0, "/tmp");
        assert_eq!(paths[0].1, RootMode::Direct);
    }

    #[test]
    fn load_project_paths_falls_back_to_legacy_roots() {
        let content = r#"
roots = ["/tmp"]
"#;

        let cfg: Config = toml::from_str(content).expect("config should parse");
        let paths = load_project_paths(&cfg).expect("paths should load");
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].0, "/tmp");
        assert_eq!(paths[0].1, RootMode::Children);
    }

    #[test]
    fn get_folders_children_scans_subdirectories() {
        let paths = vec![("/tmp".to_string(), RootMode::Children)];
        let folders = get_folders(paths).expect("should succeed");
        // /tmp should have subdirectories
        assert!(!folders.is_empty());
    }

    #[test]
    fn get_folders_direct_uses_path_as_folder() {
        let tmp_dir = std::env::temp_dir().join("finter_test_direct");
        let _ = fs::create_dir_all(&tmp_dir);
        let path = tmp_dir.to_str().unwrap().to_string();
        let folders_name = Path::new(&path)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let paths = vec![(path.clone(), RootMode::Direct)];
        let folders = get_folders(paths).expect("should succeed");
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].0, folders_name);
        assert_eq!(folders[0].1, path);
        let _ = fs::remove_dir_all(&tmp_dir);
    }
}
