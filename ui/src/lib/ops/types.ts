export type OpsTaskId = string;
export type OpsSubtaskId = string;
export type OpsTaskState = 'queued' | 'running' | 'waiting_approval' | 'blocked' | 'completed' | 'failed' | 'cancelled' | 'interrupted';
export type OpsBlockReason = 'budget_exhausted' | 'needs_verification' | 'target_changed' | 'unresolved_effect' | 'worker_interrupted' | 'user_decision';

export interface PrivateRef {
  session: string;
  reference: string;
}

export interface OpsTarget {
  host_id: string;
  fingerprint: string;
  input?: PrivateRef | null;
}

export interface OpsSubtask {
  id: OpsSubtaskId;
  host_id: string;
  fingerprint: string;
  state: OpsTaskState;
  session: string | null;
  checkpoint: PrivateRef | null;
  inputs: PrivateRef[];
  input_version: number;
  evidence: PrivateRef[];
  reason: OpsBlockReason | null;
  error_code: string | null;
  uncertain_effect: boolean;
  attempts: number;
}

export interface OpsTask {
  id: OpsTaskId;
  revision: number;
  request_id: string;
  conversation: string;
  source: { kind: 'user' } | { kind: 'schedule'; schedule: string; due: number };
  state: OpsTaskState;
  scope: { targets: OpsTarget[]; can_modify: boolean };
  inputs: PrivateRef[];
  acceptance: PrivateRef | null;
  report: PrivateRef | null;
  subtasks: OpsSubtask[];
  created_at: number;
  updated_at: number;
}
