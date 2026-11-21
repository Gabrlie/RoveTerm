<script lang="ts">
  // Create or edit a host. The draft stays in the form when saving fails (for example a denied unlock).
  import Dialog from '../ui/Dialog.svelte';
  import Select from '../ui/Select.svelte';
  import { onMount } from 'svelte';
  import { hostCreate, hostNotesGet, hostNotesSet, hostUpdate, type AuthMethod, type HostDraft, type HostEntry, type HostRoute, type Secret } from '../ipc';
  import { errorText, t } from '../i18n';
  import { hosts } from './hosts.svelte';

  import { keys } from '../keys/keys.svelte';
  import { findDuplicateHost } from '../tailscale/discovery';

  interface Props {
    host: HostEntry | null;
    onclose: () => void;
    /** Opens the key library above the editor. */
    onmanagekeys: () => void;
    prefill?: { name: string; hostname: string; username?: string };
    reuseExisting?: boolean;
    onreuse?: (host: HostEntry) => void;
    onsaved?: (host: HostEntry) => void;
    submitLabel?: string;
  }

  let { host, onclose, onmanagekeys, prefill, reuseExisting = false, onreuse, onsaved, submitLabel }: Props = $props();

  type Method = AuthMethod['method'];
  type RouteKind = 'direct' | 'jump' | 'http' | 'socks5';

  // svelte-ignore state_referenced_locally
  const initial = host;
  // svelte-ignore state_referenced_locally
  const imported = prefill;
  let name = $state(initial?.name ?? imported?.name ?? '');
  let group = $state(initial?.group ?? '');
  let hostname = $state(initial?.hostname ?? imported?.hostname ?? '');
  let port = $state(initial?.port ?? 22);
  let username = $state(initial?.username ?? imported?.username ?? 'root');
  let method = $state<Method>(initial?.auth.method ?? 'password');
  let keyPath = $state(initial?.auth.method === 'key_file' ? initial.auth.path : '~/.ssh/id_ed25519');
  let libraryKey = $state(initial?.auth.method === 'key' ? initial.auth.key : '');
  const keyOptions = $derived(keys.items.map((key) => ({ value: key.id, label: `${key.name} (${key.algorithm})` })));
  // Keep a selection as keys load or get added in the library.
  $effect(() => {
    if (method === 'key' && !libraryKey && keyOptions[0]) libraryKey = keyOptions[0].value;
  });
  /** The per-host stored key is kept only for hosts that already use it. */
  const legacyPrivateKey = initial?.auth.method === 'private_key';
  let password = $state('');
  let privateKey = $state('');
  let passphrase = $state('');
  const initialRoute = initial?.route ?? { kind: 'direct' };
  let routeKind = $state<RouteKind>(initialRoute.kind === 'proxy' ? initialRoute.protocol : initialRoute.kind);
  let jumpHost = $state(initialRoute.kind === 'jump' ? initialRoute.host : '');
  let proxyHost = $state(initialRoute.kind === 'proxy' ? initialRoute.host : '127.0.0.1');
  let proxyPort = $state<number | null>(initialRoute.kind === 'proxy' ? initialRoute.port : null);
  /** Hosts this host may jump through: every other saved host. */
  const jumpOptions = hosts.items
    .filter((candidate) => candidate.id !== initial?.id)
    .map((candidate) => ({ value: candidate.id, label: `${candidate.name} (${candidate.username}@${candidate.hostname})` }));
  let error = $state('');
  let saving = $state(false);
  let duplicate = $state<HostEntry | null>(null);
  const duplicateMatch = $derived(duplicate ? findDuplicateHost([duplicate], { hostname, port: Number(port), username, route: route() }) : undefined);
  /** Notes kept about the host, for the user and the AI agents; saved with the host. */
  let notes = $state('');
  let notesLoaded = '';

  onMount(() => {
    if (!initial) return;
    hostNotesGet(initial.id)
      .then((text) => {
        notes = text;
        notesLoaded = text;
      })
      .catch((caught: unknown) => console.warn('host notes unavailable', caught));
  });
  const hasSecret = initial?.has_secret ?? false;

  function auth(): AuthMethod {
    if (method === 'key_file') return { method, path: keyPath.trim() };
    if (method === 'key') return { method, key: libraryKey };
    return { method } as AuthMethod;
  }

  // Picking "jump" preselects the first candidate so the drop-down never shows an empty value.
  $effect(() => {
    if (routeKind === 'jump' && !jumpHost && jumpOptions[0]) jumpHost = jumpOptions[0].value;
  });

  function route(): HostRoute {
    switch (routeKind) {
      case 'jump':
        return { kind: 'jump', host: jumpHost };
      case 'http':
      case 'socks5':
        return { kind: 'proxy', protocol: routeKind, host: proxyHost.trim(), port: Number(proxyPort ?? 0) };
      default:
        return { kind: 'direct' };
    }
  }

  function secret(): Secret | null {
    switch (method) {
      case 'password':
        return password ? { kind: 'password', password } : null;
      case 'private_key':
        return privateKey.trim() ? { kind: 'private_key', key: privateKey, passphrase: passphrase || null } : null;
      case 'key_file':
        return passphrase ? { kind: 'passphrase', passphrase } : null;
      default:
        return null;
    }
  }

  async function save(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (method === 'private_key' && !hasSecret && !privateKey.trim()) {
      error = t('host.privateKeyRequired');
      return;
    }
    if (method === 'key' && !libraryKey) {
      error = t('keys.none');
      return;
    }
    if (routeKind === 'jump' && !jumpHost) {
      error = t('host.jumpRequired');
      return;
    }
    const draft: HostDraft = {
      name,
      group: group.trim() || null,
      hostname,
      port: Number(port),
      username,
      auth: auth(),
      route: route(),
      tags: initial?.tags ?? [],
    };
    saving = true;
    error = '';
    duplicate = null;
    try {
      if (!initial && reuseExisting) {
        await hosts.refresh();
        const existing = findDuplicateHost(hosts.items, draft);
        if (existing) {
          duplicate = existing;
          return;
        }
      }
      const saved = initial ? await hostUpdate(initial.id, draft, secret(), false) : await hostCreate(draft, secret());
      if (notes.trim() !== notesLoaded.trim()) await hostNotesSet(saved.id, notes);
      await hosts.refresh();
      const entry = hosts.find(saved.id);
      if (entry) onsaved?.(entry);
      onclose();
    } catch (caught) {
      error = errorText(caught);
    } finally {
      saving = false;
    }
  }
