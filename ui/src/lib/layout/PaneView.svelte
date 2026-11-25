<script lang="ts" module>
  /** Last Tailscale connection timestamp auto-retried for each pane target, preventing retry loops. */
  const tailscaleRetriedAt = new Map<string, number>();
</script>

<script lang="ts">
  // One tab's content: a terminal or a file browser, with the connection failure banner. Stays
  // mounted while another tab of its group is shown (hidden, keeping its size).
  import { onDestroy, untrack } from 'svelte';
  import KeyBarCustomizeDialog from '../terminal/KeyBarCustomizeDialog.svelte';
  import MobileTerminalDock from '../terminal/MobileTerminalDock.svelte';
  import TermView from '../terminal/TermView.svelte';
  import TerminalKeyBar from '../terminal/TerminalKeyBar.svelte';
  import TerminalStatusBar from '../terminal/TerminalStatusBar.svelte';
  import TerminalToolsSheet from '../terminal/TerminalToolsSheet.svelte';
  import { consumeModifier, cycleModifier, type ModifierState } from '../terminal/keybar';
  import { editPrefs } from '../settings/edit.svelte';
  import { prefs } from '../settings/prefs.svelte';
  import { ai } from '../ai/ai.svelte';
  import AiPanel from '../ai/AiPanel.svelte';
  import FileBrowser from '../files/FileBrowser.svelte';
  import TextEditor from '../files/TextEditor.svelte';
  import { editors } from '../files/editors.svelte';
  import { terminals } from '../terminal/registry';
  import { isTerminalTarget } from '../workspace/singleSession';
  import { hostOf, remoteSourceOf, sourceKey, workspace, type PaneState } from '../workspace/workspace.svelte';
  import { sessionOpenLocal, sessionOpenQuick, sessionOpenSsh, credentialKindOf, type CredentialKind, type HostEntry, type HostId, type OutputMessage, type StatsSource, type TermSize } from '../ipc';
  import { hasOtherLivePaneForStats, invalidateServerStats, paneStatsSource, statsSourceKey } from '../panels/serverStats.svelte';
  import type { PaneId } from './layout';
  import { errorText, t } from '../i18n';
  import Icon from '../ui/Icon.svelte';
  import { readClipboard } from '../ui/clipboard';
  import { pushBackHandler } from '../ui/backStack';
  import { forwardStatuses } from '../forwards/forwards.svelte';
  import { hosts } from '../hosts/hosts.svelte';
  import MobilePageHeader from '../mobile/MobilePageHeader.svelte';
  import { isMobile } from '../platform';
  import { tailscale } from '../tailscale/tailscale.svelte';
  import { viewport } from './viewport.svelte';
  import {
    shouldAutoRetryTailscale,
    showPasswordFallbackAction,
    showTailscaleSettingsAction,
    usesTailscaleRoute,
  } from '../tailscale/autoRetry';

  interface Props {
    pane: PaneState;
    /** The tab is its group's current one. */
    shown: boolean;
    /** Shown in the focused group: takes keyboard focus. */
    focused: boolean;
    showKeyBar?: boolean;
    coarse?: boolean;
    compact?: boolean;
    onsnippets?: () => void;
    onback?: () => void;
    ondisconnect?: (paneId: PaneId) => void;
    onai?: () => void;
    onfiles?: () => void;
    onstatus?: () => void;
    onforwards?: (host: HostEntry) => void;
    /**
     * Called when the pane needs a typed credential before it can connect. `override` asks for a
     * login password for a host that normally uses keys.
     */
    oncredentials: (paneId: PaneId, kind: CredentialKind, override?: boolean) => void;
    /** Opens the editor of a saved host. */
    onedithost: (hostId: string) => void;
    onaisettings: () => void;
    ontailscalesettings: () => void;
    /** Opens the fleet lead's scheduled tasks. */
    onschedules: () => void;
  }

  let {
    pane, shown, focused, showKeyBar = isMobile, coarse = isMobile, compact = false, onsnippets,
    onback, ondisconnect, onai, onfiles, onstatus, onforwards,
    oncredentials, onedithost, onaisettings, ontailscalesettings, onschedules,
  }: Props = $props();
  let view = $state<TermView>();
  let ctrl = $state<ModifierState>('off');
  let alt = $state<ModifierState>('off');
  let terminalFocused = $state(false);
  let toolsOpen = $state(false);
  let customizingKeybar = $state(false);
  const compactTerminal = $derived(compact && isTerminalTarget(pane.target));
  const editorDraft = $derived(pane.target.kind === 'editor' ? editors.find(pane.target.draftId) : undefined);

  function setModifiers(nextCtrl: ModifierState, nextAlt: ModifierState): void {
    ctrl = nextCtrl;
    alt = nextAlt;
  }

  function sendKey(data: string, consume?: boolean): void {
    if (consume) setModifiers(consumeModifier(ctrl), consumeModifier(alt));
    view?.send(data);
  }

  function pasteKey(): void {
    void readClipboard().then((text) => {
      if (text) view?.paste(text);
    });
  }

  function toggleKeyboard(): void {
    if (compact || isMobile) {
      if (viewport.keyboardOpen) view?.dismissKeyboard();
      else view?.summonKeyboard();
    } else {
      if (view?.isFocused()) view.blur();
      else view?.focus();
    }
  }

  const host = $derived.by(() => {
    const hostId = hostOf(pane.target);
    return hostId ? hosts.find(hostId) ?? null : null;
  });
  const dockStatus = $derived(pane.failure ? 'failed' : pane.exited ? 'ended' : pane.sessionId ? 'live' : 'connecting');
  const dockTitle = $derived(host ? `${host.username}@${host.name}` : pane.title);
  const hostAiWaiting = $derived.by(() => {
    if (!compactTerminal) return 0;
    const target = pane.target;
    if (target.kind === 'ssh') return ai.chat({ kind: 'host', host_id: target.hostId }).waiting;
    if (target.kind === 'quick') return ai.chat({ kind: 'quick', address: target.address }).waiting;
    return ai.chat({ kind: 'local' }).waiting;
  });
  const activeForwardsCount = $derived(host ? forwardStatuses.running(host.id) : 0);

  function returnHome(): void {
    view?.dismissKeyboard();
    if (onback) onback();
    else workspace.activateHome();
  }

  function closeFiles(): void {
    workspace.closePane(pane.id);
    returnHome();
  }

  function disconnect(): void {
    if (ondisconnect) ondisconnect(pane.id);
    else void workspace.requestClosePane(pane.id);
  }

  $effect.pre(() => {
    if (!compact || !shown) return;
    if (pane.target.kind === 'sftp') return pushBackHandler(closeFiles);
    if (pane.target.kind !== 'editor') return pushBackHandler(returnHome);
    const paneId = pane.id;
    const closeEditor = (): void => {
      void workspace.requestClosePane(paneId).then(() => {
        if (active && workspace.focusedPane?.id === paneId) dispose = pushBackHandler(closeEditor);
      });
    };
    let active = true;
    let dispose = pushBackHandler(closeEditor);
    return () => {
      active = false;
      dispose();
    };
  });

  $effect(() => {
    if (!compactTerminal || !shown) {
      toolsOpen = false;
      customizingKeybar = false;
    }
  });

  const isTailscaleHost = $derived(usesTailscaleRoute(host, (id) => hosts.find(id)));
  const retryTargetKey = $derived.by(() => {
    if (pane.target.kind === 'ssh') return `ssh:${pane.target.hostId}`;
    if (pane.target.kind === 'sftp') return `sftp:${sourceKey(pane.target.source)}`;
    return pane.id;
  });

  /** The saved host was deleted (here or on another device) and the tab holds no live session:
   *  it can never connect again, so it says so instead of failing with a lookup error. */
  const orphaned = $derived.by(() => {
    if (pane.target.kind === 'editor') return false;
    const hostId = hostOf(pane.target);
    return hostId !== null && hosts.loaded && !hosts.find(hostId) && !pane.sessionId;
  });

  const hostExists = (id: HostId) => !hosts.loaded || !!hosts.find(id);

  function invalidateIfLast(source: StatsSource | null): void {
    if (source && !hasOtherLivePaneForStats(Object.values(workspace.panes), pane.id, statsSourceKey(source), hostExists)) {
      invalidateServerStats(source);
    }
  }

  function invalidateTargetStats(): void {
    invalidateIfLast(pane.target.kind === 'local' ? { kind: 'local' } : remoteSourceOf(pane.target));
  }

  /** Records the failure and requests any missing credential. */
  function onOpenError(error: unknown): void {
    workspace.failed(pane.id, error);
    invalidateTargetStats();
    const kind = credentialKindOf(error);
    if (kind) oncredentials(pane.id, kind);
  }

  function opener(state: PaneState) {
    return (size: TermSize, onOutput: (message: OutputMessage) => void) => {
      const target = state.target;
      switch (target.kind) {
        case 'local':
          return sessionOpenLocal(size, onOutput);
        case 'ssh':
          return sessionOpenSsh(target.hostId, state.password, state.passwordKind, state.remember, size, onOutput);
        case 'quick':
          return sessionOpenQuick(target.address, state.password, size, onOutput);
        case 'sftp':
        case 'fleet':
        case 'editor':
          return Promise.reject(new Error('this tab has no terminal'));
      }
    };
  }

  function onExit(code: number | null): void {
    workspace.exited(pane.id, { code, signal: null });
    invalidateTargetStats();
    // A local shell that exits normally closes its tab, like a terminal window.
    if (pane.target.kind === 'local' && code === 0) workspace.closePane(pane.id);
  }

  onDestroy(() => {
    if (!workspace.panes[pane.id]) invalidateIfLast(paneStatsSource(pane, hostExists));
  });

  // Restored tabs connect the first time they are shown.
  $effect(() => {
    if (shown) workspace.undefer(pane.id);
  });

  $effect(() => {
    if (view) terminals.set(pane.id, view);
  });

  $effect(() => {
    if (pane.target.kind === 'editor' && !editorDraft) workspace.closePane(pane.id);
  });

  // Automatically retry tabs that failed before embedded Tailscale finished connecting.
  let wasTailscaleConnected = tailscale.connected;
  $effect(() => {
    const connected = tailscale.connected;
    const prevConnected = wasTailscaleConnected;
    wasTailscaleConnected = connected;
    const key = retryTargetKey;
    if (!connected) {
      tailscaleRetriedAt.delete(key);
      return;
    }
    const failureCode = pane.failure?.code ?? null;
    if (pane.deferred || orphaned) return;
    const epoch = tailscale.status.updated_at;
    const alreadyRetriedForEpoch = tailscaleRetriedAt.get(key) === epoch;
    if (shouldAutoRetryTailscale({ isTailscaleHost, failureCode, connected, wasConnected: prevConnected, alreadyRetriedForEpoch })) {
      tailscaleRetriedAt.set(key, epoch);
      untrack(() => workspace.reopen(pane.id, null, false));
    }
  });
