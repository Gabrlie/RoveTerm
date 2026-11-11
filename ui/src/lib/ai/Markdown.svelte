<script lang="ts">
  // Renders an agent's Markdown as plain elements and text nodes, never as HTML.
  // Links show their address on hover rather than navigating the WebView.
  import { parseMarkdown, type Block, type Inline } from './markdown';
  import { writeClipboard } from '../ui/clipboard';
  import Icon from '../ui/Icon.svelte';
  import { t } from '../i18n';

  interface Props {
    text: string;
    /** Puts a code block's text into the user's terminal; offered on shell code when given. */
    oninsert?: (code: string) => void;
  }

  let { text, oninsert }: Props = $props();
  /** The code block just copied, by its path in the tree. */
  let copied = $state<string | null>(null);

  const SHELL = new Set(['sh', 'bash', 'zsh', 'shell', 'console', 'fish', 'powershell', 'ps1']);

  function copy(code: string, path: string): void {
    void writeClipboard(code).then(() => {
      copied = path;
      setTimeout(() => (copied = copied === path ? null : copied), 1500);
    });
  }
  const blocks = $derived(parseMarkdown(text));
</script>

{#snippet inline(nodes: Inline[])}
  {#each nodes as node, index (index)}
    {#if node.kind === 'text'}{node.text}{:else if node.kind === 'code'}<code>{node.text}</code>{:else if node.kind === 'strong'}<strong>{@render inline(node.children)}</strong>{:else if node.kind === 'em'}<em>{@render inline(node.children)}</em>{:else if node.kind === 'del'}<del>{@render inline(node.children)}</del>{:else if node.kind === 'break'}<br />{:else}<span class="link" title={node.href}>{@render inline(node.children)}</span>{/if}
  {/each}
{/snippet}

{#snippet blockList(items: Block[], path: string)}
  {#each items as block, index (index)}
    {@const at = `${path}.${index}`}
    {#if block.kind === 'heading'}
      <p class="heading h{Math.min(block.level, 3)}">{@render inline(block.content)}</p>
    {:else if block.kind === 'paragraph'}
      <p>{@render inline(block.content)}</p>
    {:else if block.kind === 'code'}
      <div class="code">
        <div class="code-head">
          <span class="lang">{block.lang || 'text'}</span>
          <span class="spacer"></span>
          {#if oninsert && SHELL.has(block.lang.toLowerCase())}
            <button type="button" title={t('ai.insertHint')} onclick={() => oninsert?.(block.text)}><Icon name="terminal" size={12} />{t('ai.insert')}</button>
          {/if}
          <button type="button" onclick={() => copy(block.text, at)}><Icon name={copied === at ? 'check' : 'copy'} size={12} />{copied === at ? t('ai.copied') : t('edit.copy')}</button>
        </div>
        <pre><code>{block.text}</code></pre>
      </div>
    {:else if block.kind === 'list'}
      <!-- Markers are drawn as their own column: WebKit's list markers can come apart from a long
           item's text while a reply streams in. -->
      <svelte:element this={block.ordered ? 'ol' : 'ul'} start={block.ordered ? block.start : undefined}>
        {#each block.items as item, itemAt (itemAt)}
          <li>
            <span class="marker" aria-hidden="true">
              {#if item.task !== null}<input type="checkbox" checked={item.task} disabled />{:else if block.ordered}{block.start + itemAt}.{:else}•{/if}
            </span>
            <div class="item">{@render blockList(item.blocks, `${at}.${itemAt}`)}</div>
          </li>
        {/each}
      </svelte:element>
    {:else if block.kind === 'quote'}
      <blockquote>{@render blockList(block.blocks, at)}</blockquote>
    {:else if block.kind === 'table'}
      <div class="table">
        <table>
          <thead>
            <tr>{#each block.header as cell, column (column)}<th style:text-align={block.align[column] ?? undefined}>{@render inline(cell)}</th>{/each}</tr>
          </thead>
          <tbody>
            {#each block.rows as row, rowAt (rowAt)}
              <tr>{#each row as cell, column (column)}<td style:text-align={block.align[column] ?? undefined}>{@render inline(cell)}</td>{/each}</tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <hr />
    {/if}
  {/each}
{/snippet}

<div class="markdown selectable">
  {@render blockList(blocks, 'b')}
</div>

<style>
  .markdown {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    line-height: 1.5;
    overflow-wrap: anywhere;
  }
  p,
  ul,
  ol,
  pre,
  blockquote,
  hr {
    margin: 0;
  }
  .heading {
    font-weight: 600;
  }
  .heading.h1 {
    font-size: 1.15em;
  }
  ul,
  ol {
    padding-left: var(--space-1);
    list-style: none;
  }
  li {
    display: grid;
    grid-template-columns: minmax(14px, auto) minmax(0, 1fr);
    column-gap: 6px;
    align-items: baseline;
  }
  li + li {
    margin-top: 2px;
  }
  .marker {
    justify-self: end;
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .marker input {
    margin: 0;
  }
  /* A list item's blocks: tight items read as one line, loose ones keep their spacing. Plain
     block flow: WebKit can set the bullet on a line of its own beside a flex container. */
  .item {
    display: block;
    min-width: 0;
  }
  .item > :global(* + *) {
    margin-top: var(--space-1);
  }
  del {
    color: var(--fg-muted);
  }
  .table {
    max-width: 100%;
    overflow-x: auto;
    border-radius: var(--radius-sm);
    box-shadow: 0 0 0 1px var(--separator);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-footnote);
  }
  th,
  td {
    padding: 4px var(--space-2);
    text-align: left;
    vertical-align: top;
    overflow-wrap: normal;
    word-break: normal;
  }
  th {
    background: var(--fill);
    font-weight: 600;
    white-space: nowrap;
  }
  tr + tr td,
  tbody tr:first-child td {
    box-shadow: inset 0 1px 0 var(--separator);
  }
  blockquote {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  code {
    overflow-wrap: anywhere;
    padding: 0 3px;
    border-radius: 4px;
    background: var(--fill);
    font-family: var(--font-mono);
    font-size: 0.92em;
  }
  .code {
    border-radius: var(--radius-sm);
    background: var(--fill);
    overflow: hidden;
  }
  .code-head {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: 2px var(--space-1) 2px var(--space-3);
    box-shadow: inset 0 -1px 0 var(--separator);
    color: var(--fg-muted);
    font-size: 11px;
  }
  .code-head .spacer {
    flex: 1;
  }
  .code-head button {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 6px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  .code-head button:hover {
    background: var(--fill-hover);
    color: var(--fg);
  }
  pre {
    padding: var(--space-2) var(--space-3);
    overflow-x: auto;
  }
  pre code {
    padding: 0;
    background: none;
    white-space: pre;
  }
  blockquote {
    padding-left: var(--space-3);
    box-shadow: inset 2px 0 0 var(--separator);
    color: var(--fg-muted);
  }
  hr {
    border: none;
    height: 1px;
    background: var(--separator);
  }
  .link {
    color: var(--accent);
    text-decoration: underline;
    text-decoration-color: color-mix(in srgb, var(--accent) 40%, transparent);
  }
</style>
