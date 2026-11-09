<script lang="ts">
  import { onMount } from 'svelte';
  import TitleBar from './lib/layout/TitleBar.svelte';
  import EditorGroups from './lib/layout/EditorGroups.svelte';
  import CommandPalette from './lib/ui/CommandPalette.svelte';
  import HostSidebar from './lib/hosts/HostSidebar.svelte';
  import HostEditor from './lib/hosts/HostEditor.svelte';
  import PasswordPrompt from './lib/hosts/PasswordPrompt.svelte';
  import SettingsDialog, { type SettingsSection } from './lib/settings/SettingsDialog.svelte';
  import ConfirmHost from './lib/ui/ConfirmHost.svelte';
  import KeysDialog from './lib/keys/KeysDialog.svelte';
  import HomePage from './lib/home/HomePage.svelte';
  import RightPanel from './lib/panels/RightPanel.svelte';
  import BottomPanel from './lib/panels/BottomPanel.svelte';
  import { keys } from './lib/keys/keys.svelte';
  import QuickConnect from './lib/hosts/QuickConnect.svelte';
  import HostKeyDialog from './lib/hosts/HostKeyDialog.svelte';
  import ForwardsDialog from './lib/forwards/ForwardsDialog.svelte';
  import { forwardStatuses } from './lib/forwards/forwards.svelte';
  import { hosts, needsPassword } from './lib/hosts/hosts.svelte';
  import { vault } from './lib/vault/vault.svelte';
  import { sync } from './lib/sync/sync.svelte';
  import { actionFor, isMac, type ActionId } from './lib/actions';
  import { isActionId, nativeMenuLabels, nativeMenus } from './lib/menu';
  import { terminals } from './lib/terminal/registry';
  import { workspace, type PaneTarget } from './lib/workspace/workspace.svelte';
  import { menuSet, onHostKeyPrompt, onMenuAction, workspaceLoad, workspaceSave, type CredentialKind, type HostEntry, type HostKeyPrompt } from './lib/ipc';
  import { parseSnapshot } from './lib/workspace/snapshot';
  import { errorText, t } from './lib/i18n';
  import type { PaneId } from './lib/layout/layout';

  type PasswordRequest =
    | { kind: 'host'; host: HostEntry }
    | { kind: 'files'; host: HostEntry }
    | { kind: 'pane'; paneId: PaneId; title: string; secret: CredentialKind; allowRemember: boolean; override: boolean };

  let paletteOpen = $state(false);
  /** Per-device UI state kept in localStorage; storage may be unavailable. */
  function stored(key: string, fallback: boolean): boolean {
    try {
      const value = localStorage.getItem(key);
      return value === null ? fallback : value === '1';
    } catch {
      return fallback;
    }
  }
  function store(key: string, value: boolean): void {
    try {
      localStorage.setItem(key, value ? '1' : '0');
    } catch {
      // Storage unavailable: the choice lasts for this run only.
    }
  }

  function storedSize(key: string, fallback: number): number {
    try {
      const value = Number(localStorage.getItem(key));
      return Number.isFinite(value) && value > 0 ? value : fallback;
    } catch {
      return fallback;
    }
  }
  function storeSize(key: string, value: number): void {
    try {
      localStorage.setItem(key, String(Math.round(value)));
    } catch {
      // Storage unavailable: the size lasts for this run only.
    }
  }

  let sidebarOpen = $state(stored('ui.sidebar', true));
  $effect(() => store('ui.sidebar', sidebarOpen));
  let rightOpen = $state(stored('ui.panel.right', false));
  $effect(() => store('ui.panel.right', rightOpen));
  let bottomOpen = $state(stored('ui.panel.bottom', false));
  $effect(() => store('ui.panel.bottom', bottomOpen));
  let bottomMounted = $state(false);
  $effect(() => {
    if (bottomOpen) bottomMounted = true;
  });
  let rightSize = $state(storedSize('ui.panel.rightSize', 300));
  $effect(() => storeSize('ui.panel.rightSize', rightSize));
  let bottomSize = $state(storedSize('ui.panel.bottomSize', 260));
  $effect(() => storeSize('ui.panel.bottomSize', bottomSize));

  /** Drags a panel edge; `apply` receives the pointer movement since the drag started. */
  function dragEdge(event: PointerEvent, apply: (delta: number) => void, axis: 'x' | 'y'): void {
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    const start = axis === 'x' ? event.clientX : event.clientY;
    const move = (moveEvent: PointerEvent): void => apply((axis === 'x' ? moveEvent.clientX : moveEvent.clientY) - start);
    const end = (): void => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', end);
      handle.removeEventListener('pointercancel', end);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', end);
    handle.addEventListener('pointercancel', end);
  }

  function resizeRight(event: PointerEvent): void {
    const initial = rightSize;
    dragEdge(event, (delta) => (rightSize = Math.min(Math.max(initial - delta, 220), window.innerWidth * 0.6)), 'x');
  }

  function resizeBottom(event: PointerEvent): void {
    const initial = bottomSize;
    dragEdge(event, (delta) => (bottomSize = Math.min(Math.max(initial - delta, 140), window.innerHeight * 0.7)), 'y');
  }
  let quickOpen = $state(false);
  let editing = $state<HostEntry | null | undefined>(undefined);
  let passwordRequest = $state<PasswordRequest | null>(null);
  let hostKeyPrompts = $state<HostKeyPrompt[]>([]);
  let forwardsHost = $state<HostEntry | null>(null);
  let settingsOpen = $state(false);
  /** Section the settings dialog opens on; the first one when unset. */
  let settingsSection = $state<SettingsSection | undefined>(undefined);

  async function importSshConfig(): Promise<void> {
    try {
      const report = await hosts.importSshConfig();
      showNotice(t('host.imported', { created: report.created.length, skipped: report.skipped.length }));
    } catch (error) {
      showNotice(errorText(error));
    }
  }
  let keysOpen = $state(false);
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

  function requestPaneCredentials(paneId: PaneId, secret: CredentialKind, override = false): void {
    const pane = workspace.panes[paneId];
    if (!pane || pane.target.kind === 'local') return;
    const hostId = hostIdOf(pane.target);
    const host = hostId ? hosts.find(hostId) : undefined;
    passwordRequest = {
      kind: 'pane',
      paneId,
      title: pane.title,
      secret,
      // A password typed for a key host is used once and never stored.
      allowRemember: canRemember(host, secret) && !(override && host?.auth.method !== 'password'),
      override,
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
      workspace.reopen(request.paneId, password, remember, request.override ? 'password' : 'natural');
    }
  }

  // macOS: the menu bar follows the interface language; its items run actions here.
  $effect(() => {
    if (!isMac) return;
    menuSet(nativeMenus(), nativeMenuLabels()).catch((error: unknown) => console.warn('menu bar unavailable', error));
  });
  $effect(() => {
    if (!isMac) return;
    const pending = onMenuAction((id) => {
      if (isActionId(id)) run(id);
    });
    return () => void pending.then((unlisten) => unlisten());
  });

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
      case 'settings.open':
        settingsSection = undefined;
        settingsOpen = true;
        break;
      case 'help.shortcuts':
        settingsSection = 'shortcuts';
        settingsOpen = true;
        break;
      case 'help.about':
        settingsSection = 'about';
        settingsOpen = true;
        break;
      case 'host.import':
        void importSshConfig();
        break;
      case 'keys.manage':
        keysOpen = true;
        break;
      case 'panel.right':
        rightOpen = !rightOpen;
        break;
      case 'panel.bottom':
        bottomOpen = !bottomOpen;
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

  // Sync runs once the vault is unlocked (the sync key is sealed with the device key), then
  // every few minutes.
  let syncStarted = false;
  $effect(() => {
    if (vault.unlocked && !syncStarted) {
      syncStarted = true;
      sync.startAuto();
    }
  });

  // Session restore: reopen the last workspace once, then record changes (debounced).
  let restored = $state(false);
  let saveTimer = 0;
  $effect(() => {
    if (!restored) return;
    const snapshot = workspace.snapshot();
    const text = JSON.stringify(snapshot);
    clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => {
      workspaceSave(JSON.parse(text)).catch((error: unknown) => console.warn('saving the workspace failed', error));
    }, 1500);
  });

  onMount(() => {
    workspaceLoad()
      .then((stored) => {
        const snapshot = parseSnapshot(stored);
        if (snapshot) workspace.restore(snapshot);
      })
      .catch((error: unknown) => console.warn('workspace restore unavailable', error))
      .finally(() => (restored = true));
    vault.init().catch((error: unknown) => console.warn('vault state unavailable', error));
    hosts.refresh().catch((error: unknown) => showNotice(errorText(error)));
    keys.refresh().catch((error: unknown) => console.warn('key library unavailable', error));
    forwardStatuses.start().catch((error: unknown) => console.warn('forward events unavailable', error));
    const unlisten = onHostKeyPrompt((prompt) => (hostKeyPrompts = [...hostKeyPrompts, prompt]));
    return () => void unlisten.then((stop) => stop());
  });
