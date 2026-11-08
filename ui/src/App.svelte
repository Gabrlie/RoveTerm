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
  import { hosts, needsPassword } from './lib/hosts/hosts.svelte';
  import { vault } from './lib/vault/vault.svelte';
  import { actionFor, type ActionId } from './lib/actions';
  import { terminals } from './lib/terminal/registry';
  import { workspace } from './lib/workspace/workspace.svelte';
  import { onHostKeyPrompt, type HostEntry, type HostKeyPrompt } from './lib/ipc';
  import { errorText, t } from './lib/i18n';
  import type { PaneId } from './lib/layout/layout';

  type PasswordRequest =
    | { kind: 'host'; host: HostEntry }
    | { kind: 'pane'; paneId: PaneId; title: string; allowRemember: boolean };

  let paletteOpen = $state(false);
  let sidebarOpen = $state(true);
  let quickOpen = $state(false);
  let editing = $state<HostEntry | null | undefined>(undefined);
  let passwordRequest = $state<PasswordRequest | null>(null);
  let hostKeyPrompts = $state<HostKeyPrompt[]>([]);
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

  function requestPaneCredentials(paneId: PaneId): void {
    const pane = workspace.panes[paneId];
    if (!pane || pane.target.kind === 'local') return;
    const host = pane.target.kind === 'ssh' ? hosts.find(pane.target.hostId) : undefined;
    passwordRequest = {
      kind: 'pane',
      paneId,
      title: pane.title,
      allowRemember: host?.auth.method === 'password',
    };
  }

  function submitPassword(password: string, remember: boolean): void {
    const request = passwordRequest;
    passwordRequest = null;
    if (!request) return;
    if (request.kind === 'host') {
      const { host } = request;
      workspace.openTab({ kind: 'ssh', hostId: host.id, title: host.name }, password, remember);
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
    const unlisten = onHostKeyPrompt((prompt) => (hostKeyPrompts = [...hostKeyPrompts, prompt]));
    return () => void unlisten.then((stop) => stop());
  });
</script>

<svelte:window onkeydowncapture={onKeydown} />

<div class="app">
  {#if sidebarOpen}
    <HostSidebar onconnect={connect} onedit={(host) => (editing = host)} onquick={() => (quickOpen = true)} onnotice={showNotice} />
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
  <PasswordPrompt
    title={t('host.passwordFor', {
      address: passwordRequest.kind === 'host' ? `${passwordRequest.host.username}@${passwordRequest.host.hostname}` : passwordRequest.title,
    })}
    allowRemember={passwordRequest.kind === 'host' ? passwordRequest.host.auth.method === 'password' : passwordRequest.allowRemember}
    onsubmit={submitPassword}
    onclose={() => (passwordRequest = null)}
  />
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
