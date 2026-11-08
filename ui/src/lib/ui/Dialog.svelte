<script lang="ts">
  // Modal sheet. The first focusable field receives focus so typing works without a click.
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    onclose: () => void;
    children: Snippet;
    actions?: Snippet;
    width?: number;
  }

  let { title, onclose, children, actions, width = 440 }: Props = $props();
  let panel: HTMLDivElement;

  $effect(() => {
    const first = panel.querySelector<HTMLElement>('input, select, textarea, button:not(.close)');
    first?.focus();
  });

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.stopPropagation();
      onclose();
    }
  }
</script>

<div class="backdrop" role="presentation" onkeydown={onKeydown}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" style:width="min({width}px, 92vw)" bind:this={panel}>
    <header>{title}</header>
    <div class="body">{@render children()}</div>
    {#if actions}
      <footer>{@render actions()}</footer>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--bg-sunken) 55%, transparent);
  }
  .dialog {
    max-height: 88vh;
    display: flex;
    flex-direction: column;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: 0 16px 40px rgb(0 0 0 / 0.2);
    outline: none;
  }
  header {
    padding: var(--space-4) var(--space-4) var(--space-2);
    font-size: var(--text-title);
    font-weight: 600;
  }
  .body {
    padding: var(--space-2) var(--space-4);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4) var(--space-4);
  }
  .dialog :global(label) {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .dialog :global(input),
  .dialog :global(select),
  .dialog :global(textarea) {
    padding: 6px var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg);
    color: var(--fg);
    font: inherit;
  }
  .dialog :global(textarea) {
    font-family: var(--font-mono);
    font-size: var(--text-footnote);
    resize: vertical;
  }
  .dialog :global(input:focus),
  .dialog :global(select:focus),
  .dialog :global(textarea:focus) {
    outline: 2px solid color-mix(in srgb, var(--accent) 60%, transparent);
    outline-offset: -1px;
  }
  .dialog :global(.row) {
    display: flex;
    gap: var(--space-3);
  }
  .dialog :global(.row > label) {
    flex: 1;
  }
  .dialog :global(.check) {
    flex-direction: row;
    align-items: center;
    gap: var(--space-2);
    color: var(--fg);
  }
  .dialog :global(.error) {
    margin: 0;
    color: var(--danger);
    font-size: var(--text-footnote);
  }
  .dialog :global(button.primary),
  .dialog :global(button.secondary) {
    min-width: 76px;
    padding: 6px var(--space-4);
    border-radius: 999px;
    font: inherit;
    cursor: pointer;
  }
  .dialog :global(button.primary) {
    border: none;
    background: var(--accent);
    color: #fff;
  }
  .dialog :global(button.primary.danger) {
    background: var(--danger);
  }
  .dialog :global(button.secondary) {
    border: 1px solid var(--border);
    background: transparent;
    color: var(--fg);
  }
  .dialog :global(button:disabled) {
    opacity: 0.5;
    cursor: default;
  }
</style>
