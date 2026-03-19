#!/usr/bin/env python
"""
matrix.sh — Digital rain TUI shell wrapper.

Runs your $SHELL inside a PTY, parses its VT100 output with pyte to maintain
a virtual screen buffer, then composites the shell content over the Matrix
digital-rain animation on every frame.

Architecture:
  ┌──────────────┐   bytes   ┌─────────┐  virtual screen  ┌────────────┐
  │  $SHELL      │ ────────► │  pyte   │ ───────────────► │ compositor │ ──► terminal
  │  (under PTY) │           │ (VT100) │                  │ + rain     │
  └──────────────┘           └─────────┘                  └────────────┘

Shell content wins every cell it touches; rain fills the rest.

Requires:  pip install pyte
"""

from __future__ import annotations

import fcntl
import os
import pty
import random
import select
import shutil
import signal
import struct
import sys
import termios
import time
import tty
from typing import Optional

try:
    import pyte
except ImportError:
    sys.exit("matrix.sh: pyte is required — run: pip install pyte")

# Newer pyte streams pass `private=True` to csi_dispatch handlers (including
# select_graphic_rendition) but pyte's own Screen doesn't accept that kwarg,
# causing a crash on certain escape sequences (e.g. from vi).  Absorb it here.
class _Screen(pyte.Screen):
    def select_graphic_rendition(self, *attrs, private=False, **kwargs):
        super().select_graphic_rendition(*attrs, **kwargs)

try:
    from wcwidth import wcwidth as _wcwidth
except ImportError:
    def _wcwidth(c: str) -> int:  # type: ignore[misc]
        return 1


# ─── Matrix rain characters ───────────────────────────────────────────────────

CHARS = "ｦｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789|+=*\"^~><-:Z@#$%&"


# ─── pyte color → ANSI escape ─────────────────────────────────────────────────

_FG_NAMED = {
    "black": 30, "red": 31, "green": 32, "yellow": 33,
    "blue": 34, "magenta": 35, "cyan": 36, "white": 37,
    "bright-black": 90, "bright-red": 91, "bright-green": 92, "bright-yellow": 93,
    "bright-blue": 94, "bright-magenta": 95, "bright-cyan": 96, "bright-white": 97,
}
_BG_NAMED = {k: v + 10 for k, v in _FG_NAMED.items()}


def _color_esc(c, fg: bool) -> str:
    """Convert a pyte color value (str | int | tuple) to an ANSI escape."""
    reset = 39 if fg else 49
    table = _FG_NAMED if fg else _BG_NAMED
    base  = 38 if fg else 48

    if not c or c == "default":
        return f"\033[{reset}m"
    if isinstance(c, int):                         # 256-colour index
        return f"\033[{base};5;{c}m"
    if isinstance(c, tuple):                       # (r, g, b) true-colour
        return f"\033[{base};2;{c[0]};{c[1]};{c[2]}m"
    if isinstance(c, str):
        if c in table:
            return f"\033[{table[c]}m"
        if c.startswith("color#") and len(c) == 13:   # pyte hex true-colour
            r = int(c[6:8], 16)
            g = int(c[8:10], 16)
            b = int(c[10:12], 16)
            return f"\033[{base};2;{r};{g};{b}m"
    return f"\033[{reset}m"


def _attr_esc(ch: "pyte.screens.Char") -> str:
    """Build an SGR string for text attributes (bold, italic, …)."""
    codes: list[str] = []
    if ch.bold:          codes.append("1")
    if ch.italics:       codes.append("3")
    if ch.underscore:    codes.append("4")
    if ch.blink:         codes.append("5")
    if ch.reverse:       codes.append("7")
    if ch.strikethrough: codes.append("9")
    return f"\033[{';'.join(codes)}m" if codes else ""


# ─── Rain column ──────────────────────────────────────────────────────────────

