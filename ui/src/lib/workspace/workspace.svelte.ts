// Editor groups, their tabs, and what each tab (pane) is connected to. The work area is a split
// tree of groups; each group has its own tabs and shows one of them, or the home page.
import { sessionClose, type CommandError, type ExitStatus, type HostId, type PasswordKind, type RemoteSource, type SessionId } from '../ipc';
import { terminals } from '../terminal/registry';
import { SNAPSHOT_VERSION, type Snapshot, type SnapshotNode } from './snapshot';
import { computeLayout, pane, paneIds, removePane, setRatio, splitPane, type LayoutNode, type PaneId, type Path, type SplitDir } from '../layout/layout';

/** What a pane opens. Splitting a group opens its current tab's target again. */
export type PaneTarget =
  | { kind: 'local' }
  | { kind: 'ssh'; hostId: HostId; title: string }
  | { kind: 'quick'; address: string }
  | { kind: 'sftp'; source: RemoteSource; title: string };

export interface PaneState {
  id: PaneId;
  target: PaneTarget;
  /** One-shot credentials for the first open; cleared once the session starts. */
  password: string | null;
  /** How `password` is used; see `PasswordKind`. */
  passwordKind: PasswordKind;
  remember: boolean;
  sessionId: SessionId | null;
  /** Why opening failed, until the pane is reopened. */
  failure: CommandError | null;
  /** Restored from a snapshot and not yet shown: connects when it first becomes its group's tab. */
  deferred: boolean;
  title: string;
  exited: ExitStatus | null;
  /** When the session started (Unix milliseconds), for the status bar. */
  connectedAt: number | null;
}

export type GroupId = string;

/** One editor group: its tabs (panes) in order and the one shown, or `null` for the home page. */
export interface Group {
  id: GroupId;
  tabs: PaneId[];
  active: PaneId | null;
}

function isError(value: unknown): value is CommandError {
  return typeof value === 'object' && value !== null && 'code' in value && 'message' in value;
}

let counter = 0;
const nextId = (prefix: string): string => `${prefix}-${Date.now().toString(36)}-${(counter++).toString(36)}`;

function initialTitle(target: PaneTarget): string {
  switch (target.kind) {
    case 'local':
      return 'Terminal';
    case 'ssh':
      return target.title;
    case 'quick':
      return target.address;
    case 'sftp':
      return target.title;
  }
}

/** The saved host a tab belongs to, or null for local terminals and ad-hoc connections. */
export function hostOf(target: PaneTarget): HostId | null {
  if (target.kind === 'ssh') return target.hostId;
  if (target.kind === 'sftp' && 'hostId' in target.source) return target.source.hostId;
  return null;
}

/** The SFTP target for the same remote as `target`, or null for local terminals. */
export function filesTargetFor(target: PaneTarget): PaneTarget | null {
  switch (target.kind) {
    case 'local':
      return null;
    case 'ssh':
      return { kind: 'sftp', source: { hostId: target.hostId }, title: target.title };
    case 'quick':
      return { kind: 'sftp', source: { address: target.address }, title: target.address };
    case 'sftp':
      return target;
  }
}

/** The remote behind a pane, or null for local terminals. Panels follow the focused pane's remote. */
export function remoteSourceOf(target: PaneTarget): RemoteSource | null {
  switch (target.kind) {
    case 'local':
      return null;
    case 'ssh':
      return { hostId: target.hostId };
    case 'quick':
      return { address: target.address };
    case 'sftp':
      return target.source;
  }
}

/** Stable key of a remote, to remount views when it changes. */
export function sourceKey(source: RemoteSource): string {
  return 'hostId' in source ? `host:${source.hostId}` : `quick:${source.address}`;
}

class Workspace {
  /** Split tree whose leaves are group ids. */
  root = $state<LayoutNode>(pane('group-0'));
  groups = $state<Record<GroupId, Group>>({ 'group-0': { id: 'group-0', tabs: [], active: null } });
  panes = $state<Record<PaneId, PaneState>>({});
  focusedGroupId = $state<GroupId>('group-0');

  /** Group ids in reading order. */
  get groupIds(): GroupId[] {
    return paneIds(this.root);
  }

  get focusedGroup(): Group | undefined {
    return this.groups[this.focusedGroupId];
  }

  /** The tab shown in the focused group; panels, copy and paste follow it. */
  get focusedPane(): PaneState | undefined {
    const active = this.focusedGroup?.active;
    return active ? this.panes[active] : undefined;
  }

