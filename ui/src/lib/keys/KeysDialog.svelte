<script lang="ts">
  import { writeClipboard } from '../ui/clipboard';
  import { onDestroy, onMount } from 'svelte';
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import ContextMenu, { type MenuItem } from '../ui/ContextMenu.svelte';
  import Dialog from '../ui/Dialog.svelte';
  import Select from '../ui/Select.svelte';
  import Switch from '../ui/Switch.svelte';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { ask } from '../ui/confirm.svelte';
  import { viewport } from '../layout/viewport.svelte';
  import { pushBackHandler } from '../ui/backStack';
  import { initialCompactDetail, masterDetailView } from '../ui/masterDetail';
  import {
    credentialKindOf,
    hostUpdate,
    keyDelete,
    keyGenerate,
    keyImport,
    keyInstall,
    keyDefaultFiles,
    keyLoadFile,
    keyReveal,
    keyUpdate,
    type HostEntry,
    type KeyType,
    type SshKey,
  } from '../ipc';
  import { errorText, t } from '../i18n';
  import { hosts } from '../hosts/hosts.svelte';
  import { keys } from './keys.svelte';

  let { onclose, initial = null }: { onclose: () => void; initial?: string | null } = $props();

  type Mode = 'view' | 'generate' | 'import';

  // svelte-ignore state_referenced_locally
  let selectedId = $state<string | null>(initial);
  // svelte-ignore state_referenced_locally
  let compactDetail = $state(initialCompactDetail(!!initial));
  const layoutMode = $derived(masterDetailView(viewport.compact, compactDetail));
  let mode = $state<Mode>('view');
  let error = $state('');
  let notice = $state('');
  let busy = $state(false);

  // Form fields shared by generate, import and edit.
  let name = $state('');
  let kind = $state<KeyType>('ed25519');
  let comment = $state('');
  let privateKey = $state('');
  let publicKey = $state('');
  let passphrase = $state('');
  let needsPassphrase = $state(false);

  // Viewing the selected key.
  let revealed = $state<{ private_key: string; passphrase: string | null } | null>(null);
  let renaming = $state(false);
  /** Editing the selected key's public key. */
  let editingPublic = $state(false);
  let installHost = $state('');
  let installPassword = $state('');
  let installNeedsPassword = $state(false);
  let switchAuth = $state(true);
  let installFeedback = $state<{ kind: 'success' | 'warning' | 'error'; text: string } | null>(null);
  type CopyField = 'fingerprint' | 'public' | 'private';
  let copied = $state<CopyField | null>(null);
  let copyTimer: number | undefined;
  let copyRequest = 0;

  const selected = $derived(keys.items.find((key) => key.id === selectedId) ?? null);
  let query = $state('');
  const shown = $derived(keys.items.filter((key) => !query.trim() || `${key.name} ${key.algorithm}`.toLowerCase().includes(query.trim().toLowerCase())));
  const hostsByKey = $derived.by(() => {
    const grouped = new Map<string, HostEntry[]>();
    for (const host of hosts.items) {
      if (host.auth.method !== 'key') continue;
      const group = grouped.get(host.auth.key);
      if (group) group.push(host);
      else grouped.set(host.auth.key, [host]);
    }
    return grouped;
  });
  const usedBy = $derived(selected ? hostsByKey.get(selected.id) ?? [] : []);
  const installableHosts = $derived(selected ? hosts.items.filter((host) => !(host.auth.method === 'key' && host.auth.key === selected.id)) : []);
  const installOptions = $derived(installableHosts.map((host) => ({ value: host.id, label: `${host.name} (${host.username}@${host.hostname})` })));

  $effect(() => {
    if (layoutMode === 'detail') return pushBackHandler(() => { compactDetail = false; });
  });

  $effect(() => {
    if (!selected?.id || !installHost || !installableHosts.some((host) => host.id === installHost)) {
      installHost = installableHosts[0]?.id ?? '';
      clearInstallPassword();
    }
  });

  /** `ssh-ed25519` as `Ed25519`, for the list's tag. */
  function algorithmTag(algorithm: string): string {
    const lower = algorithm.toLowerCase();
    if (lower.includes('ed25519')) return t('keys.algorithm.ed25519');
    if (lower.includes('ecdsa')) return t('keys.algorithm.ecdsa');
    if (lower.includes('rsa')) return t('keys.algorithm.rsa');
    return algorithm;
  }

  function clearInstallPassword(): void {
    installPassword = '';
    installNeedsPassword = false;
  }

  function clearCopyFeedback(): void {
    copyRequest += 1;
    window.clearTimeout(copyTimer);
    copied = null;
  }

  function reset(next: Mode): void {
    mode = next;
    error = '';
    notice = '';
    name = '';
    comment = '';
    privateKey = '';
    publicKey = '';
    passphrase = '';
    editingPublic = false;
    needsPassphrase = false;
    revealed = null;
    renaming = false;
    clearInstallPassword();
    clearCopyFeedback();
    switchAuth = true;
    installFeedback = null;
  }

  function select(key: SshKey, showDetail = true): void {
    reset('view');
    selectedId = key.id;
    name = key.name;
    compactDetail = showDetail;
  }

  function openMode(next: 'generate' | 'import'): void {
    reset(next);
    compactDetail = true;
  }

  async function submitNew(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    busy = true;
    error = '';
    try {
      const created =
        mode === 'generate'
          ? await keyGenerate(name, kind, comment, passphrase || null)
          : await keyImport(name, privateKey, passphrase || null, publicKey.trim() || null);
      await keys.refresh();
      select(created, compactDetail);
    } catch (caught) {
      if (credentialKindOf(caught) === 'passphrase') needsPassphrase = true;
      error = errorText(caught);
    } finally {
      busy = false;
    }
  }

  let keyMenu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  /** `~/.ssh/id_ed25519` for a path inside the user's `.ssh` directory. */
  function shortPath(path: string): string {
    const at = path.replace(/\\/g, '/').lastIndexOf('/.ssh/');
    return at >= 0 ? `~${path.replace(/\\/g, '/').slice(at)}` : path;
  }

  /**
   * Fills the form from the user's own keys: one default key in ~/.ssh is read directly, several
   * are offered in a menu, and with none the file picker opens. The chosen file may hold either
   * half; the other half is read from beside it.
   */
  async function loadKey(event: MouseEvent): Promise<void> {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const found = await keyDefaultFiles().catch(() => [] as string[]);
    const [only] = found;
    if (found.length === 1 && only) {
      await applyKeyFile(only);
    } else if (found.length > 1) {
      keyMenu = {
        x: rect.left,
        y: rect.bottom + 4,
        items: [
          ...found.map((path) => ({ label: shortPath(path), icon: 'key' as const, onselect: () => void applyKeyFile(path) })),
          { label: t('keys.chooseOther'), icon: 'folder' as const, separated: true, onselect: () => void chooseKeyFile() },
        ],
      };
    } else {
      await chooseKeyFile();
    }
  }

  async function chooseKeyFile(): Promise<void> {
    const path = await openDialog({ multiple: false, directory: false, title: t('keys.chooseFile') });
    if (typeof path === 'string') await applyKeyFile(path);
  }

  async function applyKeyFile(path: string): Promise<void> {
    error = '';
    try {
      const loaded = await keyLoadFile(path);
      if (loaded.private_key) privateKey = loaded.private_key;
      if (loaded.public_key) publicKey = loaded.public_key;
      if (!name.trim()) name = loaded.name;
      notice = t('keys.loadedFrom', { path: shortPath(loaded.path) });
      if (!loaded.private_key) error = t('keys.publicOnly');
    } catch (caught) {
      error = errorText(caught);
    }
  }

  function startEditPublic(): void {
    if (!selected) return;
    publicKey = selected.public_key;
    editingPublic = true;
    error = '';
  }

  async function savePublic(): Promise<void> {
    if (!selected) return;
    try {
      await keyUpdate(selected.id, selected.name, null, null, publicKey);
      await keys.refresh();
      editingPublic = false;
    } catch (caught) {
      error = errorText(caught);
    }
  }

  async function rename(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!selected) return;
    try {
      await keyUpdate(selected.id, name, null, null);
      await keys.refresh();
      renaming = false;
    } catch (caught) {
      error = errorText(caught);
    }
  }

  async function reveal(): Promise<void> {
    if (revealed) {
      revealed = null;
      clearCopyFeedback();
      return;
    }
    if (!selected || busy) return;
    busy = true;
    error = '';
    try {
      revealed = await keyReveal(selected.id);
    } catch (caught) {
      error = errorText(caught);
    } finally {
      busy = false;
    }
  }

  async function copy(field: CopyField, text: string): Promise<void> {
    clearCopyFeedback();
    const request = copyRequest;
    error = '';
    try {
      await writeClipboard(text);
      if (request !== copyRequest) return;
      copied = field;
      copyTimer = window.setTimeout(() => (copied = null), 1600);
    } catch (caught) {
      if (request === copyRequest) error = errorText(caught);
    }
  }

  async function install(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const key = selected;
    const host = installableHosts.find((item) => item.id === installHost);
    if (busy || !key || !host) return;
    const updateAuth = switchAuth;
    busy = true;
    installFeedback = null;
    let installed = false;
    let authUpdated = false;
    try {
      const typed = installNeedsPassword ? installPassword : null;
      await keyInstall(key.id, host.id, typed, typed === null ? 'natural' : 'password');
      installed = true;
      clearInstallPassword();
      if (updateAuth) {
        const { name, group, hostname, port, username, route, tags } = hosts.find(host.id) ?? host;
        await hostUpdate(host.id, {
          name, group, hostname, port, username, route, tags,
          auth: { method: 'key', key: key.id },
        }, null, false);
        authUpdated = true;
        await hosts.refresh();
      }
      installFeedback = {
        kind: 'success',
        text: t(authUpdated ? 'keys.installedAndSwitched' : 'keys.installed', { host: host.name }),
      };
    } catch (caught) {
      if (installed) {
        installFeedback = {
          kind: 'warning',
          text: t(authUpdated ? 'keys.installRefreshFailed' : 'keys.installAuthFailed', { host: host.name, error: errorText(caught) }),
        };
      } else {
        const sshError = typeof caught === 'object' && caught !== null && 'code' in caught && String(caught.code).startsWith('ssh.');
        if (installHost === host.id && (credentialKindOf(caught) || sshError)) {
          installNeedsPassword = true;
        }
        installFeedback = { kind: 'error', text: errorText(caught) };
      }
    } finally {
      busy = false;
    }
  }

  async function remove(): Promise<void> {
    if (!selected) return;
    const confirmed = await ask({
      title: t('keys.delete'),
      message: t('keys.confirmDelete', { name: selected.name }),
      confirmLabel: t('common.delete'),
      danger: true,
    });
    if (!confirmed) return;
    try {
      await keyDelete(selected.id);
      await keys.refresh();
      const first = keys.items[0];
      if (first) select(first, compactDetail);
      else {
        selectedId = null;
        reset('generate');
      }
      if (viewport.compact) compactDetail = false;
    } catch (caught) {
      error = errorText(caught);
    }
  }

  onMount(() => {
    void keys.refresh().then(() => {
      if (mode !== 'view' || selectedId !== initial) return;
      const target = initial ? keys.find(initial) : undefined;
      const first = keys.items[0];
      if (target) select(target, compactDetail);
      else if (first) select(first, false);
      else {
        selectedId = null;
        reset('generate');
        compactDetail = false;
      }
    });
  });

  onDestroy(clearCopyFeedback);
