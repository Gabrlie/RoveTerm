<script lang="ts">
  // The key library: list on the left, the selected key or a generate/import form on the right.
  import { onMount } from 'svelte';
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import Dialog from '../ui/Dialog.svelte';
  import Select from '../ui/Select.svelte';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { ask } from '../ui/confirm.svelte';
  import {
    credentialKindOf,
    keyDelete,
    keyGenerate,
    keyImport,
    keyInstall,
    keyReadFile,
    keyReveal,
    keyUpdate,
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

  const selected = $derived(keys.items.find((key) => key.id === selectedId) ?? null);
  const hostOptions = $derived(hosts.items.map((host) => ({ value: host.id, label: `${host.name} (${host.username}@${host.hostname})` })));

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
    installPassword = '';
    installNeedsPassword = false;
  }

  function select(key: SshKey): void {
    reset('view');
    selectedId = key.id;
    name = key.name;
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
      select(created);
    } catch (caught) {
      if (credentialKindOf(caught) === 'passphrase') needsPassphrase = true;
      error = errorText(caught);
    } finally {
      busy = false;
    }
  }

  async function loadFile(): Promise<void> {
    const path = await openDialog({ multiple: false, directory: false, title: t('keys.chooseFile') });
    if (typeof path !== 'string') return;
    try {
      privateKey = await keyReadFile(path);
      if (!name) name = path.split(/[\\/]/).pop() ?? '';
    } catch (caught) {
      error = errorText(caught);
      return;
    }
    // OpenSSH keeps the public key next to the private one; use it when present.
    if (!publicKey.trim()) {
      publicKey = await keyReadFile(`${path}.pub`).catch(() => '');
    }
  }

  async function loadPublicFile(): Promise<void> {
    const path = await openDialog({ multiple: false, directory: false, title: t('keys.choosePublicFile') });
    if (typeof path !== 'string') return;
    try {
      publicKey = (await keyReadFile(path)).trim();
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
    if (!selected) return;
    try {
      revealed = revealed ? null : await keyReveal(selected.id);
    } catch (caught) {
      error = errorText(caught);
    }
  }

  async function copy(text: string): Promise<void> {
    await navigator.clipboard.writeText(text).catch((caught: unknown) => console.warn('copy failed', caught));
    notice = t('keys.copied');
  }

  async function install(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!selected || !installHost) return;
    busy = true;
    error = '';
    notice = '';
    try {
      const typed = installNeedsPassword ? installPassword : null;
      await keyInstall(selected.id, installHost, typed, typed === null ? 'natural' : 'password');
      notice = t('keys.installed', { host: hosts.find(installHost)?.name ?? '' });
      installNeedsPassword = false;
      installPassword = '';
    } catch (caught) {
      // Installing a key usually happens before key login works: offer a password.
      if (credentialKindOf(caught) || (typeof caught === 'object' && caught !== null && 'code' in caught && String(caught.code).startsWith('ssh.'))) {
        installNeedsPassword = true;
      }
      error = errorText(caught);
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
      selectedId = null;
      await keys.refresh();
    } catch (caught) {
      error = errorText(caught);
    }
  }

  onMount(() => {
    void keys.refresh().then(() => {
      const first = selectedId ? keys.find(selectedId) : keys.items[0];
      if (first) select(first);
      else reset('generate');
    });
    if (hosts.items[0]) installHost = hosts.items[0].id;
  });
</script>

