<script lang="ts">
  // The home tab: saved hosts as cards, most recently connected first, plus the ways to start
  // something new.
  import HostIcon from '../hosts/HostIcon.svelte';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { hosts } from '../hosts/hosts.svelte';
  import { forwardStatuses } from '../forwards/forwards.svelte';
  import { t } from '../i18n';
  import type { AiRecord, AiSessionSummary, HostEntry } from '../ipc';
  import FleetHome from '../fleet/FleetHome.svelte';
  import ContextMenu, { type MenuItem } from '../ui/ContextMenu.svelte';
  import { hostMenuItems } from '../hosts/hostMenu';
  import { hostAddress, hostSubtitle } from '../hosts/route';
  import { actions, type ActionId } from '../actions';

  interface Props {
    onconnect: (host: HostEntry) => void;
    onfiles: (host: HostEntry) => void;
    onforwards: (host: HostEntry) => void;
    onedit: (host: HostEntry | null) => void;
    onquick: () => void;
    onlocal: () => void;
    onnotice: (message: string) => void;
    /** Runs an action from the empty-space menu. */
    onaction: (id: ActionId) => void;
    /** Opens the fleet lead's tab, sending `text` when given. */
    onfleet: (text: string | null) => void;
    /** Continues an earlier conversation with the fleet lead. */
    onfleetresume: (summary: AiSessionSummary, records: AiRecord[]) => void;
  }

  let { onconnect, onfiles, onforwards, onedit, onquick, onlocal, onnotice, onaction, onfleet, onfleetresume }: Props = $props();
  let query = $state('');
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  /** Actions offered when right-clicking empty space. */
  const PAGE_ACTIONS: ActionId[] = ['host.new', 'host.quickConnect', 'host.import', 'tab.newLocal'];

  function hostMenu(event: MouseEvent, host: HostEntry): void {
    event.preventDefault();
    event.stopPropagation();
    menu = { x: event.clientX, y: event.clientY, items: hostMenuItems(host, { onconnect, onfiles, onforwards, onedit, onnotice }) };
  }

  function pageMenu(event: MouseEvent): void {
    event.preventDefault();
    const items = PAGE_ACTIONS.flatMap((id) => {
      const action = actions.find((candidate) => candidate.id === id);
      return action ? [{ label: t(action.label), onselect: () => onaction(id) }] : [];
    });
    menu = { x: event.clientX, y: event.clientY, items };
  }

  /** Hosts matching `query`, recently connected first, then never-connected hosts by name. */
  const sorted = $derived(
    [...hosts.items]
      .filter((host) => {
        const needle = query.trim().toLowerCase();
        if (!needle) return true;
        return [host.name, host.hostname, host.username, host.group ?? '', ...host.tags].join(' ').toLowerCase().includes(needle);
      })
      .sort((a: HostEntry, b: HostEntry) => (b.last_connected_at ?? 0) - (a.last_connected_at ?? 0) || a.name.localeCompare(b.name)),
  );

  /** "3 minutes ago"-style text, or a "never connected" note. */
  function lastSeen(host: HostEntry): string {
    if (!host.last_connected_at) return t('home.never');
    const minutes = Math.round((Date.now() - host.last_connected_at) / 60_000);
    if (minutes < 1) return t('home.justNow');
    if (minutes < 60) return t('home.minutesAgo', { n: minutes });
    const hours = Math.round(minutes / 60);
    if (hours < 24) return t('home.hoursAgo', { n: hours });
    return t('home.daysAgo', { n: Math.round(hours / 24) });
  }
</script>