</script>

<Dialog title={t('keys.title')} {onclose} width={800}>
  <div class="layout" class:compact={viewport.compact}>
    {#if layoutMode !== 'detail'}
      <nav aria-label={t('keys.title')}>
        <div class="search">
          <Icon name="search" size={13} />
          <input bind:value={query} placeholder={t('keys.search')} aria-label={t('keys.search')} spellcheck="false" />
        </div>
        <div class="list">
          {#each shown as key (key.id)}
            {@const count = hostsByKey.get(key.id)?.length ?? 0}
            <button type="button" class="item" class:active={layoutMode === 'both' && mode === 'view' && key.id === selectedId} disabled={busy} onclick={() => select(key)}>
              <span class="glyph"><Icon name="key" size={12} /></span>
              <span class="text">
                <span class="name" title={key.name}>{key.name}</span>
                <span class="meta">
                  <span class="tag">{algorithmTag(key.algorithm)}</span>
                  {#if count > 0}
                    <span class="tag count" title={t('keys.usageCount', { count })} aria-label={t('keys.usageCount', { count })}>{count}</span>
                  {/if}
                  {#if !key.has_private}<span class="warn">{t('keys.missingPrivate')}</span>{/if}
                </span>
              </span>
              {#if layoutMode === 'list'}
                <span class="item-chevron"><Icon name="chevron" size={14} /></span>
              {/if}
            </button>
          {:else}
            <p class="empty">{keys.items.length === 0 ? t('keys.empty') : t('host.noMatch')}</p>
          {/each}
        </div>
      </nav>
    {/if}

    {#if layoutMode !== 'list'}
      <div class="pane">
        {#if layoutMode === 'detail'}
          <div class="compact-bar">
            <button type="button" class="plain back-btn" onclick={() => (compactDetail = false)}>
              <Icon name="back" size={16} />{t('keys.backToList')}
            </button>
          </div>
        {/if}
        {#if mode === 'view' && selected}
          <section class="card summary">
            <div class="head">
              {#if renaming}
                <form class="rename" onsubmit={rename}>
                  <!-- svelte-ignore a11y_autofocus -->
                  <input class="field" bind:value={name} required autofocus aria-label={t('keys.name')} />
                  <button type="submit" class="primary" disabled={busy}>{t('common.save')}</button>
                  <button type="button" class="secondary" onclick={() => ((renaming = false), (name = selected?.name ?? ''))}>{t('common.cancel')}</button>
                </form>
              {:else}
                <h2 title={selected.name}>{selected.name}</h2>
                <IconButton icon="edit" label={t('keys.rename')} small disabled={busy} onclick={() => (renaming = true)} />
                <span class="spacer"></span>
                <button type="button" class="secondary danger-text" disabled={busy} onclick={() => void remove()}>{t('keys.delete')}</button>
              {/if}
            </div>
            <div class="meta">
              <span class="tag">{algorithmTag(selected.algorithm)}</span>
              <span class="tag" class:stored={selected.has_private} class:missing={!selected.has_private}>
                {t(selected.has_private ? 'keys.privateStored' : 'keys.missingPrivate')}
              </span>
            </div>
            <div class="fingerprint">
              <span class="fingerprint-label">{t('keys.fingerprint')}</span>
              <code class="mono selectable">{selected.fingerprint}</code>
              <div class="copy-action">
                <IconButton icon={copied === 'fingerprint' ? 'check' : 'copy'} small active={copied === 'fingerprint'}
                  label={t(copied === 'fingerprint' ? 'keys.copied' : 'keys.copyFingerprint')}
                  onclick={() => selected && void copy('fingerprint', selected.fingerprint)} />
              </div>
            </div>
          </section>

          <section class="card">
            <div class="card-header">
              <h3>{t('keys.usedBy')}</h3>
              <span class="tag count">{usedBy.length}</span>
            </div>
            {#if usedBy.length}
              <ul class="host-chips">
                {#each usedBy as host (host.id)}
                  <li class="tag" title={`${host.username}@${host.hostname}`}>{host.name}</li>
                {/each}
              </ul>
            {:else}
              <p class="hint">{t('keys.unused')}</p>
            {/if}
          </section>

          <section class="card">
            <div class="card-header">
              <h3>{t('keys.publicKey')}</h3>
              {#if !editingPublic}
                <div class="actions">
                  {#if selected.has_private}<button type="button" class="secondary" onclick={startEditPublic}>{t('keys.editPublic')}</button>{/if}
                  <button type="button" class="secondary" class:copied={copied === 'public'} onclick={() => selected && void copy('public', selected.public_key)}>
                    <Icon name={copied === 'public' ? 'check' : 'copy'} size={13} />
                    {t(copied === 'public' ? 'keys.copied' : 'keys.copyPublic')}
                  </button>
                </div>
              {/if}
            </div>
            {#if editingPublic}
              <textarea class="mono" rows="3" bind:value={publicKey} spellcheck="false" aria-label={t('keys.publicKey')}></textarea>
              <p class="hint">{t('keys.publicHint')}</p>
              <div class="actions">
                <button type="button" class="secondary" onclick={() => (editingPublic = false)}>{t('common.cancel')}</button>
                <button type="button" class="primary" disabled={busy} onclick={() => void savePublic()}>{t('common.save')}</button>
              </div>
            {:else}
              <pre class="key selectable">{selected.public_key}</pre>
            {/if}
          </section>

          <section class="card">
            <div class="card-header">
              <h3>{t('keys.privateKey')}</h3>
              {#if revealed?.passphrase}<span class="tag">{t('keys.hasPassphrase')}</span>{/if}
              {#if selected.has_private}
                <div class="actions">
                  <button type="button" class="secondary" disabled={busy && !revealed} onclick={() => void reveal()}>{t(revealed ? 'keys.hidePrivate' : 'keys.showPrivate')}</button>
                  {#if revealed}
                    <button type="button" class="secondary" class:copied={copied === 'private'} onclick={() => revealed && void copy('private', revealed.private_key)}>
                      <Icon name={copied === 'private' ? 'check' : 'copy'} size={13} />
                      {t(copied === 'private' ? 'keys.copied' : 'keys.copyPrivate')}
                    </button>
                  {/if}
                </div>
              {/if}
            </div>
            {#if revealed}
              <pre class="key selectable">{revealed.private_key}</pre>
            {:else}
              <p class="hint" class:warn={!selected.has_private}>{t(selected.has_private ? 'keys.privateStoredHint' : 'keys.missingPrivateHint')}</p>
            {/if}
          </section>

          <form class="card" onsubmit={install}>
            <div class="card-header"><h3>{t('keys.installTitle')}</h3></div>
            {#if hosts.items.length === 0}
              <p class="hint">{t('keys.noHosts')}</p>
            {:else if installableHosts.length === 0}
              <p class="hint">{t('keys.allHostsUsing')}</p>
            {:else}
              <p class="hint">{t('keys.installHint')}</p>
              <div class="install-field">
                <span>{t('keys.installHost')}</span>
                <Select bind:value={installHost} label={t('keys.installHost')} options={installOptions} disabled={busy}
                  onchange={() => { clearInstallPassword(); installFeedback = null; }} />
              </div>
              <div class="install-field">
                <span>{t('keys.switchHostAuth')}</span>
                <Switch bind:checked={switchAuth} label={t('keys.switchHostAuth')} disabled={busy} />
              </div>
              {#if installNeedsPassword}
                <label class="install-field">
                  <span>{t('host.password')}</span>
                  <input class="field" type="password" bind:value={installPassword} autocomplete="off" required disabled={busy} />
                </label>
              {/if}
            {/if}
            {#if installFeedback}
              <p class="feedback selectable {installFeedback.kind}" role={installFeedback.kind === 'error' ? 'alert' : 'status'}>
                <Icon name={installFeedback.kind === 'success' ? 'check' : 'warning'} size={14} />
                <span>{installFeedback.text}</span>
              </p>
            {/if}
            <div class="actions">
              <button type="submit" class="primary" disabled={busy || !installHost || installableHosts.length === 0}>{t('keys.install')}</button>
            </div>
          </form>
        {:else if mode === 'generate' || mode === 'import'}
          <form id="key-form" onsubmit={submitNew}>
            <div class="head"><h2>{mode === 'generate' ? t('keys.generate') : t('keys.import')}</h2></div>
            <div class="form-group">
              <div class="form-row">
                <span class="form-label">{t('keys.name')}</span>
                <input class="field" bind:value={name} required autocomplete="off" aria-label={t('keys.name')} />
              </div>
              {#if mode === 'generate'}
                <div class="form-row">
                  <span class="form-label">{t('keys.type')}</span>
                  <div class="control">
                    <Select
                      bind:value={kind}
                      options={[
                        { value: 'ed25519', label: t('keys.algorithm.ed25519') },
                        { value: 'ecdsa_p256', label: t('keys.algorithm.p256') },
                        { value: 'rsa3072', label: t('keys.algorithm.rsa3072') },
                      ]}
                    />
                  </div>
                </div>
                <div class="form-row">
                  <span class="form-label">{t('keys.comment')}</span>
                  <input class="field" bind:value={comment} placeholder={t('keys.commentExample')} autocomplete="off" aria-label={t('keys.comment')} />
                </div>
              {/if}
              <div class="form-row">
                <span class="form-label">{mode === 'generate' ? t('keys.newPassphrase') : t('host.passphrase')}</span>
                <input class="field" type="password" bind:value={passphrase} autocomplete="new-password" class:attention={needsPassphrase} aria-label={t('host.passphrase')} />
              </div>
            </div>
            {#if mode === 'import'}
              <p class="section-label">{t('keys.privateKey')}</p>
              <div class="form-group">
                <div class="form-row stacked">
                  <textarea class="mono" rows="7" bind:value={privateKey} required spellcheck="false" placeholder={t('keys.privateExample')} aria-label={t('keys.privateKey')}></textarea>
                  <div class="actions"><button type="button" class="secondary" onclick={(event) => void loadKey(event)}>{t('keys.fromFile')}</button></div>
                </div>
              </div>
              <p class="section-label">{t('keys.publicKeyOptional')}</p>
              <div class="form-group">
                <div class="form-row stacked">
                  <textarea class="mono" rows="2" bind:value={publicKey} spellcheck="false" placeholder={t('keys.publicExample')} aria-label={t('keys.publicKeyOptional')}></textarea>
                  <p class="hint">{t('keys.publicHint')}</p>
                </div>
              </div>
            {/if}
            <div class="submit">
              <button type="submit" class="primary" disabled={busy}>{busy && mode === 'generate' ? t('keys.generating') : mode === 'generate' ? t('keys.generate') : t('keys.import')}</button>
            </div>
          </form>
        {/if}
        {#if error}<p class="message error">{error}</p>{/if}
        {#if notice}<p class="message notice">{notice}</p>{/if}
      </div>
    {/if}
  </div>
  {#snippet actions()}
    {#if layoutMode !== 'detail'}
      <div class="footer-actions" class:compact={viewport.compact}>
        <button type="button" class="secondary" class:active={layoutMode === 'both' && mode === 'generate'} disabled={busy} onclick={() => openMode('generate')}>
          <Icon name="add" size={13} />{t('keys.generate')}
        </button>
        <button type="button" class="secondary" class:active={layoutMode === 'both' && mode === 'import'} disabled={busy} onclick={() => openMode('import')}>
          <Icon name="importConfig" size={13} />{t('keys.import')}
        </button>
      </div>
    {/if}
    <button type="button" class="primary" onclick={onclose}>{t('settings.done')}</button>
  {/snippet}
</Dialog>

{#if keyMenu}
  <ContextMenu x={keyMenu.x} y={keyMenu.y} items={keyMenu.items} onclose={() => (keyMenu = null)} />
{/if}

<style>
  /* Matches SettingsDialog: full-bleed split bordered cleanly above the unified dialog footer. */
  .layout {
    display: flex;
    height: min(580px, 70vh);
    margin: 0 calc(-1 * var(--space-5));
    box-shadow:
      inset 0 1px 0 var(--separator),
      inset 0 -1px 0 var(--separator);
  }
  /* A sidebar of its own, the full height of the split view. */
  nav {
    flex: 0 0 232px;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    box-shadow: inset -1px 0 0 var(--separator);
    min-height: 0;
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--control-height);
    padding: 0 var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg-muted);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    box-shadow: none;
    color: var(--fg);
    font: inherit;
    font-size: var(--text-footnote);
    outline: none;
    padding: 0;
  }
  .list {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    overflow-y: auto;
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: left;
    cursor: default;
  }
  .item:hover:not(.active) {
    background: var(--fill-hover);
  }
  .item.active {
    background: var(--accent-soft);
    color: var(--fg);
  }
  .glyph {
    display: grid;
    flex: none;
    place-items: center;
    width: var(--space-6);
    height: var(--space-6);
    border-radius: var(--radius-sm);
    background: var(--warning-soft);
    color: var(--warning);
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    display: flex;
    gap: var(--space-1);
    align-items: center;
    flex-wrap: wrap;
    font-size: var(--text-caption);
  }
  .tag {
    padding: 0 var(--space-2);
    border-radius: var(--radius-pill);
    background: var(--fill);
    color: var(--fg-muted);
  }
  .count {
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
  }
  .stored {
    background: var(--success-soft);
    color: var(--success);
  }
  .missing {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .warn {
    color: var(--warning);
  }
  .empty {
    margin: var(--space-3) var(--space-2);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .footer-actions {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 6px;
    width: calc(232px - var(--space-3) * 2);
    margin-left: calc(var(--space-3) - var(--space-5));
    margin-right: auto;
  }
  .footer-actions button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    min-width: 0;
    padding: 0 var(--space-2);
    font-size: var(--text-footnote);
  }
  .footer-actions button.active {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4) var(--space-5) var(--space-6);
    overflow-y: auto;
    background: color-mix(in srgb, var(--fill) 50%, transparent);
  }
  .pane > :global(*),
  form > :global(*) {
    flex-shrink: 0;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .pane :global(.section-label) {
    margin-top: var(--space-3);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-height: 32px;
  }
  h2 {
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-large);
    font-weight: 700;
    letter-spacing: -0.01em;
  }
  .spacer {
    flex: 1;
  }
  .rename {
    flex: 1;
    flex-direction: row;
    align-items: center;
  }
  .rename .field {
    flex: 1;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--separator);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
  }
  .summary {
    gap: var(--space-3);
  }
  .card-header,
  .fingerprint,
  .install-field {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .card-header .actions {
    margin-inline-start: auto;
  }
  .card-header h3 {
    margin: 0;
    font-size: var(--text-body);
    font-weight: 650;
  }
  .fingerprint {
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
  }
  .fingerprint-label {
    color: var(--fg-muted);
    font-size: var(--text-caption);
  }
  .fingerprint code {
    flex: 1 1 15em;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .copy-action {
    flex: none;
    margin-inline-start: auto;
  }
  .host-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .host-chips li {
    padding-block: var(--space-1);
    color: var(--fg);
    overflow-wrap: anywhere;
  }
  .install-field {
    justify-content: space-between;
  }
  .install-field > span {
    flex: 1 1 auto;
  }
  .feedback {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin: 0;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    font-size: var(--text-footnote);
    overflow-wrap: anywhere;
  }
  .feedback.success {
    background: var(--success-soft);
    color: var(--success);
  }
  .feedback.warning {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .feedback.error {
    background: var(--danger-soft);
  }
  .copied {
    color: var(--success);
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-footnote);
  }
  .stacked {
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-2);
  }
  .key {
    margin: 0;
    max-height: 160px;
    overflow: auto;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
    font-family: var(--font-mono);
    font-size: var(--text-footnote);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  textarea {
    resize: vertical;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--separator);
    color: var(--fg);
  }
  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .hint {
    margin: 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .hint.warn {
    color: var(--warning);
  }
  .field {
    width: 240px;
    max-width: 100%;
    min-width: 0;
  }
  .control {
    width: 260px;
    max-width: 100%;
  }
  .submit {
    display: flex;
    justify-content: flex-end;
    margin-top: var(--space-2);
  }
  .attention {
    box-shadow: 0 0 0 1px var(--danger);
  }
  .danger-text {
    color: var(--danger);
  }
  .message {
    margin: 0;
    font-size: var(--text-footnote);
  }
  .error {
    color: var(--danger);
  }
  .notice {
    color: var(--success);
  }
  .layout.compact {
    flex-direction: column;
    height: min(72dvh, 640px);
  }
  .compact nav {
    flex: 1;
    min-width: 0;
    box-shadow: none;
  }
  .compact .search,
  .compact .item {
    flex-shrink: 0;
  }
  .compact .item {
    min-height: 48px;
    cursor: pointer;
  }
  .item-chevron {
    display: inline-flex;
    flex: none;
    margin-inline-start: auto;
    color: var(--fg-muted);
  }
  .footer-actions.compact {
    display: flex;
    flex-wrap: wrap;
    width: auto;
    min-width: 0;
    margin-left: 0;
  }
  .footer-actions.compact button,
  .compact .pane button {
    max-width: 100%;
    white-space: normal;
    overflow-wrap: anywhere;
    height: auto;
    min-height: var(--control-height);
  }
  .compact .pane {
    padding: var(--space-3) var(--space-3) var(--space-5);
  }
  .compact :is(.head, .card-header, .fingerprint, .install-field, .rename, .actions) {
    flex-wrap: wrap;
    min-width: 0;
  }
  .compact .head h2 {
    flex: 1 1 8rem;
    min-width: 0;
  }
  .compact .install-field > span,
  .compact .control {
    min-width: 0;
  }
  @media (max-width: 560px) {
    .compact :is(.control, .field) {
      width: 100%;
      max-width: 100%;
    }
    .compact .rename .field {
      flex-basis: 100%;
    }
  }
  @media (pointer: coarse) {
    .compact .pane :global(.icon-button) {
      min-width: var(--control-height);
      min-height: var(--control-height);
    }
  }
</style>
