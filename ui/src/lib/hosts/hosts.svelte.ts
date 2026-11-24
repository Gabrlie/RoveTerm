// Saved hosts, refreshed after every change.
import { hostDelete, hostDetectOs, hostImportSshConfig, hostList, type HostEntry, type ImportReport } from '../ipc';

class HostStore {
  items = $state<HostEntry[]>([]);
  query = $state('');
  /** The list has been read at least once, so a missing host really is gone. */
  loaded = $state(false);

  /** Hosts matching the search query, grouped by `group` (ungrouped first). */
  get grouped(): [string | null, HostEntry[]][] {
    const needle = this.query.trim().toLowerCase();
    const groups = new Map<string | null, HostEntry[]>();
    for (const host of this.items) {
      const haystack = [host.name, host.hostname, host.username, host.group ?? '', ...host.tags].join(' ').toLowerCase();
      if (needle && !haystack.includes(needle)) continue;
      const list = groups.get(host.group) ?? [];
      list.push(host);
      groups.set(host.group, list);
    }
    return [...groups.entries()];
  }

  async refresh(): Promise<void> {
    this.items = await hostList();
    this.loaded = true;
  }

  /** Hosts whose system was detected since the app started. */
  private detected = new Set<string>();

  /**
   * Called when a terminal of saved host `id` connected: refreshes the list (the backend recorded
   * the connection time) and, once per run for each host, detects its system for the icon.
   */
  async connected(id: string): Promise<void> {
    if (!this.detected.has(id)) {
      this.detected.add(id);
      await hostDetectOs(id).catch((error: unknown) => console.info('system detection skipped', error));
    }
    await this.refresh();
  }

  async remove(id: string): Promise<void> {
    await hostDelete(id);
    await this.refresh();
  }

  async importSshConfig(path?: string | null): Promise<ImportReport> {
    const report = await hostImportSshConfig(path);
    await this.refresh();
    return report;
  }

  find(id: string): HostEntry | undefined {
    return this.items.find((host) => host.id === id);
  }
}

export const hosts = new HostStore();

/** `true` when connecting to `host` needs a password typed now. */
export function needsPassword(host: HostEntry): boolean {
  return host.auth.method === 'interactive' || (host.auth.method === 'password' && !host.has_secret);
}
