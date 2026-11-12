<script lang="ts">
  // The band along the bottom of the window: on the left the focused session (state, name,
  // user@host, time connected) and what runs (sessions, forwards); on the right AI approvals,
  // sync, a local terminal, the theme, the command palette and the time.
  import { onMount } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import ContextMenu, { type MenuItem } from '../ui/ContextMenu.svelte';
  import { t } from '../i18n';
  import type { ActionId } from '../actions';
  import type { IconName } from '../ui/icons';
  import type { Theme } from '../ipc';
  import { workspace, hostOf } from '../workspace/workspace.svelte';
  import { hosts } from '../hosts/hosts.svelte';
  import { forwardStatuses } from '../forwards/forwards.svelte';
  import { ai } from '../ai/ai.svelte';
  import { sync } from '../sync/sync.svelte';
  import { prefs } from '../settings/prefs.svelte';
  import { editPrefs } from '../settings/edit.svelte';
  import { clock, durationKind, durationParts, untilNextMinute } from './status';

  let { onaction }: { onaction: (action: ActionId) => void } = $props();

  let now = $state(new Date());
  let themeMenu = $state<{ x: number; y: number } | null>(null);

  const pane = $derived(workspace.focusedPane);
  /** The focused session as name and `user@host`. */
  const who = $derived.by(() => {
    if (!pane) return null;
    const target = pane.target;
    const hostId = hostOf(target);
    const host = hostId ? hosts.find(hostId) : undefined;
    if (host) return { name: host.name, address: `${host.username}@${host.hostname}` };
    if (target.kind === 'quick') return { name: target.address, address: '' };
    if (target.kind === 'sftp' && 'address' in target.source) return { name: target.source.address, address: '' };
    if (target.kind === 'local') return { name: t('status.localTerminal'), address: pane.title !== 'Terminal' ? pane.title : '' };
    if (target.kind === 'fleet') return { name: t('fleet.title'), address: t('fleet.allHosts') };
    return { name: pane.title, address: '' };
  });
  const connection = $derived.by((): 'live' | 'connecting' | 'failed' | 'ended' => {
    if (!pane) return 'ended';
    if (pane.failure) return 'failed';
    if (pane.exited) return 'ended';
    return pane.sessionId || pane.target.kind === 'sftp' || pane.target.kind === 'fleet' ? 'live' : 'connecting';
  });
  const connectedFor = $derived.by(() => {
    if (!pane?.connectedAt || connection !== 'live') return null;
    const parts = durationParts(now.getTime() - pane.connectedAt);
    const kind = durationKind(parts);
    return t(`status.duration.${kind}`, { d: parts.days, h: parts.hours, m: parts.minutes });
  });
  const sessions = $derived(Object.values(workspace.panes).filter((entry) => entry.sessionId && !entry.exited).length);
  const forwards = $derived(forwardStatuses.runningTotal);

  const THEMES: { value: Theme; label: 'settings.themeSystem' | 'settings.themeLight' | 'settings.themeDark'; icon: IconName }[] = [
    { value: 'system', label: 'settings.themeSystem', icon: 'sunMoon' },
    { value: 'light', label: 'settings.themeLight', icon: 'sun' },
    { value: 'dark', label: 'settings.themeDark', icon: 'moon' },
  ];
  const themeIcon = $derived<IconName>(THEMES.find((entry) => entry.value === prefs.value.appearance.theme)?.icon ?? 'sunMoon');

  function themeItems(): MenuItem[] {
    return THEMES.map((entry) => ({
      label: t(entry.label),
      icon: entry.value === prefs.value.appearance.theme ? 'check' : entry.icon,
      onselect: () => void editPrefs((draft) => (draft.appearance.theme = entry.value)),
    }));
  }

  function openThemeMenu(event: MouseEvent): void {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    // Opens upwards, its right edge at the button's, so it stays inside the window.
    themeMenu = { x: Math.max(8, rect.right - 180), y: rect.top - 4 - 3 * 32 };
  }

  const syncTitle = $derived.by(() => {
    if (sync.running) return t('status.syncing');
    if (sync.lastError) return t('status.syncFailed', { error: sync.lastError });
    return sync.lastAt ? t('status.syncedAt', { time: clock(new Date(sync.lastAt)) }) : t('status.syncNever');
  });

  onMount(() => {
    let timer = 0;
    const tick = (): void => {
      now = new Date();
      timer = window.setTimeout(tick, untilNextMinute(now) + 50);
    };
    timer = window.setTimeout(tick, untilNextMinute(now) + 50);
    return () => clearTimeout(timer);
  });
</script>

<footer class="statusbar">
  <div class="left">
    {#if who}
      <span class="who" title={t(`status.state.${connection}`)}>
        <span class="dot {connection}"></span>
        <span class="name">{who.name}</span>
        {#if who.address}<span class="muted address">{who.address}</span>{/if}
      </span>
      {#if connectedFor}
        <span class="item" title={t('status.connectedFor', { time: connectedFor })}><Icon name="clock" size={12} />{connectedFor}</span>
      {/if}
    {:else}
      <span class="muted">{t('status.noSession')}</span>
    {/if}
    {#if sessions > 0}<span class="item"><Icon name="terminal" size={12} />{t('status.sessions', { n: sessions })}</span>{/if}
    {#if forwards > 0}<span class="item"><Icon name="forwards" size={12} />{t('status.forwardsRunning', { n: forwards })}</span>{/if}
  </div>
  <div class="right">
    {#if ai.waiting > 0}
      <button type="button" class="pill" onclick={() => onaction('ai.showWaiting')}>{t('status.waiting', { n: ai.waiting })}</button>
    {/if}
    {#if sync.status?.ready}
      <span class:failed={!!sync.lastError} class:spinning={sync.running} class="sync">
        <IconButton icon={sync.lastError ? 'warning' : 'cloudCheck'} small label={syncTitle} onclick={() => void sync.syncNow().catch(() => undefined)} />
      </span>
    {/if}
    <IconButton icon="terminal" small label={t('action.tab.newLocal')} onclick={() => onaction('tab.newLocal')} />
    <IconButton icon={themeIcon} small label={t('status.theme')} onclick={openThemeMenu} />
    <IconButton icon="command" small label={t('action.palette.open')} onclick={() => onaction('palette.open')} />
    <span class="clock">{clock(now)}</span>
  </div>
</footer>
{#if themeMenu}
  <ContextMenu x={themeMenu.x} y={themeMenu.y} items={themeItems()} onclose={() => (themeMenu = null)} />
{/if}

<style>
  .statusbar {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    height: 26px;
    padding: 0 var(--space-2) 0 var(--space-3);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .left,
  .right {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
  }
  .left {
    flex: 1;
    overflow: hidden;
  }
  .right {
    flex: none;
    gap: var(--space-1);
  }
  .who {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .name {
    flex: none;
    color: var(--fg);
    font-weight: 600;
  }
  .address {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item {
    display: flex;
    flex: none;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
  }
  .dot {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--fg-muted);
  }
  .dot.live {
    background: var(--success);
  }
  .dot.connecting {
    background: var(--warning);
  }
  .dot.failed {
    background: var(--danger);
  }
  .pill {
    height: 18px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-pill);
    background: color-mix(in srgb, var(--warning) 18%, transparent);
    color: var(--warning);
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
  }
  .sync.failed :global(svg) {
    color: var(--danger);
  }
  .sync.spinning :global(svg) {
    animation: spin 1.2s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .clock {
    margin-left: var(--space-1);
    font-variant-numeric: tabular-nums;
  }
</style>
