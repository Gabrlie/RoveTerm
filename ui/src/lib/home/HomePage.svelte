<script lang="ts">
  // The home tab: saved hosts as cards, most recently connected first, plus the ways to start
  // something new.
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { hosts } from '../hosts/hosts.svelte';
  import { forwardStatuses } from '../forwards/forwards.svelte';
  import { t } from '../i18n';
  import type { HostEntry } from '../ipc';

  interface Props {
    onconnect: (host: HostEntry) => void;
    onfiles: (host: HostEntry) => void;
    onedit: (host: HostEntry | null) => void;
    onquick: () => void;
    onlocal: () => void;
  }

  let { onconnect, onfiles, onedit, onquick, onlocal }: Props = $props();
  let query = $state('');

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

<div class="home">
  <header>
    <div class="search">
      <Icon name="search" size={15} />
      <input class="bare" bind:value={query} placeholder={t('host.search')} spellcheck="false" />
    </div>
    <button type="button" class="action" onclick={onquick}><Icon name="quickConnect" size={15} />{t('action.host.quickConnect')}</button>
    <button type="button" class="action" onclick={() => onedit(null)}><Icon name="add" size={15} />{t('host.new')}</button>
    <button type="button" class="action" onclick={onlocal}><Icon name="terminal" size={15} />{t('action.tab.newLocal')}</button>
  </header>

  {#if hosts.items.length === 0}
    <div class="empty">
      <p>{t('home.empty')}</p>
      <button type="button" class="primary" onclick={() => onedit(null)}>{t('host.new')}</button>
    </div>
  {:else}
    <h2>{t('home.hosts')}</h2>
    <div class="grid">
      {#each sorted as host (host.id)}
        <div
          class="card"
          role="button"
          tabindex="0"
          onclick={() => onconnect(host)}
          onkeydown={(event) => event.key === 'Enter' && onconnect(host)}
          title={`${host.username}@${host.hostname}:${host.port}`}
        >
          <div class="icon"><Icon name="server" size={18} /></div>
          <div class="text">
            <span class="name">
              <span class="label">{host.name}</span>
              {#if host.route.kind !== 'direct'}<Icon name={host.route.kind === 'jump' ? 'link' : 'forwards'} size={12} />{/if}
              {#if forwardStatuses.running(host.id) > 0}<span class="forwarding"><Icon name="forwards" size={11} />{forwardStatuses.running(host.id)}</span>{/if}
            </span>
            <span class="address">{host.username}@{host.hostname}{host.port === 22 ? '' : `:${host.port}`}</span>
            <span class="seen">{lastSeen(host)}</span>
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

<style>
  .home {
    position: absolute;
    inset: 0;
    overflow-y: auto;
    padding: var(--space-6);
    background: var(--bg);
  }
  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-6);
  }
  .search {
    flex: 1 1 260px;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-3);
    height: 36px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    color: var(--fg-muted);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--fg);
    font: inherit;
  }
  .action {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    height: 36px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    color: var(--fg);
    font: inherit;
    cursor: pointer;
  }
  .action:hover {
    border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  }
  h2 {
    margin: 0 0 var(--space-3);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: var(--space-3);
  }
  .card {
    position: relative;
    display: flex;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-elevated);
    cursor: pointer;
    outline: none;
  }
  .card:hover,
  .card:focus-visible {
    border-color: color-mix(in srgb, var(--accent) 55%, var(--border));
  }
  .icon {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .name {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--fg);
    font-weight: 600;
    min-width: 0;
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
    font-size: var(--text-caption);
  }
  .forwarding {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    color: var(--accent);
    font-size: var(--text-caption);
  }
  .actions {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    display: none;
    gap: 2px;
  }
  .card:hover .actions,
  .card:focus-within .actions {
    display: flex;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    margin-top: 15vh;
    color: var(--fg-muted);
  }
  .primary {
    padding: 6px var(--space-4);
    border: none;
    border-radius: 999px;
    background: var(--accent);
    color: #fff;
    font: inherit;
    cursor: pointer;
  }
</style>
