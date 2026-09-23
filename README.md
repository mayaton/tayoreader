# TayoReader

Reeder ライクな Mac 向け RSS リーダー（開発中）。
core ライブラリ・CLI・デスクトップアプリ（Tauri）からなる。

## 開発

```sh
cargo test --workspace
cargo run -p tayoreader-cli -- --help
```

デスクトップアプリは Node 26 と pnpm 12 が必要（macOS のみ）。

```sh
cd apps/desktop
pnpm install
pnpm tauri dev                                  # 開発サーバーで起動
TAYOREADER_DB=/tmp/try.db pnpm tauri dev        # 一時 DB で起動
pnpm check && pnpm test                         # svelte-check と vitest
pnpm tauri build --debug                        # target/debug/bundle/macos/TayoReader.app
```

## CLI

```sh
tayoreader add https://blog.rust-lang.org/feed.xml --folder Rust
tayoreader import-opml ~/Downloads/inoreader.opml
tayoreader refresh
tayoreader entries --unread
tayoreader read 1 2 3
```

DB は既定で OS のアプリデータディレクトリ（macOS では `~/Library/Application Support/tayoreader/tayoreader.db`）に作られる。
`--db <PATH>` で別の場所を指定できる。
デスクトップアプリも同じ DB を使う。環境変数 `TAYOREADER_DB` で別の場所を指定できる。

## デスクトップアプリのキー操作

| キー | 動作 |
|---|---|
| `j` / `k`（`↓` / `↑`） | 次 / 前の記事 |
| `Space` / `Shift+Space` | 本文をスクロール（末尾なら次の記事） |
| `]` / `[` | サイドバーの次 / 前の項目 |
| `m` / `s` | 既読・未読 / スターの切り替え |
| `o` / `Enter` | 元記事をブラウザで開く |
| `u` | 未読のみ / すべて |
| `r` | 更新 |
| `a` | 購読を追加 |
| `Shift+A` | 表示中の一覧をすべて既読 |
| `Cmd+Shift+O` | OPML を読み込む |

## ライセンス

MIT License。詳細は [LICENSE](LICENSE)。
