<script lang="ts" module>
  export type MobileDestination = 'hosts' | 'tools' | 'settings';
</script>

<script lang="ts">
  import { t } from '../i18n';
  import Icon from '../ui/Icon.svelte';
  import type { IconName } from '../ui/icons';

  let { active, toolsBadge = 0, settingsBadge = false, onselect }: {
    active: MobileDestination;
    toolsBadge?: number;
    settingsBadge?: boolean;
    onselect: (destination: MobileDestination) => void;
  } = $props();

  const destinations: { id: MobileDestination; icon: IconName }[] = [
    { id: 'hosts', icon: 'server' },
    { id: 'tools', icon: 'wrench' },
    { id: 'settings', icon: 'settings' },
  ];
</script>

<nav class="nav-bar" aria-label={t('mobile.nav.label')}>
  {#each destinations as { id, icon } (id)}
    <button type="button" aria-current={active === id ? 'page' : undefined} onclick={() => onselect(id)}>
      <span class="icon">
        <Icon name={icon} size={20} />
        {#if id === 'tools' && toolsBadge > 0}
          <span class="badge" role="img" aria-label={t('mobile.tools.activeCount', { n: toolsBadge })}>{toolsBadge}</span>
        {:else if id === 'settings' && settingsBadge}
          <span class="update-dot" role="img" aria-label={t('mobile.settings.updateAvailable')}></span>
        {/if}
      </span>
      <span class="caption">{t(`mobile.nav.${id}`)}</span>
    </button>
  {/each}
</nav>

<style>
  .nav-bar {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    flex: 0 0 54px;
    padding-top: 0.5px;
    background: var(--material-bar);
    box-shadow: inset 0 0.5px 0 var(--separator);
  }
  button {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    min-height: 48px;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    cursor: pointer;
  }
  button[aria-current] {
    color: var(--accent);
  }
  button:active {
    background: var(--fill-pressed);
  }
  button:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .icon {
    position: relative;
    display: flex;
  }
  .caption {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--m-text-meta, 11px);
    font-weight: 600;
  }
  .badge {
    position: absolute;
    top: calc(-1 * var(--space-1));
    left: calc(100% - var(--space-1));
    min-width: 16px;
    padding-inline: var(--space-1);
    border-radius: var(--radius-pill);
    background: var(--accent-soft);
    color: var(--accent);
    text-align: center;
    font: 600 var(--text-caption) var(--font-ui);
  }
  .update-dot {
    position: absolute;
    top: calc(-1 * var(--space-1));
    left: 100%;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--danger);
  }
</style>