class Column:
    """One column of falling green glyphs."""

    __slots__ = ("x", "height", "head", "speed", "length", "chars", "_ttg")

    # Visual zones from head (bright white) → tail (very dark green)
    _HEAD = "\033[1;97m"      # bold bright white
    _NEAR = "\033[1;92m"      # bold bright green
    _MID  = "\033[32m"        # green
    _FAR  = "\033[2;32m"      # dim green
    _TAIL = "\033[38;5;22m"   # very dark green

    def __init__(self, x: int, height: int, stagger: bool = True) -> None:
        self.x = x
        self.height = height
        self._init(stagger=stagger)

    def _init(self, stagger: bool = False) -> None:
        self.speed  = random.uniform(1.6, 5.2)     # screen-heights per second
        self.length = random.randint(6, min(self.height, 30))
        self.chars  = [random.choice(CHARS) for _ in range(max(self.height, 1))]
        self._ttg   = random.uniform(0.004, 0.04)    # seconds until next glitch
        # Stagger starting positions so columns don't all start together
        lo = -self.height * 0.8 if stagger else -10
        hi =  self.height * 0.8 if stagger else   0
        self.head = random.uniform(lo, hi)

    def tick(self, dt: float) -> None:
        self.head += self.speed * dt * (self.height / 20.0)
        if self.head - self.length > self.height:
            self._init()
        self._ttg -= dt
        if self._ttg <= 0:
            self.chars[random.randrange(len(self.chars))] = random.choice(CHARS)
            self._ttg = random.uniform(0.004, 0.04)

    def cell_at(self, row: int) -> Optional[tuple[str, str]]:
        """Return (char, ansi_color) for *row*, or None if no rain here."""
        dist = self.head - row      # positive → head has passed this row
        if dist < 0 or dist > self.length:
            return None
        ch   = self.chars[row % len(self.chars)]
        fade = dist / self.length   # 0.0 = head, 1.0 = far tail

        if   fade < 0.07:  return ch, self._HEAD
        elif fade < 0.18:  return ch, self._NEAR
        elif fade < 0.45:  return ch, self._MID
        elif fade < 0.70:  return ch, self._FAR
        else:              return ch, self._TAIL


# ─── Application ──────────────────────────────────────────────────────────────

