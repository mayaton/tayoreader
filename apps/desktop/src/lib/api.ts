// Tauri コマンドの型付きラッパー。型は src-tauri/src/view.rs と手で対応させる。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Selection =
  | { kind: "all" }
  | { kind: "starred" }
  | { kind: "folder"; name: string }
  | { kind: "feed"; id: number };

export type FeedItem = { id: number; title: string; unread: number; error: string | null };
export type FolderItem = { name: string; unread: number; feeds: FeedItem[] };
export type SidebarView = {
  totalUnread: number;
  starred: number;
  folders: FolderItem[];
  unfiled: FeedItem[];
};
export type EntrySummary = {
  id: number;
  feedId: number;
  feedTitle: string;
  title: string;
  snippet: string;
  publishedAt: number;
  isRead: boolean;
  isStarred: boolean;
};
export type Article = {
  id: number;
  title: string;
  url: string | null;
  author: string | null;
  feedTitle: string;
  publishedAt: number;
  html: string;
  isRead: boolean;
  isStarred: boolean;
};
export type RefreshSummary = { newEntries: number; failed: { url: string; message: string }[] };
export type RefreshProgress = { done: number; total: number };
export type AddedFeed = { feedId: number; title: string; newEntries: number };
export type ImportSummary = { added: number; merged: number };
export type CommandError = { kind: string; message: string };

export interface Api {
  sidebar(): Promise<SidebarView>;
  // query は検索語（null なら検索しない）。
  listEntries(
    selection: Selection,
    unreadOnly: boolean,
    query: string | null,
    offset: number,
    limit: number,
  ): Promise<EntrySummary[]>;
  countEntries(selection: Selection, unreadOnly: boolean, query: string | null): Promise<number>;
  getArticle(id: number): Promise<Article>;
  setRead(ids: number[], read: boolean): Promise<number>;
  setStarred(ids: number[], starred: boolean): Promise<number>;
  markAllRead(selection: Selection, query: string | null): Promise<number>;
  refresh(feedId?: number): Promise<RefreshSummary>;
  addFeed(url: string, folders: string[]): Promise<AddedFeed>;
  removeFeed(id: number): Promise<null>;
  importOpml(path: string): Promise<ImportSummary>;
  listFolders(): Promise<string[]>;
}

export const api: Api = {
  sidebar: () => invoke("sidebar"),
  listEntries: (selection, unreadOnly, query, offset, limit) =>
    invoke("list_entries", { selection, unreadOnly, query, offset, limit }),
  countEntries: (selection, unreadOnly, query) =>
    invoke("count_entries", { selection, unreadOnly, query }),
  getArticle: (id) => invoke("get_article", { id }),
  setRead: (ids, read) => invoke("set_read", { ids, read }),
  setStarred: (ids, starred) => invoke("set_starred", { ids, starred }),
  markAllRead: (selection, query) => invoke("mark_all_read", { selection, query }),
  refresh: (feedId) => invoke("refresh", { feedId: feedId ?? null }),
  addFeed: (url, folders) => invoke("add_feed", { url, folders }),
  removeFeed: (id) => invoke("remove_feed", { id }),
  importOpml: (path) => invoke("import_opml", { path }),
  listFolders: () => invoke("list_folders"),
};

export function onRefreshProgress(cb: (p: RefreshProgress) => void): Promise<UnlistenFn> {
  return listen<RefreshProgress>("refresh-progress", (e) => cb(e.payload));
}

export function onMenuImportOpml(cb: () => void): Promise<UnlistenFn> {
  return listen("menu-import-opml", () => cb());
}

export function isCommandError(e: unknown): e is CommandError {
  return typeof e === "object" && e !== null && "kind" in e && "message" in e;
}

// 例外を画面に出す文字列にする。
export function errorMessage(e: unknown): string {
  if (isCommandError(e)) return e.message;
  return e instanceof Error ? e.message : String(e);
}
