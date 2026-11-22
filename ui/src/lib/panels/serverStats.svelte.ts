import { untrack } from 'svelte';
import { isCommandError, serverStats, type CpuTimes, type MemoryDisplay, type NetInterface, type NetworkUnit, type RemoteSource, type ServerStats, type SessionId } from '../ipc';
import { prefs } from '../settings/prefs.svelte';
import type { TailscalePath as TailnetPath } from '../tailscale/types';
import { sourceKey } from '../workspace/workspace.svelte';

/** Samples kept per core, oldest first. */
export const HISTORY = 40;

interface Rate {
  rx: number;
  tx: number;
}

export interface HostStatsState {
  stats: ServerStats | null;
  cpu: number | null;
  coreHistory: number[][];
  rates: Record<string, Rate>;
  totalRate: Rate | null;
  problem: string | null;
  error: unknown;
}

const EMPTY: HostStatsState = {
  stats: null, cpu: null, coreHistory: [], rates: {}, totalRate: null, problem: null, error: null,
};

export function cpuUsage(now: CpuTimes, before: CpuTimes | undefined): number | null {
  if (!before) return null;
  const total = now.total - before.total;
  const idle = now.idle - before.idle;
  if (total <= 0 || idle < 0) return null;
  return Math.max(0, ((total - idle) / total) * 100);
}

export function networkRates(now: NetInterface[], before: NetInterface[], seconds: number) {
  const rates: Record<string, Rate> = Object.create(null);
  let totalRate: Rate | null = null;
  if (seconds > 0) {
    const previous = new Map(before.map((net) => [net.name, net]));
    for (const net of now) {
      const old = previous.get(net.name);
      if (!old) continue;
      const rx = Math.max(0, (net.rx - old.rx) / seconds);
      const tx = Math.max(0, (net.tx - old.tx) / seconds);
      rates[net.name] = { rx, tx };
      if (net.name !== 'lo') {
        totalRate ??= { rx: 0, tx: 0 };
        totalRate.rx += rx;
        totalRate.tx += tx;
      }
    }
  }
  return { rates, totalRate };
}

export function formatNetworkRate(bytesPerSec: number | undefined, unit: NetworkUnit): string {
  if (bytesPerSec === undefined) return '…';
  const bits = unit === 'bits';
  const base = bits ? 1000 : 1024;
  const suffixes = bits ? ['bps', 'Kbps', 'Mbps', 'Gbps'] : ['B/s', 'KB/s', 'MB/s', 'GB/s'];
  let value = bytesPerSec * (bits ? 8 : 1);
  let scale = 0;
  while (value >= base && scale < suffixes.length - 1) {
    value /= base;
    scale += 1;
  }
  return `${value.toFixed(scale === 0 ? 0 : 1)} ${suffixes[scale]}`;
}

/** Combined ingress and egress rate in bytes/s, or undefined before the second sample. */
export function combinedNetworkRate(rate: Rate | null | undefined): number | undefined {
  return rate ? rate.rx + rate.tx : undefined;
}

/** Formats terminal status bar memory according to the configured display style. */
export function formatMemoryBar(
  usedBytes: number,
  totalBytes: number,
  display: MemoryDisplay,
  formatBytes: (bytes: number) => string,
): string {
  const percent = totalBytes > 0 ? ((usedBytes / totalBytes) * 100).toFixed(0) : '0';
  switch (display) {
    case 'percent':
      return `${percent}%`;
    case 'used':
      return formatBytes(usedBytes);
    case 'full':
      return `${formatBytes(usedBytes)} / ${formatBytes(totalBytes)}`;
    case 'full_percent':
      return `${formatBytes(usedBytes)} / ${formatBytes(totalBytes)} (${percent}%)`;
  }
}

/** Formats segment times in microseconds as milliseconds, in path order. */
export function formatLatencyMs(segments: number[]): string {
  return segments.map((us) => (us < 10_000 ? (us / 1000).toFixed(1) : Math.round(us / 1000).toString())).join('+');
}

export function describeTailscalePath(path: TailnetPath | null | undefined) {
  if (path?.kind === 'direct') return { kind: 'direct', title: path.endpoint } as const;
  if (path?.kind !== 'relay') return null;
  const code = path.region_code.toUpperCase();
  const name = path.region_name;
  const region = code || name;
  return { kind: 'relay', region, title: name && code ? `${name} (${code})` : region } as const;
}

