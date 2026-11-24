// Editor groups, their tabs, and what each tab (pane) is connected to. The work area is a split
// tree of groups; each group has its own tabs and shows one of them, or the home page.
import { sessionClose, type CommandError, type ExitStatus, type HostId, type PasswordKind, type RemoteSource, type SessionId } from '../ipc';
import { hosts } from '../hosts/hosts.svelte';
import { reachKey } from '../hosts/reach';
import { terminals } from '../terminal/registry';
import { editors } from '../files/editors.svelte';
import { baseName } from '../files/format';
import { cycleFlatTarget, flatTabAt, flattenTabs, type FlatTab } from './flatTabs';
import { SNAPSHOT_VERSION, snapshotGroup, type Snapshot, type SnapshotNode } from './snapshot';
import { computeLayout, pane, paneIds, removePane, setRatio, splitPane, type LayoutNode, type PaneId, type Path, type SplitDir } from '../layout/layout';

/** What a pane opens. An editor target refers to an in-memory draft. */
export type PaneTarget =
  | { kind: 'local' }
  | { kind: 'ssh'; hostId: HostId; title: string }
  | { kind: 'quick'; address: string }
  | { kind: 'sftp'; source: RemoteSource; title: string }
  | { kind: 'editor'; draftId: string; title: string; path: string; source: RemoteSource }
  /** The fleet lead's conversation on all saved hosts. */
  | { kind: 'fleet' };

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
    case 'editor':
      return target.title;
    case 'fleet':
      return 'Fleet lead';
  }
}

/** The saved host a tab belongs to, or null for local terminals and ad-hoc connections. */
export function hostOf(target: PaneTarget): HostId | null {
  if (target.kind === 'ssh') return target.hostId;
  if ((target.kind === 'sftp' || target.kind === 'editor') && 'hostId' in target.source) return target.source.hostId;
  return null;
}


/** The SFTP target for the same remote as `target`, or null for local terminals. */
export function filesTargetFor(target: PaneTarget): PaneTarget | null {
  switch (target.kind) {
    case 'local':
    case 'fleet':
      return null;
    case 'ssh':
      return { kind: 'sftp', source: { hostId: target.hostId }, title: target.title };
    case 'quick':
      return { kind: 'sftp', source: { address: target.address }, title: target.address };
    case 'sftp':
      return target;
    case 'editor': {
      const source = target.source;
      const title = 'hostId' in source ? hosts.find(source.hostId)?.name ?? source.hostId : source.address;
      return { kind: 'sftp', source, title };
    }
  }
}

/** The remote behind a pane, or null for local terminals. Panels follow the focused pane's remote. */
export function remoteSourceOf(target: PaneTarget): RemoteSource | null {
  switch (target.kind) {
    case 'local':
    case 'fleet':
      return null;
    case 'ssh':
      return { hostId: target.hostId };
    case 'quick':
      return { address: target.address };
    case 'sftp':
    case 'editor':
      return target.source;
  }
}

/** Stable key of a remote, to remount views when it changes: for a saved host, also how it is
 *  reached, so editing its address, user, port or route starts its views over. */
