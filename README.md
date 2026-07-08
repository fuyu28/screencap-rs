# screencap-rs

Rust rewrite of [screencap](https://github.com/fuyu28/screencap) focused on
Windows.Graphics.Capture (WGC) screenshots.

## Binaries

| exe | crate | description |
| --- | --- | --- |
| `screencap.exe` | `screencap-gui` | WGC window-picker GUI. Spawns `screencap-cli.exe` (placed next to it) for the actual capture. |
| `screencap-cli.exe` | `screencap-cli` | WGC screenshot CLI: `cap` / `list windows` / `list monitors`, JSON output, global hotkey. |

## Build (Windows)

```
cargo build --release
```

Binaries land in `target/x86_64-pc-windows-msvc/release/`
(the default target is pinned in `.cargo/config.toml`, which also lets
`cargo check` type-check on non-Windows hosts).

## Capture methods

- `wgc-window`, `wgc-monitor` (Windows.Graphics.Capture, ContentSize-cropped)

GDI/PrintWindow and DXGI output duplication paths are not included in this
variant. Recording is also intentionally out of scope.
