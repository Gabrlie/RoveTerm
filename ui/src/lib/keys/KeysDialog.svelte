<script lang="ts">
  import { writeClipboard } from '../ui/clipboard';
  // The key library, laid out like the settings: keys on the left, the selected key (or the
  // generate / import form) on the right as grouped cards.
  import { onMount } from 'svelte';
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import ContextMenu, { type MenuItem } from '../ui/ContextMenu.svelte';
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
    keyDefaultFiles,
    keyLoadFile,
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
  let query = $state('');
  const shown = $derived(keys.items.filter((key) => !query.trim() || `${key.name} ${key.algorithm}`.toLowerCase().includes(query.trim().toLowerCase())));
  /** Hosts that log in with the selected key. */
  const usedBy = $derived(selected ? hosts.items.filter((host) => host.auth.method === 'key' && host.auth.key === selected.id) : []);

  /** `ssh-ed25519` as `Ed25519`, for the list's tag. */
  function algorithmTag(algorithm: string): string {
    const lower = algorithm.toLowerCase();
    if (lower.includes('ed25519')) return 'Ed25519';
    if (lower.includes('ecdsa')) return 'ECDSA';
    if (lower.includes('rsa')) return 'RSA';
    return algorithm;
  }
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
    if (!selected) return;
    try {
      revealed = revealed ? null : await keyReveal(selected.id);
    } catch (caught) {
      error = errorText(caught);
    }
  }

  async function copy(text: string): Promise<void> {
    await writeClipboard(text).catch((caught: unknown) => console.warn('copy failed', caught));
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

<Dialog title={t('keys.title')} {onclose} width={800}>
  <div class="layout">
    <nav aria-label={t('keys.title')}>
      <div class="search">
        <Icon name="search" size={13} />
        <input bind:value={query} placeholder={t('keys.search')} aria-label={t('keys.search')} spellcheck="false" />
      </div>
      <div class="list">
        {#each shown as key (key.id)}
          <button type="button" class="item" class:active={mode === 'view' && key.id === selectedId} onclick={() => select(key)}>
            <span class="glyph"><Icon name="key" size={12} /></span>
            <span class="text">
              <span class="name">{key.name}</span>
              <span class="meta">
                <span class="tag">{algorithmTag(key.algorithm)}</span>
                {#if !key.has_private}<span class="warn">{t('keys.missingPrivate')}</span>{/if}
              </span>
            </span>
          </button>
        {:else}
          <p class="empty">{keys.items.length === 0 ? t('keys.empty') : t('host.noMatch')}</p>
        {/each}
      </div>
      <!-- At the foot of the sidebar. -->
      <div class="add">
        <button type="button" class:active={mode === 'generate'} onclick={() => reset('generate')}><Icon name="add" size={13} />{t('keys.generate')}</button>
        <button type="button" class:active={mode === 'import'} onclick={() => reset('import')}><Icon name="importConfig" size={13} />{t('keys.import')}</button>
      </div>
    </nav>

    <div class="main">
    <div class="pane">
      {#if mode === 'view' && selected}
        <div class="head">
          {#if renaming}
            <form class="rename" onsubmit={rename}>
              <!-- svelte-ignore a11y_autofocus -->
              <input class="field" bind:value={name} required autofocus aria-label={t('keys.name')} />
              <button type="submit" class="primary">{t('common.save')}</button>
              <button type="button" class="secondary" onclick={() => ((renaming = false), (name = selected?.name ?? ''))}>{t('common.cancel')}</button>
            </form>
          {:else}
            <h2>{selected.name}</h2>
            <IconButton icon="edit" label={t('keys.rename')} small onclick={() => (renaming = true)} />
            <span class="spacer"></span>
            <button type="button" class="secondary danger-text" onclick={() => void remove()}>{t('keys.delete')}</button>
          {/if}
        </div>

        <div class="form-group">
          <div class="form-row">
            <span class="form-label">{t('keys.type')}</span>
            <span class="value">{selected.algorithm}</span>
          </div>
          <div class="form-row">
            <span class="form-label">{t('keys.fingerprint')}</span>
            <span class="value mono selectable">{selected.fingerprint}</span>
            <IconButton icon="copy" small label={t('edit.copy')} onclick={() => void copy(selected.fingerprint)} />
          </div>
          <div class="form-row">
            <span class="form-label">{t('keys.usedBy')}</span>
            <span class="value">{usedBy.length > 0 ? usedBy.map((host) => host.name).join(', ') : t('keys.unused')}</span>
          </div>
        </div>

        <p class="section-label">{t('keys.publicKey')}</p>
        <div class="form-group">
          <div class="form-row stacked">
            {#if editingPublic}
              <textarea class="mono" rows="3" bind:value={publicKey} spellcheck="false" aria-label={t('keys.publicKey')}></textarea>
              <p class="hint">{t('keys.publicHint')}</p>
              <div class="actions">
                <button type="button" class="secondary" onclick={() => (editingPublic = false)}>{t('common.cancel')}</button>
                <button type="button" class="primary" onclick={() => void savePublic()}>{t('common.save')}</button>
              </div>
            {:else}
              <pre class="key selectable">{selected.public_key}</pre>
              <div class="actions">
                {#if selected.has_private}<button type="button" class="secondary" onclick={startEditPublic}>{t('keys.editPublic')}</button>{/if}
                <button type="button" class="primary" onclick={() => void copy(selected.public_key)}>{t('keys.copyPublic')}</button>
              </div>
            {/if}
          </div>
        </div>

        <p class="section-label">{t('keys.privateKey')}</p>
        <div class="form-group">
          {#if selected.has_private}
            {#if revealed}
              <div class="form-row stacked">
                <pre class="key selectable">{revealed.private_key}</pre>
                <div class="actions">
                  {#if revealed.passphrase}<span class="hint grow">{t('keys.hasPassphrase')}</span>{/if}
                  <button type="button" class="secondary" onclick={() => void reveal()}>{t('keys.hidePrivate')}</button>
                  <button type="button" class="primary" onclick={() => revealed && void copy(revealed.private_key)}>{t('keys.copyPrivate')}</button>
                </div>
              </div>
            {:else}
              <div class="form-row">
                <span class="form-label">{t('keys.privateStored')}<small>{t('keys.privateStoredHint')}</small></span>
                <button type="button" class="secondary" onclick={() => void reveal()}>{t('keys.showPrivate')}</button>
              </div>
            {/if}
          {:else}
            <div class="form-row"><span class="form-label"><small class="warn">{t('keys.missingPrivateHint')}</small></span></div>
          {/if}
        </div>

        <p class="section-label">{t('keys.installTitle')}</p>
        <form class="form-group" onsubmit={install}>
          {#if hostOptions.length > 0}
            <div class="form-row">
              <span class="form-label">{t('keys.installHost')}<small>{t('keys.installHint')}</small></span>
              <div class="control"><Select bind:value={installHost} options={hostOptions} /></div>
            </div>
            {#if installNeedsPassword}
              <div class="form-row">
                <span class="form-label">{t('host.password')}</span>
                <input class="field" type="password" bind:value={installPassword} autocomplete="off" aria-label={t('host.password')} />
              </div>
            {/if}
            <div class="form-row end">
              <button type="submit" class="primary" disabled={busy}>{t('keys.install')}</button>
            </div>
          {:else}
            <div class="form-row"><span class="form-label"><small>{t('keys.noHosts')}</small></span></div>
          {/if}
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
                      { value: 'ed25519', label: 'Ed25519' },
                      { value: 'ecdsa_p256', label: 'ECDSA P-256' },
                      { value: 'rsa3072', label: 'RSA 3072' },
                    ]}
                  />
                </div>
              </div>
              <div class="form-row">
                <span class="form-label">{t('keys.comment')}</span>
                <input class="field" bind:value={comment} placeholder="me@laptop" autocomplete="off" aria-label={t('keys.comment')} />
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
                <textarea class="mono" rows="7" bind:value={privateKey} required spellcheck="false" placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" aria-label={t('keys.privateKey')}></textarea>
                <div class="actions"><button type="button" class="secondary" onclick={(event) => void loadKey(event)}>{t('keys.fromFile')}</button></div>
              </div>
            </div>
            <p class="section-label">{t('keys.publicKeyOptional')}</p>
            <div class="form-group">
              <div class="form-row stacked">
                <textarea class="mono" rows="2" bind:value={publicKey} spellcheck="false" placeholder="ssh-ed25519 AAAA… comment" aria-label={t('keys.publicKeyOptional')}></textarea>
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
    <div class="foot">
      <button type="button" class="primary" onclick={onclose}>{t('settings.done')}</button>
    </div>
    </div>
  </div>
</Dialog>

{#if keyMenu}
  <ContextMenu x={keyMenu.x} y={keyMenu.y} items={keyMenu.items} onclose={() => (keyMenu = null)} />
{/if}

<style>
  /* Runs to the bottom of the dialog, so the line between the list and the pane does too. */
  .layout {
    display: flex;
    height: min(620px, 74vh);
    margin: 0 calc(-1 * var(--space-5)) calc(-1 * var(--space-2));
    box-shadow: inset 0 1px 0 var(--separator);
  }
  /* A sidebar of its own, the full height of the dialog. */
  nav {
    flex: 0 0 220px;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-2);
    box-shadow: inset -1px 0 0 var(--separator);
    min-height: 0;
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 28px;
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
    gap: 2px;
    overflow-y: auto;
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 5px var(--space-2);
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
    background: var(--accent);
    color: var(--accent-fg);
  }
  .glyph {
    display: grid;
    flex: none;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    background: linear-gradient(to bottom, color-mix(in srgb, var(--sys-orange, #ff9f0a) 80%, #fff), var(--sys-orange, #ff9f0a));
    color: #fff;
    box-shadow: 0 0.5px 1px rgb(0 0 0 / 0.15);
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
    font-size: 11px;
  }
  .tag {
    padding: 0 5px;
    border-radius: var(--radius-pill);
    background: var(--fill);
    color: var(--fg-muted);
  }
  .item.active .tag {
    background: rgb(255 255 255 / 0.2);
    color: inherit;
  }
  .warn {
    color: var(--warning);
  }
  .item.active .warn {
    color: inherit;
  }
  .empty {
    margin: var(--space-3) var(--space-2);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .add {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-top: var(--space-2);
    box-shadow: inset 0 1px 0 var(--separator);
  }
  .add button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 28px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--accent);
    font: inherit;
    font-size: var(--text-footnote);
    text-align: left;
  }
  .add button:hover,
  .add button.active {
    background: var(--fill-hover);
  }
  /* The right side: the pane above its bottom bar. */
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  /* The bottom bar of the right side, apart from the sidebar's buttons. */
  .foot {
    flex: none;
    display: flex;
    justify-content: flex-end;
    padding: var(--space-3) var(--space-5) var(--space-4);
    box-shadow: inset 0 1px 0 var(--separator);
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
  .spacer,
  .grow {
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
  .value {
    min-width: 0;
    max-width: 60%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    text-align: right;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: 11.5px;
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
    font-size: 11.5px;
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
    gap: var(--space-2);
  }
  .end {
    justify-content: flex-end;
  }
  .hint {
    margin: 0;
    color: var(--fg-muted);
    font-size: 11px;
  }
  .field {
    width: 240px;
  }
  .control {
    width: 260px;
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
</style>
