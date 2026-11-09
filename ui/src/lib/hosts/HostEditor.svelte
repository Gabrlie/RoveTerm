<script lang="ts">
  // Create or edit a host. The draft stays in the form when saving fails (for example a denied unlock).
  import Dialog from '../ui/Dialog.svelte';
  import Select from '../ui/Select.svelte';
  import { hostCreate, hostUpdate, type AuthMethod, type HostDraft, type HostEntry, type HostRoute, type Secret } from '../ipc';
  import { errorText, t } from '../i18n';
  import { hosts } from './hosts.svelte';

  let { host, onclose }: { host: HostEntry | null; onclose: () => void } = $props();

  type Method = AuthMethod['method'];
  type RouteKind = 'direct' | 'jump' | 'http' | 'socks5';

  // svelte-ignore state_referenced_locally
  const initial = host;
  let name = $state(initial?.name ?? '');
  let group = $state(initial?.group ?? '');
  let hostname = $state(initial?.hostname ?? '');
  let port = $state(initial?.port ?? 22);
  let username = $state(initial?.username ?? 'root');
  let method = $state<Method>(initial?.auth.method ?? 'password');
  let keyPath = $state(initial?.auth.method === 'key_file' ? initial.auth.path : '~/.ssh/id_ed25519');
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
  const hasSecret = initial?.has_secret ?? false;

  function auth(): AuthMethod {
    return method === 'key_file' ? { method, path: keyPath.trim() } : ({ method } as AuthMethod);
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
    try {
      if (initial) await hostUpdate(initial.id, draft, secret(), false);
      else await hostCreate(draft, secret());
      await hosts.refresh();
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
          { value: 'private_key', label: t('host.auth.privateKey') },
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
    {#if error}<p class="error">{error}</p>{/if}
  </form>
  {#snippet actions()}
    <button type="button" class="secondary" onclick={onclose}>{t('common.cancel')}</button>
    <button type="submit" class="primary" form="host-form" disabled={saving}>{t('common.save')}</button>
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
  .hint {
    margin: calc(-1 * var(--space-2)) 0 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
</style>
