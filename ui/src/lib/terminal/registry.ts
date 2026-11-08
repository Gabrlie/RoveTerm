// Live terminal views by pane, so actions can reach the focused terminal.
import type { PaneId } from '../layout/layout';

export interface TerminalHandle {
  focus(): void;
  selection(): string;
  paste(text: string): void;
}

export const terminals = new Map<PaneId, TerminalHandle>();
