// In-app confirmation, replacing the browser's `confirm()` so it follows the theme.
export interface ConfirmRequest {
  title: string;
  message: string;
  /** Label of the confirming button. */
  confirmLabel: string;
  danger?: boolean;
}

interface Pending extends ConfirmRequest {
  resolve: (confirmed: boolean) => void;
}

class ConfirmQueue {
  current = $state<Pending | null>(null);
  private waiting: Pending[] = [];

  /** Shows a confirmation; resolves `true` when confirmed, `false` when cancelled or dismissed. */
  ask(request: ConfirmRequest): Promise<boolean> {
    return new Promise((resolve) => {
      this.waiting.push({ ...request, resolve });
      if (!this.current) this.next();
    });
  }

  answer(confirmed: boolean): void {
    this.current?.resolve(confirmed);
    this.next();
  }

  private next(): void {
    this.current = this.waiting.shift() ?? null;
  }
}

export const confirmQueue = new ConfirmQueue();

export function ask(request: ConfirmRequest): Promise<boolean> {
  return confirmQueue.ask(request);
}
