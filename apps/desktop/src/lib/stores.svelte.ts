// 画面の状態と操作。Tauri への依存は Api に閉じ込め、テストではモックを渡す。
import {
  errorMessage,
  isCommandError,
  type Api,
  type Article,
  type EntrySummary,
  type RefreshProgress,
  type RefreshSummary,
  type Selection,
  type SidebarView,
} from "./api";

// 記事一覧を 1 回に読み込む件数。
export const PAGE_SIZE = 200;
// 自動更新の間隔。
export const AUTO_REFRESH_MINUTES = 30;
const TOAST_MS = { info: 4_000, error: 10_000 } as const;

export type Toast = { id: number; kind: "info" | "error"; message: string };

export function sameSelection(a: Selection, b: Selection): boolean {
  switch (a.kind) {
    case "folder":
      return b.kind === "folder" && a.name === b.name;
    case "feed":
      return b.kind === "feed" && a.id === b.id;
    default:
      return a.kind === b.kind;
  }
}

// `]` / `[` で巡る順にサイドバーの項目を並べる。複数フォルダにあるフィードは最初の 1 回だけ。
export function sidebarItems(view: SidebarView | null): Selection[] {
  const items: Selection[] = [{ kind: "all" }, { kind: "starred" }];
  if (!view) return items;
  const seen = new Set<number>();
  const pushFeed = (id: number) => {
    if (seen.has(id)) return;
    seen.add(id);
    items.push({ kind: "feed", id });
  };
  for (const folder of view.folders) {
    items.push({ kind: "folder", name: folder.name });
    folder.feeds.forEach((f) => pushFeed(f.id));
  }
  view.unfiled.forEach((f) => pushFeed(f.id));
  return items;
}

// 一覧ヘッダに出す名前。
export function selectionTitle(
  view: SidebarView | null,
  selection: Selection,
  unreadOnly: boolean,
): string {
  switch (selection.kind) {
    case "all":
      return unreadOnly ? "すべての未読" : "すべての記事";
    case "starred":
      return "スター";
    case "folder":
      return selection.name;
    case "feed": {
      const feeds = [...(view?.folders.flatMap((f) => f.feeds) ?? []), ...(view?.unfiled ?? [])];
      return feeds.find((f) => f.id === selection.id)?.title ?? "";
    }
  }
}

// サイドバーの未読数を、フィード・所属フォルダ・合計の各所で `delta` だけ動かす。
export function adjustUnread(view: SidebarView, feedId: number, delta: number): void {
  const bump = (n: number) => Math.max(0, n + delta);
  let found = false;
  for (const folder of view.folders) {
    for (const feed of folder.feeds) {
      if (feed.id === feedId) {
        feed.unread = bump(feed.unread);
        folder.unread = bump(folder.unread);
        found = true;
      }
    }
  }
  for (const feed of view.unfiled) {
    if (feed.id === feedId) {
      feed.unread = bump(feed.unread);
      found = true;
    }
  }
  if (found) view.totalUnread = bump(view.totalUnread);
}

export function refreshMessage(summary: RefreshSummary): string {
  const failed = summary.failed.length;
  return failed > 0
    ? `${summary.newEntries} 件の新着、${failed} 件のフィードで失敗`
    : `${summary.newEntries} 件の新着`;
}

// 新しい順（publishedAt、同じなら id の降順）の位置に差し込む。
function insertSorted(entries: EntrySummary[], entry: EntrySummary): void {
  const index = entries.findIndex(
    (e) =>
      e.publishedAt < entry.publishedAt ||
      (e.publishedAt === entry.publishedAt && e.id < entry.id),
  );
  entries.splice(index === -1 ? entries.length : index, 0, entry);
}

export class Reader {
  sidebar = $state<SidebarView | null>(null);
  selection = $state<Selection>({ kind: "all" });
  unreadOnly = $state(true);
  entries = $state<EntrySummary[]>([]);
  hasMore = $state(false);
  selectedId = $state<number | null>(null);
  article = $state<Article | null>(null);
  refreshing = $state<RefreshProgress | null>(null);
  toasts = $state<Toast[]>([]);
  addDialogOpen = $state(false);

  #api: Api;
  #nextToastId = 1;
  #loadingMore = false;
  // 一覧を読み直すたびに増やし、古い応答で上書きしないようにする。
  #listVersion = 0;

