# Agent Guide - Finter

## Project Overview

**Finter** is a Rust CLI tool for tmux users to quickly create and switch between tmux sessions based on project folders, on this machine and on configured remote hosts, and to track which coding agents (Claude Code) are waiting for input.

- **Language:** Rust (edition 2021)
- **Current Version:** see `Cargo.toml`
- **Primary Dependencies:** skim (fuzzy finder), clap (CLI parser), serde + toml + serde_json (config, hooks, `list --json`)

## What It Does

1. Scans configured project root directories
2. Lists child folders + existing tmux sessions + each `[[hosts]]` entry's projects (`mini/X`) in a fuzzy picker with agent state, wait time and a preview
3. On selection:
   - **Existing session:** switches/attaches to it
   - **New local project session:** creates 2 windows in project directory and stays on window 1
   - **Remote project:** creates local session `<host>/<X>` with 2 windows that run `ssh -t <alias> finter attach X [--term]`
4. `finter hook claude` records agent state in pane options; `status`, `next`, `seen` read them

## Repository Structure

```
finter/
├── .github/workflows/publish.yml  # Auto-publish to crates.io on version change
├── src/
│   ├── main.rs      # clap CLI: -d and subcommands (hook, status, next, seen, preview, copy-mode, kill, list, attach)
│   ├── lib.rs       # glue: picker flow, list, attach
│   ├── config.rs    # ~/.finter.toml: roots, [[hosts]], folder scanning
│   ├── tmux.rs      # tmux runner (default server and `-L finter`), pane listing, option argv builders
│   ├── state.rs     # AgentState, AgentEntry, ordering, next selection, age format
│   ├── hook.rs      # `finter hook claude`, transcript tail
│   ├── remote.rs    # list --json schema, ssh + cache, attach, wrapper session, inner.conf use
│   ├── status.rs    # status, next, seen, preview, copy-mode, kill
│   ├── picker.rs    # picker rows + skim
│   └── inner.conf   # tmux config for the remote `-L finter` server (include_str!)
├── tests/fixtures/transcript.jsonl  # Claude transcript sample for hook tests
├── docs/            # behavior, usage, tmux-integration, troubleshooting
├── Cargo.toml
└── .finter.toml.example
```

## Core Architecture

### Contracts

- **Pane options:** `@agent`, `@agent_state` (`working|asking|done`), `@agent_since` (unix secs), `@agent_msg`; `@finter_remote=<alias>:<inner session>` on wrapper panes. Agent-agnostic: only the hook adapter knows about Claude.
- **Inner server:** remote sessions live on `tmux -L finter` (socket name overridable with `FINTER_INNER_SOCKET`, used by smoke tests). Sessions are `X` and `X~term`.
- **`finter list --json`:** `{"v":1,"projects":[...],"panes":[...]}`; bump `v` on incompatible changes (`remote::LIST_VERSION`).
- **Cache:** `$XDG_RUNTIME_DIR/finter/<host>.json`, stale after 4s, offline flag keeps last data.

### Testing seams

Logic is pure and unit-tested: `hook::decide`, `hook::last_assistant_text`, `status::render_status`, `state::pick_next`, `picker::build_rows`, `remote::merge_fetch`, `remote::load_host_with` (takes the fetch function, so ssh is mocked). tmux and ssh execution is not unit-tested; smoke test on a scratch server (`tmux -L finter-test`, `FINTER_INNER_SOCKET`, a fake `HOME`/`XDG_RUNTIME_DIR` with a seeded cache file).

## Configuration

**Location:** `~/.finter.toml`

```toml
roots = [
    "/home/username/projects",
    "/home/username/work",
]

[[hosts]]
name = "mini"
ssh = "mac-mini"
```

The legacy `[ssh]` table is rejected with a message pointing to `[[hosts]]`.

## Development Workflow

### Build & Test
```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
cargo run -- -d /path1 /path2  # Update config
```

### Code Style
- **Formatter:** `rustfmt`
- **Comments:** Document WHY, not WHAT
- **Error handling:** `Box<dyn Error>` for user-facing commands; hooks and status never fail loudly

