# matri.sh

Wraps your terminal in a pretty 60 FPS digital rain effect "inspired by" The Matrix.

![demo](./demo.gif)

⚠️ Warning: Like The Matrix itself, `matri.sh` was created by an AI agent.  Claude is no Smith, but I still trust him about as far as I can throw him and those server racks are _heavy_. Don't use this stupid toy for anything important.

## Requirements

- Python 3.10+
- [`pyte`](https://pypi.org/project/pyte/) - VT100 terminal emulator library
- [`wcwidth`](https://pypi.org/project/wcwidth/) *(optional)* - correct width for CJK/wide characters; falls back to width 1 if absent

## Usage

```bash
chmod +x matri.sh
./matri.sh
```

## Configuration
You can configure the rain speed, trail length, glitch frequency, colors, and character set by exporting these environment variables:

| Variable                      | Default                                | Description                                       |
|-------------------------------|----------------------------------------|---------------------------------------------------|
| `MATRISH_CHARS`               | half-width kana + digits + Z + symbols | Characters to use for rain glyphs                 |
| `MATRISH_MIN_SPEED`           | `1.6`                                  | Slowest column speed (screen-heights per second)  |
| `MATRISH_MAX_SPEED`           | `5.2`                                  | Fastest column speed (screen-heights per second)  |
| `MATRISH_MIN_LENGTH`          | `6`                                    | Shortest rain trail (rows)                        |
| `MATRISH_MAX_LENGTH`          | `30`                                   | Longest rain trail (rows)                         |
| `MATRISH_SPEED_SCALE_ROWS`    | `20.0`                                 | Reference terminal height for speed normalisation |
| `MATRISH_MIN_GLITCH_INTERVAL` | `0.004`                                | Minimum seconds between character glitches        |
| `MATRISH_MAX_GLITCH_INTERVAL` | `0.04`                                 | Maximum seconds between character glitches        |

