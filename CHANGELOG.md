# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
starting with v1.0.0. Pre-1.0 releases used minor bumps for breaking CLI / default
behavior changes.

## [Unreleased]

## [0.4.1] - 2026-07-19

### Added

- GUI: Window / Monitor target switching, with monitor pick from the ListView
  (same interaction as windows) and selection preserved across Refresh.
- GUI: after a successful capture, advance the output filename so the next save
  does not overwrite.
- MIT license; CHANGELOG; CLI smoke tests and release asset checksums in CI.

### Fixed

- WGC: detach `FrameArrived` on all capture-loop exits (avoids leaking the
  handler when the loop returns early).
- Force-alpha walk uses `row_pitch` instead of flat chunks (correct for padded
  frames).
- Reject `--virtual-screen` at parse time; accept only `wgc-window` /
  `wgc-monitor` as `--method` values.
- GUI: pass `--no-log` to the CLI child and surface CLI JSON error messages.

## [0.4.0] - 2026-07-11

### Breaking

- Captures **exclude the mouse cursor by default**. Pass `--cursor` (GUI: "Include
  cursor" checkbox) to include it. Cursor exclusion requires Windows 10 1903
  (build 18362) or later; `--cursor` works on any WGC-capable build.

### Added

- Release binaries are built with a **static CRT** (no Visual C++ runtime
  dependency). A standalone `screencap-cli-<tag>-windows-x86_64.exe` asset is
  published alongside the zip for hosts that bundle only the CLI.
- `--version` prints `screencap-cli <version>` with no filesystem side effects
  (bare `--version` / `--help` no longer create `./logs`).
- `--no-log` disables file logging entirely.
- `--json` output shapes and exit codes are documented as a stable contract in
  README's Embedding section, anchored by shape tests.

## [0.3.0] - 2026-07-11

### Breaking

- **`wgc-window2` / `wgc-monitor2` method aliases removed.** They were exact
  aliases of `wgc-window` / `wgc-monitor` (legacy names from the pre-port
  comparison tool). Using one now fails with an error naming the replacement.

### Added

- **JPEG output**: `--format jpg` (alias `jpeg`) with `--quality 1-100`
  (default 90, JPEG only). PNG stays the default and is byte-identical to
  previous releases. The GUI gains a PNG/JPG dropdown; the filename extension
  and save dialog follow the selection.

### Fixed

- Output-path validation is consistent across CLI and GUI: invalid characters,
  no-file-name paths (`C:\`, trailing separators), and directory targets all
  fail with clear messages instead of opaque WIC errors.

### Changed

- Migrated to Rust edition 2024 (`rust-version` 1.85).
- Format knowledge centralized in an `ImageFormat` enum.

## [0.2.0] - 2026-07-10

### Breaking

- **GDI/DXGI capture backends removed — WGC only.** Methods
  `gdi-printwindow`, `gdi-bitblt-client`, `gdi-bitblt-windowdc`,
  `gdi-bitblt-screen`, `dxgi-monitor`, and `dxgi-window` are gone. Supported
  methods at the time: `wgc-window`, `wgc-window2`, `wgc-monitor`,
  `wgc-monitor2`. Non-WGC `--method` values fail with a validation error.
- GUI method dropdown offered `wgc-window` / `wgc-window2` only.

### Fixed

- Success output (CLI JSON / log / GUI) reports the file actually written,
  resolving real on-disk casing.
- Forward slashes in output paths are normalized and work.
- Missing output directory is reported clearly
  (`output directory does not exist: ...`) instead of an opaque WIC error
  (`0x80070003`).

### Changed

- Backend dispatch scaffolding removed along with the backends.
- Headless unit tests and CI (fmt / clippy / test) on every push and PR.

## [0.1.1] - 2026-07-10

### Fixed

- Capture correctness and input-validation fixes.

### Changed

- Remove needless allocations and redundant work.
- Consolidate logging; trim WGC step-trace logging.
- Resolve remaining clippy warnings.

## [0.1.0] - 2026-07-08

### Added

- Initial release: WGC-based `screencap-cli` and Win32 GUI (`screencap`).

### Note

A parallel `v0.1.0-wgc-only` tag shipped the same day as a WGC-only derivative
of the then-multi-backend tree; that line was later absorbed by v0.2.0.

[Unreleased]: https://github.com/fuyu28/screencap-rs/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/fuyu28/screencap-rs/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/fuyu28/screencap-rs/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/fuyu28/screencap-rs/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/fuyu28/screencap-rs/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/fuyu28/screencap-rs/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/fuyu28/screencap-rs/releases/tag/v0.1.0
