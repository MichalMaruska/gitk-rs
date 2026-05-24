# gitk-rs

A Rust/[egui](https://github.com/emilk/egui) port of **gitk** — the classic Tcl/Tk Git repository browser.

## Features

| Feature | Description |
|---------|-------------|
| **Commit graph** | Full DAG with coloured branch lanes, merge edges, bezier elbows |
| **Ref badges** | HEAD (green), branches (blue), remotes (dark blue), tags (gold) |
| **Three-pane layout** | Graph top · Diff/Blame bottom-left · File list bottom-right |
| **Draggable splitters** | Resize all three panels freely |
| **Unified diff** | Syntax-coloured `+/-/@@/diff/index` lines |
| **Blame view** | Per-line SHA · author · line number · content |
| **Branch sidebar** | Collapsible branch/remote list |
| **Search** | Filter by summary · author · SHA, with `▲▼` navigation |
| **Context menu** | Right-click → copy full SHA / short SHA / commit message / find by author |
| **Keyboard nav** | `↑↓` select commits · `Ctrl+R` / `F5` reload · `Ctrl+F` focus search |
| **Copied flash** | Status bar confirms clipboard copy for 2.5 s |
| **Max-commits** | View menu lets you choose 500 / 2 000 / 5 000 / 10 000 |
| **Dark theme** | VS Code–inspired dark palette throughout |

## Requirements

- **Rust ≥ 1.85** (stable)  
- **libgit2** development headers  
  - Ubuntu/Debian: `sudo apt install libgit2-dev`  
  - macOS: `brew install libgit2`  
  - Windows: bundled automatically via `git2` vendored feature
- A display (X11 or Wayland on Linux; native on macOS/Windows)

## Build & run

```bash
# Debug build (fast compile)
cargo build
./target/debug/gitk-rs [path/to/repo]

# Release build (optimised, smaller binary)
cargo build --release
./target/release/gitk-rs [path/to/repo]

# Run directly (opens current directory)
cargo run --release

# Open a specific repo
cargo run --release -- /path/to/your/repo
```

## Windows / vendored libgit2

On Windows, or if you don't want to install libgit2 system-wide, change `Cargo.toml`:

```toml
git2 = { version = "0.18", features = ["vendored-libgit2"] }
```

This compiles libgit2 from source — slower first build, but no system dependency.

## Layout

```
┌─────────────────────────────────────────────────────┐
│ File │ View │ Help │              Find: [______] ▲▼  │
├──────────────┬──────────────────────────────────────┤
│ Branch       │  ● main  Fix login bug    Alice  2024 │
│ sidebar      │  │ ● dev  Add dark mode   Bob    2024 │
│ (all)        │  │─● feat  WIP            Alice  2024 │
│ main         │  ● origin/main            …           │
│ dev          ├───────────────────────┬───────────────┤
│ origin/main  │ abc1234  [main]       │ Changed files │
│              │ Author: Alice <a@b>   │ M src/main.rs │
│              │ Date:   2024-01-15    │ A src/new.rs  │
│              │ ─────────────────     │               │
│              │ [Diff] [Blame]        │ +42  -3       │
│              │ +added line …        │               │
│              │ -removed line …      │               │
└──────────────┴───────────────────────┴───────────────┘
```

## Architecture

| File | Purpose |
|------|---------|
| `src/main.rs` | Entry point, window setup |
| `src/git.rs` | libgit2 wrapper — commits, diffs, blame, refs |
| `src/graph.rs` | DAG lane-layout algorithm + branch colour palette |
| `src/ui.rs` | Full egui application — all panels, menus, interactions |

## Differences from original gitk

- Written in Rust (not Tcl/Tk) — single static binary, no runtime required
- Uses egui immediate-mode GUI instead of Tk widgets
- Blame view integrated as a tab (gitk has it as a separate window)
- Branch sidebar replaces gitk's "Refs" listbox
- No `git bisect` integration (yet)
- No patch export / `git format-patch` (yet)
