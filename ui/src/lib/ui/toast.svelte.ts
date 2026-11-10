// Notifications floating at the top-right corner, above dialogs. Anything that reports to the
// user without asking a question uses them: errors, results of background work, sync.

export type ToastKind = 'info' | 'success' | 'error';

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

/** How long a toast stays, by kind; errors stay longer so they can be read. */
const DURATION: Record<ToastKind, number> = { info: 4000, success: 4000, error: 8000 };
/** At most this many are shown; older ones make room. */
const MAX_SHOWN = 4;

class ToastStore {
  items = $state<Toast[]>([]);
  private next = 0;
  private timers = new Map<number, number>();

  show(message: string, kind: ToastKind = 'info'): void {
    // The same message again only restarts its timer.
    const same = this.items.find((toast) => toast.message === message && toast.kind === kind);
    if (same) {
      this.schedule(same);
      return;
    }
    const toast = { id: this.next++, kind, message };
    this.items = [...this.items, toast].slice(-MAX_SHOWN);
    this.schedule(toast);
  }

  dismiss(id: number): void {
    clearTimeout(this.timers.get(id));
    this.timers.delete(id);
    this.items = this.items.filter((toast) => toast.id !== id);
  }

  /** Keeps a toast while the pointer is over it. */
  hold(id: number): void {
    clearTimeout(this.timers.get(id));
  }

  release(id: number): void {
    const toast = this.items.find((item) => item.id === id);
    if (toast) this.schedule(toast);
  }

  private schedule(toast: Toast): void {
    clearTimeout(this.timers.get(toast.id));
    this.timers.set(toast.id, window.setTimeout(() => this.dismiss(toast.id), DURATION[toast.kind]));
  }
}

export const toasts = new ToastStore();

/** Shows a notification. */
export function notify(message: string, kind: ToastKind = 'info'): void {
  toasts.show(message, kind);
}
