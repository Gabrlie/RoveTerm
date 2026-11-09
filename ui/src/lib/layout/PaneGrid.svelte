<script lang="ts">
  // Renders one tab's panes as absolutely positioned views. Panes stay mounted while the layout
  // changes, so splitting or resizing never restarts a terminal.
  import TermView from '../terminal/TermView.svelte';
  import FileBrowser from '../files/FileBrowser.svelte';
  import { terminals } from '../terminal/registry';
  import { workspace, type PaneState, type Tab } from '../workspace/workspace.svelte';
  import { sessionOpenLocal, sessionOpenQuick, sessionOpenSsh, type OutputMessage, type TermSize } from '../ipc';
  import type { Divider, PaneId } from './layout';
  import { credentialKindOf, type CredentialKind } from '../ipc';
  import { errorText, t } from '../i18n';
  import Icon from '../ui/Icon.svelte';
  import { hosts } from '../hosts/hosts.svelte';

  interface Props {
    tab: Tab;
    visible: boolean;
    /**
     * Called when a pane needs a typed credential before it can connect. `override` asks for a
     * login password for a host that normally uses keys.
     */
    oncredentials: (paneId: PaneId, kind: CredentialKind, override?: boolean) => void;
    /** Opens the editor of a saved host. */
    onedithost: (hostId: string) => void;
  }

  let { tab, visible, oncredentials, onedithost }: Props = $props();

  /** Records the failure; a missing password or passphrase is asked for right away. */
  function onOpenError(id: PaneId, error: unknown): void {
    workspace.failed(id, error);
    const kind = credentialKindOf(error);
    if (kind) oncredentials(id, kind);
  }

  let grid: HTMLDivElement;
  const views: Record<PaneId, TermView> = $state({});
  const layout = $derived(workspace.layoutOf(tab));
  const multiple = $derived(layout.panes.size > 1);

  function opener(pane: PaneState) {
    return (size: TermSize, onOutput: (message: OutputMessage) => void) => {
      const target = pane.target;
      switch (target.kind) {
        case 'local':
          return sessionOpenLocal(size, onOutput);
        case 'ssh':
          return sessionOpenSsh(target.hostId, pane.password, pane.passwordKind, pane.remember, size, onOutput);
        case 'quick':
          return sessionOpenQuick(target.address, pane.password, size, onOutput);
        case 'sftp':
          return Promise.reject(new Error('file panes have no terminal'));
      }
    };
  }

  function percent(value: number): string {
    return `${(value * 100).toFixed(4)}%`;
  }

  function dividerStyle(divider: Divider): string {
    const { area, ratio, dir } = divider;
    return dir === 'row'
      ? `left: calc(${percent(area.x + area.w * ratio)} - 3px); top: ${percent(area.y)}; width: 6px; height: ${percent(area.h)};`
      : `top: calc(${percent(area.y + area.h * ratio)} - 3px); left: ${percent(area.x)}; height: 6px; width: ${percent(area.w)};`;
  }

  function startDrag(event: PointerEvent, divider: Divider): void {
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const bounds = grid.getBoundingClientRect();
    const move = (moveEvent: PointerEvent): void => {
      const { area, dir } = divider;
      const ratio =
        dir === 'row'
          ? (moveEvent.clientX - bounds.left - area.x * bounds.width) / (area.w * bounds.width)
          : (moveEvent.clientY - bounds.top - area.y * bounds.height) / (area.h * bounds.height);
      workspace.resize(tab.id, divider.path, ratio);
    };
    const end = (): void => {
      target.removeEventListener('pointermove', move);
      target.removeEventListener('pointerup', end);
      target.removeEventListener('pointercancel', end);
    };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', end);
    target.addEventListener('pointercancel', end);
  }

  // Restored panes connect the first time their tab is shown, however it got activated.
  $effect(() => {
    if (visible) workspace.undefer(tab.id);
  });

  $effect(() => {
    for (const [id, view] of Object.entries(views)) {
      if (view) terminals.set(id, view);
    }
  });

  function onExit(pane: PaneState, code: number | null): void {
    workspace.exited(pane.id, { code, signal: null });
    // A local shell that exits normally closes its pane, like a terminal window.
    if (pane.target.kind === 'local' && code === 0) workspace.closePane(pane.id);
  }
</script>

