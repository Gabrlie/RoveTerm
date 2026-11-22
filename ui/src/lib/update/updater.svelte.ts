// Reactive application update state shared by StatusBar, AboutSettings and UpdateDialog.
import { errorText, t } from '../i18n';
import {
  updateCheck,
  updateInstall,
  updateOpenRelease,
  type UpdateInfo,
  type UpdateProgress,
} from '../ipc';
import { editDevice } from '../settings/edit.svelte';
import { prefs } from '../settings/prefs.svelte';
import { notify } from '../ui/toast.svelte';
import {
  downloadPercent,
  isSkippedVersion,
  normalizeVersion,
  shouldShowUpdateBadge,
  type UpdatePhase,
} from './updaterLogic';

const STARTUP_DELAY_MS = 8000;

export interface CheckOptions {
  /** Silent startup check: suppresses toasts and avoids surfacing offline errors. */
  silent?: boolean;
  /** Opens the update dialog when a newer version is found, or shows a toast when up to date. */
  interactive?: boolean;
}

class UpdaterStore {
  phase = $state<UpdatePhase>('idle');
  info = $state<UpdateInfo | null>(null);
  downloaded = $state(0);
  total = $state<number | null>(null);
  error = $state('');
  dialogOpen = $state(false);
  lastCheckedAt = $state<number | null>(null);

  readonly percent = $derived(downloadPercent(this.downloaded, this.total));
  readonly skipped = $derived(
    isSkippedVersion(this.info?.version, prefs.device.updates.skipped_version),
  );
  readonly badgeVisible = $derived(
    shouldShowUpdateBadge(this.phase, this.info?.version, prefs.device.updates.skipped_version),
  );

  /** Runs a single silent check shortly after startup when auto-check is enabled. */
  scheduleStartupCheck(): () => void {
    const timer = window.setTimeout(() => {
      if (!prefs.device.updates.auto_check) return;
      if (this.phase !== 'idle') return;
      void this.check({ silent: true, interactive: false });
    }, STARTUP_DELAY_MS);
    return () => window.clearTimeout(timer);
  }

  openDialog(): void {
    this.dialogOpen = true;
  }

  closeDialog(): void {
    this.dialogOpen = false;
  }

  async check(options: CheckOptions = {}): Promise<void> {
    const { silent = false, interactive = false } = options;
    if (this.phase === 'downloading' || this.phase === 'installing' || this.phase === 'ready') {
      if (interactive) this.dialogOpen = true;
      return;
    }
    if (this.phase === 'checking') {
      if (interactive) this.dialogOpen = true;
      return;
    }

    this.phase = 'checking';
    this.error = '';
    try {
      const result = await updateCheck();
      this.info = result;
      this.lastCheckedAt = Date.now();
      if (result.available && result.version) {
        this.phase = 'available';
        if (interactive) this.dialogOpen = true;
      } else {
        this.phase = 'up_to_date';
        if (interactive && !silent) {
          notify(t('update.upToDateToast', { version: result.current_version }), 'info');
        }
      }
    } catch (err) {
      if (silent) {
        this.phase = 'idle';
        console.warn('automatic update check skipped', err);
        return;
      }
      this.error = errorText(err);
      this.phase = 'error';
      if (interactive) {
        notify(this.error, 'error');
      }
    }
  }

  async install(): Promise<void> {
    const current = this.info;
    if (!current?.available || !current.version) return;
    if (current.install_mode !== 'in_app') {
      await this.openRelease();
      return;
    }
    if (this.phase === 'downloading' || this.phase === 'installing' || this.phase === 'ready') {
      return;
    }

    this.phase = 'downloading';
    this.downloaded = 0;
    this.total = null;
    this.error = '';
    try {
      await updateInstall(current.version, (progress: UpdateProgress) => {
        switch (progress.stage) {
          case 'started':
            this.phase = 'downloading';
            this.total = progress.total;
            break;
          case 'downloading':
            this.phase = 'downloading';
            this.downloaded = progress.downloaded;
            this.total = progress.total;
            break;
          case 'installing':
            this.phase = 'installing';
            break;
          case 'finished':
            this.phase = 'ready';
            break;
        }
      });
      this.phase = 'ready';
    } catch (err) {
      this.error = errorText(err);
      this.phase = 'error';
    }
  }

  async openRelease(): Promise<void> {
    const url = this.info?.release_url;
    if (!url) return;
    try {
      await updateOpenRelease(url);
    } catch (err) {
      notify(errorText(err), 'error');
    }
  }

  async skipCurrent(): Promise<void> {
    const version = normalizeVersion(this.info?.version);
    if (!version) return;
    await editDevice((draft) => {
      draft.updates.skipped_version = version;
    });
    this.dialogOpen = false;
  }

  async clearSkipped(): Promise<void> {
    await editDevice((draft) => {
      draft.updates.skipped_version = '';
    });
  }
}

export const updater = new UpdaterStore();
