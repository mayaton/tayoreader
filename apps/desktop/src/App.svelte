<script lang="ts">
  import { onMount } from "svelte";
  import { ask, open } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { api, errorMessage, onMenuImportOpml, onRefreshProgress } from "./lib/api";
  import { actionFor, type Action } from "./lib/keyboard";
  import { AUTO_REFRESH_MINUTES, Reader } from "./lib/stores.svelte";
  import AddFeedDialog from "./components/AddFeedDialog.svelte";
  import ArticleView from "./components/ArticleView.svelte";
  import EntryList from "./components/EntryList.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import Toast from "./components/Toast.svelte";

  const reader = new Reader(api);
  let articlePane: HTMLElement | undefined = $state();

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
    const ok = await ask("表示中の記事をすべて既読にしますか？", {
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
  };

  function onKeydown(event: KeyboardEvent): void {
    const action = actionFor(
      {
        key: event.key,
        shiftKey: event.shiftKey,
        metaKey: event.metaKey,
        ctrlKey: event.ctrlKey,
        altKey: event.altKey,
        target: event.target as HTMLElement | null,
      },
      { dialogOpen: reader.addDialogOpen },
    );
    if (!action || (action === "closeDialog" && !reader.addDialogOpen)) return;
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
  <EntryList {reader} />
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
