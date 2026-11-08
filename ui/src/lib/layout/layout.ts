// Split layout tree. Pure functions: every operation returns a new tree.

export type PaneId = string;
export type SplitDir = 'row' | 'column';

export type LayoutNode =
  | { kind: 'pane'; id: PaneId }
  | { kind: 'split'; dir: SplitDir; ratio: number; a: LayoutNode; b: LayoutNode };

/** Child selector along a path from the root: 'a' is left/top, 'b' is right/bottom. */
export type Path = ('a' | 'b')[];

/** A rectangle in fractions of the container (0..1). */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Divider {
  path: Path;
  dir: SplitDir;
  /** Rectangle of the split node the divider belongs to. */
  area: Rect;
  ratio: number;
}

export const MIN_RATIO = 0.1;
export const MAX_RATIO = 0.9;

export function pane(id: PaneId): LayoutNode {
  return { kind: 'pane', id };
}

/** Pane ids in reading order (left-to-right, top-to-bottom). */
export function paneIds(node: LayoutNode): PaneId[] {
  return node.kind === 'pane' ? [node.id] : [...paneIds(node.a), ...paneIds(node.b)];
}

/** Replaces pane `target` with a split holding it and `added`; `added` goes right or below. */
export function splitPane(node: LayoutNode, target: PaneId, dir: SplitDir, added: PaneId): LayoutNode {
  if (node.kind === 'pane') {
    return node.id === target ? { kind: 'split', dir, ratio: 0.5, a: node, b: pane(added) } : node;
  }
  return { ...node, a: splitPane(node.a, target, dir, added), b: splitPane(node.b, target, dir, added) };
}

/** Removes pane `target`; its sibling takes the parent's place. Returns null when the tree empties. */
export function removePane(node: LayoutNode, target: PaneId): LayoutNode | null {
  if (node.kind === 'pane') return node.id === target ? null : node;
  const a = removePane(node.a, target);
  const b = removePane(node.b, target);
  if (a === null) return b;
  if (b === null) return a;
  return { ...node, a, b };
}

/** Sets the ratio of the split at `path`, clamped to [MIN_RATIO, MAX_RATIO]. */
export function setRatio(node: LayoutNode, path: Path, ratio: number): LayoutNode {
  if (node.kind === 'pane') return node;
  const [head, ...rest] = path;
  if (head === undefined) {
    return { ...node, ratio: Math.min(MAX_RATIO, Math.max(MIN_RATIO, ratio)) };
  }
  return head === 'a' ? { ...node, a: setRatio(node.a, rest, ratio) } : { ...node, b: setRatio(node.b, rest, ratio) };
}

/** Computes each pane's rectangle and each split's divider. */
export function computeLayout(node: LayoutNode): { panes: Map<PaneId, Rect>; dividers: Divider[] } {
  const panes = new Map<PaneId, Rect>();
  const dividers: Divider[] = [];
  const walk = (current: LayoutNode, area: Rect, path: Path): void => {
    if (current.kind === 'pane') {
      panes.set(current.id, area);
      return;
    }
    const { dir, ratio } = current;
    const first: Rect =
      dir === 'row' ? { ...area, w: area.w * ratio } : { ...area, h: area.h * ratio };
    const second: Rect =
      dir === 'row'
        ? { ...area, x: area.x + area.w * ratio, w: area.w * (1 - ratio) }
        : { ...area, y: area.y + area.h * ratio, h: area.h * (1 - ratio) };
    dividers.push({ path, dir, area, ratio });
    walk(current.a, first, [...path, 'a']);
    walk(current.b, second, [...path, 'b']);
  };
  walk(node, { x: 0, y: 0, w: 1, h: 1 }, []);
  return { panes, dividers };
}
