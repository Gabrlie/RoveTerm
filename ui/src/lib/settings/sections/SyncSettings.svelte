<script lang="ts">
  import { onMount } from 'svelte';
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import { t } from '../../i18n';
  import type { SyncTarget } from '../../ipc';
  import { sync } from '../../sync/sync.svelte';

  let storage = $state<'folder' | 'webdav'>('folder');
  let folder = $state('');
  let url = $state('');
  let username = $state('');
  let storagePassword = $state('');
  let password = $state('');
  let confirm = $state('');
  /** Creating a new space rather than joining one. */
  let creating = $state(false);
  let error = $state('');

  const status = $derived(sync.status);
  /** The configured target, or the one being entered. */
  const target = $derived<SyncTarget>(
    status?.target ?? (storage === 'folder' ? { kind: 'folder', path: folder.trim() } : { kind: 'webdav', url: url.trim(), username: username.trim() }),
  );
  const filled = $derived(target.kind === 'folder' ? !!target.path : !!target.url && !!target.username && !!storagePassword);

  async function chooseFolder(): Promise<void> {
    const picked = await openDialog({ directory: true, multiple: false, title: t('sync.chooseFolder') });
    if (typeof picked === 'string') folder = picked;
  }

  async function setup(): Promise<void> {
    error = '';
    if (target.kind === 'folder' && !target.path) {
      error = t('sync.folderRequired');
      return;
    }
    if (target.kind === 'webdav' && (!target.url || !target.username)) {
      error = t('sync.urlRequired');
      return;
    }
    if (creating && password !== confirm) {
      error = t('sync.passwordMismatch');
      return;
    }
    try {
      await sync.setup(target, password, creating, target.kind === 'webdav' ? storagePassword : null);
      password = '';
      confirm = '';
      storagePassword = '';
    } catch {
      error = sync.lastError;
    }
  }

  async function syncNow(): Promise<void> {
    error = '';
    await sync.syncNow().catch(() => (error = sync.lastError));
  }

  function lastLine(): string {
    if (!sync.lastAt || !sync.lastReport) return t('sync.never');
    const time = new Date(sync.lastAt).toLocaleTimeString();
    return t('sync.lastRound', { time, applied: sync.lastReport.applied, pushed: sync.lastReport.pushed });
  }

  function where(current: SyncTarget): string {
    return current.kind === 'folder' ? current.path : `${current.username} @ ${current.url}`;
  }

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
    <div class="form-row">
      <span class="form-label">{t('sync.records')}</span>
      <span class="value">{status.records}</span>
    </div>
    <div class="form-row">
      <span class="form-label">{t('sync.last')}<small>{sync.lastError || lastLine()}</small></span>
      {#if status.ready}
        <button type="button" class="primary" disabled={sync.running} onclick={() => void syncNow()}>{sync.running ? t('sync.running') : t('sync.now')}</button>
      {/if}
    </div>
  </div>
  {#if !status.ready}
    <p class="section-label">{t('sync.enterPassword')}</p>
    <div class="form-group">
      {#if status.target.kind === 'webdav'}
        <div class="form-row">
          <span class="form-label">{t('sync.storagePassword')}</span>
          <input class="field" type="password" bind:value={storagePassword} autocomplete="off" />
        </div>
      {/if}
      <div class="form-row">
        <span class="form-label">{t('sync.password')}</span>
        <input class="field" type="password" bind:value={password} autocomplete="off" />
        <button type="button" class="primary" disabled={sync.running || !password || (status.target.kind === 'webdav' && !storagePassword)} onclick={() => void setup()}>{t('sync.unlock')}</button>
      </div>
    </div>
  {/if}
  <div class="actions">
    <button type="button" class="secondary danger-text" onclick={() => void sync.disable()}>{t('sync.disable')}</button>
  </div>
  <p class="form-note">{t('sync.disableHint')}</p>
{:else}
  <p class="form-note intro">{t('sync.intro')}</p>
  <p class="section-label">{t('sync.space')}</p>
  <div class="form-group">
    <div class="form-row">
      <span class="form-label">{t('sync.storage')}</span>
      <div class="segmented" role="group" aria-label={t('sync.storage')}>
        <button type="button" aria-pressed={storage === 'folder'} onclick={() => (storage = 'folder')}>{t('sync.storageFolder')}</button>
        <button type="button" aria-pressed={storage === 'webdav'} onclick={() => (storage = 'webdav')}>{t('sync.storageWebdav')}</button>
      </div>
    </div>
    {#if storage === 'folder'}
      <div class="form-row">
        <span class="form-label">{t('sync.folder')}<small>{t('sync.folderHint')}</small></span>
        <input class="field path-input" bind:value={folder} spellcheck="false" placeholder="/Users/me/Dropbox/RoveTerm" aria-label={t('sync.folder')} />
        <button type="button" class="secondary" onclick={() => void chooseFolder()}>{t('settings.choose')}</button>
      </div>
    {:else}
      <div class="form-row">
        <span class="form-label">{t('sync.url')}<small>{t('sync.urlHint')}</small></span>
        <input class="field path-input" bind:value={url} spellcheck="false" placeholder="https://dav.example.com/RoveTerm" aria-label={t('sync.url')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('sync.username')}</span>
        <input class="field" bind:value={username} spellcheck="false" autocomplete="off" aria-label={t('sync.username')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('sync.storagePassword')}<small>{t('sync.storagePasswordHint')}</small></span>
        <input class="field" type="password" bind:value={storagePassword} autocomplete="off" aria-label={t('sync.storagePassword')} />
      </div>
    {/if}
    <div class="form-row">
      <span class="form-label">{t('sync.mode')}</span>
      <div class="segmented" role="group" aria-label={t('sync.mode')}>
        <button type="button" aria-pressed={!creating} onclick={() => (creating = false)}>{t('sync.join')}</button>
        <button type="button" aria-pressed={creating} onclick={() => (creating = true)}>{t('sync.create')}</button>
      </div>
    </div>
    <div class="form-row">
      <span class="form-label">{t('sync.password')}<small>{creating ? t('sync.passwordHint') : t('sync.joinHint')}</small></span>
      <input class="field" type="password" bind:value={password} autocomplete="new-password" aria-label={t('sync.password')} />
    </div>
    {#if creating}
      <div class="form-row">
        <span class="form-label">{t('sync.confirm')}</span>
        <input class="field" type="password" bind:value={confirm} autocomplete="new-password" aria-label={t('sync.confirm')} />
      </div>
    {/if}
  </div>
  <div class="actions">
    <button type="button" class="primary" disabled={sync.running || !filled || !password} onclick={() => void setup()}>
      {sync.running ? t('sync.running') : creating ? t('sync.createAction') : t('sync.joinAction')}
    </button>
  </div>
  <p class="form-note">{t('sync.what')}</p>
{/if}
{#if error}<p class="error">{error}</p>{/if}

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
  .error {
    margin: var(--space-2) 0 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--text-footnote);
  }
</style>
