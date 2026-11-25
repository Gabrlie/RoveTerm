<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '../i18n';
  import { handleBack, pushBackHandler } from '../ui/backStack';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import type { IconName } from '../ui/icons';
  import Switch from '../ui/Switch.svelte';

  interface Props {
    remote: boolean;
    canForwards: boolean;
    aiWaiting?: number;
    activeForwards?: number;
    statusBarEnabled: boolean;
    onclose: () => void;
    onai: () => void;
    onfiles: () => void;
    onstatus: () => void;
    onsnippets: () => void;
    onforwards: () => void;
    ontogglestatusbar: () => void;
    oncustomizekeys: () => void;
  }

  let {
    remote, canForwards, aiWaiting = 0, activeForwards = 0, statusBarEnabled,
    onclose, onai, onfiles, onstatus, onsnippets, onforwards, ontogglestatusbar, oncustomizekeys,
  }: Props = $props();

  let sheet: HTMLDivElement;
  const titleId = $props.id();

  onMount(() => {
    const previous = document.activeElement;
    const disposeBack = pushBackHandler(() => onclose());
    sheet.focus();
    return () => {
      disposeBack();
      if (previous instanceof HTMLButtonElement && previous.isConnected) previous.focus();
    };
  });

  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape' && !event.defaultPrevented && handleBack()) event.preventDefault();
  }

  function openTool(action: () => void): void {
    onclose();
    action();
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#snippet tile(icon: IconName, tint: string, label: string, action: () => void, count = 0)}
  <button type="button" class="tile" style:--tile-tint={tint} onclick={() => openTool(action)}>
    <span class="tile-icon">
      <Icon name={icon} size={17} />
      {#if count > 0}<span class="badge">{count}</span>{/if}
    </span>
    <span>{label}</span>
  </button>
{/snippet}

<div class="overlay" use:portal>
  <button type="button" class="backdrop" tabindex="-1" aria-label={t('common.close')} onclick={onclose}></button>
  <div class="sheet" role="dialog" aria-modal="true" aria-labelledby={titleId} tabindex="-1" bind:this={sheet}>
    <div class="handle" aria-hidden="true"></div>
    <header>
      <h2 id={titleId}>{t('terminal.dock.toolsTitle')}</h2>
      <IconButton icon="close" label={t('common.close')} onclick={onclose} />
    </header>

    <div class="body">
      <div class="tools-grid">
        {@render tile('sparkles', 'var(--sys-pink)', t('ai.title'), onai, aiWaiting)}
        {#if remote}
          {@render tile('files', 'var(--sys-teal)', t('rail.files'), onfiles)}
          {@render tile('pulse', 'var(--sys-green)', t('action.panel.status'), onstatus)}
        {/if}
        {@render tile('snippets', 'var(--sys-orange)', t('action.panel.snippets'), onsnippets)}
        {#if canForwards}
          {@render tile('forwards', 'var(--sys-blue)', t('host.forwards'), onforwards, activeForwards)}
        {/if}
        {@render tile('keyboard', 'var(--sys-graphite)', t('terminal.dock.customizeKeys'), oncustomizekeys)}
      </div>
      <div class="form-group">
        <div class="form-row">
          <div class="form-label">
            {t('terminal.dock.statusBar')}
            <small>{t(statusBarEnabled ? 'terminal.dock.statusBarOn' : 'terminal.dock.statusBarOff')}</small>
          </div>
          <Switch checked={statusBarEnabled} label={t('terminal.dock.statusBar')} onchange={ontogglestatusbar} />
        </div>
      </div>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 35;
    display: grid;
    align-items: end;
    padding-inline: var(--safe-left) var(--safe-right);
  }
  .backdrop {
    position: absolute;
    inset: 0;
    border: 0;
    background: var(--overlay);
  }
  .sheet {
    position: relative;
    display: flex;
    flex-direction: column;
    max-height: calc(100dvh - var(--safe-top));
    padding-bottom: calc(max(var(--safe-bottom), var(--ime-bottom)) + var(--space-4));
    overflow: hidden;
    border-radius: 16px 16px 0 0;
    background: var(--material-popover);
    box-shadow: var(--glass-edge), var(--shadow-lg);
    outline: none;
  }
  .handle {
    flex: none;
    align-self: center;
    width: 32px;
    height: 4px;
    margin-top: var(--space-2);
    border-radius: var(--radius-pill);
    background: var(--fill-pressed);
  }
  header {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-1) var(--m-page-x, 16px) var(--space-2);
  }
  h2 {
    margin: 0;
    font-size: var(--m-text-nav, 16px);
    font-weight: 600;
  }
  .body {
    display: grid;
    gap: var(--space-4);
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: 2px var(--m-page-x, 16px);
  }
  .tools-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--space-2);
  }
  @media (max-width: 320px) {
    .tools-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  .tile {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    min-height: 68px;
    padding: var(--space-2);
    border: 0;
    border-radius: var(--m-card-radius, 12px);
    background: var(--fill);
    color: var(--fg);
    font: 500 var(--m-text-sub, 12.5px) var(--font-ui);
    text-align: center;
    overflow-wrap: anywhere;
    cursor: pointer;
  }
  .tile:active {
    background: var(--fill-pressed);
  }
  .tile > span {
    max-width: 100%;
  }
  .tile-icon {
    position: relative;
    display: grid;
    place-items: center;
    width: var(--m-icon-tile, 32px);
    height: var(--m-icon-tile, 32px);
    border-radius: var(--space-2);
    background: color-mix(in srgb, var(--tile-tint) 18%, transparent);
    color: var(--tile-tint);
  }
  .badge {
    position: absolute;
    top: 0;
    right: 0;
    transform: translate(35%, -25%);
    min-width: 16px;
    padding: 0 3px;
    border-radius: var(--radius-pill);
    background: var(--danger);
    color: var(--accent-fg);
    font-size: var(--text-caption);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    line-height: 16px;
  }
</style>
