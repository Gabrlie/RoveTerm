// Dragging a tab with the pointer: onto a tab bar to reorder it or move it to that group, or onto a
// group's content to move it there (centre) or into a new group beside it (edges), as in VSCode.
// Pointer events rather than HTML drag and drop, which the window's file-drop handling swallows on
// Windows.
import type { PaneId, SplitDir } from '../layout/layout';
import { workspace, type GroupId } from '../workspace/workspace.svelte';

export type DropTarget =
  | { kind: 'tabs'; groupId: GroupId; index: number }
  | { kind: 'group'; groupId: GroupId }
  | { kind: 'split'; groupId: GroupId; dir: SplitDir; before: boolean };

/** Pointer travel, in CSS pixels, before a press on a tab becomes a drag. */
const THRESHOLD = 5;
/** Share of a group's content, from each edge, that drops into a new group on that side. */
const EDGE = 0.25;

class TabDrag {
  /** The tab being dragged, while a drag is in progress. */
  paneId = $state<PaneId | null>(null);
  x = $state(0);
  y = $state(0);
  target = $state<DropTarget | null>(null);
}

export const tabDrag = new TabDrag();

/** Where a drop at (`x`, `y`) would go. Tab bars carry `data-tabbar`, tabs `data-tab-index`, and
 *  group contents `data-stage`, each with the group id. */
function hit(x: number, y: number): DropTarget | null {
  const element = document.elementFromPoint(x, y);
  if (!(element instanceof Element)) return null;
  const bar = element.closest<HTMLElement>('[data-tabbar]');
  if (bar?.dataset.tabbar) {
    const tabs = [...bar.querySelectorAll<HTMLElement>('[data-tab-index]')];
    let index = tabs.length;
    for (const [at, tab] of tabs.entries()) {
      const rect = tab.getBoundingClientRect();
      if (x < rect.left + rect.width / 2) {
        index = at;
        break;
      }
    }
    return { kind: 'tabs', groupId: bar.dataset.tabbar, index };
  }
  const stage = element.closest<HTMLElement>('[data-stage]');
  if (stage?.dataset.stage) {
    const groupId = stage.dataset.stage;
    const rect = stage.getBoundingClientRect();
    const fx = (x - rect.left) / rect.width;
    const fy = (y - rect.top) / rect.height;
    // The nearest edge within reach wins; otherwise the group itself.
    const edges: [number, SplitDir, boolean][] = [
      [fx, 'row', true],
      [1 - fx, 'row', false],
      [fy, 'column', true],
      [1 - fy, 'column', false],
    ];
    const [distance, dir, before] = edges.reduce((best, edge) => (edge[0] < best[0] ? edge : best));
    return distance < EDGE ? { kind: 'split', groupId, dir, before } : { kind: 'group', groupId };
  }
  return null;
}

function drop(paneId: PaneId, target: DropTarget): void {
  switch (target.kind) {
    case 'tabs':
      workspace.moveTab(paneId, target.groupId, target.index);
      break;
    case 'group':
      workspace.moveTab(paneId, target.groupId);
      break;
    case 'split':
      workspace.moveTabToNewGroup(paneId, target.dir, target.before, target.groupId);
      break;
  }
}

/** When the last drag ended; the click the browser sends right after it is not a click. */
let droppedAt = 0;

/** Whether a click now is the tail of a drag rather than a click on the tab. */
export function justDragged(): boolean {
  return performance.now() - droppedAt < 300;
}

/** Tracks a primary press and returns a cleanup for when its tab bar unmounts. */
export function pressTab(event: PointerEvent, paneId: PaneId): (() => void) | undefined {
  if (event.button !== 0 || !event.isPrimary || tabDrag.paneId !== null) return;
  const handle = event.currentTarget as HTMLElement;
  const { clientX: startX, clientY: startY, pointerId } = event;
  let dragging = false;
  const move = (moveEvent: PointerEvent): void => {
    if (moveEvent.pointerId !== pointerId) return;
    if (!dragging) {
      if (tabDrag.paneId !== null) return;
      if (Math.hypot(moveEvent.clientX - startX, moveEvent.clientY - startY) < THRESHOLD) return;
      handle.setPointerCapture(pointerId);
      dragging = true;
      tabDrag.paneId = paneId;
    }
    tabDrag.x = moveEvent.clientX;
    tabDrag.y = moveEvent.clientY;
    tabDrag.target = hit(moveEvent.clientX, moveEvent.clientY);
  };
  const cancel = (): void => {
    handle.removeEventListener('pointermove', move);
    handle.removeEventListener('pointerup', end);
    handle.removeEventListener('pointercancel', end);
    if (!dragging) return;
    dragging = false;
    tabDrag.paneId = null;
    tabDrag.target = null;
    droppedAt = performance.now();
  };
  const end = (endEvent: PointerEvent): void => {
    if (endEvent.pointerId !== pointerId) return;
    const target = dragging && endEvent.type === 'pointerup' ? tabDrag.target : null;
    cancel();
    if (target) drop(paneId, target);
  };
  handle.addEventListener('pointermove', move);
  handle.addEventListener('pointerup', end);
  handle.addEventListener('pointercancel', end);
  return cancel;
}
