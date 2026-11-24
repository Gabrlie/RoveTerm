<script lang="ts" module>
  export type MobileDestination = 'home' | 'workspace' | 'fleet' | 'settings';
</script>

<script lang="ts">
  import { t } from '../i18n';
  import Icon from '../ui/Icon.svelte';
  import type { IconName } from '../ui/icons';

  let { active, tabCount, fleetWaiting, onselect }: {
    active: MobileDestination;
    tabCount: number;
    fleetWaiting: number;
    onselect: (destination: MobileDestination) => void;
  } = $props();

  const destinations: { id: MobileDestination; icon: IconName }[] = [
    { id: 'home', icon: 'home' },
    { id: 'workspace', icon: 'terminal' },
    { id: 'fleet', icon: 'bot' },
    { id: 'settings', icon: 'settings' },
  ];
</script>

<nav class="nav-bar" aria-label={t('mobile.nav.label')}>
  {#each destinations as { id, icon } (id)}
    {@const count = id === 'workspace' ? tabCount : id === 'fleet' ? fleetWaiting : 0}
    <button type="button" aria-current={active === id ? 'page' : undefined} onclick={() => onselect(id)}>
      <span class="icon">
        <Icon name={icon} size={19} />
        {#if count > 0}
          <span class="badge" class:waiting={id === 'fleet'} aria-label={id === 'fleet' ? t('status.waiting', { n: count }) : undefined}>{count}</span>
        {/if}
      </span>
      <span class="caption">{t(`mobile.nav.${id}`)}</span>
    </button>
  {/each}
</nav>

<style>
  .nav-bar {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    flex: 0 0 52px;
    padding-top: 0.5px;
    background: var(--material-bar);
    box-shadow: inset 0 0.5px 0 var(--separator);
  }
  button {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
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
    font-size: var(--text-caption);
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
  .badge.waiting {
    background: var(--danger);
    color: var(--accent-fg);
  }
</style>
