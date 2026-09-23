<script lang="ts">
  import { Menu } from "@tauri-apps/api/menu";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { errorMessage, type FeedItem, type Selection } from "../lib/api";
  import { sameSelection, type Reader } from "../lib/stores.svelte";

  let { reader }: { reader: Reader } = $props();

  const isSelected = (s: Selection) => sameSelection(reader.selection, s);

  async function confirmRemove(feed: FeedItem): Promise<void> {
    const ok = await ask(`「${feed.title}」の購読を解除しますか？\n記事も削除されます。`, {
      title: "購読を解除",
      kind: "warning",
      okLabel: "解除する",
      cancelLabel: "キャンセル",
    });
    if (ok) await reader.removeFeed(feed.id);
  }

  async function showFeedMenu(event: MouseEvent, feed: FeedItem): Promise<void> {
    event.preventDefault();
    try {
      const menu = await Menu.new({
        items: [
          { id: "refresh", text: "このフィードを更新", action: () => void reader.refresh({ feedId: feed.id }) },
          {
            id: "remove",
            text: "購読を解除…",
            action: () => void confirmRemove(feed).catch((e) => reader.toast("error", errorMessage(e))),
          },
        ],
      });
      await menu.popup();
    } catch (e) {
      reader.toast("error", errorMessage(e));
    }
  }
</script>

{#snippet source(selection: Selection, label: string, count: number)}
  <button
    class="item"
    class:selected={isSelected(selection)}
    tabindex="-1"
    onclick={() => reader.select(selection)}
  >
    <span class="label">{label}</span>
    <span class="count">{count || ""}</span>
  </button>
{/snippet}

{#snippet feedRow(feed: FeedItem)}
  <button
    class="item feed"
    class:selected={isSelected({ kind: "feed", id: feed.id })}
    tabindex="-1"
    title={feed.title}
    onclick={() => reader.select({ kind: "feed", id: feed.id })}
    oncontextmenu={(e) => showFeedMenu(e, feed)}
  >
    <span class="label">{feed.title}</span>
    {#if feed.error}<span class="warn" title={feed.error}>⚠</span>{/if}
    <span class="count">{feed.unread || ""}</span>
  </button>
{/snippet}

<nav class="sidebar">
  <div class="scroll">
    {@render source({ kind: "all" }, "すべての未読", reader.sidebar?.totalUnread ?? 0)}
    {@render source({ kind: "starred" }, "スター", reader.sidebar?.starred ?? 0)}
    {#each reader.sidebar?.folders ?? [] as folder (folder.name)}
      <div class="folder">
        {@render source({ kind: "folder", name: folder.name }, folder.name, folder.unread)}
        {#each folder.feeds as feed (feed.id)}
          {@render feedRow(feed)}
        {/each}
      </div>
    {/each}
    {#if reader.sidebar?.unfiled.length}
      <div class="folder">
        {#each reader.sidebar.unfiled as feed (feed.id)}
          {@render feedRow(feed)}
        {/each}
      </div>
    {/if}
  </div>
  <footer>
    <button title="購読を追加（a）" tabindex="-1" onclick={() => (reader.addDialogOpen = true)}>＋</button>
    <button
      title="更新（r）"
      tabindex="-1"
      disabled={reader.refreshing !== null}
      onclick={() => reader.refresh()}
    >
      {#if reader.refreshing}
        {reader.refreshing.done}/{reader.refreshing.total}
      {:else}
        ↻
      {/if}
    </button>
  </footer>
</nav>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border);
    min-height: 0;
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 8px 0;
  }
  .folder {
    margin-top: 10px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 12px;
  }
  .feed {
    padding-left: 24px;
  }
  .folder > .item:first-child:not(.feed) {
    font-weight: 600;
  }
  .item.selected {
    background: var(--bg-selected);
    color: var(--fg-selected);
  }
  .label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    color: var(--fg-muted);
    font-size: 12px;
  }
  .selected .count {
    color: inherit;
  }
  .warn {
    color: var(--accent);
  }
  footer {
    display: flex;
    justify-content: space-between;
    padding: 6px 12px;
    border-top: 1px solid var(--border);
    font-size: 16px;
  }
  footer button:disabled {
    color: var(--fg-muted);
    font-size: 12px;
  }
</style>
