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
      ? `left: calc(${percent(area.x + area.w * ratio)} - 2px); top: ${percent(area.y)}; width: 4px; height: ${percent(area.h)};`
      : `top: calc(${percent(area.y + area.h * ratio)} - 2px); left: ${percent(area.x)}; height: 4px; width: ${percent(area.w)};`;
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
        {#if pane.target.kind === 'sftp'}
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
            onsession={(sessionId) => workspace.attached(id, sessionId)}
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
                  <button type="button" onclick={() => oncredentials(id, 'passphrase')}>{t('connect.enterPassphrase')}</button>
                {/if}
                {#if pane.target.kind !== 'local' && failure.code !== 'ssh.connect' && failure.code !== 'ssh.timeout'}
                  <button type="button" onclick={() => oncredentials(id, 'password', true)}>{t('connect.usePassword')}</button>
                {/if}
                {#if pane.target.kind === 'ssh'}
                  {@const hostId = pane.target.hostId}
                  <button type="button" onclick={() => onedithost(hostId)}>{t('connect.editHost')}</button>
                {/if}
                <button type="button" onclick={() => workspace.closePane(id)}>{t('common.close')}</button>
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
  }
  .pane.focused {
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
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
    padding: var(--space-2) var(--space-3);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, var(--border));
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    box-shadow: 0 6px 20px rgb(0 0 0 / 14%);
    color: var(--fg);
    font-size: var(--text-body);
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
    padding: 4px var(--space-3);
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: var(--text-footnote);
    cursor: pointer;
  }
  .buttons button:hover {
    background: var(--bg-sunken);
  }
  .buttons button.primary {
    border-color: var(--accent);
    background: var(--accent);
    color: #fff;
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
  .divider:hover {
    background: color-mix(in srgb, var(--accent) 35%, transparent);
  }
</style>
