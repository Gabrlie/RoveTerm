<script lang="ts">
  // Sync in two steps: connect to a storage (stored on this device once it answers), then join
  // the space found there or create one. Failures show as notifications.
  import { onMount } from 'svelte';
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import { errorText, t } from '../../i18n';
  import type { SyncTarget } from '../../ipc';
  import { draft, sync } from '../../sync/sync.svelte';
  import { webdavUrl } from '../../sync/webdav';
  import { notify } from '../../ui/toast.svelte';

  const status = $derived(sync.status);

  async function chooseFolder(): Promise<void> {
    const picked = await openDialog({ directory: true, multiple: false, title: t('sync.chooseFolder') });
    if (typeof picked === 'string') draft.folder = picked;
  }

  /** The storage as typed, or null with a notification when something is missing. */
  function typedTarget(): SyncTarget | null {
    if (draft.storage === 'folder') {
      if (draft.folder.trim()) return { kind: 'folder', path: draft.folder.trim() };
      notify(t('sync.folderRequired'), 'error');
      return null;
    }
    if (draft.server.trim() && draft.remoteFolder.trim() && draft.username.trim()) {
      return { kind: 'webdav', url: webdavUrl(draft.server, draft.remoteFolder), username: draft.username.trim() };
    }
    notify(t('sync.serverRequired'), 'error');
    return null;
  }

  /** Connects to `target`, or to the typed storage. */
  async function connect(target: SyncTarget | null = typedTarget()): Promise<void> {
    if (!target) return;
    try {
      await sync.connect(target, target.kind === 'webdav' ? draft.storagePassword : null);
      draft.storagePassword = '';
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  async function start(): Promise<void> {
    if (!sync.spaceExists && draft.password !== draft.confirm) {
      notify(t('sync.passwordMismatch'), 'error');
      return;
    }
    try {
      await sync.setup(draft.password);
      draft.password = '';
      draft.confirm = '';
      notify(t('sync.started'), 'success');
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  async function syncNow(): Promise<void> {
    await sync.syncNow().catch((error: unknown) => notify(errorText(error), 'error'));
  }

  async function change(): Promise<void> {
    await sync.disable().catch((error: unknown) => notify(errorText(error), 'error'));
  }

  function lastLine(): string {
    if (!sync.lastAt || !sync.lastReport) return t('sync.never');
    const time = new Date(sync.lastAt).toLocaleTimeString();
    return t('sync.lastRound', { time, applied: sync.lastReport.applied, pushed: sync.lastReport.pushed });
  }

  function where(current: SyncTarget): string {
    return current.kind === 'folder' ? current.path : `${current.username} @ ${current.url}`;
  }

  /** The lookup below failed; it is retried only on request. */
  let probeFailed = $state(false);

  function probe(): void {
    probeFailed = false;
    sync.probe().catch((error: unknown) => {
      probeFailed = true;
      notify(errorText(error), 'error');
    });
  }

  // Connected but not syncing yet: find out whether to join or create.
  $effect(() => {
    if (status?.connected && !status.ready && sync.spaceExists === null && !sync.connecting && !probeFailed) probe();
  });

  onMount(() => {
    sync.refreshStatus().catch((caught: unknown) => console.warn('sync status unavailable', caught));
  });
</script>

{#if status?.configured && status.target}
  <p class="section-label">{t('sync.space')}</p>
  <div class="form-group">
    <div class="form-row">
      <span class="form-label">{status.target.kind === 'folder' ? t('sync.folder') : t('sync.storageWebdav')}<small class="path" title={where(status.target)}>{where(status.target)}</small></span>
    </div>
    {#if status.ready}
      <div class="form-row">
        <span class="form-label">{t('sync.records')}</span>
        <span class="value">{status.records}</span>
      </div>
      <div class="form-row">
        <span class="form-label">{t('sync.last')}<small class:failed={!!sync.lastError}>{sync.lastError || lastLine()}</small></span>
        <button type="button" class="primary" disabled={sync.running} onclick={() => void syncNow()}>{sync.running ? t('sync.running') : t('sync.now')}</button>
      </div>
    {/if}
  </div>

  {#if !status.connected && status.target.kind === 'webdav'}
    {@const target = status.target}
    <p class="form-note">{t('sync.reconnect')}</p>
    <div class="form-group">
      <div class="form-row">
        <span class="form-label">{t('sync.storagePassword')}</span>
        <input class="field" type="password" bind:value={draft.storagePassword} autocomplete="off" aria-label={t('sync.storagePassword')} />
        <button type="button" class="primary" disabled={sync.connecting || !draft.storagePassword} onclick={() => void connect(target)}>{sync.connecting ? t('sync.connecting') : t('sync.connect')}</button>
      </div>
    </div>
  {:else if !status.ready}
    {#if sync.spaceExists !== null}
      <p class="form-note">{sync.spaceExists ? t('sync.foundSpace') : t('sync.newSpace')}</p>
      <div class="form-group">
        <div class="form-row">
          <span class="form-label">{t('sync.password')}<small>{sync.spaceExists ? t('sync.joinHint') : t('sync.passwordHint')}</small></span>
          <input class="field" type="password" bind:value={draft.password} autocomplete={sync.spaceExists ? 'current-password' : 'new-password'} aria-label={t('sync.password')} />
        </div>
        {#if !sync.spaceExists}
          <div class="form-row">
            <span class="form-label">{t('sync.confirm')}</span>
            <input class="field" type="password" bind:value={draft.confirm} autocomplete="new-password" aria-label={t('sync.confirm')} />
          </div>
        {/if}
      </div>
    {/if}
  {/if}

  <div class="actions">
    {#if status.ready}
      <button type="button" class="secondary danger-text" onclick={() => void change()}>{t('sync.disable')}</button>
    {:else}
      <button type="button" class="secondary" onclick={() => void change()}>{t('sync.change')}</button>
      {#if status.connected && sync.spaceExists === null}
        <button type="button" class="primary" disabled={!probeFailed} onclick={probe}>{probeFailed ? t('sync.connect') : t('sync.connecting')}</button>
      {:else if status.connected}
        <button type="button" class="primary" disabled={sync.running || !draft.password} onclick={() => void start()}>
          {sync.running ? t('sync.running') : sync.spaceExists ? t('sync.joinAction') : t('sync.createAction')}
        </button>
      {/if}
    {/if}
  </div>
  {#if status.ready}<p class="form-note">{t('sync.disableHint')}</p>{/if}
{:else}
  <p class="form-note intro">{t('sync.intro')}</p>
  <p class="section-label">{t('sync.space')}</p>
  <div class="form-group">
    <div class="form-row">
      <span class="form-label">{t('sync.storage')}</span>
      <div class="segmented" role="group" aria-label={t('sync.storage')}>
        <button type="button" aria-pressed={draft.storage === 'folder'} onclick={() => (draft.storage = 'folder')}>{t('sync.storageFolder')}</button>
        <button type="button" aria-pressed={draft.storage === 'webdav'} onclick={() => (draft.storage = 'webdav')}>{t('sync.storageWebdav')}</button>
      </div>
    </div>
    {#if draft.storage === 'folder'}
      <div class="form-row">
        <span class="form-label">{t('sync.folder')}<small>{t('sync.folderHint')}</small></span>
        <input class="field path-input" bind:value={draft.folder} spellcheck="false" placeholder="/Users/me/Dropbox/RoveTerm" aria-label={t('sync.folder')} />
        <button type="button" class="secondary" onclick={() => void chooseFolder()}>{t('settings.choose')}</button>
      </div>
    {:else}
      <div class="form-row">
        <span class="form-label">{t('sync.server')}<small>{t('sync.serverHint')}</small></span>
        <input class="field path-input" bind:value={draft.server} spellcheck="false" placeholder="https://dav.example.com/dav/" aria-label={t('sync.server')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('sync.remoteFolder')}<small>{t('sync.remoteFolderHint')}</small></span>
        <input class="field path-input" bind:value={draft.remoteFolder} spellcheck="false" placeholder="RoveTerm" aria-label={t('sync.remoteFolder')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('sync.username')}</span>
        <input class="field" bind:value={draft.username} spellcheck="false" autocomplete="off" aria-label={t('sync.username')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('sync.storagePassword')}<small>{t('sync.storagePasswordHint')}</small></span>
        <input class="field" type="password" bind:value={draft.storagePassword} autocomplete="off" aria-label={t('sync.storagePassword')} />
      </div>
    {/if}
  </div>
  <div class="actions">
    <button type="button" class="primary" disabled={sync.connecting} onclick={() => void connect()}>
      {sync.connecting ? t('sync.connecting') : t('sync.connect')}
    </button>
  </div>
  <p class="form-note">{t('sync.what')}</p>
{/if}

<style>
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
  .value {
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .field {
    width: 220px;
  }
  .path-input {
    font-family: var(--font-mono);
    font-size: var(--text-footnote);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .intro {
    margin: 0 0 var(--space-2);
    font-size: var(--text-footnote);
  }
  .danger-text {
    color: var(--danger);
  }
  .failed {
    color: var(--danger);
  }
</style>
