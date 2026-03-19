# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**matri.sh** is a single-file Python TUI application that wraps your `$SHELL` in a PTY and composites its output over a Matrix-style digital rain animation. The shell content always takes visual precedence over rain, with a one-cell halo buffer around all shell-owned characters.

## Setup & Running

```bash
# Install dependency
pip install pyte        # or: pip install -r requirements.txt

# Run (the script is its own entry point)
./matri.sh
```

`wcwidth` is an optional dependency; if absent, wide characters default to width 1.

## Architecture

Everything lives in a single file: `matri.sh` (a Python script with a bash shebang that re-execs itself with Python).

**Data flow:**
```
$SHELL (under PTY) → pyte (VT100 parser/screen) → compositor → terminal stdout
```

**Key classes:**

- **`Column`** — One column of falling katakana/numeric glyphs. `tick()` advances position each frame; `cell_at(row)` returns the rain character and color for a given row. Implements head/tail color gradient (bright white head → dark green tail) and random glitch effects.

- **`MatrixShell`** — Main application class:
  - `_spawn_shell()` — forks the shell under a PTY
  - `_on_sigwinch()` — resizes pyte screen and adjusts rain columns on terminal resize
  - `_render()` — core render loop: ticks columns, pre-scans pyte screen to find shell-owned cells, expands a one-cell halo around them, composites rain vs. shell content, emits a full-frame update using VT100 synchronized output (`\033[?2026h/l`) to prevent flicker
  - `run()` — puts terminal in raw mode, multiplexes stdin + PTY via `select()`, feeds output to pyte, triggers `_render()` at 30 FPS, restores terminal on exit

**Shell-cell ownership logic:** Before rendering, `_render()` scans the full pyte screen and marks every non-empty cell (plus the cursor position) as shell-owned, then expands by one cell in all 8 directions (the "halo"). Rain only renders in cells not marked shell-owned.

## No Tests / No CI

There are no automated tests or CI pipelines. Manual testing means running `./matri.sh` and exercising the shell normally.
