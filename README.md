# screencap-rs

Rust rewrite of [screencap](https://github.com/fuyu28/screencap) — a Windows
screenshot comparison tool. Behavior-compatible with the C++ version.

## Binaries

| exe | crate | description |
| --- | --- | --- |
| `screencap.exe` | `screencap-gui` | Window-picker GUI. Spawns `screencap-cli.exe` (placed next to it) for the actual capture. |
| `screencap-cli.exe` | `screencap-cli` | CLI: `cap` / `list windows` / `list monitors`, JSON output, global hotkey. |

## Build (Windows)

```
cargo build --release
```

Binaries land in `target/x86_64-pc-windows-msvc/release/`
(the default target is pinned in `.cargo/config.toml`, which also lets
`cargo check` type-check on non-Windows hosts).

## Capture methods

- `wgc-window`, `wgc-window2` (window capture)
- `wgc-monitor`, `wgc-monitor2` (monitor capture)

All methods use Windows.Graphics.Capture and are cropped to the frame's
ContentSize.

See the original repo's `docs/capture-investigation.md` for the method
comparison notes.

## Output formats

`cap` writes PNG by default. Select the format with `--format`:

- `--format png` (default) — lossless, 32bpp BGRA with alpha.
- `--format jpg` (alias `jpeg`) — lossy, alpha dropped to 24bpp BGR.

For JPEG, `--quality <1-100>` sets the encode quality (default `90`);
`--quality` is rejected with `--format png`. The GUI exposes a PNG/JPG
combobox (the CLI default quality applies); the output-file extension follows
the selected format.
