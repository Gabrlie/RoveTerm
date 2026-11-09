<script lang="ts">
  import { onMount } from 'svelte';
  import TabBar from './lib/tabs/TabBar.svelte';
  import PaneGrid from './lib/layout/PaneGrid.svelte';
  import CommandPalette from './lib/ui/CommandPalette.svelte';
  import HostSidebar from './lib/hosts/HostSidebar.svelte';
  import HostEditor from './lib/hosts/HostEditor.svelte';
  import PasswordPrompt from './lib/hosts/PasswordPrompt.svelte';
  import QuickConnect from './lib/hosts/QuickConnect.svelte';
  import HostKeyDialog from './lib/hosts/HostKeyDialog.svelte';
  import ForwardsDialog from './lib/forwards/ForwardsDialog.svelte';
  import { forwardStatuses } from './lib/forwards/forwards.svelte';
  import { hosts, needsPassword } from './lib/hosts/hosts.svelte';
  import { vault } from './lib/vault/vault.svelte';
  import { actionFor, type ActionId } from './lib/actions';
  import { terminals } from './lib/terminal/registry';
  import { workspace, type PaneTarget } from './lib/workspace/workspace.svelte';
  import { onHostKeyPrompt, type CredentialKind, type HostEntry, type HostKeyPrompt } from './lib/ipc';
  import { errorText, t } from './lib/i18n';
  import type { PaneId } from './lib/layout/layout';

  type PasswordRequest =
    | { kind: 'host'; host: HostEntry }
    | { kind: 'files'; host: HostEntry }
    | { kind: 'pane'; paneId: PaneId; title: string; secret: CredentialKind; allowRemember: boolean };

  let paletteOpen = $state(false);
  let sidebarOpen = $state(true);
  let quickOpen = $state(false);
  let editing = $state<HostEntry | null | undefined>(undefined);
  let passwordRequest = $state<PasswordRequest | null>(null);
  let hostKeyPrompts = $state<HostKeyPrompt[]>([]);
  let forwardsHost = $state<HostEntry | null>(null);
  let notice = $state('');
  let noticeTimer = 0;

  function showNotice(message: string): void {
    notice = message;
    clearTimeout(noticeTimer);
    noticeTimer = window.setTimeout(() => (notice = ''), 4000);
  }

  function focusedTerminal() {
    const pane = workspace.focusedPane;
    return pane ? terminals.get(pane.id) : undefined;
  }

  function connect(host: HostEntry): void {
    if (needsPassword(host)) {
      passwordRequest = { kind: 'host', host };
      return;
    }
    workspace.openTab({ kind: 'ssh', hostId: host.id, title: host.name });
  }

  function openFiles(host: HostEntry): void {
    if (needsPassword(host)) {
      passwordRequest = { kind: 'files', host };
      return;
    }
    workspace.openTab({ kind: 'sftp', source: { hostId: host.id }, title: host.name });
  }

  function hostIdOf(target: PaneTarget): string | undefined {
    if (target.kind === 'ssh') return target.hostId;
    if (target.kind === 'sftp' && 'hostId' in target.source) return target.source.hostId;
    return undefined;
  }

  /** Whether what the user types for `host` can be stored: a password for password hosts, a passphrase for key hosts. */
  function canRemember(host: HostEntry | undefined, secret: CredentialKind): boolean {
    const method = host?.auth.method;
    return secret === 'password' ? method === 'password' : method === 'key_file' || method === 'private_key';
  }

  function requestPaneCredentials(paneId: PaneId, secret: CredentialKind): void {
    const pane = workspace.panes[paneId];
    if (!pane || pane.target.kind === 'local') return;
    const hostId = hostIdOf(pane.target);
    const host = hostId ? hosts.find(hostId) : undefined;
    passwordRequest = {
      kind: 'pane',
      paneId,
      title: pane.title,
      secret,
      allowRemember: canRemember(host, secret),
    };
  }

  function submitPassword(password: string, remember: boolean): void {
    const request = passwordRequest;
    passwordRequest = null;
    if (!request) return;
    if (request.kind === 'host' || request.kind === 'files') {
      const { host } = request;
      const target: PaneTarget =
        request.kind === 'host'
          ? { kind: 'ssh', hostId: host.id, title: host.name }
          : { kind: 'sftp', source: { hostId: host.id }, title: host.name };
      workspace.openTab(target, password, remember);
      if (remember) setTimeout(() => void hosts.refresh(), 3000);
    } else {
      workspace.reopen(request.paneId, password, remember);
    }
  }

  function run(id: ActionId): void {
    switch (id) {
      case 'tab.newLocal':
        workspace.openTab({ kind: 'local' });
        break;
      case 'tab.close':
        workspace.closeFocused();
        break;
      case 'tab.next':
        workspace.cycleTab(1);
        break;
      case 'tab.prev':
        workspace.cycleTab(-1);
        break;
      case 'pane.splitRight':
        workspace.splitFocused('row');
        break;
      case 'pane.splitDown':
        workspace.splitFocused('column');
        break;
      case 'pane.focusNext':
        workspace.cyclePane(1);
        break;
      case 'pane.focusPrev':
        workspace.cyclePane(-1);
        break;
      case 'terminal.copy': {
        const text = focusedTerminal()?.selection();
        if (text) navigator.clipboard.writeText(text).catch((error: unknown) => console.warn('copy failed', error));
        break;
      }
      case 'terminal.paste':
        navigator.clipboard
          .readText()
          .then((text) => focusedTerminal()?.paste(text))
          .catch((error: unknown) => console.warn('paste failed', error));
        break;
      case 'palette.open':
        paletteOpen = true;
        break;
      case 'host.quickConnect':
        quickOpen = true;
        break;
      case 'host.new':
        editing = null;
        break;
      case 'sidebar.toggle':
        sidebarOpen = !sidebarOpen;
        break;
      case 'files.open':
        if (!workspace.openFilesBesideFocused()) showNotice(t('files.noRemote'));
        break;
      case 'forward.manage': {
        const pane = workspace.focusedPane;
        const hostId = pane ? hostIdOf(pane.target) : undefined;
        const host = hostId ? hosts.find(hostId) : undefined;
        if (host) forwardsHost = host;
        else showNotice(t('forward.noHost'));
        break;
      }
      default:
        if (id.startsWith('tab.goto')) workspace.gotoTab(Number(id.slice('tab.goto'.length)) - 1);
    }
  }

  function onKeydown(event: KeyboardEvent): void {
    const id = actionFor(event);
    if (!id) return;
    event.preventDefault();
    event.stopPropagation();
    run(id);
  }

  onMount(() => {
    workspace.openTab({ kind: 'local' });
    vault.init().catch((error: unknown) => console.warn('vault state unavailable', error));
    hosts.refresh().catch((error: unknown) => showNotice(errorText(error)));
    forwardStatuses.start().catch((error: unknown) => console.warn('forward events unavailable', error));
    const unlisten = onHostKeyPrompt((prompt) => (hostKeyPrompts = [...hostKeyPrompts, prompt]));
    return () => void unlisten.then((stop) => stop());
  });
