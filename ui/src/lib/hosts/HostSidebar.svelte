<script lang="ts">
  import Icon from '../ui/Icon.svelte';
  import { avatarColor, initials } from '../ui/avatar';
  import IconButton from '../ui/IconButton.svelte';
  import { hosts } from './hosts.svelte';
  import { vault } from '../vault/vault.svelte';
  import { ask } from '../ui/confirm.svelte';
  import ContextMenu, { type MenuItem } from '../ui/ContextMenu.svelte';
  import { forwardStatuses } from '../forwards/forwards.svelte';
  import { errorText, t } from '../i18n';
  import { hostCreate, type HostEntry } from '../ipc';

  interface Props {
    onconnect: (host: HostEntry) => void;
    onfiles: (host: HostEntry) => void;
    onforwards: (host: HostEntry) => void;
    onedit: (host: HostEntry | null) => void;
    onquick: () => void;
    onnotice: (message: string) => void;
    onsettings: () => void;
    onkeys: () => void;
    /** Collapses the sidebar. */
    oncollapse: () => void;
  }

  let { onconnect, onfiles, onforwards, onedit, onquick, onnotice, onsettings, onkeys, oncollapse }: Props = $props();

  let menu = $state<{ x: number; y: number; host: HostEntry } | null>(null);

  function openMenu(event: MouseEvent, host: HostEntry): void {
    event.preventDefault();
    event.stopPropagation();
    menu = { x: event.clientX, y: event.clientY, host };
  }

  function menuItems(host: HostEntry): MenuItem[] {
    return [
      { label: t('host.connect'), icon: 'terminal', onselect: () => onconnect(host) },
      { label: t('host.openFiles'), icon: 'files', onselect: () => onfiles(host) },
      { label: t('host.forwards'), icon: 'forwards', onselect: () => onforwards(host) },
      { label: t('host.edit'), icon: 'edit', separated: true, onselect: () => onedit(host) },
      { label: t('host.duplicate'), icon: 'copy', onselect: () => void duplicate(host) },
      { label: t('common.delete'), icon: 'remove', danger: true, separated: true, onselect: () => void remove(host) },
    ];
  }

  /** Saves a copy of `host` without its stored secret. */
  async function duplicate(host: HostEntry): Promise<void> {
    try {
      const { group, hostname, port, username, auth, route, tags } = host;
      await hostCreate({ name: t('host.copyName', { name: host.name }), group, hostname, port, username, auth, route, tags }, null);
      await hosts.refresh();
    } catch (error) {
      onnotice(errorText(error));
    }
  }

  /** Describes how the host is reached, for the route badge; empty for direct connections. */
  function routeTitle(host: HostEntry): string {
    const { route } = host;
    if (route.kind === 'jump') return t('host.via', { name: hosts.find(route.host)?.name ?? '?' });
    if (route.kind === 'proxy') return t('host.via', { name: `${route.protocol}://${route.host}:${route.port}` });
    return '';
  }

  async function importConfig(): Promise<void> {
    try {
      const report = await hosts.importSshConfig();
      onnotice(t('host.imported', { created: report.created.length, skipped: report.skipped.length }));
    } catch (error) {
      onnotice(errorText(error));
    }
  }

  async function remove(host: HostEntry): Promise<void> {
    const confirmed = await ask({ title: t('host.delete'), message: t('host.confirmDelete', { name: host.name }), confirmLabel: t('common.delete'), danger: true });
    if (!confirmed) return;
    await hosts.remove(host.id).catch((error: unknown) => onnotice(errorText(error)));
  }

  async function unlock(): Promise<void> {
    if (vault.keyLost) {
      const confirmed = await ask({ title: t('vault.keyLost'), message: t('vault.confirmReset'), confirmLabel: t('vault.reset'), danger: true });
      if (!confirmed) return;
      await vault.reset().catch((error: unknown) => onnotice(errorText(error)));
      return;
    }
    const state = await vault.unlock().catch((error: unknown) => {
      onnotice(errorText(error));
      return null;
    });
    if (state?.state === 'locked') onnotice(t('vault.unlockFailed'));
  }
</script>

