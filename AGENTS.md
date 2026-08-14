# screencap-rs

Windows.Graphics.Capture (WGC) ベースの Windows 専用スクリーンショットツール。
CLI (`screencap-cli.exe`) と Win32 GUI (`screencap.exe`) を提供する。
CLI は外部アプリ（cloudlaunch_go）に同梱される前提で、`--json` 出力と終了コードが
**安定契約**（README の Embedding セクションが正典、形状テストで固定済み）。

## ワークスペース構成

- `crates/screencap-core` — キャプチャ・エンコードの本体
  - `capture_wgc.rs` — WGC キャプチャループ（メソッドは `wgc-window` / `wgc-monitor` の2つのみ）
  - `encode_png.rs` — WIC による PNG/JPEG エンコード + 出力パス処理（正規化・実パス解決・親ディレクトリ検証）
  - `crop.rs` / `image_stats.rs` — 純粋ロジック（テスト厚め）
  - `window_enum.rs` / `monitor_enum.rs` — ターゲット列挙・解決
  - `types.rs` — 共有型。`ImageFormat` がフォーマット知識（`ALL` / `extension()` / `from_cli()` / `DEFAULT_JPEG_QUALITY`）を一元所有
  - `logging.rs` / `util.rs` / `d3d11_copy.rs`
- `crates/screencap-cli` — `cli.rs`（clap 定義と検証）+ `run.rs`（実行時・JSON 出力・hotkey）
- `crates/screencap-gui` — Win32 GUI。キャプチャは隣の `screencap-cli.exe` を子プロセスで叩く

## 開発環境（重要）

ホストは macOS でも開発可能。`.cargo/config.toml` がターゲットを
`x86_64-pc-windows-msvc` に固定しているため、型チェックは通る。**リンクと実行は不可**。

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings   # テストコードも型チェックされる
# cargo test はローカルでは動かない（リンク不可）。CI (windows-latest) が実行する
```

CI: `.github/workflows/ci.yml` が push(main)/PR で fmt/clippy/test を windows-latest で実行。
PR は CI green を確認してからマージする。

## リリース手順

1. `[workspace.package]` の version を bump → `cargo update --workspace` → `chore: release vX.Y.Z` コミット
2. `git tag -a vX.Y.Z -m vX.Y.Z && git push origin main vX.Y.Z`
3. `.github/workflows/release.yml` が自動で static CRT ビルドし、zip（GUI+CLI）と
   **CLI 単体 exe**（同梱用）をリリースに添付する
4. `gh release edit` でリリースノートを整える（Breaking / New / Fixes）

バージョニング: GDI/DXGI 削除(0.2.0)・エイリアス削除(0.3.0)・カーソルデフォルト変更(0.4.0) のように、
CLI 表面や既定挙動の変更は breaking としてマイナーを上げる。

## ドキュメントの役割分担

変更・レビュー時は次の4層に書き分ける（混ぜない）。

| 場所 | 書くこと | 例 |
| --- | --- | --- |
| **コード本体** | **how**（読めば分かる処理は書かない） | ループ・API 呼び出しの並び |
| **`///` / `//!`** | **what**（公開面・モジュールの責務） | `/// Validates …` / `//! WGC implementation` |
| **インライン `//`** | **why not**（採らなかった別案・落とし穴だけ） | `// ClientToScreen per-corner mirrors RTL layout; MapWindowPoints does not` |
| **テスト** | **what**（関数名 + assert のみ。`//` コメントは原則なし） | `fn rejects_unknown_wgc_prefix()` + `assert_eq!(err.message, …)` |
| **コミット body** | **why**（subject の what を補う動機。`feat`/`fix` は1行以上推奨） | `WGC has no virtual-desktop item, so parse-time rejection …` |

下記「設計判断」節は **why not の正典**（repo 横断の reject 済み判断）。ファイル内に
同趣旨の長い `//` を増やさず、ここか関数直上の短い why-not に留める。

## 守るべき不変条件・設計判断（変更前に必ず確認）

- **JSON 契約**: `--json` の成功/失敗 JSON と exit code (0/1/2) は同梱ホストが依存する。
  形状テスト（`build_cap_success_json_shape` / `build_failure_json_shape`）を壊す変更は breaking
- **`pre_parse_bootstrap`（run.rs）は意図的な二重パーサ**: ロガーを clap より先に起動し、
  parse 失敗時の JSON/ログ出力を担う。グローバルフラグ（`--log-dir` / `--log-level` / `--json` / `--no-log`）を
  増減するときは clap 側と両方更新する。削除・clap 統合の提案は既に reject 済み。
  `--version` / `--help` 単体では `./logs` を作らない（`no_log` を bootstrap で立てる）。
  `cap --help` のように argv[1] がサブコマンドの `--help` は対象外
- **カーソル**: `IsCursorCaptureEnabled` は Win10 1903/18362+（19041 は `IsBorderRequired`）。
  除外（デフォルト）時のみプロパティを設定し、`--cursor` 時は呼ばない（旧ビルド互換のため）
- **capture_wgc の session/frame_pool は単一 Close サイト**: early-return を足すときは
  `and_then` チェーンに乗せ、Close の複製を作らない
- **`ImageBuffer.row_pitch == width * 4`** が保存時の不変条件（キャプチャ経路は行単位でタイトに詰める）
- **`image_stats.rs` の2ループは意図的な性能分割**（`compute_frame_ratios` はホットパス）。統合しない
- **メソッド文字列の enum 化は reject 済み**（検証タイミングと exit code 1/2 の互換のため）
- **`wgc-` プレフィックスだけでは受理しない**（`validate_capture_method` で allowlist。
  `CaptureWithWgc` まで落とすとエラーメッセージがドキュメントとズレる）
- **`parse_log_level` は小文字のみ**（`TRACE` 等は Info にフォールバック。大小無視は reject 済み）
- **出力パスの no-file-name 判定**は Windows の `Path::file_name` セマンティクスに依存
  （`util::validate_output_path` のテストは `#[cfg(windows)]`）
- PNG 出力はバイト互換を維持する（`save_image_wic` の PNG 経路を変えるときは要注意）
- `Gdi::` / `Dxgi::` の import は WGC の D3D11 相互運用・列挙・GUI 描画に必要な正当なもの。消さない

## コード規約

- エラーは `ErrorInfo`（`with_hresult` / `to_err` / `to_err_with` + `WHERE` 定数）で統一
- ユニットテストは同ファイル内 `#[cfg(test)] mod tests`。実 FS/WIC を使うテストは
  `#[cfg(windows)]` ゲート（CI では実行される）
- コミットは conventional commits（`feat!:` = breaking）。subject は **what**、body に **why**
  を1行以上（`.github/pull_request_template.md` 参照）
- ユーザー入力で panic しない。パス系エラーは明確なメッセージで返す（`validate_output_path` 等）
