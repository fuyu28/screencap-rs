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
