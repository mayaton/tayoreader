# TayoReader

Reeder ライクな Mac 向け RSS リーダー（開発中）。
現在は core ライブラリと CLI のみ。

## 開発

```sh
cargo test --all
cargo run -p tayoreader-cli -- --help
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
