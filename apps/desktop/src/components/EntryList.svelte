<script lang="ts">
  import { onMount } from "svelte";
  import { relativeTime } from "../lib/format";
  import { selectionTitle, type Reader } from "../lib/stores.svelte";

  let { reader }: { reader: Reader } = $props();

  let list: HTMLElement | undefined = $state();
  let now = $state(Math.floor(Date.now() / 1000));

  // 選択中の記事が見えるようにスクロールする
  $effect(() => {
    const id = reader.selectedId;
    if (id !== null) list?.querySelector(`[data-id="${id}"]`)?.scrollIntoView({ block: "nearest" });
  });

  // 表示対象が変わったら先頭に戻す
  $effect(() => {
    void reader.selection;
    void reader.unreadOnly;
    if (list) list.scrollTop = 0;
  });

  function onScroll(): void {
    if (list && list.scrollTop + list.clientHeight >= list.scrollHeight - 200) void reader.loadMore();
  }

  onMount(() => {
    const timer = setInterval(() => (now = Math.floor(Date.now() / 1000)), 60_000);
    return () => clearInterval(timer);
  });
</script>

<section class="entries">
  <header>
    <h2>{selectionTitle(reader.sidebar, reader.selection, reader.unreadOnly)}</h2>
    <button
      class="toggle"
      tabindex="-1"
      title="未読のみ / すべて（u）"
      disabled={reader.selection.kind === "starred"}
      onclick={() => reader.toggleUnreadOnly()}
    >
      {reader.unreadOnly && reader.selection.kind !== "starred" ? "未読のみ" : "すべて"}
    </button>
  </header>
  <ul bind:this={list} onscroll={onScroll}>
    {#each reader.entries as entry (entry.id)}
      <li>
        <button
          class="row"
          class:selected={entry.id === reader.selectedId}
          class:read={entry.isRead}
          data-id={entry.id}
          tabindex="-1"
          onclick={() => reader.selectEntry(entry.id)}
        >
          <span class="title">
            <span class="dot" aria-label={entry.isRead ? "既読" : "未読"}></span>
            {#if entry.isStarred}<span class="star">★</span>{/if}
            {entry.title}
          </span>
          <span class="meta">{entry.feedTitle} · {relativeTime(entry.publishedAt, now)}</span>
          <span class="snippet">{entry.snippet}</span>
        </button>
      </li>
    {:else}
      <li class="empty">記事はありません</li>
    {/each}
  </ul>
</section>

<style>
  .entries {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-list);
    border-right: 1px solid var(--border);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 15px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .toggle {
    font-size: 12px;
    color: var(--fg-muted);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1px 8px;
    flex-shrink: 0;
  }
  ul {
    flex: 1;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: 8px 14px;
    border-bottom: 1px solid var(--border);
  }
  .row.selected {
    background: var(--bg-selected);
    color: var(--fg-selected);
  }
  .title {
    font-weight: 600;
    display: flex;
    gap: 6px;
    align-items: baseline;
  }
  .read .title {
    font-weight: 400;
  }
  .dot {
    flex-shrink: 0;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    transform: translateY(-1px);
  }
  .read .dot {
    visibility: hidden;
  }
  .star {
    color: var(--accent);
  }
  .selected .star {
    color: inherit;
  }
  .meta,
  .snippet {
    font-size: 12px;
    color: var(--fg-muted);
  }
  .selected .meta,
  .selected .snippet {
    color: inherit;
    opacity: 0.85;
  }
  .snippet {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .empty {
    padding: 24px;
    text-align: center;
    color: var(--fg-muted);
  }
</style>
