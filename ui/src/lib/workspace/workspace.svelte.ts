// Tabs, their split layouts, and what each pane is connected to.
import { sessionClose, type ExitStatus, type HostId, type SessionId } from '../ipc';
import { terminals } from '../terminal/registry';
import { computeLayout, pane, paneIds, removePane, setRatio, splitPane, type LayoutNode, type PaneId, type Path, type SplitDir } from '../layout/layout';

/** What a pane opens. Splitting a pane opens the same target again. */
export type PaneTarget =
  | { kind: 'local' }
  | { kind: 'ssh'; hostId: HostId; title: string }
  | { kind: 'quick'; address: string };

export interface PaneState {
  id: PaneId;
  target: PaneTarget;
  /** One-shot credentials for the first open; cleared once the session starts. */
  password: string | null;
  remember: boolean;
  sessionId: SessionId | null;
  title: string;
  exited: ExitStatus | null;
}

export interface Tab {
  id: string;
  root: LayoutNode;
  focused: PaneId;
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
  }
}

class Workspace {
  tabs = $state<Tab[]>([]);
  panes = $state<Record<PaneId, PaneState>>({});
  activeTabId = $state<string | null>(null);

  get activeTab(): Tab | undefined {
    return this.tabs.find((tab) => tab.id === this.activeTabId);
  }

  get focusedPane(): PaneState | undefined {
    const tab = this.activeTab;
    return tab ? this.panes[tab.focused] : undefined;
  }

  private createPane(target: PaneTarget, password: string | null = null, remember = false): PaneId {
    const id = nextId('pane');
    this.panes[id] = { id, target, password, remember, sessionId: null, title: initialTitle(target), exited: null };
    return id;
  }

  /** Opens `target` in a new tab and activates it. */
  openTab(target: PaneTarget, password: string | null = null, remember = false): void {
    const paneId = this.createPane(target, password, remember);
    const tab: Tab = { id: nextId('tab'), root: pane(paneId), focused: paneId };
    this.tabs.push(tab);
    this.activeTabId = tab.id;
  }

  /** Splits the focused pane, opening the same target beside (`row`) or below (`column`) it. */
  splitFocused(dir: SplitDir): void {
    const tab = this.activeTab;
    const source = this.focusedPane;
    if (!tab || !source) return;
    const added = this.createPane(source.target);
    tab.root = splitPane(tab.root, source.id, dir, added);
    tab.focused = added;
  }

  /** Closes a pane and its session; closing a tab's last pane closes the tab. */
  closePane(paneId: PaneId): void {
    const tab = this.tabs.find((candidate) => paneIds(candidate.root).includes(paneId));
    if (!tab) return;
    const sessionId = this.panes[paneId]?.sessionId;
    if (sessionId) sessionClose(sessionId).catch((error: unknown) => console.warn('close failed', error));
    delete this.panes[paneId];
    terminals.delete(paneId);
    const root = removePane(tab.root, paneId);
    if (root === null) {
      this.removeTab(tab.id);
      return;
    }
    tab.root = root;
    if (tab.focused === paneId) tab.focused = paneIds(root)[0] ?? tab.focused;
  }

  closeFocused(): void {
    const focused = this.focusedPane;
    if (focused) this.closePane(focused.id);
  }

  /** Closes a tab and every session in it. */
  closeTab(tabId: string): void {
    const tab = this.tabs.find((candidate) => candidate.id === tabId);
    if (!tab) return;
    for (const id of paneIds(tab.root)) {
      const sessionId = this.panes[id]?.sessionId;
      if (sessionId) sessionClose(sessionId).catch((error: unknown) => console.warn('close failed', error));
      delete this.panes[id];
      terminals.delete(id);
    }
    this.removeTab(tabId);
  }

  private removeTab(tabId: string): void {
    const index = this.tabs.findIndex((tab) => tab.id === tabId);
    if (index < 0) return;
    this.tabs.splice(index, 1);
    if (this.activeTabId === tabId) {
      this.activeTabId = this.tabs[Math.min(index, this.tabs.length - 1)]?.id ?? null;
    }
  }

  activate(tabId: string): void {
    this.activeTabId = tabId;
  }

  /** Activates the tab `offset` positions away, wrapping around. */
  cycleTab(offset: number): void {
    if (this.tabs.length === 0) return;
    const index = this.tabs.findIndex((tab) => tab.id === this.activeTabId);
    const next = (index + offset + this.tabs.length) % this.tabs.length;
    this.activeTabId = this.tabs[next]?.id ?? null;
  }

  /** Activates the tab at zero-based `index`, if it exists. */
  gotoTab(index: number): void {
    const tab = this.tabs[index];
    if (tab) this.activeTabId = tab.id;
  }

  focus(paneId: PaneId): void {
    const tab = this.tabs.find((candidate) => paneIds(candidate.root).includes(paneId));
    if (tab) tab.focused = paneId;
  }

  /** Moves focus to the next (`1`) or previous (`-1`) pane of the active tab. */
  cyclePane(offset: number): void {
    const tab = this.activeTab;
    if (!tab) return;
    const ids = paneIds(tab.root);
    const index = ids.indexOf(tab.focused);
    tab.focused = ids[(index + offset + ids.length) % ids.length] ?? tab.focused;
  }

  resize(tabId: string, path: Path, ratio: number): void {
    const tab = this.tabs.find((candidate) => candidate.id === tabId);
    if (tab) tab.root = setRatio(tab.root, path, ratio);
  }

  /** Records the session a pane started and drops its one-shot credentials. */
  attached(paneId: PaneId, sessionId: SessionId): void {
    const state = this.panes[paneId];
    if (!state) return;
    state.sessionId = sessionId;
    state.password = null;
  }

  exited(paneId: PaneId, status: ExitStatus): void {
    const state = this.panes[paneId];
    if (state) state.exited = status;
  }

  retitle(paneId: PaneId, title: string): void {
    const state = this.panes[paneId];
    if (state && title.trim()) state.title = title.trim();
  }

  layoutOf(tab: Tab) {
    return computeLayout(tab.root);
  }
}

export const workspace = new Workspace();