<Dialog title={t('keys.title')} {onclose} width={760}>
  <div class="layout">
    <aside>
      <div class="list">
        {#each keys.items as key (key.id)}
          <button type="button" class="item" class:active={mode === 'view' && key.id === selectedId} onclick={() => select(key)}>
            <Icon name="key" size={14} />
            <span class="text">
              <span class="name">{key.name}</span>
              <span class="meta">{key.algorithm}{key.has_private ? '' : ` · ${t('keys.missingPrivate')}`}</span>
            </span>
          </button>
        {:else}
          <p class="empty">{t('keys.empty')}</p>
        {/each}
      </div>
      <div class="add">
        <button type="button" class:active={mode === 'generate'} onclick={() => reset('generate')}><Icon name="add" size={14} />{t('keys.generate')}</button>
        <button type="button" class:active={mode === 'import'} onclick={() => reset('import')}><Icon name="importConfig" size={14} />{t('keys.import')}</button>
      </div>
    </aside>

    <section>
      {#if mode === 'view' && selected}
        <div class="head">
          {#if renaming}
            <form class="rename" onsubmit={rename}>
              <!-- svelte-ignore a11y_autofocus -->
              <input bind:value={name} required autofocus />
              <button type="submit" class="secondary">{t('common.save')}</button>
            </form>
          {:else}
            <h3>{selected.name}</h3>
            <IconButton icon="edit" label={t('keys.rename')} small onclick={() => (renaming = true)} />
            <IconButton icon="remove" label={t('keys.delete')} small danger onclick={() => void remove()} />
          {/if}
        </div>
        <p class="meta">{selected.algorithm} · {selected.fingerprint}</p>

        {#if editingPublic}
          <label
            >{t('keys.publicKey')}
            <textarea rows="3" bind:value={publicKey} spellcheck="false"></textarea>
          </label>
          <p class="hint">{t('keys.publicHint')}</p>
          <div class="row-actions">
            <button type="button" class="secondary" onclick={() => (editingPublic = false)}>{t('common.cancel')}</button>
            <button type="button" class="primary" onclick={() => void savePublic()}>{t('common.save')}</button>
          </div>
        {:else}
          <label
            >{t('keys.publicKey')}
            <textarea readonly rows="3" value={selected.public_key}></textarea>
          </label>
        {/if}
        <div class="row-actions">
          <button type="button" class="secondary" onclick={() => void copy(selected.public_key)}>{t('keys.copyPublic')}</button>
          {#if selected.has_private && !editingPublic}
            <button type="button" class="secondary" onclick={startEditPublic}>{t('keys.editPublic')}</button>
          {/if}
          {#if selected.has_private}
            <button type="button" class="secondary" onclick={() => void reveal()}>{revealed ? t('keys.hidePrivate') : t('keys.showPrivate')}</button>
          {/if}
        </div>
        {#if revealed}
          <label
            >{t('keys.privateKey')}
            <textarea readonly rows="6" value={revealed.private_key}></textarea>
          </label>
          <div class="row-actions">
            <button type="button" class="secondary" onclick={() => revealed && void copy(revealed.private_key)}>{t('keys.copyPrivate')}</button>
            {#if revealed.passphrase}<span class="meta">{t('keys.hasPassphrase')}</span>{/if}
          </div>
        {/if}

        <form class="install" onsubmit={install}>
          <h4>{t('keys.installTitle')}</h4>
          <p class="hint">{t('keys.installHint')}</p>
          {#if hostOptions.length > 0}
            <div class="row">
              <label>{t('keys.installHost')}<Select bind:value={installHost} options={hostOptions} /></label>
              {#if installNeedsPassword}
                <label>{t('host.password')}<input type="password" bind:value={installPassword} autocomplete="off" /></label>
              {/if}
            </div>
            <div class="row-actions">
              <button type="submit" class="primary" disabled={busy}>{t('keys.install')}</button>
            </div>
          {:else}
            <p class="hint">{t('keys.noHosts')}</p>
          {/if}
        </form>
      {:else if mode === 'generate' || mode === 'import'}
        <form id="key-form" class="form" onsubmit={submitNew}>
          <h3>{mode === 'generate' ? t('keys.generate') : t('keys.import')}</h3>
          <label>{t('keys.name')}<input bind:value={name} required autocomplete="off" /></label>
          {#if mode === 'generate'}
            <div class="row">
              <label
                >{t('keys.type')}
                <Select
                  bind:value={kind}
                  options={[
                    { value: 'ed25519', label: 'Ed25519' },
                    { value: 'ecdsa_p256', label: 'ECDSA P-256' },
                    { value: 'rsa3072', label: 'RSA 3072' },
                  ]}
                />
              </label>
              <label>{t('keys.comment')}<input bind:value={comment} placeholder="me@laptop" autocomplete="off" /></label>
            </div>
          {:else}
            <label
              >{t('keys.privateKey')}
              <textarea rows="7" bind:value={privateKey} required spellcheck="false" placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"></textarea>
            </label>
            <div class="row-actions">
              <button type="button" class="secondary" onclick={() => void loadFile()}>{t('keys.fromFile')}</button>
            </div>
            <label
              >{t('keys.publicKeyOptional')}
              <textarea rows="2" bind:value={publicKey} spellcheck="false" placeholder="ssh-ed25519 AAAA… comment"></textarea>
            </label>
            <p class="hint">{t('keys.publicHint')}</p>
            <div class="row-actions">
              <button type="button" class="secondary" onclick={() => void loadPublicFile()}>{t('keys.publicFromFile')}</button>
            </div>
          {/if}
          <label
            >{mode === 'generate' ? t('keys.newPassphrase') : t('host.passphrase')}
            <input type="password" bind:value={passphrase} autocomplete="new-password" class:attention={needsPassphrase} />
          </label>
          <div class="row-actions">
            <button type="submit" class="primary" disabled={busy}>{busy && mode === 'generate' ? t('keys.generating') : t('common.save')}</button>
          </div>
        </form>
      {/if}
      {#if error}<p class="error">{error}</p>{/if}
      {#if notice}<p class="notice">{notice}</p>{/if}
    </section>
  </div>
  {#snippet actions()}
    <button type="button" class="secondary" onclick={onclose}>{t('common.close')}</button>
  {/snippet}
</Dialog>

<style>
  .layout {
    display: flex;
    gap: var(--space-4);
    min-height: 380px;
  }
  aside {
    flex: 0 0 210px;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-right: var(--space-3);
    box-shadow: inset -1px 0 0 var(--separator);
  }
  .list {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
  }
  .item,
  .add button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .item:hover,
  .add button:hover {
    background: var(--fill-hover);
  }
  .item.active,
  .add button.active {
    background: var(--accent-soft);
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
  .meta,
  .hint,
  .empty {
    margin: 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    overflow-wrap: anywhere;
  }
  .add {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-top: var(--space-2);
    box-shadow: inset 0 1px 0 var(--separator);
  }
  section {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  h3 {
    flex: 1;
    margin: 0;
    font-size: var(--text-title);
  }
  h4 {
    margin: 0;
    font-size: var(--text-body);
  }
  .rename {
    flex: 1;
    display: flex;
    gap: var(--space-2);
  }
  .rename input {
    flex: 1;
  }
  .form,
  .install {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .install {
    padding-top: var(--space-3);
    box-shadow: inset 0 1px 0 var(--separator);
  }
  .row-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: calc(-1 * var(--space-1));
  }
  .notice {
    margin: 0;
    color: var(--accent);
    font-size: var(--text-footnote);
  }
  .attention {
    outline: 2px solid color-mix(in srgb, var(--danger) 60%, transparent);
  }
</style>
