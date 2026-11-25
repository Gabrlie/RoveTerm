import type { RemoteSource } from '../ipc';
import type { PaneId } from '../layout/layout';
import type { PaneState, PaneTarget } from './workspace.svelte';

export function isTerminalTarget(target: PaneTarget): boolean {
  return target.kind === 'ssh' || target.kind === 'quick' || target.kind === 'local';
}

export function sameTerminalTarget(a: PaneTarget, b: PaneTarget): boolean {
  if (a.kind === 'ssh') return b.kind === 'ssh' && a.hostId === b.hostId;
  if (a.kind === 'quick') return b.kind === 'quick' && a.address === b.address;
  return a.kind === 'local' && b.kind === 'local';
}

export function findReusableTerminalPane(panes: readonly PaneState[], nextTarget: PaneTarget): PaneState | undefined {
  return panes.find((pane) => !pane.failure && !pane.exited && sameTerminalTarget(pane.target, nextTarget));
}

function isTerminalRemote(source: RemoteSource, target: PaneTarget): boolean {
  if (target.kind === 'ssh') return 'hostId' in source && source.hostId === target.hostId;
  return target.kind === 'quick' && 'address' in source && source.address === target.address;
}

export function panesToCloseForSingleTerminal(panes: readonly PaneState[], nextTarget: PaneTarget): PaneId[] {
  return panes.filter(({ target }) => {
    if (target.kind === 'sftp' || target.kind === 'editor') return !isTerminalRemote(target.source, nextTarget);
    return isTerminalTarget(target);
  }).map(({ id }) => id);
}

/** Panes are supplied in creation order. Prefer the newest pane in each connection state. */
export function findBackgroundTerminal(panes: readonly PaneState[], focusedPaneId: PaneId | null): PaneState | undefined {
  const terminals = panes.filter(({ id, target }) => id !== focusedPaneId && isTerminalTarget(target)).reverse();
  return terminals.find((pane) => pane.sessionId && !pane.exited && !pane.failure)
    ?? terminals.find((pane) => !pane.sessionId && !pane.deferred && !pane.exited && !pane.failure)
    ?? terminals[0];
}
