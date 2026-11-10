<script lang="ts">
  // Renders the notifications of `toasts` at the top-right corner, newest at the bottom.
  import Icon from './Icon.svelte';
  import { toasts, type ToastKind } from './toast.svelte';
  import { t } from '../i18n';
  import type { IconName } from './icons';

  const ICONS: Record<ToastKind, IconName> = { info: 'info', success: 'check', error: 'warning' };
</script>

<div class="toaster" aria-live="polite">
  {#each toasts.items as toast (toast.id)}
    <div
      class="toast {toast.kind}"
      role={toast.kind === 'error' ? 'alert' : 'status'}
      onpointerenter={() => toasts.hold(toast.id)}
      onpointerleave={() => toasts.release(toast.id)}
    >
      <span class="glyph"><Icon name={ICONS[toast.kind]} size={15} /></span>
      <p class="message" class:selectable={toast.kind === 'error'}>{toast.message}</p>
      <button type="button" class="close" aria-label={t('common.close')} onclick={() => toasts.dismiss(toast.id)}><Icon name="close" size={12} /></button>
    </div>
  {/each}
</div>

<style>
  .toaster {
    position: fixed;
    top: 46px;
    right: var(--space-4);
    z-index: 50;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--space-2);
    width: min(360px, calc(100vw - 2 * var(--space-4)));
    pointer-events: none;
  }
  .toast {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    max-width: 100%;
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--material-popover);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: var(--glass-edge), var(--shadow-md);
    color: var(--fg);
    font-size: var(--text-footnote);
    pointer-events: auto;
    animation: toast-in var(--dur-slow) var(--ease);
  }
  @keyframes toast-in {
    from {
      opacity: 0;
      transform: translateX(16px);
    }
  }
  .glyph {
    display: flex;
    padding-top: 1px;
    color: var(--accent);
  }
  .success .glyph {
    color: var(--success);
  }
  .error .glyph {
    color: var(--danger);
  }
  .message {
    flex: 1;
    min-width: 0;
    margin: 0;
    line-height: 1.4;
    overflow-wrap: anywhere;
  }
  .close {
    display: grid;
    place-items: center;
    flex: none;
    width: 18px;
    height: 18px;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    cursor: pointer;
  }
  .close:hover {
    background: var(--fill-hover);
    color: var(--fg);
  }
</style>
