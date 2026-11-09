<script lang="ts">
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { hosts } from './hosts.svelte';
  import { vault } from '../vault/vault.svelte';
  import { ask } from '../ui/confirm.svelte';
  import { forwardStatuses } from '../forwards/forwards.svelte';
  import { errorText, t } from '../i18n';
  import type { HostEntry } from '../ipc';

  interface Props {
    onconnect: (host: HostEntry) => void;
    onfiles: (host: HostEntry) => void;
    onforwards: (host: HostEntry) => void;
    onedit: (host: HostEntry | null) => void;
    onquick: () => void;
    onnotice: (message: string) => void;
    onsettings: () => void;
    onkeys: () => void;
  }

  let { onconnect, onfiles, onforwards, onedit, onquick, onnotice, onsettings, onkeys }: Props = $props();

  /** Suffix naming how the host is reached, empty for direct connections. */
  function routeNote(host: HostEntry): string {
    const { route } = host;
    if (route.kind === 'jump') return ` · ${t('host.via', { name: hosts.find(route.host)?.name ?? '?' })}`;
    if (route.kind === 'proxy') return ` · ${t('host.via', { name: `${route.host}:${route.port}` })}`;
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
  </div>
  <div class="search">
    <Icon name="search" size={14} />
    <input bind:value={hosts.query} placeholder={t('host.search')} spellcheck="false" />
  </div>
  <div class="list">
    {#each hosts.grouped as [group, entries] (group ?? '')}
      {#if group}<div class="group">{group}</div>{/if}
      {#each entries as host (host.id)}
        <div class="host" role="button" tabindex="0" ondblclick={() => onconnect(host)} onkeydown={(event) => event.key === 'Enter' && onconnect(host)} title={`${host.username}@${host.hostname}:${host.port}`}>
          <Icon name="server" size={14} />
          <div class="text">
            <span class="name">
              {host.name}
              {#if forwardStatuses.running(host.id) > 0}
                <span class="forwarding" title={t('forward.activeCount', { count: forwardStatuses.running(host.id) })}>
                  <Icon name="forwards" size={11} />{forwardStatuses.running(host.id)}
                </span>
              {/if}
            </span>
            <span class="address">{host.username}@{host.hostname}{host.port === 22 ? '' : `:${host.port}`}{routeNote(host)}</span>
          </div>
          <div class="actions">
            <IconButton icon="files" label={t('host.openFiles')} small onclick={() => onfiles(host)} />
            <IconButton icon="forwards" label={t('host.forwards')} small onclick={() => onforwards(host)} />
            <IconButton icon="edit" label={t('host.edit')} small onclick={() => onedit(host)} />
            <IconButton icon="remove" label={t('common.delete')} small danger onclick={() => remove(host)} />
          </div>
        </div>
      {/each}
    {:else}
      <p class="empty">{hosts.query ? t('host.noMatch') : t('host.empty')}</p>
    {/each}
  </div>
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
    background: var(--bg-elevated);
    border-right: 1px solid var(--border);
  }
  .toolbar {
    display: flex;
    align-items: center;
    height: 36px;
    padding: 0 var(--space-1) 0 var(--space-3);
    border-bottom: 1px solid var(--border);
  }
  .heading {
    flex: 1;
    font-weight: 600;
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-2);
    padding: 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg);
    color: var(--fg-muted);
  }
  .search input {
    flex: 1;
    min-width: 0;
    padding: 5px 0;
    border: none;
    background: transparent;
    color: var(--fg);
    font: inherit;
    outline: none;
  }
  .list {
    flex: 1;
    overflow-y: auto;
    padding: 0 var(--space-1);
  }
  .group {
    padding: var(--space-2) var(--space-2) var(--space-1);
    color: var(--fg-muted);
    font-size: var(--text-caption);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .host {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    color: var(--fg-muted);
    cursor: default;
    user-select: none;
  }
  .host:hover,
  .host:focus-visible {
    background: var(--bg-sunken);
    outline: none;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .name {
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .forwarding {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    margin-left: var(--space-1);
    color: var(--accent);
    font-size: var(--text-caption);
    vertical-align: middle;
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
  }
  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: var(--space-2);
  }
  .vault {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    font-size: var(--text-footnote);
    cursor: pointer;
  }
  .vault:hover:not(:disabled) {
    background: var(--bg-sunken);
    color: var(--fg);
  }
  .vault.unlocked {
    cursor: default;
  }
</style>
