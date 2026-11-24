<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { viewport } from '../layout/viewport.svelte';
  import { handleBack, pushBackHandler } from './backStack';

  interface Props {
    title: string;
    onclose: () => void;
    children: Snippet;
    actions?: Snippet;
    width?: number;
    autofocus?: boolean;
  }

  let { title, onclose, children, actions, width = 440, autofocus }: Props = $props();
  let panel: HTMLDivElement;

  onMount(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const disposeBack = pushBackHandler(() => onclose());
    if (autofocus ?? !viewport.coarse) {
      const first = panel.querySelector<HTMLElement>('input, select, textarea, button:not(.close)');
      first?.focus();
    } else {
      panel.focus();
    }
    return () => {
      disposeBack();
      if (previous?.isConnected) previous.focus();
    };
  });

  /** Moves the dialog to <body> so overflow clipping inside panels never cuts off modal sheets. */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape' && !event.defaultPrevented && handleBack()) event.preventDefault();
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="backdrop" class:compact={viewport.compact} role="presentation" use:portal>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" style:--dialog-width="{width}px" bind:this={panel}>
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
    /* A plain dim: a small backdrop blur ghosts text on WebView2 instead of softening it. */
    background: var(--overlay);
    animation: fade var(--dur) var(--ease-out);
  }
  .dialog {
    width: min(var(--dialog-width), 92vw);
    max-height: 88vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-radius: var(--radius-lg);
    background: var(--material-popover);
    box-shadow: var(--glass-edge), var(--shadow-lg);
    outline: none;
    animation: rise var(--dur-slow) var(--ease);
  }
  .backdrop.compact {
    align-items: end;
    padding: max(var(--safe-top), 12px) var(--safe-right) max(var(--safe-bottom), var(--ime-bottom)) var(--safe-left);
  }
  .compact .dialog {
    width: 100%;
    max-width: 100%;
    max-height: calc(100dvh - max(var(--safe-top), 12px) - max(var(--safe-bottom), var(--ime-bottom)));
    border-radius: var(--radius-lg) var(--radius-lg) 0 0;
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
  @media (max-width: 520px) {
    .dialog :global(.row) {
      flex-wrap: wrap;
    }
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