  groupOf(paneId: PaneId): Group | undefined {
    return Object.values(this.groups).find((group) => group.tabs.includes(paneId));
  }

  private createPane(target: PaneTarget, password: string | null = null, remember = false, passwordKind: PasswordKind = 'natural'): PaneId {
    const id = nextId('pane');
    this.panes[id] = { id, target, password, passwordKind, remember, sessionId: null, failure: null, deferred: false, title: initialTitle(target), exited: null, connectedAt: null };
    return id;
  }

  private createGroup(tabs: PaneId[] = []): Group {
    const group: Group = { id: nextId('group'), tabs, active: tabs[0] ?? null };
    this.groups[group.id] = group;
    return this.groups[group.id] as Group;
  }

  /** Adds `paneId` to `group` after its current tab and shows it. */
  private insertTab(group: Group, paneId: PaneId): void {
    const index = group.active ? group.tabs.indexOf(group.active) + 1 : group.tabs.length;
    group.tabs.splice(index, 0, paneId);
    group.active = paneId;
  }

  /** Opens `target` as a new tab of the focused group and shows it. */
  openTab(target: PaneTarget, password: string | null = null, remember = false): void {
    const group = this.focusedGroup;
    if (!group) return;
    this.insertTab(group, this.createPane(target, password, remember));
  }

  /**
   * Splits the focused group, opening its current tab's target again in a new group beside
   * (`row`) or below (`column`) it; a group showing home splits into another empty one.
   */
  splitFocused(dir: SplitDir): void {
    const group = this.focusedGroup;
    if (!group) return;
    const source = this.focusedPane;
    const added = this.createGroup(source ? [this.createPane(source.target)] : []);
    this.root = splitPane(this.root, group.id, dir, added.id);
    this.focusedGroupId = added.id;
  }

  /** Opens tab `paneId`'s target again in a new group beside (`row`) or below (`column`) its group. */
  splitTab(paneId: PaneId, dir: SplitDir): void {
    const group = this.groupOf(paneId);
    const source = this.panes[paneId];
    if (!group || !source) return;
    const added = this.createGroup([this.createPane(source.target)]);
    this.root = splitPane(this.root, group.id, dir, added.id);
    this.focusedGroupId = added.id;
  }

  /**
   * Moves tab `paneId`, keeping its view and session, into group `groupId` at `index` (the end
   * when omitted) and shows it there. A group it leaves empty is removed.
   */
  moveTab(paneId: PaneId, groupId: GroupId, index?: number): void {
    const from = this.groupOf(paneId);
    const to = this.groups[groupId];
    if (!from || !to) return;
    const position = from.tabs.indexOf(paneId);
    from.tabs.splice(position, 1);
    if (from.active === paneId) from.active = from.tabs[Math.min(position, from.tabs.length - 1)] ?? null;
    // Within a group, positions after the old one shift left by one.
    let at = index ?? to.tabs.length;
    if (from === to && index !== undefined && index > position) at -= 1;
    to.tabs.splice(Math.max(0, Math.min(at, to.tabs.length)), 0, paneId);
    to.active = paneId;
    this.focusedGroupId = to.id;
    if (from !== to && from.tabs.length === 0) this.closeGroup(from.id);
  }

  /**
   * Moves tab `paneId` into a new group placed beside group `groupId` (its own group by default):
   * right or below, or left or above with `before`.
   */
  moveTabToNewGroup(paneId: PaneId, dir: SplitDir, before = false, groupId?: GroupId): void {
    const from = this.groupOf(paneId);
    const beside = groupId ?? from?.id;
    if (!from || !beside || !this.groups[beside]) return;
    // Moving a group's only tab beside itself would leave it where it is.
    if (beside === from.id && from.tabs.length === 1) return;
    const added = this.createGroup();
    this.root = splitPane(this.root, beside, dir, added.id, before);
    this.moveTab(paneId, added.id);
  }

  /** Moves tab `paneId` one place left (`-1`) or right (`1`) within its group. */
  shiftTab(paneId: PaneId, offset: -1 | 1): void {
    const group = this.groupOf(paneId);
    if (!group) return;
    const index = group.tabs.indexOf(paneId);
    const target = index + offset;
    if (target < 0 || target >= group.tabs.length) return;
    group.tabs.splice(index, 1);
    group.tabs.splice(target, 0, paneId);
  }