### Key Constraints
- Only scans **immediate child directories** of configured roots
- Session names: `.` and `:` replaced with `_` (tmux limitation); remote wrappers are `host/X` (tmux accepts `/`)
- Host names cannot contain whitespace, `/`, `:` or `.`
- `finter hook` must exit 0 and stay quiet; with `$TMUX_PANE` unset it does nothing
- tmux is assumed to use `base-index 1` on all hosts (`next` falls back to windows 1 and 2)

## Common Modification Patterns

### Adding an agent
Add a hook adapter in `src/hook.rs` that maps its events to `AgentState` and writes the same pane options; the rest is agent-agnostic.

### Adding Config Options
1. Update `Config` in `src/config.rs`
2. Add `parse_config` tests
3. Update `.finter.toml.example` and docs

### Changing the status look
Edit `badge` and `render_status` in `src/status.rs` and their tests.

## Common Pitfalls

1. **Update docs/** when changing session creation or the pane-option/JSON contracts
2. **Home directory is fallback path** for existing sessions without a folder
3. **Folder name collisions** across roots map to the same session name
4. `tmux` output may end with empty fields; never trim tabs from `list-panes` output (only newlines)
5. Names embedded in status output must escape `#` as `##`

## Dependencies to Watch

- `skim 0.10.4`: Fuzzy finder (popup UI; preview command gets `{}` shell-quoted)
- `clap 4.x`: CLI parsing (derive macros)
- `toml 0.8`, `serde_json`: config and JSON
- `home`: home directory detection

## Quick Reference Commands

```bash
cargo install --path .
finter -d /path/to/projects /another/path
finter                       # picker (typically via tmux binding)
finter status                # status-bar segment
finter list --json           # what other hosts see
bind C-o display-popup -E "finter"   # tmux binding example
```

## When Making Changes

**Checklist:**
1. ✓ Modify the relevant module under `src/`
2. ✓ Update relevant `docs/*.md` files
3. ✓ Add/update tests if logic changes
4. ✓ Run `cargo test` to verify
5. ✓ Update version in `Cargo.toml` (semantic versioning)
6. ✓ Run `cargo build` to update `Cargo.lock`
7. ✓ Commit `Cargo.toml` AND `Cargo.lock` together
8. ✓ Update README.md if user-facing behavior changes
9. ✓ Push - GitHub Actions will auto-publish to crates.io

**Version bumping strategy:**
- **Patch (0.2.x):** Bug fixes, internal refactoring
- **Minor (0.x.0):** New features, behavior changes
- **Major (x.0.0):** Breaking changes, API redesign

## CI/CD - Automated Publishing

**Workflow:** `.github/workflows/publish.yml`

**Trigger:** Automatically runs when `Cargo.toml` is pushed to `main`/`master` branch

**Process:**
1. Checkout code
2. Setup Rust toolchain
3. Run `cargo test` (validation)
4. Check if version is already published on crates.io
5. If new version: publish to crates.io
6. If same version: skip (idempotent)

**Publishing workflow:**
```bash
# Standard version bump workflow:

# 1. Make your code changes
vim src/lib.rs

# 2. Run tests to verify
cargo test

# 3. Update version in Cargo.toml (e.g., 0.2.0 → 0.3.0)
vim Cargo.toml

# 4. Rebuild to update Cargo.lock
cargo build

# 5. Commit BOTH Cargo.toml and Cargo.lock together
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.3.0"

# 6. Push to trigger GitHub Actions
git push

# 7. GitHub Actions automatically publishes to crates.io
# No manual intervention needed!
```

**⚠️ IMPORTANT:** Always commit `Cargo.lock` with `Cargo.toml` version changes!
- `cargo publish` requires a clean git working directory
- Running `cargo build` after version change updates `Cargo.lock`
- Both files must be committed together to avoid CI/CD failures

**Requirements:**
- GitHub repository secret `CARGO_REGISTRY_TOKEN` must be set
- Token obtained from https://crates.io/settings/tokens

**What it prevents:**
- ✓ Publishing without tests passing
- ✓ Re-publishing the same version (checks crates.io first)
- ✓ Manual `cargo publish` commands
- ✓ Forgetting to publish after version bump
