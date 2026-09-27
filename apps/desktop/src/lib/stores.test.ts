import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Api, Article, EntrySummary, Selection, SidebarView } from "./api";
import { searchTerms } from "./search";
import {
  PAGE_SIZE,
  Reader,
  SEARCH_DEBOUNCE_MS,
  adjustUnread,
  markAllReadMessage,
  refreshMessage,
  searchSummary,
  selectionTitle,
  sidebarItems,
} from "./stores.svelte";

function entry(id: number, extra: Partial<EntrySummary> = {}): EntrySummary {
  return {
    id,
    feedId: 1,
    feedTitle: "A",
    title: `title ${id}`,
    snippet: "",
    publishedAt: 1000 - id,
    isRead: false,
    isStarred: false,
    ...extra,
  };
}

function article(e: EntrySummary): Article {
  return {
    id: e.id,
    title: e.title,
    url: `https://a.example/${e.id}`,
    author: null,
    feedTitle: e.feedTitle,
    publishedAt: e.publishedAt,
    html: "<p>body</p>",
    isRead: e.isRead,
    isStarred: e.isStarred,
  };
}

// フィード 1（Tech と News）とフィード 2（フォルダなし）。
function sidebarView(): SidebarView {
  return {
    totalUnread: 5,
    starred: 1,
    folders: [
      { name: "News", unread: 3, feeds: [{ id: 1, title: "A", unread: 3, error: null }] },
      {
        name: "Tech",
        unread: 3,
        feeds: [{ id: 1, title: "A", unread: 3, error: null }],
      },
    ],
    unfiled: [{ id: 2, title: "B", unread: 2, error: null }],
  };
}

// DB 代わりの記事配列から一覧を返すモック。検索語はタイトルだけで照合する。
function mockApi(rows: EntrySummary[]) {
  const matches = (sel: Selection, unreadOnly: boolean, query: string | null, e: EntrySummary) => {
    const title = e.title.toLowerCase();
    const found = searchTerms(query ?? "").every((t) => title.includes(t.toLowerCase()));
    if (sel.kind === "starred") return e.isStarred && found;
    return (!unreadOnly || !e.isRead) && (sel.kind !== "feed" || e.feedId === sel.id) && found;
  };
  const api = {
    sidebar: vi.fn(async () => sidebarView()),
    listEntries: vi.fn(
      async (sel: Selection, unreadOnly: boolean, query: string | null, offset: number, limit: number) =>
        rows
          .filter((e) => matches(sel, unreadOnly, query, e))
          .slice(offset, offset + limit)
          .map((e) => ({ ...e })),
    ),
    countEntries: vi.fn(
      async (sel: Selection, unreadOnly: boolean, query: string | null) =>
        rows.filter((e) => matches(sel, unreadOnly, query, e)).length,
    ),
    getArticle: vi.fn(async (id: number) => article(rows.find((e) => e.id === id)!)),
    setRead: vi.fn(async (ids: number[], read: boolean) => {
      rows.filter((e) => ids.includes(e.id)).forEach((e) => (e.isRead = read));
      return ids.length;
    }),
    setStarred: vi.fn(async (ids: number[], starred: boolean) => {
      rows.filter((e) => ids.includes(e.id)).forEach((e) => (e.isStarred = starred));
      return ids.length;
    }),
    markAllRead: vi.fn(async () => {
      rows.forEach((e) => (e.isRead = true));
      return rows.length;
    }),
    refresh: vi.fn(async () => ({ newEntries: 0, failed: [] as { url: string; message: string }[] })),
    addFeed: vi.fn(async () => ({ feedId: 9, title: "New", newEntries: 1 })),
    removeFeed: vi.fn(async () => null),
    importOpml: vi.fn(async () => ({ added: 3, merged: 1 })),
    listFolders: vi.fn(async () => ["News", "Tech"]),
  } satisfies Api;
  return api;
}

