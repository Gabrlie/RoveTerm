// Runtime status of port forwards, kept current from `forward-update` events.
import { forwardList, onForwardUpdate, type ForwardId, type ForwardSpec, type ForwardStatus, type HostId } from '../ipc';

class ForwardStatuses {
  byId = $state<Record<ForwardId, ForwardStatus>>({});
  private started = false;

  /** Subscribes to events once and seeds statuses of forwards already running. */
  async start(): Promise<void> {
    if (this.started) return;
    this.started = true;
    await onForwardUpdate((status) => {
      if (status.state === 'stopped') delete this.byId[status.id];
      else this.byId[status.id] = status;
    });
    for (const entry of await forwardList(null)) {
      if (entry.status) this.byId[entry.id] ??= entry.status;
    }
  }

  /** Number of running forwards of `hostId`. */
  running(hostId: HostId): number {
    return Object.values(this.byId).filter((status) => status.host_id === hostId && status.state === 'running').length;
  }
}

export const forwardStatuses = new ForwardStatuses();

/** Describes a spec like `ssh -L`: `127.0.0.1:8080 → db:5432`. */
export function describeSpec(spec: ForwardSpec): string {
  const bind = `${hostPort(spec.bind_host, spec.bind_port)}`;
  return spec.kind === 'dynamic' ? `SOCKS5 ${bind}` : `${bind} → ${hostPort(spec.dest_host, spec.dest_port)}`;
}

function hostPort(host: string, port: number): string {
  return host.includes(':') ? `[${host}]:${port}` : `${host}:${port}`;
}

/** Whether a local bind address listens beyond this machine. */
export function isPublicBind(host: string): boolean {
  const value = host.trim().toLowerCase();
  return !(value === 'localhost' || value === '::1' || value.startsWith('127.'));
}
