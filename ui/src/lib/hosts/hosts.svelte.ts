// Saved hosts, refreshed after every change.
import { hostDelete, hostImportSshConfig, hostList, type HostEntry, type ImportReport } from '../ipc';

class HostStore {
  items = $state<HostEntry[]>([]);
  query = $state('');

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
  }

  async remove(id: string): Promise<void> {
    await hostDelete(id);
    await this.refresh();
  }

  async importSshConfig(): Promise<ImportReport> {
    const report = await hostImportSshConfig();
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
