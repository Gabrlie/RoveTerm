<script lang="ts">
  import { onMount } from 'svelte';
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import { editors } from './lib/files/editors.svelte';
  import DraftsHost from './lib/files/DraftsHost.svelte';
  import ExternalEdits from './lib/files/ExternalEdits.svelte';
  import { externalEdits } from './lib/files/external.svelte';
  import { isFinal, transfers } from './lib/files/transfers.svelte';
  import { capabilities, isMacDesktop, isMobile } from './lib/platform';
  import Dialog from './lib/ui/Dialog.svelte';
  import { ask } from './lib/ui/confirm.svelte';
  import { externalEditSetDirty, externalEditShutdown, mobileBackDefault, onExternalEditExitRequest, onMobileBack, onMobileLifecycle, updateRestart } from './lib/ipc';
  import UpdateDialog from './lib/update/UpdateDialog.svelte';
  import { updater } from './lib/update/updater.svelte';
  import TitleBar from './lib/layout/TitleBar.svelte';
  import MobileNavBar, { type MobileDestination } from './lib/layout/MobileNavBar.svelte';
  import ActivityRail from './lib/layout/ActivityRail.svelte';
  import StatusBar from './lib/layout/StatusBar.svelte';
  import EditorGroups from './lib/layout/EditorGroups.svelte';
  import CommandPalette from './lib/ui/CommandPalette.svelte';
  import HostSidebar from './lib/hosts/HostSidebar.svelte';
  import HostEditor from './lib/hosts/HostEditor.svelte';
  import PasswordPrompt from './lib/hosts/PasswordPrompt.svelte';
  import SettingsDialog, { type SettingsSection } from './lib/settings/SettingsDialog.svelte';
  import MobileSettingsPage from './lib/settings/MobileSettingsPage.svelte';
  import MobileHostsPage from './lib/mobile/MobileHostsPage.svelte';
  import MobileToolsPage from './lib/mobile/MobileToolsPage.svelte';
  import ConfirmHost from './lib/ui/ConfirmHost.svelte';
  import EditMenu from './lib/ui/EditMenu.svelte';
  import Toaster from './lib/ui/Toaster.svelte';
  import { notify, type ToastKind } from './lib/ui/toast.svelte';
  import { readClipboard, writeClipboard } from './lib/ui/clipboard';
  import KeysDialog from './lib/keys/KeysDialog.svelte';
  import HomePage from './lib/home/HomePage.svelte';
  import { ai } from './lib/ai/ai.svelte';
  import SchedulesDialog from './lib/fleet/SchedulesDialog.svelte';
  import RightPanel, { type RightPage } from './lib/panels/RightPanel.svelte';
  import BottomPanel from './lib/panels/BottomPanel.svelte';
  import { keys } from './lib/keys/keys.svelte';
  import QuickConnect from './lib/hosts/QuickConnect.svelte';
  import HostKeyDialog from './lib/hosts/HostKeyDialog.svelte';
  import TailscaleDialog from './lib/tailscale/TailscaleDialog.svelte';
  import ForwardsDialog from './lib/forwards/ForwardsDialog.svelte';
  import { forwardStatuses } from './lib/forwards/forwards.svelte';
  import { hosts, needsPassword } from './lib/hosts/hosts.svelte';
  import { vault } from './lib/vault/vault.svelte';
  import { sync } from './lib/sync/sync.svelte';
  import { tailscale } from './lib/tailscale/tailscale.svelte';
  import { actionFor, isTextField, type ActionId } from './lib/actions';
  import { terminalCopies, terminalPastes, type ClipboardContext } from './lib/terminal/claims';
  import { isActionId, nativeMenuLabels, nativeMenus } from './lib/menu';
  import { terminals } from './lib/terminal/registry';
  import { remoteSourceOf, workspace, type PaneTarget } from './lib/workspace/workspace.svelte';
  import { aiRecords, aiSessions, logsReveal, menuSet, type AiRecord, type AiSessionSummary, onHostKeyPrompt, onMenuAction, workspaceLoad, workspaceSave, type CredentialKind, type HostEntry, type HostKeyPrompt } from './lib/ipc';
  import { parseSnapshot } from './lib/workspace/snapshot';
  import { findBackgroundTerminal, findReusableTerminalPane, isTerminalTarget } from './lib/workspace/singleSession';
  import { errorText, t } from './lib/i18n';
  import { arePanelsSuspended, type PaneId } from './lib/layout/layout';
  import { viewport } from './lib/layout/viewport.svelte';
  import {
    emptyMobileSettingsNav,
    openMobileSettings,
    selectMobileSettingsSection,
    stepBackMobileSettings,
    type MobileSettingsNavState,
  } from './lib/mobile/settingsNavigation';
  import { handleBack, pushBackHandler } from './lib/ui/backStack';

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
  let mobileTab = $state<MobileDestination>('hosts');
  let mobileSettingsNav = $state<MobileSettingsNavState>(emptyMobileSettingsNav());
  const mobileSettingsSection = $derived(mobileSettingsNav.section);

  const activeCompactPane = $derived(viewport.compact ? workspace.focusedPane : undefined);
  const showingMobileTerminal = $derived(!!activeCompactPane && isTerminalTarget(activeCompactPane.target));
  const backgroundTerminal = $derived(viewport.compact
    ? findBackgroundTerminal(Object.values(workspace.panes), activeCompactPane?.id ?? null)
    : undefined);
  const activeTransfersCount = $derived(Object.values(transfers.byId).filter((transfer) => !isFinal(transfer)).length);
  const activeForwardsCount = $derived(forwardStatuses.runningTotal);
  const toolsBadge = $derived(activeTransfersCount + activeForwardsCount);
  $effect(() => {
    if (viewport.compact) transfers.start().catch((error: unknown) => console.warn('transfer events unavailable', error));
  });
  $effect.pre(() => {
    if (viewport.compact && !activeCompactPane && mobileTab !== 'hosts') {
      return pushBackHandler(() => selectMobileDestination('hosts'));
    }
  });

  const RIGHT_PAGES: RightPage[] = ['status', 'ai', 'snippets', 'transfers'];
  function storedPage(key: string): RightPage | null {
    try {
      const value = localStorage.getItem(key);
      return RIGHT_PAGES.find((page) => page === value) ?? null;
    } catch {
      return null;
    }
  }
  /** The right panel's open page; `null` when closed. One page at a time, as in VSCode. */
  let rightPage = $state<RightPage | null>(viewport.compact ? null : storedPage('ui.panel.rightPage'));
  const panelsSuspended = $derived(arePanelsSuspended(workspace.focusedPane?.target.kind));
  let suspendedTransfersOpen = $state(false);
  const effectiveRightPage = $derived.by((): RightPage | null => {
    if (!panelsSuspended) return rightPage;
    if (viewport.compact) return rightPage === 'transfers' || rightPage === 'snippets' ? rightPage : null;
    return suspendedTransfersOpen || rightPage === 'transfers' ? 'transfers' : null;
  });
  $effect(() => {
    if (!panelsSuspended) suspendedTransfersOpen = false;
  });
  /** The page `panel.right` reopens. */
  let lastRightPage: RightPage = storedPage('ui.panel.rightPage') ?? 'status';
  $effect(() => {
    if (viewport.compact) return;
    try {
      localStorage.setItem('ui.panel.rightPage', rightPage ?? '');
    } catch {
      // Storage unavailable: the panel resets next run.
    }
  });

  /** Opens `page`, or closes the panel when `page` is already open. */
  function toggleRight(page: RightPage): void {
    if (panelsSuspended && !(viewport.compact && (page === 'snippets' || page === 'transfers'))) return;
    if (rightPage === page) closeRight();
    else openRight(page);
  }

  function openRight(page: RightPage): void {
    if (viewport.compact) closeBottom();
    rightPage = page;
    lastRightPage = page;
  }

  function closeRight(): void {
    if (viewport.compact) {
      rightPage = null;
      suspendedTransfersOpen = false;
    } else if (panelsSuspended) {
      suspendedTransfersOpen = false;
      if (rightPage === 'transfers') rightPage = null;
    } else {
      rightPage = null;
    }
  }

  /** Reveals the first waiting approval in the fleet tab or the context's AI panel. */
  function showWaiting(): void {
    const chat = ai.firstWaiting();
    if (chat?.context.kind === 'fleet') {
      if (chat.parentId && chat.id) ai.openSubagent(chat.parentId, chat.id);
      openFleet();
      return;
    }
    if (chat?.context.kind === 'host') {
      const tab = workspace.tabsOfHost(chat.context.host_id)[0];
      if (tab) workspace.reveal(tab);
    }
    const context = chat?.context;
    if (panelsSuspended && context?.kind === 'local') {
      const local = Object.values(workspace.panes).find((pane) => pane.target.kind === 'local');
      if (local) workspace.reveal(local.id);
      else workspace.openTab({ kind: 'local' });
    }
    if (panelsSuspended && context?.kind === 'quick') {
      const tab = Object.values(workspace.panes).find((pane) => {
        const remote = remoteSourceOf(pane.target);
        return remote && 'address' in remote && remote.address === context.address;
      });
      if (tab) workspace.reveal(tab.id);
    }
    if (panelsSuspended) return;
    openRight('ai');
  }

  /** The focused tab is on a remote host. */
  const focusedRemote = $derived.by(() => {
    const pane = workspace.focusedPane;
    return !!pane && remoteSourceOf(pane.target) !== null;
  });
  let bottomOpen = $state(viewport.compact ? false : stored('ui.panel.bottom', false));
  const effectiveBottomOpen = $derived(bottomOpen && !panelsSuspended
    && (!viewport.compact || (showingMobileTerminal && effectiveRightPage === null)));
  const terminalImmersion = $derived(showingMobileTerminal && !effectiveRightPage && !effectiveBottomOpen);
  const showMobileNav = $derived(viewport.compact && !viewport.keyboardOpen && !activeCompactPane
    && !effectiveRightPage && !effectiveBottomOpen && !(mobileTab === 'settings' && mobileSettingsSection !== null));
  function closeBottom(): void {
    bottomOpen = false;
  }
  function toggleBottom(): void {
    if (panelsSuspended || (viewport.compact && !showingMobileTerminal)) return;
    if (effectiveBottomOpen) {
      closeBottom();
    } else {
      if (viewport.compact) closeRight();
      bottomOpen = true;
    }
  }
  $effect(() => {
    if (viewport.compact && effectiveRightPage) return pushBackHandler(closeRight);
  });
  $effect(() => {
    if (viewport.compact && effectiveBottomOpen) return pushBackHandler(closeBottom);
  });
  $effect(() => {
    if (paletteOpen) return pushBackHandler(() => (paletteOpen = false));
  });
  $effect(() => {
    if (!viewport.compact) store('ui.panel.bottom', bottomOpen);
  });
  let bottomMounted = $state(false);
  $effect(() => {
    if (effectiveBottomOpen) bottomMounted = true;
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
  let tailscaleOpen = $state(false);
  /** Section the settings dialog opens on; the first one when unset. */
  let settingsSection = $state<SettingsSection | undefined>(undefined);

  /** Opens settings on `section` (the first one when unset). */
  function openSettings(section?: SettingsSection): void {
    if (viewport.compact) {
      const returnTarget =
        activeCompactPane || effectiveRightPage || mobileTab !== 'settings'
          ? {
              tab: mobileTab,
              paneId: activeCompactPane?.id ?? null,
              rightPage: effectiveRightPage,
            }
          : null;
      selectMobileDestination('settings');
      mobileSettingsNav = openMobileSettings(section ?? null, returnTarget);
      return;
    }
    settingsSection = section;
    settingsOpen = true;
  }

  function backFromMobileSettings(): void {
    const outcome = stepBackMobileSettings(mobileSettingsNav);
    mobileSettingsNav = outcome.state;
    if (outcome.kind === 'restore') {
      const { tab, paneId, rightPage: savedRight } = outcome.target;
      mobileTab = tab;
      if (paneId && workspace.panes[paneId]) {
        workspace.reveal(paneId);
      } else {
        workspace.activateHome();
      }
      if (savedRight) openRight(savedRight);
    }
  }

  const fleetWaiting = $derived.by(() => {
    const fleet = ai.chat({ kind: 'fleet' });
    return fleet.waiting + ai.subagentsOf(fleet.id).reduce((count, chat) => count + chat.waiting, 0);
  });

  function selectMobileDestination(destination: MobileDestination): void {
    closeRight();
    closeBottom();
    paletteOpen = false;
    workspace.activateHome();
    mobileTab = destination;
    mobileSettingsNav = emptyMobileSettingsNav();
  }

  function showMobileHosts(): void {
    selectMobileDestination('hosts');
  }

  function prepareMobilePane(): void {
    closeRight();
    closeBottom();
    mobileTab = 'hosts';
  }

  function disconnectMobileTerminal(paneId: PaneId): void {
    const focused = workspace.focusedPane?.id === paneId;
    workspace.closePane(paneId);
    if (focused) showMobileHosts();
  }

  function resumeMobileTerminal(paneId: PaneId): void {
    prepareMobilePane();
    workspace.reveal(paneId);
  }

  /** Shows the fleet lead's tab; sends `text` to it when given. */
  function openFleet(text: string | null = null): void {
    if (viewport.compact) prepareMobilePane();
    ai.listen();
    workspace.openFleet();
    if (text) {
      const chat = ai.chat({ kind: 'fleet' });
      if (!chat.busy && !chat.starting) void chat.send(text, null);
    }
  }

  let schedulesOpen = $state(false);

  /** Opens fleet lead conversation `id` (a scheduled run's report) in its tab. */
  async function openConversation(id: string): Promise<void> {
    try {
      const summary = (await aiSessions()).find((session) => session.id === id);
      if (!summary) {
        notify(t('ai.historyEmpty'), 'info');
        return;
      }
      schedulesOpen = false;
      await resumeFleet(summary, await aiRecords(id));
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  /** Continues an earlier conversation with the fleet lead in its tab. */
  async function resumeFleet(summary: AiSessionSummary, records: AiRecord[]): Promise<void> {
    ai.listen();
    await ai.resume(summary, records);
    openFleet();
  }

  /** Opens the folder of the log files, to send them along with a problem. */
  async function openLogs(): Promise<void> {
    try {
      await logsReveal();
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  async function importSshConfig(): Promise<void> {
    try {
      let report;
      if (isMobile) {
        const picked = await openDialog({ directory: false, multiple: false, title: t('host.chooseSshConfig') });
        if (typeof picked !== 'string') return;
        report = await hosts.importSshConfig(picked);
      } else {
        report = await hosts.importSshConfig();
      }
      showNotice(t('host.imported', { created: report.created.length, skipped: report.skipped.length }));
    } catch (error) {
      showNotice(errorText(error), 'error');
    }
  }
  let keysOpen = $state(false);
  function showNotice(message: string, kind: ToastKind = 'info'): void {
    notify(message, kind);
  }

  function focusedTerminal() {
    const pane = workspace.focusedPane;
    return pane ? terminals.get(pane.id) : undefined;
  }

  function openTerminal(target: Extract<PaneTarget, { kind: 'ssh' | 'quick' | 'local' }>, password: string | null = null, remember = false): void {
    if (viewport.compact) {
      prepareMobilePane();
      void workspace.openSingleTerminal(target, password, remember).catch((error) => showNotice(errorText(error), 'error'));
    } else {
      workspace.openTab(target, password, remember);
    }
  }

  function connect(host: HostEntry): void {
    const target = { kind: 'ssh', hostId: host.id, title: host.name } as const;
    const reusable = viewport.compact && findReusableTerminalPane(Object.values(workspace.panes), target);
    if (!reusable && needsPassword(host)) {
      passwordRequest = { kind: 'host', host };
      return;
    }
    openTerminal(target);
  }

  function openFiles(host: HostEntry, password: string | null = null, remember = false): void {
    if (password === null && needsPassword(host)) {
      passwordRequest = { kind: 'files', host };
      return;
    }
    if (viewport.compact) prepareMobilePane();
    workspace.openTab({ kind: 'sftp', source: { hostId: host.id }, title: host.name }, password, remember);
  }

  function hostIdOf(target: PaneTarget): string | undefined {
    if (target.kind === 'ssh') return target.hostId;
    if ((target.kind === 'sftp' || target.kind === 'editor') && 'hostId' in target.source) return target.source.hostId;
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
      if (request.kind === 'host') openTerminal({ kind: 'ssh', hostId: host.id, title: host.name }, password, remember);
      else openFiles(host, password, remember);
      if (remember) setTimeout(() => void hosts.refresh(), 3000);
    } else {
      workspace.reopen(request.paneId, password, remember, request.override ? 'password' : 'natural');
    }
  }

  // macOS: the menu bar follows the interface language; its items run actions here.
  $effect(() => {
    if (!isMacDesktop) return;
    menuSet(nativeMenus(), nativeMenuLabels()).catch((error: unknown) => console.warn('menu bar unavailable', error));
  });
  $effect(() => {
    if (!isMacDesktop) return;
    const pending = onMenuAction((id) => {
      if (isActionId(id)) run(id);
    });
    return () => void pending.then((unlisten) => unlisten());
  });

  function run(id: ActionId): void {
    switch (id) {
      case 'editor.save':
      case 'editor.find':
      case 'editor.undo':
      case 'editor.redo':
      case 'editor.close':
        editors.run(id);
        break;
      case 'editor.drafts':
        editors.showDrafts = true;
        break;
      case 'tab.newLocal':
        if (!capabilities.localPty) break;
        openTerminal({ kind: 'local' });
        break;
      case 'tab.close':
        if (document.activeElement?.closest('.text-editor') && editors.run('editor.close')) break;
        workspace.closeFocused();
        break;
      case 'tab.next':
        workspace.cycleTab(1, viewport.compact);
        break;
      case 'tab.prev':
        workspace.cycleTab(-1, viewport.compact);
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
        if (text) writeClipboard(text).catch((error: unknown) => console.warn('copy failed', error));
        break;
      }
      case 'terminal.paste':
        void readClipboard().then((text) => text && focusedTerminal()?.paste(text));
        break;
      case 'palette.open':
        paletteOpen = true;
        break;
      case 'settings.open':
        openSettings();
        break;
      case 'help.shortcuts':
        openSettings(viewport.compact ? undefined : 'shortcuts');
        break;
      case 'fleet.open':
        openFleet();
        break;
      case 'fleet.schedules':
        if (!capabilities.schedules) break;
        ai.unreadReports = 0;
        schedulesOpen = true;
        break;
      case 'fleet.new':
        void ai.reset(ai.chat({ kind: 'fleet' }), ai.fleetAgent).then(() => openFleet());
        break;
      case 'help.logs':
        void openLogs();
        break;
      case 'help.checkUpdates':
        void updater.check({ silent: false, interactive: true });
        break;
      case 'help.about':
        openSettings('about');
        break;
      case 'host.import':
        void importSshConfig();
        break;
      case 'keys.manage':
        keysOpen = true;
        break;
      case 'panel.right':
        if (panelsSuspended) {
          if (effectiveRightPage) closeRight();
        } else if (rightPage) closeRight();
        else openRight(lastRightPage);
        break;
      case 'panel.status':
        toggleRight('status');
        break;
      case 'panel.snippets':
        toggleRight('snippets');
        break;
      case 'panel.transfers':
        // Opened from the downloads popover too: it shows the page rather than toggling it.
        if (panelsSuspended && !viewport.compact) {
          suspendedTransfersOpen = true;
        } else {
          openRight('transfers');
        }
        break;
      case 'panel.bottom':
        toggleBottom();
        break;
      case 'ai.open':
        toggleRight('ai');
        break;
      case 'ai.showWaiting':
        showWaiting();
        break;
      case 'host.quickConnect':
        quickOpen = true;
        break;
      case 'host.new':
        editing = null;
        break;
      case 'sidebar.toggle':
        if (!viewport.compact) sidebarOpen = !sidebarOpen;
        break;
      case 'files.open':
        if (!workspace.openFilesBesideFocused(!viewport.compact)) showNotice(t('files.noRemote'));
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
        if (id.startsWith('tab.goto')) workspace.gotoTab(Number(id.slice('tab.goto'.length)) - 1, viewport.compact);
    }
  }

  /** Where copy and paste stand now, to tell whether the shortcut belongs to the terminal. */
  function clipboardContext(): ClipboardContext {
    const active = document.activeElement;
    // Selecting in xterm can leave a DOM selection inside the terminal itself; only one
    // elsewhere on the page counts as the page's own.
    const selection = window.getSelection();
    const anchor = selection?.anchorNode ?? null;
    const anchorElement = anchor instanceof Element ? anchor : (anchor?.parentElement ?? null);
    const pageSelection = anchorElement?.closest('.xterm') ? '' : (selection?.toString() ?? '');
    return {
      terminalSelection: focusedTerminal()?.selection() ?? '',
      pageSelection,
      focusInTerminal: !active || active === document.body || !!active.closest('.xterm'),
      focusInTextField: isTextField(active),
    };
  }

  /** The system's Copy (the macOS Edit menu takes ⌘C before the page sees it) copies the
   *  terminal's selection when that is the only one; xterm covers the case where it has the focus. */
  function onCopy(event: ClipboardEvent): void {
    const context = clipboardContext();
    if (!event.clipboardData || !terminalCopies(context) || context.focusInTerminal) return;
    event.clipboardData.setData('text/plain', context.terminalSelection);
    event.preventDefault();
  }

  function onKeydown(event: KeyboardEvent): void {
    const id = actionFor(event);
    if (!id) return;
    // Copy and paste that are not the terminal's keep the system's behaviour.
    if (id === 'terminal.copy' && !terminalCopies(clipboardContext())) return;
    if (id === 'terminal.paste' && !terminalPastes(clipboardContext())) return;
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

  // Scheduled runs report even when no AI view is open.
  $effect(() => {
    ai.listen();
    if (!capabilities.schedules) return;
    return ai.onReport((event) =>
      notify(event.ok ? t('schedule.done', { name: event.name }) : t('schedule.failed', { name: event.name, error: event.error ?? '' }), event.ok ? 'info' : 'error'),
    );
  });

  let allowingExit = false;
  let decidingExit = false;
  async function confirmExitDrafts(): Promise<boolean> {
    if (!await editors.canClose(editors.drafts.map((draft) => draft.id))) return false;
    if (externalEdits.all.length && !await ask({ title: t('external.exitTitle'), message: t('external.exitConfirm'), confirmLabel: t('external.exit'), danger: true })) return false;
    return true;
  }
  async function protectExit(): Promise<void> {
    if (decidingExit) return;
    decidingExit = true;
    editors.exiting = true;
    try {
      if (!await confirmExitDrafts()) return;
      allowingExit = true;
      await externalEditShutdown();
    } catch (error) { allowingExit = false; notify(errorText(error), 'error'); }
    finally { decidingExit = false; if (!allowingExit) editors.exiting = false; }
  }
  async function restartForUpdate(): Promise<void> {
    if (decidingExit) return;
    decidingExit = true;
    editors.exiting = true;
    try {
      if (!await confirmExitDrafts()) return;
      allowingExit = true;
      await updateRestart();
    } catch (error) {
      allowingExit = false;
      notify(errorText(error), 'error');
    } finally {
      decidingExit = false;
      if (!allowingExit) editors.exiting = false;
    }
  }
  function protectReload(event: BeforeUnloadEvent): void {
    if (!allowingExit && (editors.pending || externalEdits.all.length)) { event.preventDefault(); event.returnValue = ''; }
  }
  let lastDirty: boolean | null = null;
  $effect(() => {
    const dirty = editors.pending;
    if (lastDirty === dirty) return;
    lastDirty = dirty;
    void externalEditSetDirty(dirty).catch(() => { lastDirty = null; });
  });
  onMount(() => {
    if (!isMobile) void externalEdits.start();
    const exitListener = onExternalEditExitRequest(() => void protectExit());
    return () => {
      externalEdits.dispose();
      void exitListener.then((stop) => stop()).catch(() => { /* The WebView may already have closed. */ });
    };
  });

  onMount(() => {
    if (!isMobile) return;
    let mounted = true;
    const listeners = [
      onMobileBack(() => {
        if (mounted && !handleBack()) {
          void mobileBackDefault().catch((error: unknown) => console.warn('native back unavailable', error));
        }
      }),
      onMobileLifecycle(({ state }) => {
        if (mounted && state === 'foreground') {
          for (const term of terminals.values()) term.refresh();
        }
      }),
    ].map((listener) => listener.catch((error: unknown) => console.warn('mobile events unavailable', error)));
    return () => {
      mounted = false;
      for (const listener of listeners) void listener.then((stop) => stop?.());
    };
  });

  onMount(() => {
    if (!viewport.compact) {
      workspaceLoad()
        .then((stored) => {
          const snapshot = parseSnapshot(stored);
          if (snapshot) workspace.restore(snapshot);
        })
        .catch((error: unknown) => console.warn('workspace restore unavailable', error))
        .finally(() => (restored = true));
    }
    vault.init().catch((error: unknown) => console.warn('vault state unavailable', error));
    hosts.refresh().catch((error: unknown) => showNotice(errorText(error), 'error'));
    keys.refresh().catch((error: unknown) => console.warn('key library unavailable', error));
    forwardStatuses.start().catch((error: unknown) => console.warn('forward events unavailable', error));
    if (!isMobile) void tailscale.init();
    const cancelStartupUpdate = updater.scheduleStartupCheck();
    const unlisten = onHostKeyPrompt((prompt) => (hostKeyPrompts = [...hostKeyPrompts, prompt]));
    return () => {
      cancelStartupUpdate();
      void unlisten.then((stop) => stop());
    };
  });
</script>

<svelte:window onkeydowncapture={onKeydown} onbeforeunload={protectReload} />
<svelte:document oncopy={onCopy} />

<div class="app" class:compact={viewport.compact} class:terminal-immersion={terminalImmersion}>
  {#if !viewport.compact}
    <TitleBar {sidebarOpen} rightOpen={effectiveRightPage !== null} bottomOpen={effectiveBottomOpen} panelsDisabled={panelsSuspended} onaction={run} />
  {/if}
  <div class="body">
    {#if !viewport.compact && sidebarOpen}
      <HostSidebar
        onconnect={connect}
        onfiles={openFiles}
        onforwards={(host) => (forwardsHost = host)}
        onedit={(host) => (editing = host)}
        onnotice={(message) => showNotice(message, 'error')}
        onsettings={() => (settingsOpen = true)}
        ontailscale={() => (tailscaleOpen = true)}
        onkeys={() => run('keys.manage')}
      />
    {/if}
    <div class="main">
      <div class="workarea">
        <div class="center">
          {#snippet home()}
            {#if !viewport.compact}
              <HomePage
                onconnect={connect}
                onfiles={openFiles}
                onedit={(host) => (editing = host)}
                onquick={() => (quickOpen = true)}
                onlocal={() => run('tab.newLocal')}
                onforwards={(host) => (forwardsHost = host)}
                onnotice={(message) => showNotice(message, 'error')}
                onaction={run}
                onfleet={openFleet}
                onfleetresume={(summary, records) => void resumeFleet(summary, records)}
              />
            {:else if mobileTab === 'settings'}
              <MobileSettingsPage
                section={mobileSettingsSection}
                onselect={(section, pushCurrent) => (mobileSettingsNav = selectMobileSettingsSection(mobileSettingsNav, section, pushCurrent))}
                onback={backFromMobileSettings}
                onopentailscale={() => (tailscaleOpen = true)}
              />
            {:else if mobileTab === 'tools'}
              <MobileToolsPage
                activeTransfers={activeTransfersCount}
                activeForwards={activeForwardsCount}
                onkeys={() => (keysOpen = true)}
                onimport={() => void importSshConfig()}
                onquick={() => (quickOpen = true)}
                ontransfers={() => openRight('transfers')}
                onforwards={(host) => (forwardsHost = host)}
                onsnippets={() => openRight('snippets')}
                onnotice={showNotice}
              />
            {:else}
              <MobileHostsPage
                {fleetWaiting}
                {backgroundTerminal}
                onconnect={connect}
                onfiles={openFiles}
                onforwards={(host) => (forwardsHost = host)}
                onedit={(host) => (editing = host)}
                onquick={() => (quickOpen = true)}
                onfleet={() => openFleet()}
                onresume={resumeMobileTerminal}
                ondisconnect={disconnectMobileTerminal}
                onnotice={(message) => showNotice(message, 'error')}
              />
            {/if}
          {/snippet}
          <EditorGroups
            {home}
            compact={viewport.compact}
            coarse={viewport.coarse}
            inert={viewport.compact && (effectiveRightPage !== null || effectiveBottomOpen)}
            onsnippets={() => toggleRight('snippets')}
            oncredentials={requestPaneCredentials}
            onedithost={(hostId) => (editing = hosts.find(hostId) ?? undefined)}
            onaisettings={() => openSettings('ai')}
            ontailscalesettings={() => openSettings('tailscale')}
            onschedules={() => run('fleet.schedules')}
            onback={showMobileHosts}
            ondisconnect={disconnectMobileTerminal}
            onai={() => openRight('ai')}
            onfiles={toggleBottom}
            onstatus={() => openRight('status')}
            onforwards={(host) => (forwardsHost = host)}
          />
          {#if effectiveBottomOpen && !viewport.compact}
            <div class="edge horizontal" role="separator" aria-orientation="horizontal" onpointerdown={resizeBottom}></div>
          {/if}
          <!-- Once opened the panel stays mounted while hidden, keeping its file browsers open. -->
          {#if bottomMounted}
            <div class="bottom" hidden={!effectiveBottomOpen} style:height={viewport.compact ? undefined : bottomSize + 'px'}>
              <BottomPanel onclose={closeBottom} />
            </div>
          {/if}
        </div>
        {#if effectiveRightPage}
          {#if !viewport.compact}
            <div class="edge vertical" role="separator" aria-orientation="vertical" onpointerdown={resizeRight}></div>
          {/if}
          <div class="right" style:width={viewport.compact ? undefined : rightSize + 'px'}>
            <RightPanel page={effectiveRightPage} onclose={closeRight} onsettings={() => openSettings('ai')} />
          </div>
        {/if}
      </div>
    </div>
    {#if !viewport.compact}
      <ActivityRail page={effectiveRightPage} bottomOpen={effectiveBottomOpen} suspended={panelsSuspended} remote={focusedRemote} onaction={run} />
    {/if}
  </div>
  {#if !viewport.compact}
    <StatusBar onaction={run} />
  {:else if showMobileNav}
    <MobileNavBar active={mobileTab} {toolsBadge} settingsBadge={updater.badgeVisible} onselect={selectMobileDestination} />
  {/if}
</div>

<Toaster />
<DraftsHost />
{#if externalEdits.dialog && !isMobile}
  <Dialog title={t('external.title', { n: externalEdits.all.length })} width={600} onclose={() => (externalEdits.dialog = false)}>
    <ExternalEdits />
    {#snippet actions()}
      <button type="button" class="secondary" onclick={() => (externalEdits.dialog = false)}>{t('common.close')}</button>
    {/snippet}
  </Dialog>
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
      openTerminal({ kind: 'quick', address }, password);
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

{#if capabilities.schedules && schedulesOpen}
  <SchedulesDialog onclose={() => (schedulesOpen = false)} onopenreport={(id) => void openConversation(id)} />
{/if}

{#if keysOpen}
  <KeysDialog onclose={() => (keysOpen = false)} />
{/if}

<ConfirmHost />
<EditMenu />

{#if settingsOpen && !viewport.compact}
  <SettingsDialog initial={settingsSection} onclose={() => (settingsOpen = false)} onopentailscale={() => { settingsOpen = false; tailscaleOpen = true; }} />
{/if}

{#if tailscaleOpen}
  <TailscaleDialog onclose={() => (tailscaleOpen = false)} onconnect={connect} onmanagekeys={() => (keysOpen = true)} onsettings={() => { tailscaleOpen = false; openSettings('tailscale'); }} />
{/if}

{#if forwardsHost}
  <ForwardsDialog host={forwardsHost} onclose={() => (forwardsHost = null)} />
{/if}

{#if hostKeyPrompts[0]}
  {#key hostKeyPrompts[0].request_id}
    <HostKeyDialog prompt={hostKeyPrompts[0]} ondone={() => (hostKeyPrompts = hostKeyPrompts.slice(1))} />
  {/key}
{/if}

{#if updater.dialogOpen}
  <UpdateDialog onrestart={restartForUpdate} onclose={() => updater.closeDialog()} />
{/if}

<style>
  .app {
    height: 100vh;
    height: 100dvh;
    padding: var(--safe-top) var(--safe-right) max(var(--safe-bottom), var(--ime-bottom)) var(--safe-left);
    display: flex;
    flex-direction: column;
  }
  .app.terminal-immersion {
    background: var(--term-bg);
  }
  /* The sidebar, the work area and the right panel float as rounded cards over the window
     background (and its system material), separated by narrow gaps. */
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: var(--gap-panel);
    /* The activity rail is the right edge and the status bar the bottom: no padding there. */
    padding: 0 0 0 var(--gap-panel);
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
  .compact .body,
  .compact .workarea {
    position: relative;
    isolation: isolate;
    padding: 0;
    gap: 0;
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
  .compact .bottom,
  .compact .right {
    position: absolute;
    inset: 0;
    z-index: 4;
    border-radius: 0;
    box-shadow: none;
    background: var(--surface);
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
</style>