</script>

<svelte:window onkeydowncapture={onKeydown} />

<div class="app">
  <TitleBar {sidebarOpen} {rightOpen} {bottomOpen} onaction={run} />
  <div class="body">
  {#if sidebarOpen}
    <HostSidebar onconnect={connect} onfiles={openFiles} onforwards={(host) => (forwardsHost = host)} onedit={(host) => (editing = host)} onnotice={showNotice} onsettings={() => (settingsOpen = true)} />
  {/if}
  <div class="main">
    <div class="workarea">
    <div class="center">
    {#snippet home()}
      <HomePage
        onconnect={connect}
        onfiles={openFiles}
        onedit={(host) => (editing = host)}
        onquick={() => (quickOpen = true)}
        onlocal={() => run('tab.newLocal')}
        onforwards={(host) => (forwardsHost = host)}
        onnotice={showNotice}
        onaction={run}
      />
    {/snippet}
    <EditorGroups {home} oncredentials={requestPaneCredentials} onedithost={(hostId) => (editing = hosts.find(hostId) ?? undefined)} />
    {#if bottomOpen}
      <div class="edge horizontal" role="separator" aria-orientation="horizontal" onpointerdown={resizeBottom}></div>
    {/if}
    <!-- Once opened the panel stays mounted while hidden, keeping its file browsers open. -->
    {#if bottomMounted}
      <div class="bottom" hidden={!bottomOpen} style:height="{bottomSize}px">
        <BottomPanel onclose={() => (bottomOpen = false)} />
      </div>
    {/if}
    </div>
    {#if rightOpen}
      <div class="edge vertical" role="separator" aria-orientation="vertical" onpointerdown={resizeRight}></div>
      <div class="right" style:width="{rightSize}px">
        <RightPanel onclose={() => (rightOpen = false)} />
      </div>
    {/if}
    </div>
  </div>
  </div>
</div>

{#if notice}
  <div class="notice" role="status">{notice}</div>
{/if}

{#if paletteOpen}
  <CommandPalette onrun={run} onclose={() => (paletteOpen = false)} />
{/if}

{#if editing !== undefined}
  <HostEditor host={editing} onclose={() => (editing = undefined)} onmanagekeys={() => (keysOpen = true)} />
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

{#if keysOpen}
  <KeysDialog onclose={() => (keysOpen = false)} />
{/if}

<ConfirmHost />

{#if settingsOpen}
  <SettingsDialog initial={settingsSection} onclose={() => (settingsOpen = false)} />
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
    flex-direction: column;
  }
  /* The sidebar, the work area and the right panel float as rounded cards over the window
     background (and its system material), separated by narrow gaps. */
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: var(--gap-panel);
    padding: 0 var(--gap-panel) var(--gap-panel);
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .workarea {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: var(--gap-panel);
  }
  /* The editor groups above and the files panel below, separated by a gap like the cards. */
  .center {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--gap-panel);
  }
  .bottom[hidden] {
    display: none;
  }
  .bottom,
  .right {
    flex: none;
    min-height: 0;
    min-width: 0;
    border-radius: var(--radius-lg);
    box-shadow: var(--panel-shadow);
    overflow: hidden;
  }
  /* Panel edges are the gaps between cards; a line shows only while hovering. */
  .edge {
    position: relative;
    z-index: 5;
    flex: none;
  }
  .edge::before {
    content: '';
    position: absolute;
    border-radius: 1px;
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .edge:hover::before {
    background: var(--accent);
  }
  .edge.horizontal {
    height: var(--gap-panel);
    margin: calc(-1 * var(--gap-panel)) 0;
    cursor: row-resize;
  }
  .edge.horizontal::before {
    left: var(--space-3);
    right: var(--space-3);
    top: calc(50% - 1px);
    height: 2px;
  }
  .edge.vertical {
    width: var(--gap-panel);
    margin: 0 calc(-1 * var(--gap-panel));
    cursor: col-resize;
  }
  .edge.vertical::before {
    top: var(--space-3);
    bottom: var(--space-3);
    left: calc(50% - 1px);
    width: 2px;
  }
  .notice {
    position: fixed;
    bottom: var(--space-5);
    left: 50%;
    transform: translateX(-50%);
    z-index: 40;
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-pill);
    background: var(--material-popover);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: var(--glass-edge), var(--shadow-md);
    color: var(--fg);
    font-size: var(--text-footnote);
    font-weight: 500;
    animation: notice-in var(--dur-slow) var(--ease);
  }
  @keyframes notice-in {
    from {
      opacity: 0;
      transform: translate(-50%, 8px);
    }
  }
</style>
