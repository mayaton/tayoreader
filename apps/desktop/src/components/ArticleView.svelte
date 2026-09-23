<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { errorMessage } from "../lib/api";
  import { dateTime } from "../lib/format";
  import type { Reader } from "../lib/stores.svelte";

  let { reader, pane = $bindable() }: { reader: Reader; pane?: HTMLElement } = $props();

  // 記事が変わったら先頭から表示する
  $effect(() => {
    void reader.article?.id;
    if (pane) pane.scrollTop = 0;
  });

  // リンクは WebView 内で開かず、既定のブラウザに渡す。
  function onLinkClick(event: MouseEvent): void {
    const link = (event.target as Element | null)?.closest("a[href]");
    if (!link) return;
    event.preventDefault();
    const href = link.getAttribute("href");
    if (href) openUrl(href).catch((e) => reader.toast("error", errorMessage(e)));
  }
</script>

<!-- クリックはリンクの委譲処理だけ。リンクにフォーカスした状態の Enter でも click が届くため、
     App.svelte 側のグローバル Enter ハンドラは target が A タグのときは何もしない -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<article class="article" bind:this={pane} onclick={onLinkClick} onauxclick={onLinkClick}>
  {#if reader.article}
    {@const a = reader.article}
    <header>
      <h1>
        {#if a.url}<a href={a.url} title="ブラウザで開く（o）">{a.title}</a>{:else}{a.title}{/if}
      </h1>
      <p class="meta">
        {a.feedTitle}{#if a.author}&nbsp;· {a.author}{/if}&nbsp;· {dateTime(a.publishedAt)}
        {#if a.isStarred}<span class="star">★</span>{/if}
      </p>
    </header>
    <!-- html は Rust 側の sanitize::article_html を通したものだけが来る -->
    <div class="body">{@html a.html}</div>
  {:else}
    <p class="placeholder">記事を選んでください</p>
  {/if}
</article>

<style>
  .article {
    overflow-y: auto;
    padding: 24px 40px 80px;
    line-height: 1.75;
    min-height: 0;
  }
  header {
    max-width: 720px;
    margin: 0 auto 20px;
  }
  h1 {
    font-size: 22px;
    line-height: 1.4;
    margin: 0 0 6px;
  }
  h1 a {
    color: inherit;
    text-decoration: none;
  }
  h1 a:hover {
    text-decoration: underline;
  }
  .meta {
    margin: 0;
    font-size: 12px;
    color: var(--fg-muted);
  }
  .star {
    color: var(--accent);
    margin-left: 4px;
  }
  .body {
    max-width: 720px;
    margin: 0 auto;
    font-size: 15px;
    overflow-wrap: anywhere;
  }
  .body :global(img) {
    max-width: 100%;
    height: auto;
  }
  .body :global(pre) {
    overflow-x: auto;
    padding: 12px;
    background: var(--bg-list);
    border-radius: 6px;
  }
  .body :global(a) {
    color: var(--bg-selected);
  }
  .placeholder {
    margin-top: 30vh;
    text-align: center;
    color: var(--fg-muted);
  }
</style>
