<script lang="ts" module>
  /** Open dialogs, innermost last; Escape closes only the innermost one. */
  const stack: symbol[] = [];
</script>

<script lang="ts">
  // Modal sheet. The first focusable field receives focus so typing works without a click; closing
  // returns focus to whatever had it before (usually a terminal). Escape is handled on the window,
  // so it still works after the focused element inside the dialog disappears.
  import { onMount, type Snippet } from 'svelte';

  interface Props {
    title: string;
    onclose: () => void;
    children: Snippet;
    actions?: Snippet;
    width?: number;
  }

  let { title, onclose, children, actions, width = 440 }: Props = $props();
  let panel: HTMLDivElement;
  const id = Symbol('dialog');

  onMount(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    stack.push(id);
    const first = panel.querySelector<HTMLElement>('input, select, textarea, button:not(.close)');
    first?.focus();
    return () => {
      stack.splice(stack.indexOf(id), 1);
      if (previous?.isConnected) previous.focus();
    };
  });

  function onKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape' || stack.at(-1) !== id || event.defaultPrevented) return;
    event.preventDefault();
    onclose();
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="backdrop" role="presentation">
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
    background: var(--overlay);
    backdrop-filter: blur(3px);
    -webkit-backdrop-filter: blur(3px);
    animation: fade var(--dur) var(--ease-out);
  }
  .dialog {
    max-height: 88vh;
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-lg);
    background: var(--material-popover);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: var(--glass-edge), var(--shadow-lg);
    outline: none;
    animation: rise var(--dur-slow) var(--ease);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(6px) scale(0.97);
    }
  }
  header {
    padding: var(--space-5) var(--space-5) var(--space-2);
    font-size: var(--text-title);
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .body {
    padding: var(--space-2) var(--space-5);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-4) var(--space-5) var(--space-5);
  }
  .dialog :global(label) {
    display: flex;
    flex-direction: column;
    gap: 5px;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    font-weight: 500;
  }
  .dialog :global(textarea) {
    font-family: var(--font-mono);
    font-size: var(--text-footnote);
    resize: vertical;
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
    font-weight: normal;
  }
  .dialog :global(.error) {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--text-footnote);
  }
</style>