<div class="home" role="presentation" oncontextmenu={pageMenu}>
  <header>
    <div class="search">
      <Icon name="search" size={15} />
      <input class="bare" bind:value={query} placeholder={t('host.search')} spellcheck="false" />
    </div>
    <button type="button" class="secondary action" onclick={onquick}><Icon name="quickConnect" size={15} />{t('action.host.quickConnect')}</button>
    <button type="button" class="secondary action" onclick={() => onedit(null)}><Icon name="add" size={15} />{t('host.new')}</button>
    <button type="button" class="secondary action" onclick={onlocal}><Icon name="terminal" size={15} />{t('action.tab.newLocal')}</button>
  </header>

  {#if hosts.items.length > 0}
    <FleetHome onopen={onfleet} onresume={onfleetresume} onschedules={() => onaction('fleet.schedules')} />
  {/if}

  {#if hosts.items.length === 0}
    <div class="empty">
      <div class="empty-mark"><Icon name="server" size={28} /></div>
      <p>{t('home.empty')}</p>
      <button type="button" class="primary" onclick={() => onedit(null)}>{t('host.new')}</button>
    </div>
  {:else}
    <h2 class="section-label">{t('home.hosts')}</h2>
    <div class="grid">
      {#each sorted as host (host.id)}
        <div
          class="card"
          role="button"
          tabindex="0"
          onclick={() => onconnect(host)}
          onkeydown={(event) => event.key === 'Enter' && onconnect(host)}
          oncontextmenu={(event) => hostMenu(event, host)}
          title={hostAddress(host)}
        >
          <HostIcon os={host.os} size={28} />
          <div class="text">
            <span class="name">
              <span class="label">{host.name}</span>
              {#if forwardStatuses.running(host.id) > 0}<span class="forwarding"><Icon name="forwards" size={11} />{forwardStatuses.running(host.id)}</span>{/if}
            </span>
            <span class="address">{hostSubtitle(host)}<span class="seen">{` · ${lastSeen(host)}`}</span></span>
          </div>
          <div class="actions">
            <IconButton
              icon="files"
              label={t('host.openFiles')}
              small
              onclick={(event) => {
                event.stopPropagation();
                onfiles(host);
              }}
            />
            <IconButton
              icon="edit"
              label={t('host.edit')}
              small
              onclick={(event) => {
                event.stopPropagation();
                onedit(host);
              }}
            />
          </div>
        </div>
      {:else}
        <p class="muted">{t('host.noMatch')}</p>
      {/each}
    </div>
  {/if}
</div>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}

<style>
  .home {
    position: absolute;
    inset: 0;
    overflow-y: auto;
    padding: var(--space-6) var(--space-6) var(--space-5);
    background: var(--surface);
  }
  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    max-width: 1100px;
    margin: 0 auto var(--space-5);
  }
  .search {
    flex: 1 1 260px;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 34px;
    padding: 0 var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill);
    color: var(--fg-muted);
    transition: box-shadow var(--dur-fast) var(--ease-out);
  }
  .search:focus-within {
    background: var(--surface);
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--accent) 70%, transparent),
      var(--focus-ring);
  }
  .search input {
    flex: 1;
    min-width: 0;
    color: var(--fg);
    outline: none;
  }
  .action {
    height: 34px;
    min-width: 0;
    padding: 0 var(--space-3);
    border-radius: var(--radius-md);
  }
  .action :global(svg) {
    color: var(--accent);
  }
  h2 {
    max-width: 1100px;
    margin: 0 auto var(--space-3);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--space-2);
    max-width: 1100px;
    margin: 0 auto;
  }
  .card {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 48px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow:
      0 0 0 0.5px var(--separator),
      var(--shadow-sm);
    cursor: default;
    outline: none;
    transition:
      transform var(--dur) var(--ease),
      box-shadow var(--dur) var(--ease);
  }
  .card:hover {
    transform: translateY(-1px);
    box-shadow:
      0 0 0 0.5px var(--separator),
      var(--shadow-md);
  }
  .card:active {
    transform: translateY(0) scale(0.99);
  }
  .card:focus-visible {
    box-shadow:
      0 0 0 0.5px var(--separator),
      var(--focus-ring);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .name {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 0;
    color: var(--fg);
    font-weight: 600;
  }
  .name :global(svg) {
    flex: none;
    color: var(--fg-muted);
  }
  .label,
  .address {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .address,
  .seen,
  .muted {
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .seen {
    color: var(--fg-subtle);
  }
  .forwarding {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 0 5px;
    border-radius: var(--radius-pill);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--text-caption);
  }
  .forwarding :global(svg) {
    color: inherit;
  }
  .actions {
    position: absolute;
    top: 50%;
    right: var(--space-2);
    transform: translateY(-50%);
    display: flex;
    gap: 2px;
    padding-left: var(--space-3);
    /* Covers the end of the text it sits over. */
    background: linear-gradient(to right, transparent, var(--surface-raised) var(--space-3));
    opacity: 0;
    transition: opacity var(--dur-fast) var(--ease-out);
  }
  .card:hover .actions,
  .card:focus-within .actions {
    opacity: 1;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    margin-top: 14vh;
    color: var(--fg-muted);
  }
  .empty p {
    margin: 0 0 var(--space-2);
  }
  .empty-mark {
    display: grid;
    place-items: center;
    width: 64px;
    height: 64px;
    border-radius: 18px;
    background: linear-gradient(to bottom, color-mix(in srgb, var(--accent) 80%, #fff), var(--accent));
    box-shadow: var(--shadow-md);
    color: var(--accent-fg);
  }
</style>
