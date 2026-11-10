<script lang="ts">
  // The work area: editor groups laid out by the split tree, each a floating card with its own
  // tab bar. The gaps between cards are the handles for resizing. Every tab's content lives in one
  // layer above the cards, placed over its group's content area, so switching tabs, splitting,
  // resizing or moving a tab to another group never remounts it or restarts its terminal.
  import type { Snippet } from 'svelte';
  import GroupTabBar from '../tabs/GroupTabBar.svelte';
  import PaneView from './PaneView.svelte';
  import { workspace, type GroupId } from '../workspace/workspace.svelte';
  import { tabDrag } from '../tabs/tabDrag.svelte';
  import type { CredentialKind } from '../ipc';
  import type { Divider, PaneId, Rect } from './layout';

  interface Props {
    /** The home page, shown in a group with no current tab. */
    home: Snippet;
    oncredentials: (paneId: PaneId, kind: CredentialKind, override?: boolean) => void;
    onedithost: (hostId: string) => void;
  }

  let { home, oncredentials, onedithost }: Props = $props();

  /** Space between group cards, in CSS pixels; matches `--gap-panel`. */
  const GAP = 6;
  /** Height of a group's tab bar (`GroupTabBar`), above its content. */
  const TAB_BAR = 40;
  const EPSILON = 1e-6;

  let area: HTMLDivElement;
  const layout = $derived(workspace.layout());
  const multiple = $derived(layout.panes.size > 1);

  function percent(value: number): string {
    return `${(value * 100).toFixed(4)}%`;
  }

  /** A group's card: its rectangle less half a gap on every inner edge. */
  function cardStyle(rect: Rect): string {
    const left = rect.x > EPSILON ? GAP / 2 : 0;
    const right = rect.x + rect.w < 1 - EPSILON ? GAP / 2 : 0;
    const top = rect.y > EPSILON ? GAP / 2 : 0;
    const bottom = rect.y + rect.h < 1 - EPSILON ? GAP / 2 : 0;
    return [
      `left: calc(${percent(rect.x)} + ${left}px)`,
      `top: calc(${percent(rect.y)} + ${top}px)`,
      `width: calc(${percent(rect.w)} - ${left + right}px)`,
      `height: calc(${percent(rect.h)} - ${top + bottom}px)`,
    ].join('; ');
  }

  /** A group's content area: its card below the tab bar. */
  function stageStyle(rect: Rect): string {
    const left = rect.x > EPSILON ? GAP / 2 : 0;
    const right = rect.x + rect.w < 1 - EPSILON ? GAP / 2 : 0;
    const top = (rect.y > EPSILON ? GAP / 2 : 0) + TAB_BAR;
    const bottom = rect.y + rect.h < 1 - EPSILON ? GAP / 2 : 0;
    return [
      `left: calc(${percent(rect.x)} + ${left}px)`,
      `top: calc(${percent(rect.y)} + ${top}px)`,
      `width: calc(${percent(rect.w)} - ${left + right}px)`,
      `height: calc(${percent(rect.h)} - ${top + bottom}px)`,
    ].join('; ');
  }

  /** The part of a group's content a drop would take, as insets in percent. */
  function dropZone(target: NonNullable<typeof tabDrag.target>): string {
    if (target.kind !== 'split') return 'inset: 0';
    if (target.dir === 'row') return target.before ? 'inset: 0 50% 0 0' : 'inset: 0 0 0 50%';
    return target.before ? 'inset: 0 0 50% 0' : 'inset: 50% 0 0 0';
  }

  const dropRect = $derived(
    tabDrag.target && tabDrag.target.kind !== 'tabs' ? layout.panes.get(tabDrag.target.groupId) : undefined,
  );

  function dividerStyle(divider: Divider): string {
    const { area: rect, ratio, dir } = divider;
    return dir === 'row'
      ? `left: calc(${percent(rect.x + rect.w * ratio)} - ${GAP / 2}px); top: ${percent(rect.y)}; width: ${GAP}px; height: ${percent(rect.h)};`
      : `top: calc(${percent(rect.y + rect.h * ratio)} - ${GAP / 2}px); left: ${percent(rect.x)}; height: ${GAP}px; width: ${percent(rect.w)};`;
  }

  function startDrag(event: PointerEvent, divider: Divider): void {
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const bounds = area.getBoundingClientRect();
    const move = (moveEvent: PointerEvent): void => {
      const { area: rect, dir } = divider;
      const ratio =
        dir === 'row'
          ? (moveEvent.clientX - bounds.left - rect.x * bounds.width) / (rect.w * bounds.width)
          : (moveEvent.clientY - bounds.top - rect.y * bounds.height) / (rect.h * bounds.height);
      workspace.resize(divider.path, ratio);
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

  function focusGroup(groupId: GroupId): void {
    if (workspace.focusedGroupId !== groupId) workspace.focusGroup(groupId);
  }
</script>

<div class="groups" bind:this={area}>
  {#each [...layout.panes] as [groupId, rect] (groupId)}
    {@const group = workspace.groups[groupId]}
    {#if group}
      {@const focused = workspace.focusedGroupId === groupId}
      <section class="group" role="group" class:focused={multiple && focused} style={cardStyle(rect)} onpointerdown={() => focusGroup(groupId)} onfocusin={() => focusGroup(groupId)}>
        <GroupTabBar {group} {focused} closable={multiple} />
        <div class="stage" data-stage={groupId}>
          {#if group.active === null}
            <div class="home">{@render home()}</div>
          {/if}
        </div>
      </section>
    {/if}
  {/each}
  {#each Object.values(workspace.panes) as pane (pane.id)}
    {@const group = workspace.groupOf(pane.id)}
    {@const rect = group ? layout.panes.get(group.id) : undefined}
    {#if group && rect}
      {@const shown = group.active === pane.id}
      <div class="slot" class:hidden={!shown} style={stageStyle(rect)} data-stage={group.id} role="presentation" onpointerdown={() => focusGroup(group.id)}>
        <PaneView {pane} {shown} focused={workspace.focusedGroupId === group.id && shown} {oncredentials} {onedithost} />
      </div>
    {/if}
  {/each}
  {#if tabDrag.target && dropRect}
    <div class="drop" style={stageStyle(dropRect)}><div class="zone" style={dropZone(tabDrag.target)}></div></div>
  {/if}
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
  .groups {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  .group {
    position: absolute;
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: var(--panel-shadow);
    overflow: hidden;
    transition: box-shadow var(--dur-fast) var(--ease-out);
  }
  /* With several groups, the focused one carries a soft accent outline. */
  .group.focused {
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--accent) 45%, transparent),
      0 1px 3px rgb(0 0 0 / 0.06);
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    background: var(--term-bg);
  }
  .home {
    position: absolute;
    inset: 0;
  }
  /* A tab's content, placed over its group's content area. */
  .slot {
    position: absolute;
    z-index: 1;
    overflow: hidden;
    border-radius: 0 0 var(--radius-lg) var(--radius-lg);
  }
  .slot.hidden {
    visibility: hidden;
    pointer-events: none;
  }
  /* Where a dragged tab would go: the whole group, or the half that becomes a new group. */
  .drop {
    position: absolute;
    z-index: 3;
    pointer-events: none;
  }
  .zone {
    position: absolute;
    margin: var(--space-1);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--accent) 55%, transparent);
    transition: inset var(--dur-fast) var(--ease-out);
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
  .divider::after {
    content: '';
    position: absolute;
    border-radius: 1px;
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .divider.row::after {
    top: var(--space-3);
    bottom: var(--space-3);
    left: calc(50% - 1px);
    width: 2px;
  }
  .divider.column::after {
    left: var(--space-3);
    right: var(--space-3);
    top: calc(50% - 1px);
    height: 2px;
  }
  .divider:hover::after {
    background: var(--accent);
  }
</style>
