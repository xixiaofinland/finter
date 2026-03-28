# Agent Guide - Finter

## Project Overview

**Finter** is a Rust CLI tool for tmux users to quickly create and switch between tmux sessions based on project folders.

- **Language:** Rust (edition 2021)
- **Current Version:** 0.2.0
- **Primary Dependencies:** skim (fuzzy finder), clap (CLI parser), serde + toml (config)

## What It Does

1. Scans configured project root directories
2. Lists child folders + existing tmux sessions in a fuzzy picker
3. On selection:
   - **Existing session:** switches/attaches to it
   - **New project session:** creates 2 windows in project directory and stays on window 1
   - **New SSH session:** creates 1 window in home directory and auto-runs SSH in it

## Repository Structure

```
finter/
├── .github/
│   └── workflows/
│       └── publish.yml  # Auto-publish to crates.io on version change
├── src/
│   ├── main.rs          # CLI argument parsing (clap)
│   └── lib.rs           # Core logic (session management, tmux commands)
├── docs/
│   ├── behavior.md      # Architecture and session creation details
│   ├── usage.md         # User guide
│   ├── tmux-integration.md
│   └── troubleshooting.md
├── Cargo.toml           # Package manifest
├── .finter.toml.example # Config template
└── README.md            # User-facing documentation
```

## Core Architecture

### Entry Points (main.rs)
- `-d/--directory <paths>`: Save project roots to config
- No args: Run the fuzzy picker workflow

### Core Functions (lib.rs)

Key functions to understand:

- `run_finter()` (line 71): Main orchestration
- `build_projects()` (line 131): Merges folders + sessions into picker items
- `select_in_skim()` (line 278): Interactive fuzzy selection
- `run_tmux_with_args()` (line 311): tmux command executor

### Session Creation Logic (lib.rs:91-113)

**Project sessions (when `is_ssh_session == false`):**
```rust
1. tmux new-session -ds <name> -c <path>      # window 1
2. tmux new-window -t <name>:2 -c <path>      # window 2
3. tmux select-window -t <name>:1             # keep window 1 active
4. tmux switch-client -t <name>
```

**SSH sessions (when `is_ssh_session == true`):**
```rust
1. tmux new-session -ds <name> -c <home>      # window 1
2. tmux send-keys -t <name>:1 "ssh -t ... 'tmux new-session -A -s mac_mini'" C-m
3. tmux switch-client -t <name>
```

## Configuration

**Location:** `~/.finter.toml`

**Structure:**
```toml
roots = [
    "/home/username/projects",
    "/home/username/work",
]

[ssh]
session_name = "ssh_mac_mini"
primary = "user@192.168.1.200"
remote_session_name = "mac_mini"
# Optional Tailscale fallback
# tailscale = "user@macmini.tailnet.ts.net"
```

## Development Workflow

### Build & Test
```bash
cargo build          # Compile
cargo test           # Run 12 unit tests
cargo run -- -d /path1 /path2  # Update config
```

### Code Style
- **Formatter:** Use `rustfmt` (standard Rust formatting)
- **Comments:** Document WHY, not WHAT
- **Error handling:** Uses `Box<dyn Error>` for flexibility

### Key Constraints
- Only scans **immediate child directories** of configured roots
- Session names: `.` and `:` replaced with `_` (tmux limitation)
- Existing sessions shown with `*` prefix in picker
- SSH session always included in picker regardless of existence

## Recent Changes (v0.3.0)

**Changed from v0.2.2:**
- **Before:** SSH session opened a plain remote shell
- **After:** SSH session attaches to or creates remote tmux session `mac_mini`
- **Applies to:** SSH session creation path and SSH config parsing
- **Files modified:** `src/lib.rs`, `docs/behavior.md`, `docs/usage.md`

## Common Modification Patterns

### Adding More Windows
Modify `src/lib.rs:91-113` - add more `run_tmux_with_args` calls with window numbers

### Changing Default Window
Modify the `select-window` target in `src/lib.rs` (currently `:2`)

### Adding Config Options
1. Update `Config` or `SshConfig` struct (line 10-31)
2. Update `parse_config` test cases (line 481+)
3. Update `.finter.toml.example`

### Modifying Session Naming
See `get_folders()` function (line 223) - currently replaces `.` and `:` with `_`

## Testing Strategy

**Unit tests:** 12 tests in `src/lib.rs`
- Focus on `build_projects()` logic
- SSH session handling
- Config parsing

**No integration tests** for tmux command execution (uses real tmux binary)

**Test coverage areas:**
- ✓ SSH session always included
- ✓ Deduplication logic (folders vs sessions)
- ✓ Current session prioritization
- ✓ Config parsing validation
- ✗ Window creation (not unit-testable)

## Common Pitfalls

1. **Don't forget to update docs/** when changing session creation logic
2. **Config requires `[ssh]` section** - parse will fail without it (see test line 482)
3. **Home directory is fallback path** for existing sessions and SSH
4. **Folder name collisions** across roots map to same session name
5. **Session naming:** Ensure special chars are handled (see line 236-237)

## Dependencies to Watch

- `skim 0.10.4`: Fuzzy finder (popup UI)
- `clap 4.x`: CLI parsing (using derive macros)
- `toml 0.8`: Config file parsing
- `home`: Cross-platform home directory detection

## Quick Reference Commands

```bash
# Install from source
cargo install --path .

# Update config
finter -d /path/to/projects /another/path

# Run picker (typically via tmux binding)
finter

# Tmux binding example
bind C-o display-popup -E "finter"
```

## When Making Changes

**Checklist:**
1. ✓ Modify `src/lib.rs` as needed
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