describe("Reader", () => {
  let rows: EntrySummary[];
  let api: ReturnType<typeof mockApi>;
  let reader: Reader;

  beforeEach(async () => {
    rows = [entry(1), entry(2), entry(3, { feedId: 2 })];
    api = mockApi(rows);
    reader = new Reader(api);
    await reader.init();
  });

  it("起動時にサイドバーと未読一覧を読み、記事は自動で選ばない", () => {
    expect(reader.sidebar?.totalUnread).toBe(5);
    expect(reader.entries.map((e) => e.id)).toEqual([1, 2, 3]);
    expect(api.listEntries).toHaveBeenCalledWith({ kind: "all" }, true, null, 0, PAGE_SIZE);
    expect(api.countEntries).not.toHaveBeenCalled();
    expect(reader.selectedId).toBeNull();
    expect(reader.article).toBeNull();
  });

  it("記事を選ぶと本文を読み、既読にして未読数を減らす", async () => {
    await reader.selectEntry(1);
    expect(reader.article?.id).toBe(1);
    expect(api.setRead).toHaveBeenCalledWith([1], true);
    expect(reader.entries[0].isRead).toBe(true);
    expect(reader.article?.isRead).toBe(true);
    const view = reader.sidebar!;
    expect(view.totalUnread).toBe(4);
    expect(view.folders.map((f) => f.unread)).toEqual([2, 2]);
    expect(view.folders[0].feeds[0].unread).toBe(2);
    expect(view.unfiled[0].unread).toBe(2);
  });

  it("既読の記事を選んでも set_read を呼ばない", async () => {
    rows[0].isRead = true;
    await reader.select({ kind: "all" });
    await reader.toggleUnreadOnly();
    await reader.selectEntry(1);
    expect(api.setRead).not.toHaveBeenCalled();
  });

  it("連打で後から選んだ記事だけを表示する", async () => {
    let release!: () => void;
    const slow = new Promise<void>((r) => (release = r));
    api.getArticle.mockImplementationOnce(async (id: number) => {
      await slow;
      return article(rows.find((e) => e.id === id)!);
    });
    const first = reader.selectEntry(1);
    await reader.selectEntry(2);
    release();
    await first;
    expect(reader.selectedId).toBe(2);
    expect(reader.article?.id).toBe(2);
    expect(api.setRead).toHaveBeenCalledTimes(1);
    expect(api.setRead).toHaveBeenCalledWith([2], true);
  });

  it("CLI 側で消えた記事を選んだらエラーを出し、画面は壊さない", async () => {
    api.getArticle.mockRejectedValueOnce({ kind: "NotFound", message: "entry 1 not found" });
    await reader.selectEntry(1);
    expect(reader.toasts).toMatchObject([{ kind: "error", message: "entry 1 not found" }]);
    expect(reader.article).toBeNull();
    expect(api.setRead).not.toHaveBeenCalled();
    await reader.next();
    expect(reader.article?.id).toBe(2);
  });

  it("A を表示中に選び直した B が NotFound で失敗したら、selectedId は B のまま本文だけ消す", async () => {
    await reader.selectEntry(1);
    expect(reader.article?.id).toBe(1);
    api.getArticle.mockRejectedValueOnce({ kind: "NotFound", message: "entry 2 not found" });
    await reader.selectEntry(2);
    // selectedId は最新の選択（B）のまま。A の本文を残さず消し、o/Enter が A の URL を開かないようにする
    expect(reader.selectedId).toBe(2);
    expect(reader.article).toBeNull();
    // 未読数がずれている可能性があるのでサイドバーを読み直す（起動時の 1 回 + ここでの 1 回）
    expect(api.sidebar).toHaveBeenCalledTimes(2);
    expect(reader.toasts).toMatchObject([{ kind: "error", message: "entry 2 not found" }]);
  });

  it("未読のみ表示中に既読にした記事は、選択を変えるまで一覧に残す", async () => {
    await reader.selectEntry(1);
    await reader.next();
    expect(reader.entries.map((e) => e.id)).toEqual([1, 2, 3]);
    expect(reader.selectedId).toBe(2);

    await reader.select({ kind: "all" });
    expect(reader.entries.map((e) => e.id)).toEqual([3]);
  });

  it("j / k で前後の記事へ移る。未選択なら先頭を選ぶ", async () => {
    await reader.next();
    expect(reader.selectedId).toBe(1);
    await reader.next();
    await reader.next();
    await reader.next();
    expect(reader.selectedId).toBe(3);
    await reader.prev();
    expect(reader.selectedId).toBe(2);
  });

  it("m で既読・未読を切り替え、未読数も戻す", async () => {
    await reader.selectEntry(1);
    await reader.toggleRead();
    expect(api.setRead).toHaveBeenLastCalledWith([1], false);
    expect(reader.entries[0].isRead).toBe(false);
    expect(reader.sidebar?.totalUnread).toBe(5);
  });

  it("s でスターを切り替え、スター数を動かす", async () => {
    await reader.selectEntry(1);
    await reader.toggleStar();
    expect(api.setStarred).toHaveBeenCalledWith([1], true);
    expect(reader.entries[0].isStarred).toBe(true);
    expect(reader.article?.isStarred).toBe(true);
    expect(reader.sidebar?.starred).toBe(2);
  });

  it("すべて既読にしても一覧からは消さない", async () => {
    await reader.markAllRead();
    expect(api.markAllRead).toHaveBeenCalledWith({ kind: "all" }, null);
    expect(reader.entries.every((e) => e.isRead)).toBe(true);
    expect(reader.entries).toHaveLength(3);
    expect(api.sidebar).toHaveBeenCalledTimes(2);
  });

  it("更新中は「すべて既読」を実行せずトーストを出す", async () => {
    let release!: () => void;
    api.refresh.mockImplementationOnce(
      () => new Promise((r) => (release = () => r({ newEntries: 0, failed: [] }))),
    );
    const running = reader.refresh({ quiet: true });
    expect(reader.refreshing).not.toBeNull();

    await reader.markAllRead();
    expect(api.markAllRead).not.toHaveBeenCalled();
    expect(reader.entries.every((e) => e.isRead)).toBe(false);
    expect(reader.toasts).toMatchObject([{ kind: "info", message: "更新中はすべて既読にできません" }]);

    release();
    await running;
  });

  describe("追加読み込み", () => {
    beforeEach(async () => {
      rows.splice(0, rows.length, ...Array.from({ length: PAGE_SIZE + 5 }, (_, i) => entry(i + 1)));
      await reader.select({ kind: "all" });
    });

    it("200 件ずつ読み、足りなくなったら止める", async () => {
      expect(reader.entries).toHaveLength(PAGE_SIZE);
      expect(reader.hasMore).toBe(true);
      await reader.loadMore();
      expect(reader.entries).toHaveLength(PAGE_SIZE + 5);
      expect(reader.hasMore).toBe(false);
      await reader.loadMore();
      expect(api.listEntries).toHaveBeenCalledTimes(3);
    });

    it("一覧に残した既読記事の分だけ offset を減らし、取りこぼさない", async () => {
      await reader.selectEntry(1);
      await reader.selectEntry(2);
      await reader.loadMore();
      expect(api.listEntries).toHaveBeenLastCalledWith(
        { kind: "all" },
        true,
        null,
        PAGE_SIZE - 2,
        PAGE_SIZE,
      );
      const ids = reader.entries.map((e) => e.id);
      expect(ids).toHaveLength(PAGE_SIZE + 5);
      expect(new Set(ids).size).toBe(ids.length);
    });

    it("スター表示中に外した記事は一覧に残し、offset からは除く", async () => {
      rows.slice(0, PAGE_SIZE + 1).forEach((e) => (e.isStarred = true));
      await reader.select({ kind: "starred" });
      expect(api.listEntries).toHaveBeenLastCalledWith({ kind: "starred" }, true, null, 0, PAGE_SIZE);
      await reader.selectEntry(1);
      await reader.toggleStar();
      expect(reader.entries[0]).toMatchObject({ id: 1, isStarred: false });
      await reader.loadMore();
      expect(api.listEntries).toHaveBeenLastCalledWith(
        { kind: "starred" },
        true,
        null,
        PAGE_SIZE - 1,
        PAGE_SIZE,
      );
      expect(reader.entries.map((e) => e.id)).toEqual(
        Array.from({ length: PAGE_SIZE + 1 }, (_, i) => i + 1),
      );
    });

    it("末尾で j を押すと次のページを読んで進む", async () => {
      await reader.selectEntry(PAGE_SIZE);
      await reader.next();
      expect(reader.selectedId).toBe(PAGE_SIZE + 1);
    });
  });

  describe("更新", () => {
    it("完了後にサイドバーと一覧を読み直し、選択中の記事は残す", async () => {
      await reader.selectEntry(1);
      rows.unshift(entry(0));
      api.refresh.mockResolvedValueOnce({ newEntries: 1, failed: [] });
      await reader.refresh();
      expect(api.sidebar).toHaveBeenCalledTimes(2);
      // 既読になった 1 は DB 上は未読一覧から外れるが、選択中なので残す
      expect(reader.entries.map((e) => e.id)).toEqual([0, 1, 2, 3]);
      expect(reader.selectedId).toBe(1);
      expect(reader.article?.id).toBe(1);
      expect(reader.toasts.map((t) => t.message)).toEqual(["1 件の新着"]);
      expect(reader.refreshing).toBeNull();
    });

    it("失敗があればエラーのトーストを出す", async () => {
      api.refresh.mockResolvedValueOnce({
        newEntries: 2,
        failed: [{ url: "https://x.example/", message: "request timed out" }],
      });
      await reader.refresh();
      expect(reader.toasts).toMatchObject([{ kind: "error", message: "2 件の新着、1 件のフィードで失敗" }]);
    });

    it("自動更新（quiet）はトーストを出さない", async () => {
      api.refresh.mockResolvedValueOnce({ newEntries: 2, failed: [{ url: "u", message: "m" }] });
      await reader.refresh({ quiet: true });
      expect(reader.toasts).toEqual([]);
    });

    it("更新中の再要求は無視し、Busy エラーも表に出さない", async () => {
      let release!: () => void;
      api.refresh.mockImplementationOnce(
        () => new Promise((r) => (release = () => r({ newEntries: 0, failed: [] }))),
      );
      const running = reader.refresh({ quiet: true });
      expect(reader.refreshing).toEqual({ done: 0, total: 0 });
      reader.onRefreshProgress({ done: 1, total: 3 });
      expect(reader.refreshing).toEqual({ done: 1, total: 3 });
      await reader.refresh();
      expect(api.refresh).toHaveBeenCalledTimes(1);
      release();
      await running;

      api.refresh.mockRejectedValueOnce({ kind: "Busy", message: "refresh is already running" });
      await reader.refresh({ quiet: true });
      expect(reader.toasts).toEqual([]);
      reader.onRefreshProgress({ done: 1, total: 1 });
      expect(reader.refreshing).toBeNull();
    });

    it("その他のエラーはトーストに出す", async () => {
      api.refresh.mockRejectedValueOnce({ kind: "Db", message: "database error: locked" });
      await reader.refresh({ quiet: true });
      expect(reader.toasts).toMatchObject([{ kind: "error", message: "database error: locked" }]);
    });
  });

  describe("購読", () => {
    it("追加に成功したらダイアログを閉じ、追加したフィードを選ぶ", async () => {
      reader.addDialogOpen = true;
      await reader.addFeed("https://new.example/feed", ["Tech"]);
      expect(api.addFeed).toHaveBeenCalledWith("https://new.example/feed", ["Tech"]);
      expect(reader.addDialogOpen).toBe(false);
      expect(reader.selection).toEqual({ kind: "feed", id: 9 });
    });

    it("追加の失敗は呼び出し元に投げ、ダイアログは閉じない", async () => {
      reader.addDialogOpen = true;
      api.addFeed.mockRejectedValueOnce({ kind: "Parse", message: "failed to parse feed: x" });
      await expect(reader.addFeed("https://bad.example/", [])).rejects.toMatchObject({ kind: "Parse" });
      expect(reader.addDialogOpen).toBe(true);
    });

    it("選択中のフィードを解除したら「すべて」に戻る", async () => {
      await reader.select({ kind: "feed", id: 2 });
      await reader.removeFeed(2);
      expect(api.removeFeed).toHaveBeenCalledWith(2);
      expect(reader.selection).toEqual({ kind: "all" });
    });

    it("OPML を取り込んだら結果を出して全体を更新する", async () => {
      await reader.importOpml("/tmp/subs.opml");
      expect(api.importOpml).toHaveBeenCalledWith("/tmp/subs.opml");
      expect(reader.toasts[0].message).toBe("3 件を追加、1 件は購読済み");
      expect(api.refresh).toHaveBeenCalledTimes(1);
    });

    it("OPML の取り込みに失敗したら更新しない", async () => {
      api.importOpml.mockRejectedValueOnce({ kind: "Opml", message: "invalid OPML: x" });
      await reader.importOpml("/tmp/bad.opml");
      expect(reader.toasts[0]).toMatchObject({ kind: "error", message: "invalid OPML: x" });
      expect(api.refresh).not.toHaveBeenCalled();
    });

    it("更新中に OPML を取り込んだら、次回の更新に任せる旨をトーストで出すだけにする", async () => {
      let release!: () => void;
      api.refresh.mockImplementationOnce(
        () => new Promise((r) => (release = () => r({ newEntries: 0, failed: [] }))),
      );
      const running = reader.refresh({ quiet: true });
      expect(reader.refreshing).not.toBeNull();

      await reader.importOpml("/tmp/subs.opml");
      expect(reader.toasts.map((t) => t.message)).toEqual([
        "3 件を追加、1 件は購読済み",
        "更新中のため、追加したフィードは次回の更新で取得されます",
      ]);
      expect(api.refresh).toHaveBeenCalledTimes(1);

      release();
      await running;
    });
  });

  describe("検索", () => {
    beforeEach(async () => {
      rows.splice(
        0,
        rows.length,
        entry(1, { title: "台風情報" }),
        entry(2, { title: "天気予報" }),
        entry(3, { feedId: 2, title: "台風の進路" }),
      );
      await reader.select({ kind: "all" });
      api.listEntries.mockClear();
      api.countEntries.mockClear();
    });

    async function search(text: string): Promise<void> {
      reader.setSearchText(text);
      await reader.applySearch();
    }

    it("入力が止まってから 1 回だけ検索し、件数も取る", async () => {
      vi.useFakeTimers();
      try {
        reader.setSearchText("台");
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS - 1);
        reader.setSearchText("台風");
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS - 1);
        expect(api.listEntries).not.toHaveBeenCalled();
        await vi.advanceTimersByTimeAsync(1);
        expect(api.listEntries).toHaveBeenCalledTimes(1);
        expect(api.listEntries).toHaveBeenCalledWith({ kind: "all" }, true, "台風", 0, PAGE_SIZE);
        expect(api.countEntries).toHaveBeenCalledWith({ kind: "all" }, true, "台風");
        expect(reader.query).toBe("台風");
        expect(reader.entries.map((e) => e.id)).toEqual([1, 3]);
        expect(reader.searchCount).toBe(2);
      } finally {
        vi.useRealTimers();
      }
    });

    it("前後の空白だけが変わったときや空白だけのときは読み直さない", async () => {
      await search("   ");
      expect(reader.query).toBe("");
      await search("台風");
      await search(" 台風　");
      expect(api.listEntries).toHaveBeenCalledTimes(1);
    });

    it("検索を解除すると全件を読み直し、件数を消す", async () => {
      await search("台風");
      await reader.clearSearch();
      expect(reader.searchText).toBe("");
      expect(reader.query).toBe("");
      expect(reader.searchCount).toBeNull();
      expect(api.listEntries).toHaveBeenLastCalledWith({ kind: "all" }, true, null, 0, PAGE_SIZE);
      expect(reader.entries).toHaveLength(3);
    });

    it("サイドバーの選択を変えても検索語を保つ", async () => {
      await search("台風");
      await reader.select({ kind: "feed", id: 2 });
      expect(reader.query).toBe("台風");
      expect(api.listEntries).toHaveBeenLastCalledWith(
        { kind: "feed", id: 2 },
        true,
        "台風",
        0,
        PAGE_SIZE,
      );
      expect(api.countEntries).toHaveBeenLastCalledWith({ kind: "feed", id: 2 }, true, "台風");
      expect(reader.entries.map((e) => e.id)).toEqual([3]);
      expect(reader.searchCount).toBe(1);
    });

    it("すべて既読は検索結果だけを対象にする", async () => {
      await search("台風");
      await reader.markAllRead();
      expect(api.markAllRead).toHaveBeenCalledWith({ kind: "all" }, "台風");
    });

    it("更新後の再取得でも検索語を渡し、件数を取り直す", async () => {
      await search("台風");
      rows.unshift(entry(0, { title: "新しい台風" }));
      await reader.refresh({ quiet: true });
      expect(api.listEntries).toHaveBeenLastCalledWith({ kind: "all" }, true, "台風", 0, PAGE_SIZE);
      expect(reader.entries.map((e) => e.id)).toEqual([0, 1, 3]);
      expect(reader.searchCount).toBe(3);
    });

    it("古い検索の応答で一覧と件数を上書きしない", async () => {
      let release!: () => void;
      const slow = new Promise<void>((r) => (release = r));
      api.listEntries.mockImplementationOnce(async () => {
        await slow;
        return [entry(9, { title: "台" })];
      });
      api.countEntries.mockImplementationOnce(async () => {
        await slow;
        return 99;
      });
      reader.setSearchText("台");
      const first = reader.applySearch();
      await search("台風の");
      release();
      await first;
      expect(reader.query).toBe("台風の");
      expect(reader.entries.map((e) => e.id)).toEqual([3]);
      expect(reader.searchCount).toBe(1);
    });

    it("追加読み込みでも検索語を渡す", async () => {
      rows.splice(
        0,
        rows.length,
        ...Array.from({ length: PAGE_SIZE + 5 }, (_, i) => entry(i + 1, { title: `台風 ${i + 1}` })),
      );
      await search("台風");
      expect(reader.entries).toHaveLength(PAGE_SIZE);
      await reader.loadMore();
      expect(api.listEntries).toHaveBeenLastCalledWith(
        { kind: "all" },
        true,
        "台風",
        PAGE_SIZE,
        PAGE_SIZE,
      );
      expect(reader.entries).toHaveLength(PAGE_SIZE + 5);
    });
  });

  it("] / [ でサイドバーの項目を巡る", async () => {
    await reader.nextSource();
    expect(reader.selection).toEqual({ kind: "starred" });
    await reader.nextSource();
    expect(reader.selection).toEqual({ kind: "folder", name: "News" });
    await reader.prevSource();
    await reader.prevSource();
    expect(reader.selection).toEqual({ kind: "all" });
    await reader.prevSource();
    expect(reader.selection).toEqual({ kind: "all" });
  });

  it("エラーのトーストは 10 秒で消える", async () => {
    vi.useFakeTimers();
    try {
      reader.toast("error", "boom");
      vi.advanceTimersByTime(9_999);
      expect(reader.toasts).toHaveLength(1);
      vi.advanceTimersByTime(1);
      expect(reader.toasts).toHaveLength(0);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("純粋関数", () => {
  it("sidebarItems は複数フォルダのフィードを 1 回だけ並べる", () => {
    expect(sidebarItems(sidebarView())).toEqual([
      { kind: "all" },
      { kind: "starred" },
      { kind: "folder", name: "News" },
      { kind: "feed", id: 1 },
      { kind: "folder", name: "Tech" },
      { kind: "feed", id: 2 },
    ]);
    expect(sidebarItems(null)).toEqual([{ kind: "all" }, { kind: "starred" }]);
  });

  it("adjustUnread は 0 未満にしない", () => {
    const view = sidebarView();
    adjustUnread(view, 2, -5);
    expect(view.unfiled[0].unread).toBe(0);
    expect(view.totalUnread).toBe(0);
    adjustUnread(view, 99, -1);
    expect(view.totalUnread).toBe(0);
  });

  it("selectionTitle", () => {
    const view = sidebarView();
    expect(selectionTitle(view, { kind: "all" }, true)).toBe("すべての未読");
    expect(selectionTitle(view, { kind: "all" }, false)).toBe("すべての記事");
    expect(selectionTitle(view, { kind: "starred" }, true)).toBe("スター");
    expect(selectionTitle(view, { kind: "folder", name: "Tech" }, true)).toBe("Tech");
    expect(selectionTitle(view, { kind: "feed", id: 2 }, true)).toBe("B");
  });

  it("searchSummary", () => {
    expect(searchSummary("台風", 3)).toBe('"台風" の検索結果（3 件）');
    expect(searchSummary("台風", null)).toBe('"台風" の検索結果');
  });

  it("markAllReadMessage", () => {
    expect(markAllReadMessage("", null)).toBe("表示中の記事をすべて既読にしますか？");
    expect(markAllReadMessage("台風", 2)).toBe("検索結果の 2 件をすべて既読にしますか？");
    expect(markAllReadMessage("台風", null)).toBe("検索結果をすべて既読にしますか？");
  });

  it("refreshMessage", () => {
    expect(refreshMessage({ newEntries: 0, failed: [] })).toBe("0 件の新着");
  });
});
