# 正式リリース前のコード読解ガイド

正式リリース前は、全ファイルを同じ深さで追うのではなく、外部契約から
実装の境界へ順に読む。目的は実装の細部を暗記することではなく、今回の変更が
既存の契約、不変条件、Windows API の寿命管理を壊していないと説明できる状態に
すること。

各段階では、対応する `#[cfg(test)] mod tests` も続けて読む。テストは現行仕様の
固定点であり、リリース前に意図を確認する最短経路になる。

## 読む順番

| 順番 | ファイル | 確認すること |
| --- | --- | --- |
| 1 | [`README.md`](../README.md) の Embedding | `--json` の形、終了コード 0/1/2、カーソル既定値、出力形式。ここが同梱ホスト向けの正典。 |
| 2 | [`AGENTS.md`](../AGENTS.md) の「守るべき不変条件・設計判断」 | 過去に reject された案と、変更してはいけない境界。README と実装の間で判断に迷ったときはここを優先する。 |
| 3 | [`types.rs`](../crates/screencap-core/src/types.rs) | 共有データ構造、`ErrorInfo`、`ImageFormat`、`ImageBuffer`。後続のコードで値がどの形で受け渡されるかを先に把握する。 |
| 4 | [`cli.rs`](../crates/screencap-cli/src/cli.rs) | CLI の受理範囲と検証タイミング。特にキャプチャ方式の allowlist、形式と品質の組み合わせ、引数エラーが exit code 2 になる境界。 |
| 5 | [`run.rs`](../crates/screencap-cli/src/run.rs) | 実行の主経路。`pre_parse_bootstrap`、`run`、`run_cap`、ターゲット解決、リトライ、成功・失敗 JSON の組み立てを追う。グローバルフラグは clap 定義と bootstrap の二重実装が一致しているか確認する。 |
| 6 | [`crop.rs`](../crates/screencap-core/src/crop.rs)、[`image_stats.rs`](../crates/screencap-core/src/image_stats.rs)、[`util.rs`](../crates/screencap-core/src/util.rs) | OS 非依存の純粋ロジック。境界値、矩形計算、パス検証、統計値をテストと一緒に確認する。`image_stats.rs` の二つのループはホットパスのため統合しない。 |
| 7 | [`window_enum.rs`](../crates/screencap-core/src/window_enum.rs)、[`monitor_enum.rs`](../crates/screencap-core/src/monitor_enum.rs)、[`d3d11_copy.rs`](../crates/screencap-core/src/d3d11_copy.rs) | Windows 境界での列挙、座標、D3D11 相互運用。取得したハンドルやテクスチャがどこで所有・解放されるかを確認する。 |
| 8 | [`capture_wgc.rs`](../crates/screencap-core/src/capture_wgc.rs) | WGC のキャプチャループ。方式が `wgc-window` / `wgc-monitor` に限られること、最初の利用可能フレームの扱い、カーソル除外時だけの API 呼び出し、`session` と `frame_pool` の Close が単一箇所にあることを確認する。 |
| 9 | [`encode_png.rs`](../crates/screencap-core/src/encode_png.rs) | WIC エンコード、出力先、上書き、JPEG 品質。`ImageBuffer.row_pitch == width * 4` を前提に保存していることと、PNG 経路のバイト互換を確認する。 |
| 10 | [`gui.rs`](../crates/screencap-gui/src/gui.rs) | GUI から sibling の CLI を呼ぶ経路。CLI 引数の組み立て、JSON 失敗応答の表示、GUI の既定値と CLI の既定値が一致しているかを確認する。 |

## 読み方

各段階で次の三点を短くメモする。

- 外部に見える契約: 引数、JSON、終了コード、出力ファイル、GUI の既定値。
- 内部の不変条件: 型、座標系、バッファ、リソース寿命、エラー変換。
- 変更時の危険箇所: 既存テストが守る境界と、`AGENTS.md` に記録された reject 済み判断。

疑問が出た箇所は呼び出し元・呼び出し先へ一段だけ移動して解消する。最初から
Win32/WGC API の詳細へ潜ると、CLI 契約とのつながりを見失いやすい。まず `run.rs`
で「どの条件でどの core API を呼ぶか」を押さえてから、Windows 境界を読む。

## リリース前の確認

コードを読み終えたら、少なくとも次を確認する。

- README の Embedding 契約と `run.rs` の JSON 形状テストが一致している。
- CLI の検証エラーと実行時エラーが、意図した exit code と出力先に分かれている。
- CLI と GUI で `--cursor`、形式、出力先、エラー表示の既定値が矛盾していない。
- WGC の Close、D3D11/WIC のリソース、出力バッファの行ピッチに変更漏れがない。
- 変更した公開面があるなら、バージョン、CHANGELOG、README、同梱ホストへの影響を確認する。

ローカルでは Windows 向けターゲットのためテストをリンク・実行できない。次を実行し、
その後に Windows CI の `cargo test` が green であることを確認する。

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```
