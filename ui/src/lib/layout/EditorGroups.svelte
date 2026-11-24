<script lang="ts">
  // The workspace shows either one compact group or the groups of the split tree. Both modes
  // share a pane layer keyed by pane id, preserving mounted views across layout changes.
  import type { Snippet } from 'svelte';
  import GroupTabBar from '../tabs/GroupTabBar.svelte';
  import PaneView from './PaneView.svelte';
  import { workspace, type GroupId } from '../workspace/workspace.svelte';
  import { tabDrag } from '../tabs/tabDrag.svelte';
  import type { CredentialKind } from '../ipc';
  import type { Divider, PaneId, Rect } from './layout';
  import { isMobile } from '../platform';

  interface Props {
    compact?: boolean;
    coarse?: boolean;
    inert?: boolean;
    /** The home page, shown in a group with no current tab. */
    home: Snippet;
    oncredentials: (paneId: PaneId, kind: CredentialKind, override?: boolean) => void;
    onedithost: (hostId: string) => void;
    /** Opens the AI settings (from the fleet lead's tab). */
    onaisettings: () => void;
    /** Opens the Tailscale settings (from a failed Tailscale tab). */
    ontailscalesettings: () => void;
    /** Opens the fleet lead's scheduled tasks. */
    onschedules: () => void;
    onsnippets?: () => void;
  }

  let { compact = false, coarse = false, inert = false, home, oncredentials, onedithost, onaisettings, ontailscalesettings, onschedules, onsnippets }: Props = $props();

  /** Space between group cards, in CSS pixels; matches `--gap-panel`. */
  const GAP = 6;
  const TOUCH_DIVIDER = 20;
  const EPSILON = 1e-6;

  let area: HTMLDivElement;
  const tabBarHeight = $derived(compact || coarse ? 44 : 40);
  const focusedGroup = $derived(workspace.focusedGroup);
  const activePaneId = $derived(workspace.focusedPane?.id ?? null);
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
    const top = (rect.y > EPSILON ? GAP / 2 : 0) + tabBarHeight;
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
    const size = coarse ? TOUCH_DIVIDER : GAP;
    return dir === 'row'
      ? `left: calc(${percent(rect.x + rect.w * ratio)} - ${size / 2}px); top: ${percent(rect.y)}; width: ${size}px; height: ${percent(rect.h)};`
      : `top: calc(${percent(rect.y + rect.h * ratio)} - ${size / 2}px); left: ${percent(rect.x)}; height: ${size}px; width: ${percent(rect.w)};`;
  }

  function startDrag(event: PointerEvent, divider: Divider): void {
    if (event.button !== 0 || !event.isPrimary) return;
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const bounds = area.getBoundingClientRect();
    const { area: rect, dir } = divider;
    const pointerRatio = (pointer: PointerEvent): number => dir === 'row'
      ? (pointer.clientX - bounds.left - rect.x * bounds.width) / (rect.w * bounds.width)
      : (pointer.clientY - bounds.top - rect.y * bounds.height) / (rect.h * bounds.height);
    const grabOffset = coarse ? pointerRatio(event) - divider.ratio : 0;
    const move = (moveEvent: PointerEvent): void => {
      if (moveEvent.pointerId !== event.pointerId) return;
      workspace.resize(divider.path, pointerRatio(moveEvent) - grabOffset);
    };
    const end = (endEvent: PointerEvent): void => {
      if (endEvent.pointerId !== event.pointerId) return;
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

<div class="groups" bind:this={area} {inert}>
  {#if compact}
    {#if focusedGroup}
      <section class="group compact" role="group">
        <GroupTabBar group={focusedGroup} focused={true} closable={false} compact={true} {coarse} flatTabs={workspace.flatTabs} {activePaneId} />
        <div class="stage">
          {#if activePaneId === null}
            <div class="home">{@render home()}</div>
          {/if}
        </div>
      </section>
    {/if}
  {:else}
    {#each [...layout.panes] as [groupId, rect] (groupId)}
      {@const group = workspace.groups[groupId]}
      {#if group}
        {@const focused = workspace.focusedGroupId === groupId}
        <section class="group" role="group" class:focused={multiple && focused} style={cardStyle(rect)} onpointerdown={() => focusGroup(groupId)} onfocusin={() => focusGroup(groupId)}>
          <GroupTabBar {group} {focused} closable={multiple} {coarse} />
          <div class="stage" data-stage={groupId}>
            {#if group.active === null}
              <div class="home">{@render home()}</div>
            {/if}
          </div>
        </section>
      {/if}
    {/each}
  {/if}
  {#each Object.values(workspace.panes) as pane (pane.id)}
    {@const group = workspace.groupOf(pane.id)}
    {@const rect = group ? layout.panes.get(group.id) : undefined}
    {#if group && rect}
      {@const shown = compact ? activePaneId === pane.id : group.active === pane.id}
      {@const focused = shown && (compact || workspace.focusedGroupId === group.id)}
      {@const style = compact ? `left: 0; top: ${tabBarHeight}px; width: 100%; height: calc(100% - ${tabBarHeight}px);` : stageStyle(rect)}
      <div class="slot" class:compact class:hidden={!shown} {style} data-stage={compact ? undefined : group.id} role="presentation" onpointerdown={() => focusGroup(group.id)}>
        <PaneView {pane} {shown} {focused} showKeyBar={isMobile || coarse} {coarse} {onsnippets} {oncredentials} {onedithost} {onaisettings} {ontailscalesettings} {onschedules} />
      </div>
    {/if}
  {/each}
  {#if !compact}
    {#if tabDrag.target && dropRect}
      <div class="drop" style={stageStyle(dropRect)}><div class="zone" style={dropZone(tabDrag.target)}></div></div>
    {/if}
    {#each layout.dividers as divider (divider.path.join('') || 'root')}
      <div
        class="divider {divider.dir}"
        class:coarse
        style={dividerStyle(divider)}
        role="separator"
        aria-orientation={divider.dir === 'row' ? 'vertical' : 'horizontal'}
        onpointerdown={(event) => startDrag(event, divider)}
      ></div>
    {/each}
  {/if}
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
  .group.compact {
    inset: 0;
    border-radius: 0;
    box-shadow: none;
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    /* The app's theme; only a terminal paints the terminal scheme (TermView). */
    background: var(--surface);
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
  .slot.compact {
    border-radius: 0;
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
  .divider.coarse {
    touch-action: none;
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
  .divider:hover::after,
  .divider.coarse:active::after {
    background: var(--accent);
  }
</style>