  /** Closes the other tabs of `paneId`'s group (`'others'`) or those to its right (`'right'`). */
  closeTabsBeside(paneId: PaneId, which: 'others' | 'right'): void {
    const group = this.groupOf(paneId);
    if (!group) return;
    const index = group.tabs.indexOf(paneId);
    const closing = group.tabs.filter((id, at) => id !== paneId && (which === 'others' || at > index));
    for (const id of closing) this.closePane(id);
    group.active = paneId;
  }

  /**
   * Opens a file browser for the focused tab's remote in a new group to its right. Returns false
   * when the focused tab is not remote.
   */
  openFilesBesideFocused(): boolean {
    const group = this.focusedGroup;
    const source = this.focusedPane;
    if (!group || !source) return false;
    const target = filesTargetFor(source.target);
    if (!target) return false;
    const added = this.createGroup([this.createPane(target)]);
    this.root = splitPane(this.root, group.id, 'row', added.id);
    this.focusedGroupId = added.id;
    return true;
  }

  private dropPane(paneId: PaneId): void {
    const sessionId = this.panes[paneId]?.sessionId;
    if (sessionId) sessionClose(sessionId).catch((error: unknown) => console.warn('close failed', error));
    delete this.panes[paneId];
    terminals.delete(paneId);
  }

  /**
   * Closes a tab and its session. The group then shows its neighbouring tab; a group left without
   * tabs is removed when other groups remain, and otherwise shows the home page.
   */
  /** Tabs (terminals and files) of saved host `hostId`. */
  tabsOfHost(hostId: HostId): PaneId[] {
    return Object.values(this.panes)
      .filter((pane) => hostOf(pane.target) === hostId)
      .map((pane) => pane.id);
  }

  closePane(paneId: PaneId): void {
    const group = this.groupOf(paneId);
    if (!group) return;
    this.dropPane(paneId);
    const index = group.tabs.indexOf(paneId);
    group.tabs.splice(index, 1);
    if (group.active === paneId) group.active = group.tabs[Math.min(index, group.tabs.length - 1)] ?? null;
    if (group.tabs.length === 0) this.closeGroup(group.id);
  }

  /** Closes group `groupId` with all its tabs and sessions, unless it is the last group. */
  closeGroupWithTabs(groupId: GroupId): void {
    const group = this.groups[groupId];
    if (!group || this.groupIds.length <= 1) return;
    for (const paneId of group.tabs) this.dropPane(paneId);
    group.tabs = [];
    group.active = null;
    this.closeGroup(groupId);
  }

  /** Removes an empty group, unless it is the last one. */
  private closeGroup(groupId: GroupId): void {
    const group = this.groups[groupId];
    if (!group || group.tabs.length > 0 || this.groupIds.length <= 1) return;
    const root = removePane(this.root, groupId);
    if (!root) return;
    const ids = this.groupIds;
    const index = ids.indexOf(groupId);
    this.root = root;
    delete this.groups[groupId];
    if (this.focusedGroupId === groupId) {
      const remaining = paneIds(root);
      this.focusedGroupId = remaining[Math.max(0, Math.min(index - 1, remaining.length - 1))] ?? remaining[0] ?? groupId;
    }
  }

  /** Closes the focused group's tab; on a group showing home, closes the group if others remain. */
  closeFocused(): void {
    const group = this.focusedGroup;
    if (!group) return;
    if (group.active) this.closePane(group.active);
    else this.closeGroup(group.id);
  }

  /** Shows tab `paneId` (or the home page with `null`) in `groupId` and focuses the group. */
  activate(groupId: GroupId, paneId: PaneId | null): void {
    const group = this.groups[groupId];
    if (!group) return;
    group.active = paneId !== null && group.tabs.includes(paneId) ? paneId : null;
    this.focusedGroupId = groupId;
  }

  focusGroup(groupId: GroupId): void {
    if (this.groups[groupId]) this.focusedGroupId = groupId;
  }

  focus(paneId: PaneId): void {
    const group = this.groupOf(paneId);
    if (group) this.focusedGroupId = group.id;
  }

  /** Lets a restored tab connect; called when it is first shown. */
  undefer(paneId: PaneId): void {
    const state = this.panes[paneId];
    if (state?.deferred) state.deferred = false;
  }

  /** Shows the tab `offset` positions away in the focused group, wrapping; home comes first. */
  cycleTab(offset: number): void {
    const group = this.focusedGroup;
    if (!group) return;
    const ids: (PaneId | null)[] = [null, ...group.tabs];
    const index = ids.indexOf(group.active);
    group.active = ids[(index + offset + ids.length) % ids.length] ?? null;
  }

