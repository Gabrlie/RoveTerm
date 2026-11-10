<script lang="ts">
  // Renders an agent's Markdown as plain elements and text nodes, never as HTML.
  // Links show their address on hover rather than navigating the WebView.
  import { parseMarkdown, type Inline } from './markdown';

  let { text }: { text: string } = $props();
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
      <pre><code>{block.text}</code></pre>
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
    padding: 0 3px;
    border-radius: 4px;
    background: var(--fill);
    font-family: var(--font-mono);
    font-size: 0.92em;
  }
  pre {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--fill);
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
