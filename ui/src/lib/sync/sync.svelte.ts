// Sync state for the UI: status, the last round, and automatic rounds while the app runs.
import { syncConnect, syncDisable, syncNow, syncProbe, syncSetup, syncStatus, type SyncReport, type SyncStatus, type SyncTarget } from '../ipc';
import { errorText, t } from '../i18n';
import { notify } from '../ui/toast.svelte';
import { runAutoSync } from './auto';
import { hosts } from '../hosts/hosts.svelte';
import { keys } from '../keys/keys.svelte';
import { prefs } from '../settings/prefs.svelte';

/** Minutes between automatic rounds. */
const AUTO_MINUTES = 5;

class SyncStore {
  status = $state<SyncStatus | null>(null);
  /** Whether a space exists at the connected storage; null until looked up. */
  spaceExists = $state<boolean | null>(null);
  /** Connecting to the storage or looking for a space. */
  connecting = $state(false);
  running = $state(false);
  lastReport = $state<SyncReport | null>(null);
  lastAt = $state<number | null>(null);
  lastError = $state('');
  /** Bumped whenever a round wrote remote changes, so views that load data themselves reload. */
  generation = $state(0);
  private timer = 0;

  async refreshStatus(): Promise<SyncStatus> {
    const status = await syncStatus();
    this.status = status;
    return status;
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
      // A failed round may still have written other devices' changes, or dropped the key of a
      // space that changed: reload what may differ.
      this.generation += 1;
      await Promise.all([hosts.refresh(), keys.refresh(), prefs.load(), this.refreshStatus()]).catch((reload: unknown) => console.warn('reload after sync failed', reload));
      throw error;
    } finally {
      this.running = false;
    }
  }

  /** Checks and stores the storage, then looks for a space there. */
  async connect(target: SyncTarget, storagePassword: string | null): Promise<void> {
    this.connecting = true;
    try {
      this.spaceExists = (await syncConnect(target, storagePassword)).exists;
      await this.refreshStatus();
    } finally {
      this.connecting = false;
    }
  }

  /** Looks up whether a space exists at the connected storage, once per settings visit. */
  async probe(): Promise<void> {
    this.connecting = true;
    try {
      this.spaceExists = (await syncProbe()).exists;
    } finally {
      this.connecting = false;
    }
  }

  /** Joins the space at the connected storage, or creates one there. */
  setup(password: string): Promise<boolean> {
    return this.round(() => syncSetup(password));
  }

  syncNow(): Promise<boolean> {
    return this.round(syncNow);
  }

  async disable(): Promise<void> {
    await syncDisable();
    this.spaceExists = null;
    this.lastReport = null;
    this.lastAt = null;
    this.lastError = '';
    await this.refreshStatus();
  }

  /** Syncs now and every few minutes while sync is set up and ready; failures wait for the next turn. */
  startAuto(): void {
    const tick = () => runAutoSync(
      () => this.refreshStatus(),
      () => this.syncNow(),
      (error) => {
        this.lastError = errorText(error);
        notify(t('status.syncFailed', { error: this.lastError }), 'error');
      },
    );
    void tick();
    window.clearInterval(this.timer);
    this.timer = window.setInterval(() => void tick(), AUTO_MINUTES * 60_000);
  }
}

export const sync = new SyncStore();

/** What the settings page has typed so far. Kept outside the page so switching sections, the
 *  storage kind or closing settings after an error loses nothing; the passwords stay in memory
 *  only and are cleared once used. */
export const draft = $state({
  storage: 'folder' as 'folder' | 'webdav',
  folder: '',
  server: '',
  remoteFolder: 'RoveTerm',
  username: '',
  storagePassword: '',
  password: '',
  confirm: '',
});