class HostStats {
  state = $state.raw<HostStatsState>(EMPTY);
  private previousAt = 0;
  private consumers = new Map<symbol, SessionId | null>();
  private timer: ReturnType<typeof setTimeout> | undefined;
  private inFlight = false;
  private refreshPending = false;
  private stopWatchingInterval: (() => void) | undefined;
  private readonly source: RemoteSource;

  constructor(source: RemoteSource) {
    this.source = source;
  }

  acquire(session: SessionId | null) {
    const consumer = Symbol();
    if (this.consumers.size === 0) {
      this.consumers.set(consumer, session);
      // The interval watcher belongs to the host and survives any individual consumer.
      queueMicrotask(() => this.watchInterval());
      this.trigger();
    } else {
      this.consumers.set(consumer, null);
      this.setSession(consumer, session);
    }
    const host = this;
    return {
      get state() { return host.state; },
      trigger: () => this.trigger(),
      setSession: (next: SessionId | null) => this.setSession(consumer, next),
      release: () => this.release(consumer),
    };
  }

  private watchInterval(): void {
    if (this.consumers.size === 0 || this.stopWatchingInterval) return;
    let interval = prefs.value.status.interval_secs;
    this.stopWatchingInterval = $effect.root(() => {
      $effect(() => {
        const next = prefs.value.status.interval_secs;
        if (next !== interval) {
          interval = next;
          this.trigger();
        }
      });
    });
  }

  private setSession(consumer: symbol, session: SessionId | null): void {
    if (!this.consumers.has(consumer) || this.consumers.get(consumer) === session) return;
    const newSession = session !== null && !Array.from(this.consumers.values()).includes(session);
    this.consumers.set(consumer, session);
    if (newSession) this.trigger();
  }

  private release(consumer: symbol): void {
    if (!this.consumers.delete(consumer) || this.consumers.size > 0) return;
    clearTimeout(this.timer);
    this.refreshPending = false;
    this.stopWatchingInterval?.();
    this.stopWatchingInterval = undefined;
  }

  private trigger(): void {
    if (this.consumers.size === 0) return;
    clearTimeout(this.timer);
    if (this.inFlight) {
      this.refreshPending = true;
    } else {
      this.timer = setTimeout(() => void this.sample(), 0);
    }
  }

  private async sample(): Promise<void> {
    this.inFlight = true;
    try {
      const stats = await serverStats(this.source);
      const at = performance.now();
      const before = this.state.stats;
      const coreHistory = stats.cpus.map((core, index) => {
        const history = this.state.coreHistory[index] ?? [];
        const usage = cpuUsage(core, before?.cpus[index]);
        return usage === null ? history : [...history.slice(1 - HISTORY), usage];
      });
      this.state = {
        stats,
        cpu: cpuUsage(stats.cpu, before?.cpu),
        coreHistory,
        ...networkRates(stats.interfaces, before?.interfaces ?? [], (at - this.previousAt) / 1000),
        problem: null,
        error: null,
      };
      this.previousAt = at;
    } catch (error) {
      const code = isCommandError(error) ? error.code : 'unknown';
      if (code !== 'ssh.not_connected') this.state = { ...this.state, problem: code, error };
    } finally {
      this.inFlight = false;
      if (this.consumers.size > 0) {
        const delay = this.refreshPending ? 0 : prefs.value.status.interval_secs * 1000;
        this.refreshPending = false;
        this.timer = setTimeout(() => void this.sample(), delay);
      }
    }
  }
}

const hosts = new Map<string, HostStats>();

/** Shares one polling loop per remote. Release the handle when the consumer stops observing. */
export function acquire(source: RemoteSource, sessionHint: SessionId | null = null) {
  const key = sourceKey(source);
  let host = hosts.get(key);
  if (!host) {
    host = new HostStats(source);
    hosts.set(key, host);
  }
  return host.acquire(sessionHint);
}

/** Observes the current remote while the caller supplies one, retaining its snapshot between views. */
export function observeServerStats(source: () => RemoteSource | null, session: () => SessionId | null) {
  let handle = $state.raw<ReturnType<typeof acquire> | null>(null);
  $effect(() => {
    const remote = source();
    const current = remote ? acquire(remote, untrack(session)) : null;
    handle = current;
    return () => current?.release();
  });
  $effect(() => handle?.setSession(session()));
  return { get state() { return handle?.state ?? EMPTY; } };
}