</script>

<svelte:window onkeydowncapture={onKeydown} />

<div class="app">
  {#if sidebarOpen}
    <HostSidebar onconnect={connect} onfiles={openFiles} onforwards={(host) => (forwardsHost = host)} onedit={(host) => (editing = host)} onquick={() => (quickOpen = true)} onnotice={showNotice} />
  {/if}
  <div class="main">
    <TabBar onnew={() => run('tab.newLocal')} />
    <main class="stage">
      {#each workspace.tabs as tab (tab.id)}
        <PaneGrid {tab} visible={tab.id === workspace.activeTabId} oncredentials={requestPaneCredentials} />
      {:else}
        <div class="empty">{t('workspace.empty')}</div>
      {/each}
    </main>
  </div>
</div>

{#if notice}
  <div class="notice" role="status">{notice}</div>
{/if}

{#if paletteOpen}
  <CommandPalette onrun={run} onclose={() => (paletteOpen = false)} />
{/if}

{#if editing !== undefined}
  <HostEditor host={editing} onclose={() => (editing = undefined)} />
{/if}

{#if quickOpen}
  <QuickConnect
    onclose={() => (quickOpen = false)}
    onsubmit={(address, password) => {
      quickOpen = false;
      workspace.openTab({ kind: 'quick', address }, password);
    }}
  />
{/if}

{#if passwordRequest}
  {@const secret = passwordRequest.kind === 'pane' ? passwordRequest.secret : 'password'}
  <PasswordPrompt
    {secret}
    title={t(secret === 'password' ? 'host.passwordFor' : 'host.passphraseFor', {
      address: passwordRequest.kind === 'pane' ? passwordRequest.title : `${passwordRequest.host.username}@${passwordRequest.host.hostname}`,
    })}
    allowRemember={passwordRequest.kind === 'pane' ? passwordRequest.allowRemember : canRemember(passwordRequest.host, 'password')}
    onsubmit={submitPassword}
    onclose={() => (passwordRequest = null)}
  />
{/if}

{#if forwardsHost}
  <ForwardsDialog host={forwardsHost} onclose={() => (forwardsHost = null)} />
{/if}

{#if hostKeyPrompts[0]}
  {#key hostKeyPrompts[0].request_id}
    <HostKeyDialog prompt={hostKeyPrompts[0]} ondone={() => (hostKeyPrompts = hostKeyPrompts.slice(1))} />
  {/key}
{/if}

<style>
  .app {
    height: 100vh;
    display: flex;
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    background: var(--term-bg);
  }
  .empty {
    height: 100%;
    display: grid;
    place-items: center;
    color: var(--fg-muted);
  }
  .notice {
    position: fixed;
    bottom: var(--space-4);
    left: 50%;
    transform: translateX(-50%);
    z-index: 40;
    padding: var(--space-2) var(--space-4);
    border-radius: 999px;
    background: var(--fg);
    color: var(--bg);
    font-size: var(--text-footnote);
  }
</style>
