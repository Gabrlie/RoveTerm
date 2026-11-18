import type { SyncStatus } from '../ipc';

/** A background tick reports either failure once and leaves the timer available for a retry. */
export async function runAutoSync(
  refreshStatus: () => Promise<Pick<SyncStatus, 'configured' | 'ready'>>,
  syncNow: () => Promise<boolean>,
  reportFailure: (error: unknown) => void,
): Promise<void> {
  try {
    const status = await refreshStatus();
    if (status.configured && status.ready) await syncNow();
  } catch (error) {
    reportFailure(error);
  }
}
