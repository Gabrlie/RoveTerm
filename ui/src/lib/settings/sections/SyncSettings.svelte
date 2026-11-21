<script lang="ts">
  // Sync in two steps: connect to a storage (stored on this device once it answers), then join
  // the space found there or create one. Failures show both inline in the panel and as notifications.
  import { onMount } from 'svelte';
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import Icon from '../../ui/Icon.svelte';
  import { errorText, t } from '../../i18n';
  import type { SyncTarget } from '../../ipc';
  import { draft, sync } from '../../sync/sync.svelte';
  import { splitWebdavUrl, webdavUrl } from '../../sync/webdav';
  import { notify } from '../../ui/toast.svelte';

  const status = $derived(sync.status);

  let editing = $state(false);
  let panelError = $state('');

  function reportError(message: string): void {
    panelError = message;
    notify(message, 'error');
  }

  function clearError(): void {
    if (panelError) panelError = '';
  }

  async function chooseFolder(): Promise<void> {
    clearError();
    const picked = await openDialog({ directory: true, multiple: false, title: t('sync.chooseFolder') });
    if (typeof picked === 'string') draft.folder = picked;
  }

  function startEdit(): void {
    if (!status?.target) return;
    clearError();
    if (status.target.kind === 'folder') {
      draft.storage = 'folder';
      draft.folder = status.target.path;
    } else {
      const split = splitWebdavUrl(status.target.url);
      draft.storage = 'webdav';
      draft.server = split.server;
      draft.remoteFolder = split.folder || 'RoveTerm';
      draft.username = status.target.username;
      draft.storagePassword = '';
    }
    editing = true;
  }

  function cancelEdit(): void {
    editing = false;
    draft.storagePassword = '';
    clearError();
  }

  /** The storage as typed, or null with an inline error and notification when something is missing. */
  function typedTarget(): SyncTarget | null {
    if (draft.storage === 'folder') {
      if (draft.folder.trim()) return { kind: 'folder', path: draft.folder.trim() };
      reportError(t('sync.folderRequired'));
      return null;
    }
    if (draft.server.trim() && draft.remoteFolder.trim() && draft.username.trim()) {
      return { kind: 'webdav', url: webdavUrl(draft.server, draft.remoteFolder), username: draft.username.trim() };
    }
    reportError(t('sync.serverRequired'));
    return null;
  }

  /** Connects to `target`, or to the typed storage. */
  async function connect(target: SyncTarget | null = typedTarget()): Promise<void> {
    if (!target) return;
    const wasEditing = editing;
    clearError();
    try {
      const storagePassword = target.kind === 'webdav' ? (draft.storagePassword || null) : null;
      await sync.connect(target, storagePassword);
      draft.storagePassword = '';
      if (wasEditing) {
        editing = false;
        notify(t('sync.configSaved'), 'success');
      }
    } catch (error) {
      reportError(errorText(error));
    }
  }

  async function start(): Promise<void> {
    clearError();
    if (!sync.spaceExists && draft.password !== draft.confirm) {
      reportError(t('sync.passwordMismatch'));
      return;
    }
    try {
      await sync.setup(draft.password);
      draft.password = '';
      draft.confirm = '';
      notify(t('sync.started'), 'success');
    } catch (error) {
      reportError(errorText(error));
    }
  }

  async function syncNow(): Promise<void> {
    clearError();
    try {
      await sync.syncNow();
    } catch (error) {
      reportError(errorText(error));
    }
  }

  async function change(): Promise<void> {
    clearError();
    editing = false;
    await sync.disable().catch((error: unknown) => reportError(errorText(error)));
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
    clearError();
    sync.probe().catch((error: unknown) => {
      probeFailed = true;
      reportError(errorText(error));
    });
  }

  // Connected but not syncing yet: find out whether to join or create.
  $effect(() => {
    if (!editing && status?.connected && !status.ready && sync.spaceExists === null && !sync.connecting && !probeFailed) probe();
  });

  onMount(() => {
    sync.refreshStatus().catch((caught: unknown) => console.warn('sync status unavailable', caught));
  });
</script>

