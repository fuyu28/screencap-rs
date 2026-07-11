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

- `wgc-window` (window capture)
- `wgc-monitor` (monitor capture)

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

## Embedding

`screencap-cli.exe` is designed to be bundled next to a host application and
driven programmatically. The machine contract below is stable.

### Recommended invocation

```
screencap-cli.exe cap --method wgc-window --foreground \
  --out C:\path\shot.png --json --overwrite --no-log
```

- `--json` — emit a single JSON object on stdout (see shapes below).
- `--overwrite` — replace an existing file instead of failing.
- `--no-log` — disable file logging (default is `./logs`, unwanted when embedded).
- Always pass an explicit `--out`.

### Exit codes

| code | meaning |
| --- | --- |
| `0` | capture succeeded |
| `1` | capture / runtime failure (with `--json`, the failure JSON is on stdout) |
| `2` | argument / validation error from the CLI parser (with `--json`, a failure-shaped JSON with `command: "unknown"` is on stdout; otherwise the clap message is on stderr) |

### Success JSON (`cap --json`, exit 0)

```json
{
  "ok": true,
  "command": "cap",
  "method": "wgc-window",
  "target": "window",
  "out_path": "C:\\path\\shot.png",
  "format": "png",
  "timestamp": "2026-01-01T00:00:00+09:00",
  "duration_ms": 42,
  "dpi_mode": "per-monitor-v2",
  "window": { "hwnd": 0, "pid": 0, "title": "", "class": "", "rect": {}, "visible": true, "iconic": false, "cloaked": false, "client_rect_screen": {} },
  "monitor": { "index": 0, "desktop": {}, "primary": true },
  "crop": { "mode": "none", "rect": {}, "pad": {} },
  "image_stats": { "black_ratio": 0.0, "transparent_ratio": 0.0, "avg_luma": 0.0 },
  "error": null
}
```

`window` is present only when a window was resolved; `monitor` only when a
monitor was resolved. `rect`/`desktop`/`client_rect_screen` are
`{left, top, right, bottom}`; `crop.rect` is `{x, y, w, h}`; `pad` is
`{l, t, r, b}`.

### Failure JSON (exit 1, or exit 2 with `--json`)

```json
{
  "ok": false,
  "command": "cap",
  "method": "wgc-window",
  "target": "window",
  "out_path": "C:\\path\\shot.png",
  "format": "png",
  "timestamp": "2026-01-01T00:00:00+09:00",
  "duration_ms": 0,
  "dpi_mode": "per-monitor-v2",
  "window": null,
  "monitor": null,
  "crop": null,
  "image_stats": null,
  "error": { "message": "...", "where": "...", "hresult": 2147942405, "win32_error": 5 }
}
```

`error.hresult` and `error.win32_error` are present only when known.

### Version pinning

Query the bundled build with `screencap-cli.exe --version` (prints
`screencap-cli <version>`, exit 0) to pin/verify the CLI from the host app.

### Runtime dependency

From the next release, the CLI is built with a static CRT
(`-C target-feature=+crt-static`), so it does not require the Visual C++
runtime redistributable on the target machine.
