<script lang="ts">
  import HostIcon from '../hosts/HostIcon.svelte';
  import { t } from '../i18n';
  import type { HostOs } from '../ipc';
  import Icon from '../ui/Icon.svelte';

  interface Props {
    title: string;
    os?: HostOs | null;
    status: 'connecting' | 'live' | 'failed' | 'ended';
    keyboardOpen?: boolean;
    toolsBadge?: number;
    onback: () => void;
    ondisconnect: () => void;
    onpillclick?: () => void;
    ontools: () => void;
    ontogglekeyboard: () => void;
  }

  let {
    title, os, status, keyboardOpen = false, toolsBadge = 0,
    onback, ondisconnect, onpillclick, ontools, ontogglekeyboard,
  }: Props = $props();
</script>

<div class="dock">
  <button
    type="button"
    class="dock-button"
    aria-label={t('terminal.dock.back')}
    onpointerdown={(event) => event.preventDefault()}
    onclick={onback}
  >
    <Icon name="back" size={17} />
  </button>

  <div class="session-pill">
    <button
      type="button"
      class="session"
      aria-label={`${title} · ${t(`status.state.${status}`)}`}
      title={title}
      disabled={!onpillclick}
      onpointerdown={(event) => event.preventDefault()}
      onclick={onpillclick}
    >
      {#if os}<HostIcon {os} size={15} />{/if}
      <span class="status-dot" data-status={status} aria-hidden="true"></span>
      <span class="session-title">{title}</span>
    </button>
    <button
      type="button"
      class="disconnect"
      aria-label={t('terminal.dock.disconnect')}
      onpointerdown={(event) => event.preventDefault()}
      onclick={ondisconnect}
    >
      <Icon name="close" size={17} />
    </button>
  </div>

  <button
    type="button"
    class="dock-button"
    aria-label={t('terminal.dock.tools')}
    aria-haspopup="dialog"
    onpointerdown={(event) => event.preventDefault()}
    onclick={ontools}
  >
    <Icon name="add" size={17} />
    {#if toolsBadge > 0}<span class="badge" aria-hidden="true">{toolsBadge}</span>{/if}
  </button>

  <button
    type="button"
    class="dock-button"
    class:active={keyboardOpen}
    aria-label={t('terminal.dock.keyboard')}
    aria-pressed={keyboardOpen}
    onpointerdown={(event) => event.preventDefault()}
    onclick={ontogglekeyboard}
  >
    <Icon name="keyboard" size={17} />
  </button>
</div>

<style>
  .dock {
    --dock-fill: color-mix(in srgb, var(--term-fg) 11%, var(--term-bg));
    --dock-pressed: color-mix(in srgb, var(--term-fg) 18%, var(--term-bg));
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 4px var(--space-2) 2px;
    background: var(--term-bg);
    color: var(--term-fg);
  }
  button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }
  button:active:not(:disabled) {
    background: var(--dock-pressed);
  }
  .dock-button {
    flex: none;
    position: relative;
    width: var(--m-icon-btn, 36px);
    height: 34px;
    border-radius: 9px;
    background: var(--dock-fill);
  }
  .dock-button.active {
    background: color-mix(in srgb, var(--accent) 22%, var(--term-bg));
    color: var(--term-fg);
  }
  .session-pill {
    flex: 1;
    min-width: 0;
    height: 34px;
    display: flex;
    align-items: center;
    overflow: hidden;
    border-radius: 9px;
    background: var(--dock-fill);
  }
  .session {
    flex: 1;
    min-width: 0;
    height: 100%;
    justify-content: flex-start;
    gap: 7px;
    padding: 0 6px 0 10px;
  }
  .session:disabled {
    cursor: default;
  }
  .session-pill button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .session-title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: var(--m-text-sub, 12.5px);
    font-weight: 500;
  }
  .status-dot {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--fg-subtle);
  }
  .status-dot[data-status='live'] {
    background: var(--success);
  }
  .status-dot[data-status='connecting'] {
    background: var(--warning);
  }
  .status-dot[data-status='failed'] {
    background: var(--danger);
  }
  .disconnect {
    flex: none;
    width: var(--m-icon-btn, 36px);
    height: 34px;
  }
  .badge {
    position: absolute;
    top: 1px;
    right: 1px;
    min-width: 14px;
    padding: 0 3px;
    border-radius: var(--radius-pill);
    background: var(--danger);
    color: var(--accent-fg);
    font-size: 10px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    line-height: 14px;
  }
</style>
