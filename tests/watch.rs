use assert_cmd::Command;
use std::{
    os::unix::process::CommandExt,
    path::PathBuf,
    process,
    thread::sleep,
    time::{Duration, Instant},
};

fn watch() -> Command {
    let mut cmd = Command::cargo_bin("finter").unwrap();
    cmd.env_remove("TMUX").env_remove("TMUX_PANE");
    cmd.arg("watch").arg("--");
    cmd
}

#[test]
fn outside_tmux_it_runs_the_command_and_returns_its_exit_code() {
    watch().args(["sh", "-c", "exit 3"]).assert().code(3);
    watch().arg("true").assert().code(0);
    watch()
        .args(["sh", "-c", "echo hi"])
        .assert()
        .success()
        .stdout("hi\n");
}

/// A throwaway tmux server on its own socket, killed on drop; never the user's.
struct Scratch {
    dir: PathBuf,
    socket: PathBuf,
    pane: String,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("finter-watch-{tag}-{}", process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("sock");
        let mut s = Scratch {
            dir,
            socket,
            pane: String::new(),
        };
        let out = s.tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "sleep 60",
        ]);
        assert!(out.status.success(), "{out:?}");
        s.pane = String::from_utf8_lossy(&out.stdout).trim().to_string();
        s
    }

    fn tmux(&self, args: &[&str]) -> process::Output {
        process::Command::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .args(args)
            .env_remove("TMUX")
            .output()
            .unwrap()
    }

    fn option(&self, name: &str) -> String {
        let out = self.tmux(&[
            "display-message",
            "-p",
            "-t",
            &self.pane,
            &format!("#{{{name}}}"),
        ]);
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// `$TMUX` as tmux sets it inside a pane: socket path, server pid, session.
    fn tmux_env(&self) -> String {
        format!("{},0,0", self.socket.display())
    }

    /// Shell snippet, run by the watched command, that prints the pane's state and message.
    fn probe(&self) -> String {
        format!(
            "tmux -S '{}' display-message -p -t '{}' '#{{@agent_state}}|#{{@agent_msg}}'",
            self.socket.display(),
            self.pane
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        self.tmux(&["kill-server"]);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn pane_is_working_while_the_command_runs_and_done_after() {
    let s = Scratch::new("exit");
    for code in [0, 5] {
        let script = format!("{}; exit {code}", s.probe());
        let out = Command::cargo_bin("finter")
            .unwrap()
            .env("TMUX", s.tmux_env())
            .env("TMUX_PANE", &s.pane)
            .args(["watch", "--", "sh", "-c", &script])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(code));
        let seen = String::from_utf8_lossy(&out.stdout);
        let msg = s.option("@agent_msg");
        // The script is long, so the command line is cut like hook messages.
        assert!(
            msg.starts_with("sh -c tmux -S") && msg.ends_with('…'),
            "{msg}"
        );
        assert_eq!(seen.trim(), format!("working|{msg}"));
        assert_eq!(s.option("@agent_state"), "done");
        assert!(s.option("@agent_since").parse::<u64>().unwrap() > 0);
    }
}

#[test]
fn interrupted_command_still_leaves_the_pane_done() {
    let s = Scratch::new("int");
    let mut child = process::Command::new(assert_cmd::cargo::cargo_bin("finter"))
        .env("TMUX", s.tmux_env())
        .env("TMUX_PANE", &s.pane)
        .args(["watch", "--", "sleep", "30"])
        // Own process group, so the SIGINT below hits finter and sleep together like Ctrl-C.
        .process_group(0)
        .spawn()
        .unwrap();
    let started = Instant::now();
    while s.option("@agent_state") != "working" {
        assert!(started.elapsed() < Duration::from_secs(10), "never working");
        sleep(Duration::from_millis(20));
    }
    // Give finter time to spawn sleep after marking the pane.
    sleep(Duration::from_millis(200));
    let pgid = child.id().to_string();
    let killed = process::Command::new("kill")
        .args(["-INT", "--", &format!("-{pgid}")])
        .status()
        .unwrap();
    assert!(killed.success());
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(130));
    assert_eq!(s.option("@agent_state"), "done");
    assert_eq!(s.option("@agent_msg"), "sleep 30");
}
