<script lang="ts">
  import { onMount } from "svelte";
  import { errorMessage } from "../lib/api";
  import type { Reader } from "../lib/stores.svelte";

  let { reader }: { reader: Reader } = $props();

  let url = $state("");
  let candidates = $state<string[]>([]);
  let chosen = $state<string[]>([]);
  let newFolder = $state("");
  let error = $state<string | null>(null);
  let input: HTMLInputElement | undefined = $state();

  function toggle(folder: string): void {
    chosen = chosen.includes(folder) ? chosen.filter((f) => f !== folder) : [...chosen, folder];
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (reader.addFeedBusy || url.trim() === "") return;
    error = null;
    const folders = [...new Set([...chosen, newFolder.trim()].filter((f) => f !== ""))];
    try {
      await reader.addFeed(url.trim(), folders);
    } catch (e) {
      error = `購読できませんでした: ${errorMessage(e)}`;
    }
  }

  onMount(() => {
    input?.focus();
    reader
      .listFolders()
      .then((names) => (candidates = names))
      .catch(() => {
        // 候補が出ないだけなので無視する
      });
  });
</script>

<div
  class="backdrop"
  role="presentation"
  onclick={(e) =>
    !reader.addFeedBusy && e.target === e.currentTarget && (reader.addDialogOpen = false)}
>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="add-feed-title">
    <form onsubmit={submit}>
      <h2 id="add-feed-title">購読を追加</h2>
      <label>
        フィードの URL
        <input
          bind:this={input}
          bind:value={url}
          type="url"
          placeholder="https://example.com/feed.xml"
          required
        />
      </label>
      <fieldset>
        <legend>フォルダ（複数選べます）</legend>
        <div class="chips">
          {#each candidates as folder (folder)}
            <button
              type="button"
              class="chip"
              aria-pressed={chosen.includes(folder)}
              onclick={() => toggle(folder)}
            >
              {folder}
            </button>
          {/each}
        </div>
        <input bind:value={newFolder} placeholder="新しいフォルダ名" />
      </fieldset>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <div class="actions">
        <button
          type="button"
          disabled={reader.addFeedBusy}
          onclick={() => (reader.addDialogOpen = false)}
        >
          キャンセル
        </button>
        <button type="submit" class="primary" disabled={reader.addFeedBusy || url.trim() === ""}>
          {#if reader.addFeedBusy}<span class="spinner" aria-hidden="true"></span>取得中…{:else}購読{/if}
        </button>
      </div>
    </form>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.3);
    display: grid;
    place-items: center;
  }
  .dialog {
    width: 440px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 20px;
    box-shadow: 0 10px 40px rgb(0 0 0 / 0.25);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  h2 {
    margin: 0;
    font-size: 16px;
  }
  label,
  fieldset {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12px;
    color: var(--fg-muted);
  }
  fieldset {
    border: none;
    padding: 0;
    margin: 0;
  }
  legend {
    padding: 0;
    margin-bottom: 6px;
  }
  input {
    font: inherit;
    font-size: 14px;
    color: var(--fg);
    background: var(--bg-list);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 8px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 2px 10px;
    color: var(--fg);
  }
  .chip[aria-pressed="true"] {
    background: var(--bg-selected);
    border-color: var(--bg-selected);
    color: var(--fg-selected);
  }
  .error {
    margin: 0;
    color: var(--error);
    font-size: 13px;
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .actions button {
    padding: 5px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
  }
  .primary {
    background: var(--bg-selected);
    border-color: var(--bg-selected) !important;
    color: var(--fg-selected);
  }
  .primary:disabled {
    opacity: 0.5;
  }
  .spinner {
    display: inline-block;
    width: 10px;
    height: 10px;
    margin-right: 6px;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
