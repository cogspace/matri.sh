# matri.sh

Wraps your terminal in a digital rain effect "inspired by" The Matrix.

![demo](./demo.gif)

⚠️ Warning: Like the Matrix itself, `matri.sh` was (mostly) created by an AI
agent. Claude is no Smith, but I still trust him about as far as I can throw him
and those server racks are _heavy_. Don't use this stupid toy for anything
important.

## Quick Install

```sh
curl -fsSL https://matri.sh/install.sh | sh
```

## Building from source

Matri.sh is written in Rust. You can install the Rust tools with this command:

```sh
curl https://sh.rustup.rs -sSf | sh
```

You also need `make`, if you don't already have it.

Then build matri.sh:

```sh
make build
./target/release/matri.sh
```

## Configuration

| Variable                      | Default                                | Description                                          |
|-------------------------------|----------------------------------------|------------------------------------------------------|
| `MATRISH_CHARS`               | half-width kana + digits + Z + symbols | Characters to use for rain glyphs                    |
| `MATRISH_MIN_SPEED`           | `1.6`                                  | Slowest column speed (screen-heights per second)     |
| `MATRISH_MAX_SPEED`           | `5.2`                                  | Fastest column speed (screen-heights per second)     |
| `MATRISH_MIN_LENGTH`          | `1`                                    | Shortest rain trail (rows)                           |
| `MATRISH_SPEED_SCALE_ROWS`    | `20.0`                                 | Reference terminal height for speed normalisation    |
| `MATRISH_MIN_GLITCH_INTERVAL` | `0.004`                                | Minimum seconds between character glitches           |
| `MATRISH_MAX_GLITCH_INTERVAL` | `0.04`                                 | Maximum seconds between character glitches           |
| `MATRISH_SCROLL_LINES`        | `3`                                    | Lines to scroll per mouse wheel tick                 |
| `MATRISH_SCROLLBACK`          | `10000`                                | Scrollback buffer size (lines)                       |
| `MATRISH_MAX_FPS`             | `60`                                   | Frame rate cap; `-1` for unlimited                   |
| `MATRISH_SHOW_FPS`            | *(unset)*                              | Set to any value to show an FPS counter              |
