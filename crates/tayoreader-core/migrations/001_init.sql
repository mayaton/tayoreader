CREATE TABLE folders (
  id   INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE
);

CREATE TABLE feeds (
  id              INTEGER PRIMARY KEY,
  url             TEXT NOT NULL UNIQUE,
  title           TEXT,
  site_url        TEXT,
  etag            TEXT,
  last_modified   TEXT,
  last_fetched_at INTEGER,
  last_error      TEXT,
  created_at      INTEGER NOT NULL
);

CREATE TABLE feed_folders (
  feed_id   INTEGER NOT NULL REFERENCES feeds(id)   ON DELETE CASCADE,
  folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
  PRIMARY KEY (feed_id, folder_id)
);

CREATE TABLE entries (
  id           INTEGER PRIMARY KEY,
  feed_id      INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
  guid         TEXT NOT NULL,
  url          TEXT,
  title        TEXT,
  author       TEXT,
  summary      TEXT,
  content      TEXT,
  published_at INTEGER,
  fetched_at   INTEGER NOT NULL,
  is_read      INTEGER NOT NULL DEFAULT 0,
  is_starred   INTEGER NOT NULL DEFAULT 0,
  UNIQUE (feed_id, guid)
);

CREATE INDEX entries_feed_published ON entries(feed_id, published_at);
CREATE INDEX entries_read_published ON entries(is_read, published_at);
