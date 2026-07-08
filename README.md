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

- `gdi-printwindow`, `gdi-bitblt-client`, `gdi-bitblt-windowdc`, `gdi-bitblt-screen`
- `dxgi-monitor`, `dxgi-window` (output duplication)
- `wgc-window`, `wgc-monitor` (Windows.Graphics.Capture, ContentSize-cropped)

See the original repo's `docs/capture-investigation.md` for the method
comparison notes.
