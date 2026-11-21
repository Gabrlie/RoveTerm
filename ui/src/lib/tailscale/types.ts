export type TailscalePeerId = string;

export type TailscaleState =
  | 'unconfigured'
  | 'disconnected'
  | 'starting'
  | 'needs_login'
  | 'needs_machine_auth'
  | 'running'
  | 'error';

export interface TailscaleSelfNode {
  id: TailscalePeerId;
  name: string;
  dns_name: string;
  addresses: string[];
  tailnet: string | null;
}

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
  status: TailscaleStatus;
  peers: TailscalePeer[];
  captured_at: number;
}

export interface TailscaleStatus {
  configured: boolean;
  state: TailscaleState;
  self_node: TailscaleSelfNode | null;
  auth_url: string | null;
  peer_count: number;
  updated_at: number;
  message?: string | null;
}