<aside class="sidebar">
  <div class="toolbar">
    <span class="heading">{t('host.title')}</span>
    <IconButton icon="quickConnect" label={t('action.host.quickConnect')} onclick={onquick} />
    <IconButton icon="key" label={t('action.keys.manage')} onclick={onkeys} />
    <IconButton icon="importConfig" label={t('host.import')} onclick={importConfig} />
    <IconButton icon="add" label={t('host.new')} onclick={() => onedit(null)} />
    <IconButton icon="sidebar" label={t('action.sidebar.toggle')} onclick={oncollapse} />
  </div>
  <div class="search">
    <Icon name="search" size={14} />
    <input class="bare" bind:value={hosts.query} placeholder={t('host.search')} spellcheck="false" />
  </div>
  <div class="list">
    {#each hosts.grouped as [group, entries] (group ?? '')}
      {#if group}<div class="group section-label">{group}</div>{/if}
      {#each entries as host (host.id)}
        <div
          class="host"
          role="button"
          tabindex="0"
          ondblclick={() => onconnect(host)}
          oncontextmenu={(event) => openMenu(event, host)}
          onkeydown={(event) => {
            if (event.key === 'Enter') onconnect(host);
            if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
              const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
              menu = { x: rect.left + 24, y: rect.bottom, host };
              event.preventDefault();
            }
          }}
          title={`${host.username}@${host.hostname}:${host.port}`}
        >
          <span class="avatar" style:--tint={avatarColor(host.name)} aria-hidden="true">{initials(host.name)}</span>
          <div class="text">
            <span class="name">
              <span class="label">{host.name}</span>
              {#if host.route.kind !== 'direct'}
                <span class="badge" title={routeTitle(host)}><Icon name={host.route.kind === 'jump' ? 'link' : 'forwards'} size={11} /></span>
              {/if}
              {#if forwardStatuses.running(host.id) > 0}
                <span class="forwarding" title={t('forward.activeCount', { count: forwardStatuses.running(host.id) })}>
                  <Icon name="forwards" size={11} />{forwardStatuses.running(host.id)}
                </span>
              {/if}
            </span>
            <span class="address">{host.username}@{host.hostname}{host.port === 22 ? '' : `:${host.port}`}</span>
          </div>
          <div class="actions">
            <IconButton icon="terminal" label={t('host.connect')} small onclick={() => onconnect(host)} />
            <IconButton
              icon="more"
              label={t('host.more')}
              small
              onclick={(event) => {
                const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
                menu = { x: rect.left, y: rect.bottom + 2, host };
              }}
            />
          </div>
        </div>
      {/each}
    {:else}
      <p class="empty">{hosts.query ? t('host.noMatch') : t('host.empty')}</p>
    {/each}
  </div>
  {#if menu}
    <ContextMenu x={menu.x} y={menu.y} items={menuItems(menu.host)} onclose={() => (menu = null)} />
  {/if}
  <footer>
    <button type="button" class="vault" class:unlocked={vault.unlocked} onclick={unlock} disabled={vault.unlocked}>
      <Icon name={vault.unlocked ? 'unlock' : 'lock'} size={14} />
      <span>{vault.unlocked ? t('vault.unlocked') : vault.keyLost ? t('vault.keyLost') : t('vault.locked')}</span>
    </button>
    <IconButton icon="settings" label={t('action.settings.open')} small onclick={onsettings} />
  </footer>
</aside>

<style>
  .sidebar {
    width: 248px;
    flex: none;
    display: flex;
    flex-direction: column;
    background: var(--material-sidebar);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: inset -0.5px 0 0 var(--separator);
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 44px;
    padding: 0 var(--space-2) 0 var(--space-4);
  }
  .heading {
    flex: 1;
    font-size: var(--text-title);
    font-weight: 700;
    letter-spacing: -0.01em;
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--control-height);
    margin: 0 var(--space-3) var(--space-2);
    padding: 0 var(--space-2);
    border-radius: var(--radius-sm);
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
  .list {
    flex: 1;
    overflow-y: auto;
    padding: 0 var(--space-2) var(--space-2);
  }
  .group {
    padding: var(--space-3) var(--space-2) var(--space-1);
  }
  .host {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 40px;
    padding: 0 var(--space-1) 0 var(--space-2);
    border-radius: var(--radius-sm);
    color: var(--fg-muted);
    cursor: default;
    user-select: none;
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .host:hover {
    background: var(--fill-hover);
  }
  .host:focus-visible {
    background: var(--accent);
    color: color-mix(in srgb, var(--accent-fg) 80%, transparent);
    outline: none;
  }
  .host:focus-visible .name,
  .host:focus-visible .badge,
  .host:focus-visible .forwarding {
    color: var(--accent-fg);
  }
  .host:focus-visible :global(.icon-button) {
    color: var(--accent-fg);
  }
  .avatar {
    flex: none;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 7px;
    background: linear-gradient(to bottom, color-mix(in srgb, var(--tint) 82%, #fff), var(--tint));
    box-shadow: inset 0 0.5px 0 rgb(255 255 255 / 0.3), 0 0.5px 1px rgb(0 0 0 / 0.15);
    color: #fff;
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.02em;
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
    font-weight: 500;
  }
  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .badge {
    flex: none;
    display: inline-flex;
    color: var(--fg-muted);
  }
  .forwarding {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    margin-left: var(--space-1);
    padding: 0 5px;
    border-radius: var(--radius-pill);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--text-caption);
    font-weight: 600;
  }
  .address {
    font-size: var(--text-caption);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .actions {
    display: none;
  }
  .host:hover .actions,
  .host:focus-within .actions {
    display: flex;
  }
  .empty {
    padding: var(--space-3);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    text-align: center;
  }
  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2);
    box-shadow: inset 0 0.5px 0 var(--separator);
  }
  .vault {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 26px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    font-size: var(--text-footnote);
    cursor: pointer;
  }
  .vault :global(svg) {
    color: var(--warning);
  }
  .vault.unlocked :global(svg) {
    color: var(--success);
  }
  .vault:hover:not(:disabled) {
    background: var(--fill-hover);
    color: var(--fg);
  }
  .vault.unlocked {
    cursor: default;
  }
</style>
