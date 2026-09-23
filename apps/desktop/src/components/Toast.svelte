<script lang="ts">
  import type { Reader } from "../lib/stores.svelte";

  let { reader }: { reader: Reader } = $props();
</script>

<div class="toasts" aria-live="polite">
  {#each reader.toasts as toast (toast.id)}
    <button class="toast {toast.kind}" tabindex="-1" onclick={() => reader.dismissToast(toast.id)}>
      {toast.message}
    </button>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 16px;
    bottom: 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 420px;
  }
  .toast {
    padding: 8px 14px;
    border-radius: 8px;
    background: var(--bg-sidebar);
    border: 1px solid var(--border);
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.15);
    overflow-wrap: anywhere;
  }
  .toast.error {
    border-color: var(--error);
    color: var(--error);
  }
</style>
