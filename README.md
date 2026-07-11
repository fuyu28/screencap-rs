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

## Cursor

Captures exclude the mouse cursor by default. Pass `--cursor` to include it.
The GUI exposes an "Include cursor" checkbox (unchecked by default) that maps
to the same flag.

> **Breaking change (v0.4.0):** earlier versions included the cursor. Captures
> now omit it unless `--cursor` (or the GUI checkbox) is set. Setting the cursor
> visibility requires Windows 10 version 2004 (build 19041) or later.

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
| `1` | capture / runtime failure — for `cap`, the failure JSON is printed on stdout even without `--json` |
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

Same shape as the success JSON with `ok: false`; `window`/`monitor`/`crop`/
`image_stats` are `null`, and `error` is
`{ "message": "...", "where": "...", "hresult": ..., "win32_error": ... }`
(`hresult`/`win32_error` present only when known).

### Version pinning

Query the bundled build with `screencap-cli.exe --version` (prints
`screencap-cli <version>`, exit 0, no filesystem side effects) to pin/verify
the CLI from the host app.

### Runtime dependency

Since v0.4.0, release binaries are built with a static CRT
(`-C target-feature=+crt-static`), so they do not require the Visual C++
runtime redistributable on the target machine.
