import { hostUpdate, onTailscaleStatus, tailscaleConnect, tailscaleDisconnect, tailscaleLogout, tailscaleOpenAuthUrl, tailscaleSnapshot, tailscaleStatus, type HostEntry } from '../ipc';
import { errorText, t } from '../i18n';
import { isMobile } from '../platform';
import { hosts } from '../hosts/hosts.svelte';
import { prefs } from '../settings/prefs.svelte';
import { RefreshGate } from './discovery';
import type { TailscalePeer, TailscaleState, TailscaleStatus } from './types';

const DISMISSED_KEY = 'tailscale.dismissedMatches';

function initialStatus(): TailscaleStatus {
  return { configured: false, state: 'unconfigured', self_node: null, auth_url: null, peer_count: 0, updated_at: 0 };
}

function loadDismissed(): Set<string> {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(DISMISSED_KEY) ?? '[]');
    return new Set(Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === 'string') : []);
  } catch {
    return new Set();
  }
}

class TailscaleStore {
  status = $state<TailscaleStatus>(initialStatus());
  peers = $state<TailscalePeer[]>([]);
  capturedAt = $state<number | null>(null);
  busy = $state(false);
  refreshing = $state(false);
  error = $state('');
  dismissed = $state<Set<string>>(loadDismissed());
  private initialized = false;
  private statusLoaded = $state(false);
  private statusEventReceived = false;
  private requests = new RefreshGate();

  get configured(): boolean { return this.statusLoaded ? this.status.configured : prefs.device.tailscale.configured; }
  get connected(): boolean { return this.status.state === 'running'; }
  get connecting(): boolean { return this.status.state === 'starting'; }
  get needsLogin(): boolean { return this.status.state === 'needs_login'; }
  get active(): boolean {
    return this.status.state === 'running'
      || this.status.state === 'starting'
      || this.status.state === 'needs_login'
      || this.status.state === 'needs_machine_auth';
  }

  private applyStatus(status: TailscaleStatus): void {
    this.statusLoaded = true;
    this.status = status;
    this.error = status.state === 'error'
      ? (status.message ? errorText({ code: 'tailscale.failed', message: status.message }) : t('error.tailscale.failed'))
      : '';
  }

  async init(): Promise<void> {
    if (this.initialized || isMobile) return;
    this.initialized = true;
    void onTailscaleStatus((status) => {
      this.statusEventReceived = true;
      const previous = this.status;
      this.applyStatus(status);
      if (status.configured !== previous.configured) {
        void prefs.currentDevice().catch(() => {});
      }
      if (status.state === 'running') {
        if (previous.state !== 'running' || status.updated_at !== previous.updated_at || this.capturedAt === null) {
          void this.refreshSnapshot();
        }
      } else {
        this.requests.begin();
        this.peers = [];
        this.capturedAt = null;
        this.refreshing = false;
      }
    }).then((unlisten) => {
      if (!this.initialized) unlisten();
    }).catch((error: unknown) => { this.error = errorText(error); });
    try {
      const status = await tailscaleStatus();
      if (!this.statusEventReceived) this.applyStatus(status);
      if (this.connected) await this.refreshSnapshot();
    } catch (error) {
      this.error = errorText(error);
    }
  }

  async refreshSnapshot(): Promise<void> {
    const revision = this.requests.begin();
    this.refreshing = true;
    try {
      const snapshot = await tailscaleSnapshot();
      if (!this.requests.accepts(revision)) return;
      this.applyStatus(snapshot.status);
      const failed = snapshot.status.state === 'error';
      if (!failed || this.capturedAt === null) {
        this.peers = snapshot.peers;
        this.capturedAt = snapshot.captured_at;
      }
    } catch (error) {
      if (this.requests.accepts(revision)) this.error = errorText(error);
    } finally {
      if (this.requests.accepts(revision)) this.refreshing = false;
    }
  }

  async connect(params: { authKey?: string | null; controlUrl?: string | null; hostname?: string | null } = {}): Promise<void> {
    this.requests.begin();
    this.refreshing = false;
    this.busy = true;
    this.error = '';
    try {
      this.applyStatus(await tailscaleConnect(params));
      await prefs.currentDevice();
      if (this.connected) await this.refreshSnapshot();
    } catch (error) {
      this.error = errorText(error);
      throw error;
    } finally {
      this.busy = false;
    }
  }

  async disconnect(): Promise<void> {
    this.requests.begin();
    this.refreshing = false;
    this.busy = true;
    this.error = '';
    try {
      this.applyStatus(await tailscaleDisconnect());
      this.peers = [];
      this.capturedAt = null;
    } catch (error) {
      this.error = errorText(error);
      throw error;
    } finally {
      this.busy = false;
    }
  }

  async logout(): Promise<void> {
    this.requests.begin();
    this.refreshing = false;
    this.busy = true;
    this.error = '';
    try {
      this.applyStatus(await tailscaleLogout());
      this.peers = [];
      this.capturedAt = null;
      await prefs.currentDevice();
    } catch (error) {
      this.error = errorText(error);
      throw error;
    } finally {
      this.busy = false;
    }
  }

  async openAuthUrl(): Promise<void> {
    try {
      await tailscaleOpenAuthUrl();
    } catch (error) {
      this.error = errorText(error);
      throw error;
    }
  }

  async convertHost(host: HostEntry, peer: TailscalePeer): Promise<void> {
    await hostUpdate(host.id, { name: host.name, group: host.group, hostname: host.hostname, port: host.port, username: host.username, auth: host.auth, route: { kind: 'tailscale', peer_id: peer.id }, tags: host.tags }, null, false);
    await hosts.refresh();
  }

  dismissCandidate(hostId: string, peerId: string): void {
    this.dismissed = new Set([...this.dismissed, `${hostId}:${peerId}`]);
    try {
      localStorage.setItem(DISMISSED_KEY, JSON.stringify([...this.dismissed]));
    } catch {
      // The dismissal remains active for this run when local storage is unavailable.
    }
  }

  stateLabelKey(): `tailscale.state.${TailscaleState}` {
    return `tailscale.state.${this.status.state}`;
  }
}

export const tailscale = new TailscaleStore();
