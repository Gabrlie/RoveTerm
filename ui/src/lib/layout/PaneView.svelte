<script lang="ts">
  // One tab's content: a terminal or a file browser, with the connection failure banner. Stays
  // mounted while another tab of its group is shown (hidden, keeping its size).
  import TermView from '../terminal/TermView.svelte';
  import AiPanel from '../ai/AiPanel.svelte';
  import FileBrowser from '../files/FileBrowser.svelte';
  import { terminals } from '../terminal/registry';
  import { hostOf, workspace, type PaneState } from '../workspace/workspace.svelte';
  import { sessionOpenLocal, sessionOpenQuick, sessionOpenSsh, credentialKindOf, type CredentialKind, type OutputMessage, type TermSize } from '../ipc';
  import type { PaneId } from './layout';
  import { errorText, t } from '../i18n';
  import Icon from '../ui/Icon.svelte';
  import { hosts } from '../hosts/hosts.svelte';

  interface Props {
    pane: PaneState;
    /** The tab is its group's current one. */
    shown: boolean;
    /** Shown in the focused group: takes keyboard focus. */
    focused: boolean;
    /**
     * Called when the pane needs a typed credential before it can connect. `override` asks for a
     * login password for a host that normally uses keys.
     */
    oncredentials: (paneId: PaneId, kind: CredentialKind, override?: boolean) => void;
    /** Opens the editor of a saved host. */
    onedithost: (hostId: string) => void;
    onaisettings: () => void;
  }

  let { pane, shown, focused, oncredentials, onedithost, onaisettings }: Props = $props();
  let view = $state<TermView>();

  /** The saved host was deleted (here or on another device) and the tab holds no live session:
   *  it can never connect again, so it says so instead of failing with a lookup error. */
  const orphaned = $derived.by(() => {
    const hostId = hostOf(pane.target);
    return hostId !== null && hosts.loaded && !hosts.find(hostId) && !pane.sessionId;
  });

  /** Records the failure; a missing password or passphrase is asked for right away. */
  function onOpenError(error: unknown): void {
    workspace.failed(pane.id, error);
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
          return Promise.reject(new Error('this tab has no terminal'));
      }
    };
  }

  function onExit(code: number | null): void {
    workspace.exited(pane.id, { code, signal: null });
    // A local shell that exits normally closes its tab, like a terminal window.
    if (pane.target.kind === 'local' && code === 0) workspace.closePane(pane.id);
  }

  // Restored tabs connect the first time they are shown.
  $effect(() => {
    if (shown) workspace.undefer(pane.id);
  });

  $effect(() => {
    if (view) terminals.set(pane.id, view);
  });
</script>

<div class="pane-view" class:hidden={!shown} role="group" onfocusin={() => workspace.focus(pane.id)}>
  {#if orphaned}
    <div class="gone">
      <Icon name="server" size={28} />
      <p>{t('tab.hostDeleted')}</p>
      <button type="button" class="secondary" onclick={() => workspace.closePane(pane.id)}>{t('common.close')}</button>
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
      onopened={() => workspace.opened(pane.id)}
      onerror={(error) => {
        const kind = credentialKindOf(error);
        if (kind) oncredentials(pane.id, kind);
      }}
    />
  {:else if pane.target.kind === 'fleet'}
    <div class="fleet">
      <AiPanel fleet onsettings={onaisettings} />
    </div>
  {:else}
    <TermView
      bind:this={view}
      open={opener(pane)}
      active={focused}
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
          <button type="button" class="secondary" onclick={() => workspace.closePane(pane.id)}>{t('common.close')}</button>
        </div>
      </div>
    {/if}
    {#if pane.failure}
      {@const failure = pane.failure}
      <div class="failure" role="alert">
        <Icon name="warning" size={16} />
        <span class="reason selectable">{t('connect.failed', { reason: errorText(failure) })}</span>
        <div class="buttons">
          <button type="button" class="primary" onclick={() => workspace.reopen(pane.id, null, false)}>{t('connect.retry')}</button>
          {#if failure.code === 'ssh.passphrase_required'}
            <button type="button" class="secondary" onclick={() => oncredentials(pane.id, 'passphrase')}>{t('connect.enterPassphrase')}</button>
          {/if}
          {#if pane.target.kind !== 'local' && failure.code !== 'ssh.connect' && failure.code !== 'ssh.timeout'}
            <button type="button" class="secondary" onclick={() => oncredentials(pane.id, 'password', true)}>{t('connect.usePassword')}</button>
          {/if}
          {#if pane.target.kind === 'ssh'}
            {@const hostId = pane.target.hostId}
            <button type="button" class="secondary" onclick={() => onedithost(hostId)}>{t('connect.editHost')}</button>
          {/if}
          <button type="button" class="secondary" onclick={() => workspace.closePane(pane.id)}>{t('common.close')}</button>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .pane-view {
    position: absolute;
    inset: 0;
    overflow: hidden;
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
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
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
    height: 100%;
    max-width: 920px;
    margin: 0 auto;
    padding-top: var(--space-3);
  }
</style>
