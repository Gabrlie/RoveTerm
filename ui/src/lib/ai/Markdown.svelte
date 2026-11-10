<script lang="ts">
  // Renders an agent's Markdown as plain elements and text nodes, never as HTML.
  // Links show their address on hover rather than navigating the WebView.
  import { parseMarkdown, type Inline } from './markdown';

  import Icon from '../ui/Icon.svelte';
  import { t } from '../i18n';

  interface Props {
    text: string;
    /** Puts a code block's text into the user's terminal; offered on shell code when given. */
    oninsert?: (code: string) => void;
  }

  let { text, oninsert }: Props = $props();
  let copied = $state<number | null>(null);

  const SHELL = new Set(['sh', 'bash', 'zsh', 'shell', 'console', 'fish', 'powershell', 'ps1']);

  function copy(code: string, index: number): void {
    void navigator.clipboard.writeText(code).then(() => {
      copied = index;
      setTimeout(() => (copied = copied === index ? null : copied), 1500);
    });
  }
  const blocks = $derived(parseMarkdown(text));
</script>

{#snippet inline(nodes: Inline[])}
  {#each nodes as node, index (index)}
    {#if node.kind === 'text'}{node.text}{:else if node.kind === 'code'}<code>{node.text}</code>{:else if node.kind === 'strong'}<strong>{@render inline(node.children)}</strong>{:else if node.kind === 'em'}<em>{@render inline(node.children)}</em>{:else}<span class="link" title={node.href}>{@render inline(node.children)}</span>{/if}
  {/each}
{/snippet}

<div class="markdown selectable">
  {#each blocks as block, index (index)}
    {#if block.kind === 'heading'}
      <p class="heading">{@render inline(block.content)}</p>
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
          <button type="button" onclick={() => copy(block.text, index)}><Icon name={copied === index ? 'check' : 'copy'} size={12} />{copied === index ? t('ai.copied') : t('edit.copy')}</button>
        </div>
        <pre><code>{block.text}</code></pre>
      </div>
    {:else if block.kind === 'list'}
      {#if block.ordered}
        <ol>{#each block.items as item, at (at)}<li>{@render inline(item)}</li>{/each}</ol>
      {:else}
        <ul>{#each block.items as item, at (at)}<li>{@render inline(item)}</li>{/each}</ul>
      {/if}
    {:else if block.kind === 'quote'}
      <blockquote>{@render inline(block.content)}</blockquote>
    {:else}
      <hr />
    {/if}
  {/each}
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
  ul,
  ol {
    padding-left: var(--space-5);
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