class MatrixShell:
    FPS = 60

    def __init__(self) -> None:
        self.cols, self.rows = shutil.get_terminal_size()
        self._rain: list[Column] = [
            Column(x, self.rows, stagger=True) for x in range(self.cols)
        ]
        self._screen = _Screen(self.cols, self.rows)
        self._stream = pyte.ByteStream(self._screen)
        self._fd:  int = -1     # PTY master file descriptor
        self._pid: int = -1     # shell process PID
        self._running = True

    # ── PTY management ────────────────────────────────────────────────────────

    def _spawn_shell(self) -> None:
        shell = os.environ.get("SHELL") or shutil.which("bash") or "/bin/sh"
        pid, master = pty.fork()
        if pid == 0:
            # Child: set terminal size before handing control to the shell.
            self._set_winsize(sys.stdout.fileno(), self.rows, self.cols)
            os.execvp(shell, [shell])
            os._exit(1)         # execvp failed
        self._pid = pid
        self._fd  = master
        self._set_winsize(master, self.rows, self.cols)

    @staticmethod
    def _set_winsize(fd: int, rows: int, cols: int) -> None:
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    # ── Resize handler ────────────────────────────────────────────────────────

    def _on_sigwinch(self, *_) -> None:
        self.cols, self.rows = shutil.get_terminal_size()
        self._screen.resize(self.rows, self.cols)

        # Grow or shrink the rain array to match new width
        while len(self._rain) < self.cols:
            self._rain.append(Column(len(self._rain), self.rows, stagger=False))
        self._rain = self._rain[:self.cols]
        for col in self._rain:
            col.height = self.rows

        if self._fd >= 0:
            self._set_winsize(self._fd, self.rows, self.cols)
        if self._pid > 0:
            try:
                os.kill(self._pid, signal.SIGWINCH)
            except ProcessLookupError:
                pass

    # ── Rendering ─────────────────────────────────────────────────────────────

    @staticmethod
    def _shell_owns_cell(ch: Optional["pyte.screens.Char"]) -> bool:
        """True when the shell placed visible content in this cell."""
        if ch is None:
            return False
        return (
            bool(ch.data and ch.data not in (" ", "\x00"))
            or ch.fg  not in ("default", None)
            or ch.bg  not in ("default", None)
            or ch.bold
            or ch.reverse
        )

    def _render(self, dt: float) -> None:
        for col in self._rain:
            col.tick(dt)

        parts: list[str] = [
            "\033[?2026h",  # begin synchronized update — terminal buffers until end
            "\033[H",       # cursor → top-left
        ]

        prev_esc = ""   # track last colour/attr escape to avoid redundant codes

        # Pre-scan entire screen: find all shell-owned (row, col) pairs, then
        # build the 8-neighbor border so rain never touches shell text from
        # any direction (left, right, above, below, or diagonally).
        owned_cells: set[tuple[int, int]] = {
            (r, c)
            for r in range(self.rows)
            for c, ch in self._screen.buffer.get(r, {}).items()
            if self._shell_owns_cell(ch)
        }
        # The input cursor position is always treated as owned so the halo
        # keeps rain away from wherever the user is typing.
        owned_cells.add((self._screen.cursor.y, self._screen.cursor.x))
        shell_border: set[tuple[int, int]] = {
            (r + dr, c + dc)
            for (r, c) in owned_cells
            for dr in (-1, 0, 1)
            for dc in (-1, 0, 1)
            if (dr or dc)                           # skip the cell itself
        } - owned_cells

        for row in range(self.rows):
            row_buf = self._screen.buffer.get(row, {})

            skip_next = False   # True after a wide char: terminal already advanced
            for col_idx in range(self.cols):
                if skip_next:
                    skip_next = False
                    continue

                pch = row_buf.get(col_idx)

                # pyte marks the cell to the right of a wide char with data="".
                # The wide char already physically occupies that terminal column,
                # so output nothing — not even rain — and stay in sync.
                if pch is not None and pch.data == "":
                    continue

                if self._shell_owns_cell(pch):
                    # ── Shell content wins ────────────────────────────────
                    cell_esc = (
                        "\033[0m"
                        + _color_esc(pch.fg, fg=True)
                        + _color_esc(pch.bg, fg=False)
                        + _attr_esc(pch)
                    )
                    glyph = pch.data or " "
                    # Wide chars (width 2) need two terminal columns.
                    # If the char would overflow the right margin, swap it for a
                    # space — otherwise the terminal auto-wraps AND our \r\n
                    # adds a second newline, doubling the line.
                    if _wcwidth(glyph) == 2:
                        if col_idx + 1 >= self.cols:
                            glyph = " "       # clip: can't fit, use placeholder
                        else:
                            skip_next = True  # terminal cursor is already at col+2
                elif (row, col_idx) in shell_border:
                    # ── One-cell buffer either side of shell text ─────────
                    # Keeps rain from bleeding directly into/out of words.
                    glyph, cell_esc = " ", "\033[0m"
                else:
                    # ── Digital rain (or blank) ───────────────────────────
                    rain = (
                        self._rain[col_idx].cell_at(row)
                        if col_idx < len(self._rain) else None
                    )
                    if rain:
                        glyph, cell_esc = rain
                    else:
                        glyph, cell_esc = " ", "\033[0m"

                if cell_esc != prev_esc:
                    parts.append(cell_esc)
                    prev_esc = cell_esc
                parts.append(glyph)

            if row < self.rows - 1:
                parts.append("\r\n")

        # Restore cursor to wherever the shell left it, then show it
        cy = self._screen.cursor.y
        cx = self._screen.cursor.x
        parts.append(f"\033[0m\033[{cy + 1};{cx + 1}H\033[?25h")
        parts.append("\033[?2026l")     # end synchronized update → display frame

        payload = "".join(parts).encode("utf-8", errors="replace")
        sys.stdout.buffer.write(payload)
        sys.stdout.buffer.flush()

    # ── Event loop ────────────────────────────────────────────────────────────

    def run(self) -> None:
        if not sys.stdin.isatty():
            sys.exit("matrix.sh: must be run in an interactive terminal")

        saved_tty = termios.tcgetattr(sys.stdin)
        try:
            tty.setraw(sys.stdin.fileno())
            self._spawn_shell()
            signal.signal(signal.SIGWINCH, self._on_sigwinch)

            frame_time = 1.0 / self.FPS
            last_render = time.monotonic()

            while self._running:
                now     = time.monotonic()
                timeout = max(0.0, frame_time - (now - last_render))

                try:
                    readable, _, _ = select.select(
                        [sys.stdin, self._fd], [], [], timeout
                    )
                except (InterruptedError, ValueError):
                    continue    # SIGWINCH or other signal — just loop

                # Forward raw keystrokes → shell
                if sys.stdin in readable:
                    try:
                        data = os.read(sys.stdin.fileno(), 256)
                        if data:
                            os.write(self._fd, data)
                    except OSError:
                        break

                # Shell output → pyte virtual screen
                if self._fd in readable:
                    try:
                        chunk = os.read(self._fd, 4096)
                        if chunk:
                            self._stream.feed(chunk)
                    except OSError:
                        self._running = False
                        break

                # Render at target frame rate
                now = time.monotonic()
                if now - last_render >= frame_time:
                    self._render(now - last_render)
                    last_render = now

        finally:
            # Restore terminal state unconditionally
            try:
                termios.tcsetattr(sys.stdin, termios.TCSADRAIN, saved_tty)
            except termios.error:
                pass
            sys.stdout.write("\033[0m\033[?25h\r\n")
            sys.stdout.flush()
            # Reap child so it doesn't become a zombie
            if self._pid > 0:
                try:
                    os.waitpid(self._pid, 0)
                except ChildProcessError:
                    pass


def main() -> None:
    MatrixShell().run()


if __name__ == "__main__":
    main()