  constructor(api: Api) {
    this.#api = api;
  }

  get selectedEntry(): EntrySummary | undefined {
    return this.entries.find((e) => e.id === this.selectedId);
  }

  async init(): Promise<void> {
    await Promise.all([this.loadSidebar(), this.reloadEntries()]);
  }

  async loadSidebar(): Promise<void> {
    try {
      this.sidebar = await this.#api.sidebar();
    } catch (e) {
      this.#error(e);
    }
  }

  async select(selection: Selection): Promise<void> {
    this.selection = selection;
    await this.reloadEntries();
  }

  async toggleUnreadOnly(): Promise<void> {
    this.unreadOnly = !this.unreadOnly;
    await this.reloadEntries();
  }

  // 一覧を先頭から読み直す。選択は外し、先頭の記事も自動では選ばない。
  async reloadEntries(): Promise<void> {
    const version = ++this.#listVersion;
    this.entries = [];
    this.hasMore = false;
    this.selectedId = null;
    this.article = null;
    try {
      const page = await this.#api.listEntries(this.selection, this.unreadOnly, 0, PAGE_SIZE);
      if (version !== this.#listVersion) return;
      this.entries = page;
      this.hasMore = page.length === PAGE_SIZE;
    } catch (e) {
      this.#error(e);
    }
  }

  // 次のページを読む。offset は「読み込み済みで、まだ絞り込み条件に合う件数」にする。
  // 一覧に残している既読記事（未読のみ表示中）やスターを外した記事は DB 側では条件から外れているため。
  async loadMore(): Promise<void> {
    if (!this.hasMore || this.#loadingMore) return;
    this.#loadingMore = true;
    const version = this.#listVersion;
    try {
      const offset = this.entries.filter((e) => this.#matches(e)).length;
      const page = await this.#api.listEntries(this.selection, this.unreadOnly, offset, PAGE_SIZE);
      if (version !== this.#listVersion) return;
      const known = new Set(this.entries.map((e) => e.id));
      this.entries.push(...page.filter((e) => !known.has(e.id)));
      this.hasMore = page.length === PAGE_SIZE;
    } catch (e) {
      this.#error(e);
    } finally {
      this.#loadingMore = false;
    }
  }

  // 記事を選び、本文を読み込んで未読なら既読にする。
  async selectEntry(id: number): Promise<void> {
    this.selectedId = id;
    try {
      const article = await this.#api.getArticle(id);
      // 連打で後から別の記事が選ばれていたら捨てる
      if (this.selectedId !== id) return;
      this.article = article;
      if (!article.isRead) {
        await this.#api.setRead([id], true);
        this.#applyRead(id, true);
      }
    } catch (e) {
      this.#error(e);
    }
  }

  async next(): Promise<void> {
    const index = this.#selectedIndex();
    if (index + 1 >= this.entries.length) await this.loadMore();
    const target = this.entries[index + 1];
    if (target) await this.selectEntry(target.id);
  }

  async prev(): Promise<void> {
    const index = this.#selectedIndex();
    const target = this.entries[index === -1 ? 0 : index - 1];
    if (target) await this.selectEntry(target.id);
  }

  async toggleRead(): Promise<void> {
    const entry = this.selectedEntry;
    if (!entry) return;
    const read = !entry.isRead;
    try {
      await this.#api.setRead([entry.id], read);
      this.#applyRead(entry.id, read);
    } catch (e) {
      this.#error(e);
    }
  }

  async toggleStar(): Promise<void> {
    const entry = this.selectedEntry;
    if (!entry) return;
    const starred = !entry.isStarred;
    try {
      await this.#api.setStarred([entry.id], starred);
      entry.isStarred = starred;
      if (this.article?.id === entry.id) this.article.isStarred = starred;
      if (this.sidebar) this.sidebar.starred = Math.max(0, this.sidebar.starred + (starred ? 1 : -1));
    } catch (e) {
      this.#error(e);
    }
  }

  // 表示中の一覧をすべて既読にする（一覧からは消さない）。
  async markAllRead(): Promise<void> {
    try {
      await this.#api.markAllRead(this.selection);
      this.entries.forEach((e) => (e.isRead = true));
      if (this.article) this.article.isRead = true;
      await this.loadSidebar();
    } catch (e) {
      this.#error(e);
    }
  }

  // 更新する。`quiet`（自動更新）のときは結果をトーストに出さない。
  async refresh(options: { feedId?: number; quiet?: boolean } = {}): Promise<void> {
    if (this.refreshing) return;
    this.refreshing = { done: 0, total: 0 };
    try {
      const summary = await this.#api.refresh(options.feedId);
      if (!options.quiet) {
        this.toast(summary.failed.length > 0 ? "error" : "info", refreshMessage(summary));
      }
      await this.#reloadAfterRefresh();
    } catch (e) {
      // 別の更新が走っているだけなら何もしない
      if (!(isCommandError(e) && e.kind === "Busy")) this.#error(e);
    } finally {
      this.refreshing = null;
    }
  }

  onRefreshProgress(progress: RefreshProgress): void {
    if (this.refreshing) this.refreshing = progress;
  }

  // 購読を追加する。失敗は呼び出し元（ダイアログ）に投げる。
  async addFeed(url: string, folders: string[]): Promise<void> {
    const added = await this.#api.addFeed(url, folders);
    this.addDialogOpen = false;
    await this.loadSidebar();
    await this.select({ kind: "feed", id: added.feedId });
  }

  // 購読追加ダイアログのフォルダ候補。
  listFolders(): Promise<string[]> {
    return this.#api.listFolders();
  }

  async removeFeed(id: number): Promise<void> {
    try {
      await this.#api.removeFeed(id);
      await this.loadSidebar();
      if (this.selection.kind === "feed" && this.selection.id === id) {
        await this.select({ kind: "all" });
      } else {
        await this.reloadEntries();
      }
    } catch (e) {
      this.#error(e);
    }
  }

  // OPML を取り込み、続けて全体を更新する。
  async importOpml(path: string): Promise<void> {
    try {
      const r = await this.#api.importOpml(path);
      this.toast("info", `${r.added} 件を追加、${r.merged} 件は購読済み`);
      await this.loadSidebar();
    } catch (e) {
      this.#error(e);
      return;
    }
    await this.refresh();
  }

  async nextSource(): Promise<void> {
    await this.#moveSource(1);
  }

  async prevSource(): Promise<void> {
    await this.#moveSource(-1);
  }

  toast(kind: Toast["kind"], message: string): void {
    const id = this.#nextToastId++;
    this.toasts.push({ id, kind, message });
    setTimeout(() => this.dismissToast(id), TOAST_MS[kind]);
  }

  dismissToast(id: number): void {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }

  async #moveSource(step: 1 | -1): Promise<void> {
    const items = sidebarItems(this.sidebar);
    const index = items.findIndex((s) => sameSelection(s, this.selection));
    const target = items[index === -1 ? 0 : index + step];
    if (target) await this.select(target);
  }

  // 更新後にサイドバーと一覧を読み直す。選択中の記事は一覧から消えても残す。
  async #reloadAfterRefresh(): Promise<void> {
    await this.loadSidebar();
    const version = ++this.#listVersion;
    const limit = Math.max(PAGE_SIZE, this.entries.length);
    const page = await this.#api.listEntries(this.selection, this.unreadOnly, 0, limit);
    if (version !== this.#listVersion) return;
    const hasMore = page.length === limit;
    const selected = this.selectedEntry;
    if (selected && !page.some((e) => e.id === selected.id)) insertSorted(page, selected);
    this.entries = page;
    this.hasMore = hasMore;
  }

  #applyRead(id: number, read: boolean): void {
    const entry = this.entries.find((e) => e.id === id);
    if (this.article?.id === id) this.article.isRead = read;
    if (!entry) {
      void this.loadSidebar();
      return;
    }
    if (entry.isRead === read) return;
    entry.isRead = read;
    if (this.sidebar) adjustUnread(this.sidebar, entry.feedId, read ? -1 : 1);
  }

  #matches(e: EntrySummary): boolean {
    if (this.selection.kind === "starred") return e.isStarred;
    return !this.unreadOnly || !e.isRead;
  }

  #selectedIndex(): number {
    return this.entries.findIndex((e) => e.id === this.selectedId);
  }

  #error(e: unknown): void {
    this.toast("error", errorMessage(e));
  }
}