</script>

{#snippet terminalDock()}
  <MobileTerminalDock
    title={dockTitle}
    os={host?.os ?? null}
    status={dockStatus}
    keyboardOpen={viewport.keyboardOpen}
    toolsBadge={hostAiWaiting}
    onback={returnHome}
    ondisconnect={disconnect}
    onpillclick={pane.target.kind !== 'local' && pane.sessionId && onstatus ? () => { view?.dismissKeyboard(); onstatus(); } : undefined}
    ontools={() => { view?.dismissKeyboard(); toolsOpen = true; }}
    ontogglekeyboard={toggleKeyboard}
  />
{/snippet}

<div class="pane-view" class:compact-terminal={compactTerminal} class:hidden={!shown} role="group" onfocusin={() => workspace.focus(pane.id)}>
  {#if orphaned}
    <div class="terminal-shell">
      <div class="gone">
        <Icon name="server" size={28} />
        <p>{t('tab.hostDeleted')}</p>
        <button type="button" class="secondary" onclick={compactTerminal ? disconnect : () => workspace.closePane(pane.id)}>{t('common.close')}</button>
      </div>
      {#if compactTerminal}{@render terminalDock()}{/if}
    </div>
  {:else if pane.deferred}
    <div class="deferred">{pane.title}</div>
  {:else if pane.target.kind === 'sftp'}
    <FileBrowser
      source={pane.target.source}
      password={pane.password}
      remember={pane.remember}
      active={focused}
      visible={shown}
      onclose={compact ? closeFiles : undefined}
      onopened={() => {
        pane.failure = null;
        workspace.opened(pane.id);
      }}
      onerror={onOpenError}
    />
  {:else if pane.target.kind === 'fleet'}
    <div class="fleet" class:compact-page={compact}>
      {#if compact}<MobilePageHeader title={t('fleet.title')} onback={returnHome} />{/if}
      <AiPanel fleet onsettings={onaisettings} {onschedules} />
    </div>
  {:else if pane.target.kind === 'editor'}
    {#if editorDraft}
      {#key editorDraft.id}
        <TextEditor draft={editorDraft} active={focused} onclose={() => workspace.closePane(pane.id)} />
      {/key}
    {/if}
  {:else}
    <div class="terminal-shell">
      <div class="terminal-stage" onfocusin={() => (terminalFocused = view?.isFocused() ?? false)} onfocusout={() => (terminalFocused = false)}>
        <TermView
          bind:this={view}
          open={opener(pane)}
          active={focused}
          {coarse}
          {ctrl}
          {alt}
          onmodifiers={setModifiers}
          onsession={(sessionId) => {
            workspace.attached(pane.id, sessionId);
            if (pane.target.kind === 'ssh') hosts.connected(pane.target.hostId).catch((error: unknown) => console.warn('host refresh failed', error));
          }}
          onexit={(status) => onExit(status.code)}
          remote={pane.target.kind !== 'local'}
          onrestart={() => workspace.reopen(pane.id, null, false)}
          ontitle={(title) => workspace.retitle(pane.id, title)}
          onerror={onOpenError}
        />
        {#if pane.exited && !pane.failure}
          <div class="failure ended" role="status">
            <Icon name="info" size={16} />
            <span class="reason">{pane.target.kind === 'local' ? t('terminal.endedBar') : pane.exited.code === null && !pane.exited.signal ? t('terminal.disconnectedBar') : t('terminal.sessionEndedBar')}</span>
            <div class="buttons">
              <button type="button" class="primary" onclick={() => workspace.reopen(pane.id, null, false)}>{pane.target.kind === 'local' ? t('terminal.restart') : t('terminal.reconnect')}</button>
              <button type="button" class="secondary" onclick={compact ? disconnect : () => workspace.closePane(pane.id)}>{t('common.close')}</button>
            </div>
          </div>
        {/if}
        {#if pane.failure}
          {@const failure = pane.failure}
          {@const offerTailscaleSettings = showTailscaleSettingsAction({
            isMobile,
            isTailscaleHost,
            tailscaleConnected: tailscale.connected,
            failureCode: failure.code,
          })}
          {@const offerPasswordFallback = showPasswordFallbackAction({
            isRemote: pane.target.kind !== 'local',
            isTailscaleHost,
            tailscaleConnected: tailscale.connected,
            failureCode: failure.code,
          })}
          <div class="failure" role="alert">
            <Icon name="warning" size={16} />
            <span class="reason selectable">{t('connect.failed', { reason: errorText(failure) })}</span>
            <div class="buttons">
              <button type="button" class="primary" onclick={() => workspace.reopen(pane.id, null, false)}>{t('connect.retry')}</button>
              {#if failure.code === 'ssh.passphrase_required'}
                <button type="button" class="secondary" onclick={() => oncredentials(pane.id, 'passphrase')}>{t('connect.enterPassphrase')}</button>
              {/if}
              {#if offerTailscaleSettings}
                <button type="button" class="secondary" onclick={ontailscalesettings}>{t('connect.tailscaleSettings')}</button>
              {:else if offerPasswordFallback}
                <button type="button" class="secondary" onclick={() => oncredentials(pane.id, 'password', true)}>{t('connect.usePassword')}</button>
              {/if}
              {#if pane.target.kind === 'ssh'}
                {@const hostId = pane.target.hostId}
                <button type="button" class="secondary" onclick={() => onedithost(hostId)}>{t('connect.editHost')}</button>
              {/if}
              <button type="button" class="secondary" onclick={compact ? disconnect : () => workspace.closePane(pane.id)}>{t('common.close')}</button>
            </div>
          </div>
        {/if}
      </div>
      {#if prefs.value.status.terminal_bar && (pane.target.kind === 'ssh' || pane.target.kind === 'quick' || pane.target.kind === 'local') && pane.sessionId !== null && !pane.exited && !pane.failure}
        <TerminalStatusBar source={pane.target.kind === 'local' ? { kind: 'local' } : remoteSourceOf(pane.target)!} session={pane.sessionId} visible={shown} />
      {/if}
      {#if compact}{@render terminalDock()}{/if}
      {#if showKeyBar && pane.sessionId !== null && !pane.exited && !pane.failure}
        <TerminalKeyBar
          {ctrl}
          {alt}
          compactDock={compact}
          keyboardOpen={compact || isMobile ? viewport.keyboardOpen : terminalFocused}
          oncustomize={compact ? () => (customizingKeybar = true) : undefined}
          onctrl={() => (ctrl = cycleModifier(ctrl))}
          onalt={() => (alt = cycleModifier(alt))}
          onsend={sendKey}
          onpaste={pasteKey}
          {onsnippets}
          ontogglekeyboard={toggleKeyboard}
        />
      {/if}
    </div>
  {/if}
</div>

{#if compactTerminal && shown}
  {#if toolsOpen}
    <TerminalToolsSheet
      remote={pane.target.kind !== 'local'}
      canForwards={host !== null}
      aiWaiting={hostAiWaiting}
      activeForwards={activeForwardsCount}
      statusBarEnabled={prefs.value.status.terminal_bar}
      onclose={() => (toolsOpen = false)}
      onai={() => onai?.()}
      onfiles={() => onfiles?.()}
      onstatus={() => onstatus?.()}
      onsnippets={() => onsnippets?.()}
      onforwards={() => host && onforwards?.(host)}
      ontogglestatusbar={() => void editPrefs((draft) => { draft.status.terminal_bar = !draft.status.terminal_bar; })}
      oncustomizekeys={() => (customizingKeybar = true)}
    />
  {/if}
  {#if customizingKeybar}
    <KeyBarCustomizeDialog onclose={() => (customizingKeybar = false)} />
  {/if}
{/if}

<style>
  .pane-view {
    position: absolute;
    inset: 0;
    overflow: hidden;
  }
  .pane-view.compact-terminal {
    background: var(--term-bg);
  }
  .compact-terminal .gone {
    color: var(--term-fg);
  }
  .terminal-shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .terminal-stage {
    flex: 1;
    min-height: 0;
    position: relative;
  }
  .pane-view.hidden {
    visibility: hidden;
    pointer-events: none;
  }
  .deferred {
    display: grid;
    place-items: center;
    height: 100%;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .gone {
    display: flex;
    flex: 1;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    height: 100%;
    color: var(--fg-muted);
  }
  .gone p {
    margin: 0;
    font-size: var(--text-body);
  }
  .failure {
    position: absolute;
    left: var(--space-3);
    right: var(--space-3);
    bottom: var(--space-3);
    z-index: 3;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--material-popover);
    box-shadow: var(--glass-edge), var(--shadow-md);
    color: var(--fg);
    font-size: var(--text-body);
    animation: slide-up var(--dur-slow) var(--ease);
  }
  @keyframes slide-up {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }
  .failure :global(svg) {
    color: var(--danger);
  }
  .reason {
    flex: 1 1 200px;
    min-width: 0;
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .buttons button {
    min-width: 0;
    height: 26px;
    padding: 0 var(--space-3);
    font-size: var(--text-footnote);
  }
  /* The fleet lead's conversation, as wide as reads well. */
  .fleet {
    transition: max-width var(--dur) var(--ease);
    height: 100%;
    max-width: 920px;
    margin: 0 auto;
    padding-top: var(--space-3);
  }
  .fleet:has(:global(.has-subagent)) { max-width: 1440px; }
  .fleet.compact-page {
    display: flex;
    flex-direction: column;
    max-width: none;
    margin: 0;
    padding: 0;
  }
  .compact-page > :global(.ai-split) {
    flex: 1;
    height: auto;
    min-height: 0;
  }
</style>
