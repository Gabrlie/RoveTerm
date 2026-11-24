// Live terminal views by pane, so actions can reach the focused terminal.
import type { PaneId } from '../layout/layout';

export interface TerminalHandle {
  focus(): void;
  blur(): void;
  isFocused(): boolean;
  refresh(): void;
  selection(): string;
  send(data: string): void;
  paste(text: string): void;
}

export const terminals = new Map<PaneId, TerminalHandle>();
