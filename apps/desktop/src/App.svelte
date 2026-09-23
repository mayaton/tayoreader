<script lang="ts">
  import { onMount } from "svelte";
  import { ask, open } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { api, errorMessage, onMenuImportOpml, onRefreshProgress } from "./lib/api";
  import { actionFor, type Action } from "./lib/keyboard";
  import { AUTO_REFRESH_MINUTES, Reader, markAllReadMessage } from "./lib/stores.svelte";
  import AddFeedDialog from "./components/AddFeedDialog.svelte";
  import ArticleView from "./components/ArticleView.svelte";
  import EntryList from "./components/EntryList.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import Toast from "./components/Toast.svelte";

  const reader = new Reader(api);
  let articlePane: HTMLElement | undefined = $state();
  let searchInput: HTMLInputElement | undefined = $state();

  // プラグイン呼び出しなどの失敗をトーストに出す。
  async function guard(task: () => Promise<unknown>): Promise<void> {
    try {
      await task();
    } catch (e) {
      reader.toast("error", errorMessage(e));
    }
  }

  async function importOpml(): Promise<void> {
    const path = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "OPML", extensions: ["opml", "xml"] }],
    });
    if (typeof path === "string") await reader.importOpml(path);
  }

  async function openOriginal(): Promise<void> {
    const url = reader.article?.url;
    if (url) await openUrl(url);
  }

  async function confirmMarkAllRead(): Promise<void> {
    // 確認ダイアログを開く前にガード。ダイアログ表示中に更新が始まった場合の
    // 二重チェックは markAllRead 側（reader.refreshing の確認）に任せる。
    if (reader.refreshing) {
      reader.toast("info", "更新中はすべて既読にできません");
      return;
    }
    const ok = await ask(markAllReadMessage(reader.query, reader.searchCount), {
      title: "すべて既読にする",
      kind: "warning",
      okLabel: "既読にする",
      cancelLabel: "キャンセル",
    });
    if (ok) await reader.markAllRead();
  }

  // 本文を 1 画面分スクロールする。下端で下に送ったら次の記事へ。
  async function scrollArticle(direction: 1 | -1): Promise<void> {
    const pane = articlePane;
    if (!pane) return;
    const atEnd = pane.scrollTop + pane.clientHeight >= pane.scrollHeight - 4;
    if (direction === 1 && atEnd) {
      await reader.next();
      return;
    }
    pane.scrollBy({ top: direction * pane.clientHeight * 0.85, behavior: "smooth" });
  }

  const handlers: Record<Action, () => Promise<unknown>> = {
    next: () => reader.next(),
    prev: () => reader.prev(),
    scrollDown: () => scrollArticle(1),
    scrollUp: () => scrollArticle(-1),
    nextSource: () => reader.nextSource(),
    prevSource: () => reader.prevSource(),
    toggleRead: () => reader.toggleRead(),
    toggleStar: () => reader.toggleStar(),
    openOriginal,
    toggleUnreadOnly: () => reader.toggleUnreadOnly(),
    refresh: () => reader.refresh(),
    addFeed: async () => (reader.addDialogOpen = true),
    markAllRead: confirmMarkAllRead,
    closeDialog: async () => (reader.addDialogOpen = false),
    focusSearch: async () => {
      searchInput?.focus();
      searchInput?.select();
    },
    // 検索欄から離れ、一覧のキー操作に戻る
    submitSearch: async () => {
      searchInput?.blur();
      await reader.applySearch();
    },
    clearSearch: async () => {
      searchInput?.blur();
      await reader.clearSearch();
    },
  };

  function onKeydown(event: KeyboardEvent): void {
    const action = actionFor(
      {
        key: event.key,
        shiftKey: event.shiftKey,
        metaKey: event.metaKey,
        ctrlKey: event.ctrlKey,
        altKey: event.altKey,
        // WebKit は変換を確定する Enter の isComposing が false になるため keyCode 229 も見る
        composing: event.isComposing || event.keyCode === 229,
        target: event.target as HTMLInputElement | null,
      },
      { dialogOpen: reader.addDialogOpen },
    );
    if (!action || (action === "closeDialog" && !reader.addDialogOpen)) return;
    // 購読取得中はダイアログを閉じさせない（結果を見失わないため）
    if (action === "closeDialog" && reader.addFeedBusy) return;
    // 本文中のリンクにフォーカスした状態の Enter は、ブラウザ標準のリンク処理
    // （ArticleView の click ハンドラ）に任せる。ここで元記事を開くと二重に反応する
    if (action === "openOriginal" && event.key === "Enter" && (event.target as HTMLElement)?.tagName === "A") {
      return;
    }
    event.preventDefault();
    void guard(handlers[action]);
  }

  onMount(() => {
    const unlisten = [
      onRefreshProgress((p) => reader.onRefreshProgress(p)),
      onMenuImportOpml(() => void guard(importOpml)),
    ];
    void reader.init().then(() => reader.refresh({ quiet: true }));
    const timer = setInterval(
      () => void reader.refresh({ quiet: true }),
      AUTO_REFRESH_MINUTES * 60_000,
    );
    return () => {
      clearInterval(timer);
      unlisten.forEach((p) => void p.then((stop) => stop()));
    };
  });
</script>

<svelte:window onkeydown={onKeydown} />

<div class="app">
  <Sidebar {reader} />
  <EntryList {reader} bind:searchInput />
  <ArticleView {reader} bind:pane={articlePane} />
</div>

{#if reader.addDialogOpen}
  <AddFeedDialog {reader} />
{/if}
<Toast {reader} />

<style>
  .app {
    display: grid;
    grid-template-columns: 220px 340px minmax(0, 1fr);
    height: 100%;
  }
</style>
