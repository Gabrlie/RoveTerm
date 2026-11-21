export type TailscalePeerId = string;

export type TailscaleState =
  | 'not_installed'
  | 'service_unavailable'
  | 'permission_denied'
  | 'no_state'
  | 'needs_login'
  | 'needs_machine_auth'
  | 'stopped'
  | 'starting'
  | 'running'
  | 'unknown';

/** A discovery identity is independent of the peer's name and current public key. */
export interface TailscalePeer {
  id: TailscalePeerId;
  name: string;
  dns_name: string;
  addresses: string[];
  os?: string | null;
  online: boolean;
}

export interface TailscaleSnapshot {
  state: TailscaleState;
  peers: TailscalePeer[];
  captured_at: number;
  message?: string | null;
}
