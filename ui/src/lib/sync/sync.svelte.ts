// Sync state for the UI: status, the last round, and automatic rounds while the app runs.
import { syncDisable, syncNow, syncSetup, syncStatus, type SyncReport, type SyncStatus, type SyncTarget } from '../ipc';
import { errorText } from '../i18n';
import { hosts } from '../hosts/hosts.svelte';
import { keys } from '../keys/keys.svelte';
import { prefs } from '../settings/prefs.svelte';

/** Minutes between automatic rounds. */
const AUTO_MINUTES = 5;

class SyncStore {
  status = $state<SyncStatus | null>(null);
  running = $state(false);
  lastReport = $state<SyncReport | null>(null);
  lastAt = $state<number | null>(null);
  lastError = $state('');
  /** Bumped whenever a round wrote remote changes, so views that load data themselves reload. */
  generation = $state(0);
  private timer = 0;

  async refreshStatus(): Promise<void> {
    this.status = await syncStatus();
  }

  /** Reloads everything a round may have changed. */
  private async afterRound(report: SyncReport): Promise<void> {
    this.lastReport = report;
    this.lastAt = Date.now();
    this.lastError = '';
    if (report.applied > 0) {
      this.generation += 1;
      await Promise.all([hosts.refresh(), keys.refresh(), prefs.load()]).catch((error: unknown) => console.warn('reload after sync failed', error));
    }
    await this.refreshStatus();
  }

  private async round(run: () => Promise<SyncReport>): Promise<boolean> {
    if (this.running) return false;
    this.running = true;
    try {
      await this.afterRound(await run());
      return true;
    } catch (error) {
      this.lastError = errorText(error);
      throw error;
    } finally {
      this.running = false;
    }
  }

  setup(target: SyncTarget, password: string, create: boolean, storagePassword: string | null = null): Promise<boolean> {
    return this.round(() => syncSetup(target, password, create, storagePassword));
  }

  syncNow(): Promise<boolean> {
    return this.round(syncNow);
  }

  async disable(): Promise<void> {
    await syncDisable();
    this.lastReport = null;
    this.lastAt = null;
    this.lastError = '';
    await this.refreshStatus();
  }

  /** Syncs now and every few minutes while sync is set up and ready; failures wait for the next turn. */
  startAuto(): void {
    const tick = async (): Promise<void> => {
      await this.refreshStatus().catch(() => undefined);
      if (this.status?.configured && this.status.ready) await this.syncNow().catch(() => undefined);
    };
    void tick();
    window.clearInterval(this.timer);
    this.timer = window.setInterval(() => void tick(), AUTO_MINUTES * 60_000);
  }
}

export const sync = new SyncStore();