export function sourceKey(source: RemoteSource): string {
  return 'hostId' in source ? `host:${reachKey(source.hostId, (id) => hosts.find(id))}` : `quick:${source.address}`;
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

  get flatTabs(): FlatTab[] {
    return flattenTabs(this.groupIds, this.groups);
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

  /** Shows the fleet lead's tab, opening it in the focused group when none is open. */
  openFleet(): void {
    const existing = Object.values(this.panes).find((pane) => pane.target.kind === 'fleet');
    const group = existing ? this.groupOf(existing.id) : undefined;
    if (existing && group) {
      this.activate(group.id, existing.id);
      this.focusGroup(group.id);
      return;
    }
    this.openTab({ kind: 'fleet' });
  }

  /** Shows the draft in exactly one workspace tab. */
  openEditor(draft: { id: string; path: string; source: RemoteSource }): void {
    const existing = Object.values(this.panes).find(({ target }) => target.kind === 'editor' && target.draftId === draft.id);
    if (existing?.target.kind === 'editor') {
      existing.target.source = draft.source;
      this.reveal(existing.id);
      return;
    }
    this.openTab({ kind: 'editor', draftId: draft.id, title: baseName(draft.path) || draft.path, path: draft.path, source: draft.source });
  }

  /** Opens `target` as a new tab of the focused group and shows it. */
  openTab(target: PaneTarget, password: string | null = null, remember = false): void {
    const group = this.focusedGroup;
    if (!group) return;
    this.insertTab(group, this.createPane(target, password, remember));
  }

  /**
   * Splits the focused group, adding an empty group (it shows home) beside (`row`) or below
   * (`column`) it. Opening the same host again is the tab menu's split ({@link splitTab}).
   */
  splitFocused(dir: SplitDir): void {
    const group = this.focusedGroup;
    if (!group) return;
    const added = this.createGroup([]);
    this.root = splitPane(this.root, group.id, dir, added.id);
    this.focusedGroupId = added.id;
  }

  /** Opens tab `paneId`'s target again in a new group beside (`row`) or below (`column`) its group. */
  splitTab(paneId: PaneId, dir: SplitDir): void {
    const group = this.groupOf(paneId);
    const source = this.panes[paneId];
    if (!group || !source || source.target.kind === 'editor') return;
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

  /** Closes the other tabs in the group or those to the right after resolving unsaved drafts. */
  async requestCloseTabsBeside(paneId: PaneId, which: 'others' | 'right'): Promise<void> {
    const group = this.groupOf(paneId);
    if (!group) return;
    const index = group.tabs.indexOf(paneId);
    const closing = group.tabs.filter((id, at) => id !== paneId && (which === 'others' || at > index));
    if (!await this.closeDraftsFor(closing)) return;
    for (const id of closing) this.closePane(id);
    if (group.tabs.includes(paneId)) group.active = paneId;
  }

  /**
   * Opens a file browser for the focused tab's remote. By default it splits to the right.
   * Returns false if there is no remote target.
   */
  openFilesBesideFocused(split = true): boolean {
    const group = this.focusedGroup;
    const source = this.focusedPane;
    if (!group || !source) return false;
    const target = filesTargetFor(source.target);
    if (!target) return false;
    if (!split) {
      this.openTab(target);
      return true;
    }
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

  /** Tabs of saved host `hostId`. */
  tabsOfHost(hostId: HostId): PaneId[] {
    return Object.values(this.panes)
      .filter((pane) => hostOf(pane.target) === hostId)
      .map((pane) => pane.id);
  }

  /** Closes a tab after any associated draft has been resolved. */
  async requestClosePane(paneId: PaneId): Promise<void> {
    const target = this.panes[paneId]?.target;
    if (target?.kind === 'editor' && !await editors.close(target.draftId)) return;
    this.closePane(paneId);
  }

  private closeDraftsFor(paneIds: PaneId[]): Promise<boolean> {
    const draftIds = paneIds.flatMap((id) => {
      const target = this.panes[id]?.target;
      return target?.kind === 'editor' ? [target.draftId] : [];
    });
    return draftIds.length ? editors.closeMany(draftIds) : Promise.resolve(true);
  }

  /** Removes a tab and its session, prompting first if the tab still owns a draft. */
  closePane(paneId: PaneId): void {
    const target = this.panes[paneId]?.target;
    if (target?.kind === 'editor' && editors.find(target.draftId)) {
      void this.requestClosePane(paneId);
      return;
    }
    const group = this.groupOf(paneId);
    if (!group) return;
    this.dropPane(paneId);
    const index = group.tabs.indexOf(paneId);
    group.tabs.splice(index, 1);
    if (group.active === paneId) group.active = group.tabs[Math.min(index, group.tabs.length - 1)] ?? null;
    if (group.tabs.length === 0) this.closeGroup(group.id);
  }

  /** Closes all tabs in the group after resolving unsaved drafts, unless it is the last group. */
  async requestCloseGroupWithTabs(groupId: GroupId): Promise<void> {
    const group = this.groups[groupId];
    if (!group || this.groupIds.length <= 1) return;
    const closing = [...group.tabs];
    if (!await this.closeDraftsFor(closing)) return;
    for (const paneId of closing) this.closePane(paneId);
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
    if (group.active) void this.requestClosePane(group.active);
    else this.closeGroup(group.id);
  }

  /** Shows tab `paneId` (or the home page with `null`) in `groupId` and focuses the group. */
  activate(groupId: GroupId, paneId: PaneId | null): void {
    const group = this.groups[groupId];
    if (!group) return;
    group.active = paneId !== null && group.tabs.includes(paneId) ? paneId : null;
    this.focusedGroupId = groupId;
  }

  activateHome(): void {
    const group = this.focusedGroup;
    if (group) group.active = null;
  }

  focusGroup(groupId: GroupId): void {
    if (this.groups[groupId]) this.focusedGroupId = groupId;
  }

  focus(paneId: PaneId): void {
    const group = this.groupOf(paneId);
    if (group) this.focusedGroupId = group.id;
  }

  /** Shows tab `paneId` in its group and focuses it. */
  reveal(paneId: PaneId): void {
    const group = this.groupOf(paneId);
    if (group) this.activate(group.id, paneId);
  }

  /** Lets a restored tab connect; called when it is first shown. */
  undefer(paneId: PaneId): void {
    const state = this.panes[paneId];
    if (state?.deferred) state.deferred = false;
  }

  /** Cycles through Home and the tabs in the focused group, or all groups in flat mode. */
  cycleTab(offset: number, flat = false): void {
    if (flat) {
      const target = cycleFlatTarget(this.flatTabs, this.focusedPane?.id ?? null, offset);
      if (target) this.activate(target.groupId, target.paneId);
      else this.activateHome();
      return;
    }
    const group = this.focusedGroup;
    if (!group) return;
    const ids: (PaneId | null)[] = [null, ...group.tabs];
    const index = ids.indexOf(group.active);
    group.active = ids[(index + offset + ids.length) % ids.length] ?? null;
  }

  /** Shows the tab at zero-based `index` in the focused group, or across groups in flat mode. */
  gotoTab(index: number, flat = false): void {
    if (flat) {
      const hit = flatTabAt(this.flatTabs, index);
      if (hit) this.activate(hit.groupId, hit.paneId);
      return;
    }
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
        const targets = group.tabs.map((id) => this.panes[id]?.target);
        const activeIndex = group.active ? group.tabs.indexOf(group.active) : -1;
        return { group: snapshotGroup(targets, activeIndex) };
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
