// Workspace snapshots for session restore: the editor groups, their split layout and the targets
// of their tabs. Tabs refer to saved hosts by id and never carry credentials. Stored by the
// backend as an opaque `ui/workspace` journal record; parsed defensively since an older build
// may have written it.
import type { SplitDir } from '../layout/layout';
import type { PaneTarget } from './workspace.svelte';

export const SNAPSHOT_VERSION = 2;

export interface SnapshotGroup {
  tabs: PaneTarget[];
  /** Index of the shown tab, or -1 for the home page. */
  active: number;
}

export type SnapshotNode =
  | { group: SnapshotGroup }
  | { dir: SplitDir; ratio: number; a: SnapshotNode; b: SnapshotNode };

export interface Snapshot {
  version: number;
  root: SnapshotNode;
  /** Index of the focused group in reading order. */
  focused: number;
}

/** Most tabs restored in total; a damaged record cannot open hundreds of connections. */
const MAX_TABS = 64;

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function parseTarget(value: unknown): PaneTarget | null {
  if (!isObject(value)) return null;
  const text = (key: string) => (typeof value[key] === 'string' ? (value[key] as string) : null);
  switch (value.kind) {
    case 'local':
      return { kind: 'local' };
    case 'ssh': {
      const hostId = text('hostId');
      return hostId ? { kind: 'ssh', hostId, title: text('title') ?? '' } : null;
    }
    case 'quick': {
      const address = text('address');
      return address ? { kind: 'quick', address } : null;
    }
    case 'sftp': {
      const source = value.source;
      if (!isObject(source)) return null;
      const title = text('title') ?? '';
      if (typeof source.hostId === 'string') return { kind: 'sftp', source: { hostId: source.hostId }, title };
      if (typeof source.address === 'string') return { kind: 'sftp', source: { address: source.address }, title };
      return null;
    }
    default:
      return null;
  }
}

/** Parses a split tree whose leaves `leaf` turns into nodes; drops malformed parts. */
function parseTree(value: unknown, depth: number, leaf: (value: Record<string, unknown>) => SnapshotNode | null): SnapshotNode | null {
  if (!isObject(value) || depth > 16) return null;
  const parsed = leaf(value);
  if (parsed) return parsed;
  const dir = value.dir === 'row' || value.dir === 'column' ? value.dir : null;
  const ratio = typeof value.ratio === 'number' && value.ratio > 0 && value.ratio < 1 ? value.ratio : 0.5;
  const a = parseTree(value.a, depth + 1, leaf);
  const b = parseTree(value.b, depth + 1, leaf);
  if (!dir || !a || !b) return a ?? b;
  return { dir, ratio, a, b };
}

/** Parses a version 2 snapshot: a split tree of groups. */
function parseV2(value: Record<string, unknown>, budget: { tabs: number }): Snapshot | null {
  const root = parseTree(value.root, 0, (node) => {
    if (!isObject(node.group)) return null;
    const raw = Array.isArray(node.group.tabs) ? node.group.tabs : [];
    const tabs: PaneTarget[] = [];
    for (const item of raw) {
      const target = parseTarget(item);
      if (target && budget.tabs > 0) {
        tabs.push(target);
        budget.tabs -= 1;
      }
    }
    const active = typeof node.group.active === 'number' && node.group.active < tabs.length ? node.group.active : -1;
    return { group: { tabs, active } };
  });
  if (!root) return null;
  return { version: SNAPSHOT_VERSION, root, focused: typeof value.focused === 'number' ? value.focused : 0 };
}

/**
 * Converts a version 1 snapshot (tabs, each with its own split of panes) into one group whose tabs
 * are every pane in reading order; the old active tab's focused pane is shown.
 */
function parseV1(value: Record<string, unknown>, budget: { tabs: number }): Snapshot | null {
  if (!Array.isArray(value.tabs)) return null;
  const tabs: PaneTarget[] = [];
  let active = -1;
  value.tabs.forEach((tab, index) => {
    if (!isObject(tab)) return;
    const targets: PaneTarget[] = [];
    const collect = (node: unknown, depth: number): void => {
      if (!isObject(node) || depth > 16) return;
      if ('pane' in node) {
        const target = parseTarget(node.pane);
        if (target) targets.push(target);
        return;
      }
      collect(node.a, depth + 1);
      collect(node.b, depth + 1);
    };
    collect(tab.root, 0);
    if (index === value.active && targets.length > 0) {
      const focused = typeof tab.focused === 'number' ? Math.min(Math.max(tab.focused, 0), targets.length - 1) : 0;
      active = tabs.length + focused;
    }
    for (const target of targets) {
      if (budget.tabs > 0) {
        tabs.push(target);
        budget.tabs -= 1;
      }
    }
  });
  return { version: SNAPSHOT_VERSION, root: { group: { tabs, active: active < tabs.length ? active : -1 } }, focused: 0 };
}

/** Parses a stored snapshot of either version, dropping anything malformed. Null when unusable. */
export function parseSnapshot(value: unknown): Snapshot | null {
  if (!isObject(value)) return null;
  const budget = { tabs: MAX_TABS };
  if (value.version === SNAPSHOT_VERSION) return parseV2(value, budget);
  if (value.version === 1) return parseV1(value, budget);
  return null;
}