  /** Shows the focused group's tab at zero-based `index`, if it exists. */
  gotoTab(index: number): void {
    const group = this.focusedGroup;
    const paneId = group?.tabs[index];
    if (group && paneId) group.active = paneId;
  }

  /** Moves focus to the next (`1`) or previous (`-1`) group. */
  cyclePane(offset: number): void {
    const ids = this.groupIds;
    const index = ids.indexOf(this.focusedGroupId);
    this.focusedGroupId = ids[(index + offset + ids.length) % ids.length] ?? this.focusedGroupId;
  }

  /**
   * Replaces a pane that failed to open with a fresh one carrying one-shot credentials, in the same
   * tab position. The new pane mounts a new view, which opens the target again.
   */
  reopen(paneId: PaneId, password: string | null, remember: boolean, passwordKind: PasswordKind = 'natural'): void {
    const group = this.groupOf(paneId);
    const old = this.panes[paneId];
    if (!group || !old) return;
    const added = this.createPane(old.target, password, remember, passwordKind);
    group.tabs[group.tabs.indexOf(paneId)] = added;
    if (group.active === paneId) group.active = added;
    delete this.panes[paneId];
    terminals.delete(paneId);
  }

  resize(path: Path, ratio: number): void {
    this.root = setRatio(this.root, path, ratio);
  }

  /** Records the session a pane started and drops its one-shot credentials. */
  attached(paneId: PaneId, sessionId: SessionId): void {
    const state = this.panes[paneId];
    if (!state) return;
    state.sessionId = sessionId;
    state.password = null;
    state.connectedAt = Date.now();
  }

  /** Drops one-shot credentials once a file pane has opened its session. */
  opened(paneId: PaneId): void {
    const state = this.panes[paneId];
    if (state) state.password = null;
  }

  /** Records why a pane could not open. */
  failed(paneId: PaneId, error: unknown): void {
    const state = this.panes[paneId];
    if (!state) return;
    state.password = null;
    state.failure = isError(error) ? error : { code: 'unknown', message: String(error), detail: null };
  }

  exited(paneId: PaneId, status: ExitStatus): void {
    const state = this.panes[paneId];
    if (state) state.exited = status;
  }

  retitle(paneId: PaneId, title: string): void {
    const state = this.panes[paneId];
    if (state && title.trim()) state.title = title.trim();
  }

  layout() {
    return computeLayout(this.root);
  }

  /** The groups and their tabs as a snapshot for session restore. */
  snapshot(): Snapshot {
    const node = (current: LayoutNode): SnapshotNode | null => {
      if (current.kind === 'pane') {
        const group = this.groups[current.id];
        if (!group) return null;
        const tabs = group.tabs.flatMap((id) => {
          const state = this.panes[id];
          return state ? [state.target] : [];
        });
        const active = group.active ? group.tabs.filter((id) => this.panes[id]).indexOf(group.active) : -1;
        return { group: { tabs, active } };
      }
      const a = node(current.a);
      const b = node(current.b);
      if (!a || !b) return a ?? b;
      return { dir: current.dir, ratio: current.ratio, a, b };
    };
    return {
      version: SNAPSHOT_VERSION,
      root: node(this.root) ?? { group: { tabs: [], active: -1 } },
      focused: Math.max(0, this.groupIds.indexOf(this.focusedGroupId)),
    };
  }

  /**
   * Replaces the work area with the groups of `snapshot` when nothing is open yet. Tabs start
   * deferred and connect when first shown, so restoring never opens many connections or password
   * prompts at once; each group's current tab is shown right away.
   */
  restore(snapshot: Snapshot): void {
    if (Object.keys(this.panes).length > 0) return;
    const groups: Record<GroupId, Group> = {};
    const build = (node: SnapshotNode): LayoutNode => {
      if ('group' in node) {
        const tabs = node.group.tabs.map((target) => {
          const id = this.createPane(target);
          const state = this.panes[id];
          if (state) state.deferred = true;
          return id;
        });
        const group: Group = { id: nextId('group'), tabs, active: tabs[node.group.active] ?? null };
        groups[group.id] = group;
        return pane(group.id);
      }
      return { kind: 'split', dir: node.dir, ratio: node.ratio, a: build(node.a), b: build(node.b) };
    };
    const root = build(snapshot.root);
    this.groups = groups;
    this.root = root;
    const ids = paneIds(root);
    this.focusedGroupId = ids[snapshot.focused] ?? ids[0] ?? this.focusedGroupId;
  }
}

export const workspace = new Workspace();
