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

  interface Props {
    tab: Tab;
    visible: boolean;
    /** Called when a pane needs a password before it can connect. */
    oncredentials: (paneId: PaneId, kind: CredentialKind) => void;
  }

  let { tab, visible, oncredentials }: Props = $props();

  function credentialRequest(id: PaneId, error: unknown): void {
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
          return sessionOpenSsh(target.hostId, pane.password, pane.remember, size, onOutput);
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
            onerror={(error) => credentialRequest(id, error)}
          />
        {:else}
          <TermView
            bind:this={views[id]}
            open={opener(pane)}
            active={visible && tab.focused === id}
            onsession={(sessionId) => workspace.attached(id, sessionId)}
            onexit={(status) => onExit(pane, status.code)}
            ontitle={(title) => workspace.retitle(id, title)}
            onerror={(error) => credentialRequest(id, error)}
          />
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
