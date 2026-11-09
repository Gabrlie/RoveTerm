// Workspace snapshots for session restore: the open tabs, their split layouts and what each pane
// shows. Panes refer to saved hosts by id and never carry credentials. Stored by the backend as
// an opaque `ui/workspace` journal record; parsed defensively since an older build may have
// written it.
import type { LayoutNode, SplitDir } from '../layout/layout';
import type { PaneTarget } from './workspace.svelte';

export const SNAPSHOT_VERSION = 1;

export type SnapshotNode =
  | { pane: PaneTarget }
  | { dir: SplitDir; ratio: number; a: SnapshotNode; b: SnapshotNode };

export interface SnapshotTab {
  root: SnapshotNode;
  /** Index of the focused pane in reading order. */
  focused: number;
}

export interface Snapshot {
  version: number;
  tabs: SnapshotTab[];
  /** Index of the active tab, or -1 for the home tab. */
  active: number;
}

/** Converts a layout tree, replacing pane ids with their targets. */
export function snapshotNode(node: LayoutNode, targetOf: (id: string) => PaneTarget | undefined): SnapshotNode | null {
  if (node.kind === 'pane') {
    const target = targetOf(node.id);
    return target ? { pane: target } : null;
  }
  const a = snapshotNode(node.a, targetOf);
  const b = snapshotNode(node.b, targetOf);
  if (!a || !b) return a ?? b;
  return { dir: node.dir, ratio: node.ratio, a, b };
}

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

function parseNode(value: unknown, depth: number): SnapshotNode | null {
  if (!isObject(value) || depth > 16) return null;
  if ('pane' in value) {
    const target = parseTarget(value.pane);
    return target ? { pane: target } : null;
  }
  const dir = value.dir === 'row' || value.dir === 'column' ? value.dir : null;
  const ratio = typeof value.ratio === 'number' && value.ratio > 0 && value.ratio < 1 ? value.ratio : 0.5;
  const a = parseNode(value.a, depth + 1);
  const b = parseNode(value.b, depth + 1);
  if (!dir || !a || !b) return a ?? b;
  return { dir, ratio, a, b };
}

/** Parses a stored snapshot, dropping anything malformed. Returns null when nothing is usable. */
export function parseSnapshot(value: unknown): Snapshot | null {
  if (!isObject(value) || value.version !== SNAPSHOT_VERSION || !Array.isArray(value.tabs)) return null;
  const tabs: SnapshotTab[] = [];
  for (const tab of value.tabs.slice(0, 64)) {
    if (!isObject(tab)) continue;
    const root = parseNode(tab.root, 0);
    if (root) tabs.push({ root, focused: typeof tab.focused === 'number' ? tab.focused : 0 });
  }
  const active = typeof value.active === 'number' && value.active < tabs.length ? value.active : -1;
  return { version: SNAPSHOT_VERSION, tabs, active };
}
