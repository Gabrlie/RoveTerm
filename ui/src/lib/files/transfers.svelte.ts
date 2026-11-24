// Every transfer the backend reports, kept current from `transfer-update` events, and whether
// the downloads popover is open (it opens by itself when a transfer starts, as in a browser).
import { onTransferUpdate, transferCancel, transferClear, transferExport, transferList, transferReveal, type TransferId, type TransferUpdate } from '../ipc';
import { isFinal } from './transferview';

export { isFinal };

type Listener = (update: TransferUpdate) => void;

class Transfers {
  byId = $state<Record<TransferId, TransferUpdate>>({});
  /** The downloads popover of the activity rail is open. */
  popover = $state(false);
  private listeners = new Set<Listener>();
  private started = false;

  /** Rows oldest first. */
  get all(): TransferUpdate[] {
    return Object.values(this.byId).sort((a, b) => a.id.localeCompare(b.id));
  }

  /** Subscribes to events once; later calls do nothing. */
  async start(): Promise<void> {
    if (this.started) return;
    this.started = true;
    await onTransferUpdate((update) => {
      this.byId[update.id] = update;
      for (const listener of this.listeners) listener(update);
    });
    for (const row of await transferList()) this.byId[row.id] ??= row;
  }

  /** Calls `listener` for every update until the returned function is called. */
  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  cancel(id: TransferId): Promise<boolean> {
    return transferCancel(id);
  }

  /** Shows a finished download in the desktop file manager or mobile share sheet. */
  reveal(id: TransferId): Promise<void> {
    return transferReveal(id);
  }

  exportFile(id: TransferId): Promise<boolean> {
    return transferExport(id);
  }

  /** Forgets finished transfers here and in the backend. */
  async clearFinished(): Promise<void> {
    await transferClear();
    for (const [id, row] of Object.entries(this.byId)) {
      if (isFinal(row)) delete this.byId[id];
    }
  }
}

export const transfers = new Transfers();
