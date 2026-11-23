export type OpsTaskId = string;
export type OpsSubtaskId = string;
export type OpsTaskState = 'queued' | 'running' | 'waiting_approval' | 'blocked' | 'completed' | 'failed' | 'cancelled' | 'interrupted';
export type OpsBlockReason = 'budget_exhausted' | 'needs_verification' | 'target_changed' | 'unresolved_effect' | 'worker_interrupted' | 'user_decision';

export interface OpsTaskCounts {
  total: number;
  queued: number;
  running: number;
  waiting_approval: number;
  completed: number;
  blocked: number;
  failed: number;
  cancelled: number;
  interrupted: number;
}

export interface OpsCheckView {
  criterion: string;
  summary: string | null;
}

export interface OpsSubtaskView {
  id: OpsSubtaskId | null;
  host_id: string;
  host_name: string;
  state: OpsTaskState;
  session: string | null;
  task: string;
  summary: string | null;
  reply: string | null;
  checks: OpsCheckView[];
  reason: OpsBlockReason | null;
  error: string | null;
  uncertain_effect: boolean;
  attempts: number;
}

export interface OpsTaskView {
  id: OpsTaskId;
  revision: number;
  conversation: string;
  title: string;
  source: { kind: 'user' } | { kind: 'schedule'; schedule: string; due: number };
  state: OpsTaskState;
  read_only: boolean;
  created_at: number;
  updated_at: number;
  counts: OpsTaskCounts;
  subtasks: OpsSubtaskView[];
}
