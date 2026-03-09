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
   - **New project session:** creates 4 windows in project directory, defaults to window 2
   - **New SSH session:** creates 4 windows in home directory, auto-runs SSH in window 1, defaults to window 2

## Repository Structure

```
finter/
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
3. tmux new-window -t <name>:3 -c <path>      # window 3
4. tmux new-window -t <name>:4 -c <path>      # window 4
5. tmux select-window -t <name>:2             # default to window 2
6. tmux switch-client -t <name>
```

**SSH sessions (when `is_ssh_session == true`):**
```rust
1. tmux new-session -ds <name> -c <home>      # window 1
2. tmux new-window -t <name>:2 -c <home>      # window 2
3. tmux new-window -t <name>:3 -c <home>      # window 3
4. tmux new-window -t <name>:4 -c <home>      # window 4
5. tmux send-keys -t <name>:1 "ssh ..." C-m   # auto-run SSH in window 1
6. tmux select-window -t <name>:2             # default to window 2
7. tmux switch-client -t <name>
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
# Optional Tailscale fallback
# tailscale = "user@macmini.tailnet.ts.net"
```

## Development Workflow

### Build & Test
```bash
cargo build          # Compile
cargo test           # Run 10 unit tests
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

## Recent Changes (v0.2.0)

**Changed from v0.1.19:**
- **Before:** 2 windows created, default to window 1
- **After:** 4 windows created, default to window 2
- **Applies to:** Both project and SSH sessions
- **Files modified:** `src/lib.rs:91-113`, `docs/behavior.md:47-66`

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

**Unit tests:** 10 tests in `src/lib.rs:340-507`
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
5. ✓ Run `cargo build` to verify compilation
6. ✓ Update version in `Cargo.toml` (semantic versioning)
7. ✓ Update README.md if user-facing behavior changes

**Version bumping strategy:**
- **Patch (0.2.x):** Bug fixes, internal refactoring
- **Minor (0.x.0):** New features, behavior changes
- **Major (x.0.0):** Breaking changes, API redesign