<div class="grid" class:hidden={!visible} bind:this={grid}>
  {#each [...layout.panes] as [id, rect] (id)}
    {@const pane = workspace.panes[id]}
    {#if pane}
      <div
        class="pane"
        class:focused={multiple && tab.focused === id}
        style:left={percent(rect.x)}
        style:top={percent(rect.y)}
        style:width={percent(rect.w)}
        style:height={percent(rect.h)}
        role="group"
        onfocusin={() => workspace.focus(id)}
        onpointerdown={() => workspace.focus(id)}
      >
        {#if pane.deferred}
          <div class="deferred">{pane.title}</div>
        {:else if pane.target.kind === 'sftp'}
          <FileBrowser
            source={pane.target.source}
            password={pane.password}
            remember={pane.remember}
            active={visible && tab.focused === id}
            {visible}
            onopened={() => workspace.opened(id)}
            onerror={(error) => {
              const kind = credentialKindOf(error);
              if (kind) oncredentials(id, kind);
            }}
          />
        {:else}
          <TermView
            bind:this={views[id]}
            open={opener(pane)}
            active={visible && tab.focused === id}
            onsession={(sessionId) => {
              workspace.attached(id, sessionId);
              if (pane.target.kind === 'ssh') hosts.connected(pane.target.hostId).catch((error: unknown) => console.warn('host refresh failed', error));
            }}
            onexit={(status) => onExit(pane, status.code)}
            ontitle={(title) => workspace.retitle(id, title)}
            onerror={(error) => onOpenError(id, error)}
          />
          {#if pane.failure}
            {@const failure = pane.failure}
            <div class="failure" role="alert">
              <Icon name="warning" size={16} />
              <span class="reason">{t('connect.failed', { reason: errorText(failure) })}</span>
              <div class="buttons">
                <button type="button" class="primary" onclick={() => workspace.reopen(id, null, false)}>{t('connect.retry')}</button>
                {#if failure.code === 'ssh.passphrase_required'}
                  <button type="button" class="secondary" onclick={() => oncredentials(id, 'passphrase')}>{t('connect.enterPassphrase')}</button>
                {/if}
                {#if pane.target.kind !== 'local' && failure.code !== 'ssh.connect' && failure.code !== 'ssh.timeout'}
                  <button type="button" class="secondary" onclick={() => oncredentials(id, 'password', true)}>{t('connect.usePassword')}</button>
                {/if}
                {#if pane.target.kind === 'ssh'}
                  {@const hostId = pane.target.hostId}
                  <button type="button" class="secondary" onclick={() => onedithost(hostId)}>{t('connect.editHost')}</button>
                {/if}
                <button type="button" class="secondary" onclick={() => workspace.closePane(id)}>{t('common.close')}</button>
              </div>
            </div>
          {/if}
        {/if}
      </div>
    {/if}
  {/each}
  {#each layout.dividers as divider (divider.path.join('') || 'root')}
    <div
      class="divider {divider.dir}"
      style={dividerStyle(divider)}
      role="separator"
      aria-orientation={divider.dir === 'row' ? 'vertical' : 'horizontal'}
      onpointerdown={(event) => startDrag(event, divider)}
    ></div>
  {/each}
</div>

<style>
  .grid {
    position: absolute;
    inset: 0;
  }
  .grid.hidden {
    visibility: hidden;
    pointer-events: none;
  }
  .pane {
    position: absolute;
    box-sizing: border-box;
    border: 1px solid transparent;
    /* Content stays inside its pane: wide file lists must not paint over neighbouring panels. */
    overflow: hidden;
  }
  .pane.focused {
    border-color: color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .deferred {
    display: grid;
    place-items: center;
    height: 100%;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
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
  .divider {
    position: absolute;
    z-index: 2;
  }
  .divider.row {
    cursor: col-resize;
  }
  .divider.column {
    cursor: row-resize;
  }
  /* Dividers are wide hit areas drawing a hairline in the middle. */
  .divider::after {
    content: '';
    position: absolute;
    background: var(--separator);
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .divider.row::after {
    top: 0;
    bottom: 0;
    left: 50%;
    width: 1px;
  }
  .divider.column::after {
    left: 0;
    right: 0;
    top: 50%;
    height: 1px;
  }
  .divider:hover::after {
    background: var(--accent);
  }
</style>
