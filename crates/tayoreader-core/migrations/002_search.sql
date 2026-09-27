-- 記事の全文検索。FTS5 の trigram で部分一致を引く。
-- 検索対象のテキストは Rust 側（sanitize::search_text）で作り、plain_text に入れる。
ALTER TABLE entries ADD COLUMN plain_text TEXT;

CREATE VIRTUAL TABLE entries_fts USING fts5(
  title, plain_text,
  content = 'entries', content_rowid = 'id',
  tokenize = 'trigram'
);

-- 既存の記事を、いまの内容（plain_text は NULL）で索引に入れる。
-- 索引に無い行を 'delete' すると索引が壊れるため、埋め戻しの UPDATE より前に必要。
INSERT INTO entries_fts(entries_fts) VALUES ('rebuild');

CREATE TRIGGER entries_fts_ai AFTER INSERT ON entries BEGIN
  INSERT INTO entries_fts(rowid, title, plain_text) VALUES (new.id, new.title, new.plain_text);
END;

CREATE TRIGGER entries_fts_ad AFTER DELETE ON entries BEGIN
  INSERT INTO entries_fts(entries_fts, rowid, title, plain_text) VALUES ('delete', old.id, old.title, old.plain_text);
END;

-- 更新のたびに同じ内容で上書きされるため、値が変わったときだけ索引を差し替える
CREATE TRIGGER entries_fts_au AFTER UPDATE OF title, plain_text ON entries
  WHEN old.title IS NOT new.title OR old.plain_text IS NOT new.plain_text
BEGIN
  INSERT INTO entries_fts(entries_fts, rowid, title, plain_text) VALUES ('delete', old.id, old.title, old.plain_text);
  INSERT INTO entries_fts(rowid, title, plain_text) VALUES (new.id, new.title, new.plain_text);
END;