</script>

<Dialog title={initial ? t('host.edit') : t('host.new')} {onclose}>
  <form id="host-form" onsubmit={save}>
    <div class="row">
      <label>{t('host.hostname')}<input bind:value={hostname} required autocomplete="off" spellcheck="false" /></label>
      <label class="port">{t('host.port')}<input type="number" min="1" max="65535" bind:value={port} required /></label>
    </div>
    <label>{t('host.username')}<input bind:value={username} required autocomplete="off" spellcheck="false" /></label>
    <div class="row">
      <label>{t('host.name')}<input bind:value={name} placeholder={hostname} autocomplete="off" /></label>
      <label>{t('host.group')}<input bind:value={group} autocomplete="off" /></label>
    </div>
    <label
      >{t('host.auth')}
      <Select
        bind:value={method}
        options={[
          { value: 'password', label: t('host.auth.password') },
          { value: 'key', label: t('host.auth.libraryKey') },
          ...(legacyPrivateKey ? [{ value: 'private_key' as const, label: t('host.auth.privateKey') }] : []),
          { value: 'key_file', label: t('host.auth.keyFile') },
          { value: 'agent', label: t('host.auth.agent') },
          { value: 'interactive', label: t('host.auth.interactive') },
        ]}
      />
    </label>
    {#if method === 'password'}
      <label
        >{t('host.password')}
        <input type="password" bind:value={password} placeholder={hasSecret ? t('host.keepSecret') : t('host.passwordOptional')} autocomplete="new-password" />
      </label>
    {:else if method === 'key'}
      {#if keyOptions.length > 0}
        <label>{t('host.key')}<Select bind:value={libraryKey} options={keyOptions} /></label>
      {:else}
        <p class="hint">{t('keys.none')}</p>
      {/if}
      <button type="button" class="link" onclick={onmanagekeys}>{t('keys.manage')}</button>
    {:else if method === 'private_key'}
      <label
        >{t('host.privateKey')}
        <textarea rows="5" bind:value={privateKey} placeholder={hasSecret ? t('host.keepSecret') : '-----BEGIN OPENSSH PRIVATE KEY-----'} spellcheck="false"></textarea>
      </label>
      <label>{t('host.passphrase')}<input type="password" bind:value={passphrase} autocomplete="new-password" /></label>
    {:else if method === 'key_file'}
      <label>{t('host.keyPath')}<input bind:value={keyPath} required spellcheck="false" /></label>
      <label>{t('host.passphrase')}<input type="password" bind:value={passphrase} placeholder={hasSecret ? t('host.keepSecret') : ''} autocomplete="new-password" /></label>
    {/if}
    <label
      >{t('host.route')}
      <Select
        bind:value={routeKind}
        options={[
          { value: 'direct', label: t('host.route.direct') },
          { value: 'jump', label: t('host.route.jump') },
          { value: 'http', label: t('host.route.http') },
          { value: 'socks5', label: t('host.route.socks5') },
        ]}
      />
    </label>
    {#if routeKind === 'jump'}
      {#if jumpOptions.length > 0}
        <label>{t('host.jumpHost')}<Select bind:value={jumpHost} options={jumpOptions} /></label>
        <p class="hint">{t('host.jumpHint')}</p>
      {:else}
        <p class="hint">{t('host.jumpNone')}</p>
      {/if}
    {:else if routeKind !== 'direct'}
      <div class="row">
        <label>{t('host.proxyHost')}<input bind:value={proxyHost} required spellcheck="false" autocomplete="off" /></label>
        <label class="port">{t('host.port')}<input type="number" min="1" max="65535" bind:value={proxyPort} required /></label>
      </div>
    {/if}
    <label>{t('host.notes')}<textarea bind:value={notes} rows="4" spellcheck="false" placeholder={t('host.notesHint')}></textarea></label>
    {#if error}<p class="error">{error}</p>{/if}
    {#if duplicateMatch}<p class="hint" role="status">{t('tailscale.duplicate', { name: duplicateMatch.name })}</p>{/if}
  </form>
  {#snippet actions()}
    <button type="button" class="secondary" onclick={onclose}>{t('common.cancel')}</button>
    {#if duplicateMatch && onreuse}
      <button type="button" class="primary" onclick={() => duplicateMatch && onreuse?.(duplicateMatch)}>{t('tailscale.useExisting')}</button>
    {:else}
      <button type="submit" class="primary" form="host-form" disabled={saving}>{submitLabel ?? t('common.save')}</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .port {
    flex: 0 0 96px !important;
  }
  .link {
    align-self: flex-start;
    margin-top: calc(-1 * var(--space-2));
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    font: inherit;
    font-size: var(--text-footnote);
    cursor: pointer;
  }
  .hint {
    margin: calc(-1 * var(--space-2)) 0 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
</style>