{#if status?.configured && status.target && !editing}
  <p class="section-label">{t('sync.space')}</p>
  <div class="form-group">
    <div class="form-row">
      <span class="form-label">{status.target.kind === 'folder' ? t('sync.folder') : t('sync.storageWebdav')}<small class="path" title={where(status.target)}>{where(status.target)}</small></span>
      <button type="button" class="secondary" onclick={startEdit}>{t('sync.edit')}</button>
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
        <input class="field" type="password" bind:value={draft.storagePassword} oninput={clearError} autocomplete="off" aria-label={t('sync.storagePassword')} />
        <button type="button" class="primary" disabled={sync.connecting || !draft.storagePassword} onclick={() => void connect(target)}>{sync.connecting ? t('sync.connecting') : t('sync.connect')}</button>
      </div>
    </div>
  {:else if !status.ready}
    {#if sync.spaceExists !== null}
      <p class="form-note">{sync.spaceExists ? t('sync.foundSpace') : t('sync.newSpace')}</p>
      <div class="form-group">
        <div class="form-row">
          <span class="form-label">{t('sync.password')}<small>{sync.spaceExists ? t('sync.joinHint') : t('sync.passwordHint')}</small></span>
          <input class="field" type="password" bind:value={draft.password} oninput={clearError} autocomplete={sync.spaceExists ? 'current-password' : 'new-password'} aria-label={t('sync.password')} />
        </div>
        {#if !sync.spaceExists}
          <div class="form-row">
            <span class="form-label">{t('sync.confirm')}</span>
            <input class="field" type="password" bind:value={draft.confirm} oninput={clearError} autocomplete="new-password" aria-label={t('sync.confirm')} />
          </div>
        {/if}
      </div>
    {/if}
  {/if}

  {#if panelError}
    <div class="inline-error" role="alert">
      <Icon name="warning" size={14} />
      <span>{panelError}</span>
    </div>
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
  {#if !editing}
    <p class="form-note intro">{t('sync.intro')}</p>
  {/if}
  <p class="section-label">{editing ? t('sync.edit') : t('sync.space')}</p>
  <div class="form-group">
    <div class="form-row">
      <span class="form-label">{t('sync.storage')}</span>
      <div class="segmented" role="group" aria-label={t('sync.storage')}>
        <button type="button" aria-pressed={draft.storage === 'folder'} onclick={() => { draft.storage = 'folder'; clearError(); }}>{t('sync.storageFolder')}</button>
        <button type="button" aria-pressed={draft.storage === 'webdav'} onclick={() => { draft.storage = 'webdav'; clearError(); }}>{t('sync.storageWebdav')}</button>
      </div>
    </div>
    {#if draft.storage === 'folder'}
      <div class="form-row">
        <span class="form-label">{t('sync.folder')}<small>{t('sync.folderHint')}</small></span>
        <input class="field path-input" bind:value={draft.folder} oninput={clearError} spellcheck="false" placeholder="/Users/me/Dropbox/RoveTerm" aria-label={t('sync.folder')} />
        <button type="button" class="secondary" onclick={() => void chooseFolder()}>{t('settings.choose')}</button>
      </div>
    {:else}
      <div class="form-row">
        <span class="form-label">{t('sync.server')}<small>{t('sync.serverHint')}</small></span>
        <input class="field path-input" bind:value={draft.server} oninput={clearError} spellcheck="false" placeholder="https://dav.example.com/dav/" aria-label={t('sync.server')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('sync.remoteFolder')}<small>{t('sync.remoteFolderHint')}</small></span>
        <input class="field path-input" bind:value={draft.remoteFolder} oninput={clearError} spellcheck="false" placeholder="RoveTerm" aria-label={t('sync.remoteFolder')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('sync.username')}</span>
        <input class="field" bind:value={draft.username} oninput={clearError} spellcheck="false" autocomplete="off" aria-label={t('sync.username')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('sync.storagePassword')}<small>{editing && status?.target?.kind === 'webdav' && status.connected ? t('sync.storagePasswordKeepHint') : t('sync.storagePasswordHint')}</small></span>
        <input class="field" type="password" bind:value={draft.storagePassword} oninput={clearError} autocomplete="off" aria-label={t('sync.storagePassword')} />
      </div>
    {/if}
  </div>

  {#if panelError}
    <div class="inline-error" role="alert">
      <Icon name="warning" size={14} />
      <span>{panelError}</span>
    </div>
  {/if}

  <div class="actions">
    {#if editing}
      <button type="button" class="secondary" disabled={sync.connecting} onclick={cancelEdit}>{t('common.cancel')}</button>
    {/if}
    <button type="button" class="primary" disabled={sync.connecting} onclick={() => void connect()}>
      {sync.connecting ? t('sync.connecting') : editing ? t('sync.editSave') : t('sync.connect')}
    </button>
  </div>
  {#if !editing}<p class="form-note">{t('sync.what')}</p>{/if}
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
  .inline-error {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin-top: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
    box-shadow: 0 0 0 0.5px color-mix(in srgb, var(--danger) 35%, transparent);
    color: var(--danger);
    font-size: var(--text-footnote);
    line-height: 1.45;
    word-break: break-word;
  }
  .inline-error :global(svg) {
    flex: none;
    margin-top: 2px;
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
