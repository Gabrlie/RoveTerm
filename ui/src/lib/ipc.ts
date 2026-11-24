// Typed wrappers around the Tauri commands. Components call these instead of `invoke` directly.
import { Channel, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { TailscalePath as TailnetPath, TailscalePingResult, TailscaleSnapshot, TailscaleStatus } from './tailscale/types';
import type { OpsTaskView } from './ops/types';
import type { MobileInsets } from './layout/insets';

export function mobileInsets(): Promise<MobileInsets> {
  return invoke('mobile_insets');
}

export function onMobileInsets(handler: (insets: MobileInsets) => void): Promise<UnlistenFn> {
  return listen<MobileInsets>('rove-mobile://insets', (event) => handler(event.payload));
}

export type MobileLifecycleState = 'foreground' | 'background';

export const mobileBackDefault = (): Promise<void> => invoke('mobile_back_default');

export const onMobileLifecycle = (cb: (event: { state: MobileLifecycleState }) => void): Promise<UnlistenFn> =>
  listen<{ state: MobileLifecycleState }>('rove-mobile://lifecycle', (event) => cb(event.payload));

export const onMobileBack = (cb: () => void): Promise<UnlistenFn> => listen('rove-mobile://back', () => cb());

export function opsTasks(conversation?: string): Promise<OpsTaskView[]> {
  return invoke('ops_task_list', { conversation });
}
export function opsTaskCancel(id: string): Promise<OpsTaskView> {
  return invoke('ops_task_cancel', { id });
}
export function opsSubtaskCancel(id: string, subtaskId: string): Promise<OpsTaskView> {
  return invoke('ops_subtask_cancel', { id, subtaskId });
}
export function opsTaskRetry(id: string, hostId: string | null = null, confirmUncertain = false): Promise<OpsTaskView> {
  return invoke('ops_task_retry', { id, hostId, confirmUncertain });
}
export function onOpsUpdate(handler: (task: OpsTaskView) => void): Promise<UnlistenFn> {
  return listen<OpsTaskView>('ops-update', (event) => handler(event.payload));
}

export function tailscaleSnapshot(): Promise<TailscaleSnapshot> {
  return invoke('tailscale_snapshot');
}

export function tailscaleStatus(): Promise<TailscaleStatus> {
  return invoke('tailscale_status');
}
export function tailscaleConnect(params: { authKey?: string | null; controlUrl?: string | null; hostname?: string | null } = {}): Promise<TailscaleStatus> {
  return invoke('tailscale_connect', {
    authKey: params.authKey ?? null,
    controlUrl: params.controlUrl ?? null,
    hostname: params.hostname ?? null,
  });
}
export function tailscaleDisconnect(): Promise<TailscaleStatus> {
  return invoke('tailscale_disconnect');
}
export function tailscaleLogout(): Promise<TailscaleStatus> {
  return invoke('tailscale_logout');
}
export function tailscaleOpenAuthUrl(): Promise<void> {
  return invoke('tailscale_open_auth_url');
}
export function tailscalePingPeer(peerId: string): Promise<TailscalePingResult> {
  return invoke('tailscale_ping_peer', { peerId });
}
export function onTailscaleStatus(handler: (status: TailscaleStatus) => void): Promise<UnlistenFn> {
  return listen<TailscaleStatus>('tailscale-status', (event) => handler(event.payload));
}

export type SessionId = string;
export type HostId = string;

/** Error shape returned by every command; `code` selects the localized message. */
export interface CommandError {
  code: string;
  message: string;
  detail?: unknown;
}

export function isCommandError(value: unknown): value is CommandError {
  return typeof value === 'object' && value !== null && 'code' in value && 'message' in value;
}

/** What a connection attempt needs from the user, or `null` when `error` is not a credential request. */
export type CredentialKind = 'password' | 'passphrase';

export function credentialKindOf(error: unknown): CredentialKind | null {
  if (!isCommandError(error)) return null;
  if (error.code === 'ssh.credentials_required') return 'password';
  if (error.code === 'ssh.passphrase_required') return 'passphrase';
  return null;
}

export interface ExitStatus {
  code: number | null;
  signal: string | null;
}

/** Messages delivered on a session output channel. */
/** Kind of credential in a connection step. */
export type AuthKind = 'password' | 'keyboard_interactive' | 'public_key' | 'agent';

/** One step of establishing an SSH connection; see `ConnectStep` in rove-ssh. */
export type ConnectStep =
  | { step: 'connecting'; address: string; via: string | null }
  | { step: 'handshake' }
  | { step: 'host_key_accepted'; algorithm: string; fingerprint: string }
  | { step: 'agent_unavailable'; reason: string }
  | { step: 'trying'; method: AuthKind; label: string | null }
  | { step: 'rejected'; method: AuthKind; label: string | null }
  | { step: 'authenticated' };

export type OutputMessage = ArrayBuffer | { exit: ExitStatus } | { progress: ConnectStep };

/** How a typed value is used: what the host's method asks for, or a login password for a key host. */
export type PasswordKind = 'natural' | 'password';

export type OutputHandler = (message: OutputMessage) => void;

function outputChannel(onOutput: OutputHandler): Channel<OutputMessage> {
  const channel = new Channel<OutputMessage>();
  channel.onmessage = onOutput;
  return channel;
}

export interface TermSize {
  cols: number;
  rows: number;
}

export function sessionOpenLocal(size: TermSize, onOutput: OutputHandler): Promise<SessionId> {
  return invoke('session_open_local', { onOutput: outputChannel(onOutput), ...size });
}

export function sessionOpenSsh(
  hostId: HostId,
  password: string | null,
  passwordKind: PasswordKind,
  remember: boolean,
  size: TermSize,
  onOutput: OutputHandler,
): Promise<SessionId> {
  return invoke('session_open_ssh', { onOutput: outputChannel(onOutput), hostId, password, passwordKind, remember, ...size });
}

export function sessionOpenQuick(
  address: string,
  password: string | null,
  size: TermSize,
  onOutput: OutputHandler,
): Promise<SessionId> {
  return invoke('session_open_quick', { onOutput: outputChannel(onOutput), address, password, ...size });
}

export function sessionWrite(id: SessionId, data: string): Promise<void> {
  return invoke('session_write', { id, data });
}

export function sessionResize(id: SessionId, size: TermSize): Promise<void> {
  return invoke('session_resize', { id, ...size });
}

export function sessionClose(id: SessionId): Promise<void> {
  return invoke('session_close', { id });
}

/** Re-attaches a view; resolves with the replay buffer. */
export function sessionReattach(id: SessionId, onOutput: OutputHandler): Promise<ArrayBuffer> {
  return invoke('session_reattach', { id, onOutput: outputChannel(onOutput) });
}

export type AuthMethod =
  | { method: 'password' }
  | { method: 'private_key' }
  | { method: 'key'; key: KeyId }
  | { method: 'key_file'; path: string }
  | { method: 'agent' }
  | { method: 'interactive' };

export type ProxyProtocol = 'http' | 'socks5';

/** How the connection to a host is made; see `HostRoute` in rove-vault. */
export type HostRoute =
  | { kind: 'direct' }
  | { kind: 'jump'; host: HostId }
  | { kind: 'proxy'; protocol: ProxyProtocol; host: string; port: number }
  | { kind: 'tailscale'; peer_id?: string | null };

export interface HostDraft {
  name: string;
  group: string | null;
  hostname: string;
  port: number;
  username: string;
  auth: AuthMethod;
  route: HostRoute;
  tags: string[];
}

export interface Host extends HostDraft {
  id: HostId;
  created_at: number;
  updated_at: number;
  /** Last successful connection, Unix milliseconds. */
  last_connected_at: number | null;
  /** Operating system detected on a connection; see `HostOs` in rove-vault. */
  os: HostOs | null;
}

export interface HostOs {
  /** os-release `ID`, or `macos` / `freebsd` from uname. */
  id: string;
  /** os-release `ID_LIKE`, most specific first. */
  like: string[];
  /** os-release `PRETTY_NAME`; may be empty. */
  name: string;
}

/** Detects the system of saved host `id` over its open connection; null when none is open. */
export function hostDetectOs(id: HostId): Promise<HostOs | null> {
  return invoke('host_detect_os', { id });
}

export interface HostEntry extends Host {
  has_secret: boolean;
}

export type Secret =
  | { kind: 'password'; password: string }
  | { kind: 'private_key'; key: string; passphrase: string | null }
  | { kind: 'passphrase'; passphrase: string };

export function hostList(): Promise<HostEntry[]> {
  return invoke('host_list');
}

export function hostCreate(draft: HostDraft, secret: Secret | null): Promise<Host> {
  return invoke('host_create', { draft, secret });
}

/** The notes kept about host `id` (what runs there, what to watch for); empty when none. */
export function hostNotesGet(id: HostId): Promise<string> {
  return invoke('host_notes_get', { id });
}

/** Replaces the notes kept about host `id`; empty text removes them. */
export function hostNotesSet(id: HostId, text: string): Promise<void> {
  return invoke('host_notes_set', { id, text });
}

export function hostUpdate(id: HostId, draft: HostDraft, secret: Secret | null, clearSecret: boolean): Promise<Host> {
  return invoke('host_update', { id, draft, secret, clearSecret });
}

export function hostDelete(id: HostId): Promise<void> {
  return invoke('host_delete', { id });
}

export interface ImportReport {
  created: string[];
  skipped: string[];
}

export function hostImportSshConfig(path?: string | null): Promise<ImportReport> {
  return invoke('host_import_ssh_config', path ? { path } : {});
}

export type LockReason =
  | { kind: 'not_attempted' }
  | { kind: 'store_unavailable'; detail: string }
  | { kind: 'denied' }
  | { kind: 'key_lost' };

export type LockState = { state: 'unlocked' } | { state: 'locked'; reason: LockReason };

export function vaultStatus(): Promise<LockState> {
  return invoke('vault_status');
}

export function vaultUnlock(): Promise<LockState> {
  return invoke('vault_unlock');
}

export function vaultResetSecrets(): Promise<LockState> {
  return invoke('vault_reset_secrets');
}

export interface VaultBiometricStatus {
  supported: boolean;
  enrolled: boolean;
  enabled: boolean;
}

export function vaultBiometricStatus(): Promise<VaultBiometricStatus> {
  return invoke('vault_biometric_status');
}

export function vaultSetBiometric(enabled: boolean): Promise<DevicePrefs> {
  return invoke('vault_set_biometric', { enabled });
}

/** Terminal appearance and behaviour; see `TerminalPrefs` in rove-vault. */
export interface TerminalPrefs {
  /** Empty uses the built-in font stack. */
  font_family: string;
  font_size: number;
  line_height: number;
  /** Colour scheme ids (see terminal/schemes.ts) for the light and dark interface. */
  scheme_light: string;
  scheme_dark: string;
  cursor_style: 'block' | 'bar' | 'underline';
  cursor_blink: boolean;
  scrollback: number;
  copy_on_select: boolean;
  right_click: 'menu' | 'paste';
  bell: 'none' | 'visual';
}

export type Theme = 'system' | 'light' | 'dark';
export type Accent = 'blue' | 'purple' | 'pink' | 'red' | 'orange' | 'yellow' | 'green' | 'graphite';
export type Language = 'auto' | 'zh-CN' | 'en';

export interface AppearancePrefs {
  theme: Theme;
  accent: Accent;
  /** Interface scale in percent, 90–125. */
  ui_scale: number;
  language: Language;
}

export interface FilePrefs {
  default_conflict: 'ask' | Conflict;
  show_hidden: boolean;
  confirm_delete: boolean;
}

export type CpuDisplay = 'auto' | 'per_core' | 'summary';
export type NetworkUnit = 'bytes' | 'bits';
export type MemoryDisplay = 'full_percent' | 'full' | 'used' | 'percent';
export type NetworkDisplay = 'split' | 'total';

export interface StatusPrefs {
  interval_secs: number;
  show_system: boolean;
  show_cpu: boolean;
  show_memory: boolean;
  show_swap: boolean;
  show_network: boolean;
  show_disks: boolean;
  cpu_display: CpuDisplay;
  network_unit: NetworkUnit;
  terminal_bar: boolean;
  terminal_bar_cpu: boolean;
  terminal_bar_memory: boolean;
  terminal_bar_memory_display: MemoryDisplay;
  terminal_bar_network: boolean;
  terminal_bar_network_display: NetworkDisplay;
  terminal_bar_latency: boolean;
  terminal_bar_tailscale_path: boolean;
}

/** Preferences that follow the user; see `Preferences` in rove-vault. */
/** How much the user is asked before an AI agent acts. */
export type AiMode = 'ask_by_risk' | 'ask_all' | 'auto';

/** How much a model reasons; each API takes the nearest level it knows. */
export type ReasoningEffort = 'minimal' | 'low' | 'medium' | 'high' | 'xhigh' | 'max';

export type ProviderFormat = 'chat' | 'responses' | 'anthropic';

/** A model provider of the built-in assistant; its key is kept apart, on each device. */
export interface ProviderPrefs {
  id: string;
  name: string;
  format: ProviderFormat;
  /** Including the API version path, such as `https://api.openai.com/v1`. */
  base_url: string;
  model: string;
  /** Other models to switch to in a conversation. */
  models: string[];
  max_tokens: number | null;
  /** Context windows of the models (tokens), as the provider's model list told them. */
  contexts: Record<string, number>;
  /** Context window of the other models, when the user set one. */
  context_window: number | null;
  effort: ReasoningEffort;
}

/** A model in a provider's list, with its context window when the list says. */
export interface ListedModel {
  id: string;
  context: number | null;
}

export interface AiPrefs {
  mode: AiMode;
  providers: ProviderPrefs[];
  /** `builtin:<provider id>` or `acp:<agent id>`. */
  default_agent: string | null;
  /** Extra programs treated as read-only. */
  read_only: string[];
  /** Text that makes any command containing it dangerous. */
  dangerous: string[];
  /** Most model requests of the built-in assistant in one turn, 1–200. */
  max_steps: number;
  /** How long the built-in assistant waits for a reply, 30–1800 seconds. */
  request_timeout_secs: number;
  /** The judge of the auto mode: the conversation's model, a chosen one, or Jev. */
  reviewer: 'model' | 'chosen' | 'jev';
  /** The `chosen` judge's model, by provider id. */
  review_model: { provider: string; model: string } | null;
  /** How sure the reviewer must be, in percent (50–99). */
  review_confidence: number;
  /** Tasks the fleet lead runs on a schedule while the app is open. */
  schedules: Schedule[];
}

/** When a scheduled task runs, in local time (weekday 0 is Monday). */
export type Repeat =
  | { kind: 'once'; at: number }
  | { kind: 'hourly'; minute: number }
  | { kind: 'daily'; hour: number; minute: number }
  | { kind: 'weekly'; weekday: number; hour: number; minute: number };

/** A task the fleet lead runs on its own at set times. */
export interface Schedule {
  owner: string | null;
  id: string;
  name: string;
  instruction: string;
  repeat: Repeat;
  /** Changes the judge is sure of may be made; otherwise the run only checks and reports. */
  allow_changes: boolean;
  /** A run missed while the app was closed is made at the next start. */
  catch_up: boolean;
  enabled: boolean;
  created_at: number;
}

/** How a scheduled task last ran. */
export interface ScheduleStatus {
  schedule: string;
  due: number;
  status: 'started' | 'done' | 'failed' | 'missed';
  conversation: string | null;
  at: number;
  running: boolean;
}

export function scheduleClaim(id: string): Promise<void> { return invoke('schedule_claim', { id }); }
export function scheduleDevice(): Promise<string> { return invoke('schedule_device'); }

export function scheduleStatus(): Promise<ScheduleStatus[]> {
  return invoke('schedule_status');
}

/** Runs scheduled task `id` now; `false` when it is running already. */
export function scheduleRunNow(id: string): Promise<boolean> {
  return invoke('schedule_run_now', { id });
}

/** A scheduled run ended. */
export interface ReportEvent {
  schedule: string;
  name: string;
  conversation: string | null;
  ok: boolean;
  error: string | null;
}

export function onAiReport(handler: (event: ReportEvent) => void): Promise<UnlistenFn> {
  return listen<ReportEvent>('ai-report', (event) => handler(event.payload));
}

export interface Preferences {
  appearance: AppearancePrefs;
  terminal: TerminalPrefs;
  files: FilePrefs;
  status: StatusPrefs;
  ai: AiPrefs;
}

export function prefsGet(): Promise<Preferences> {
  return invoke('prefs_get');
}

export function prefsSet(prefs: Preferences): Promise<Preferences> {
  return invoke('prefs_set', { prefs });
}

/** Preferences of this device only; see `DevicePrefs` in rove-vault. */
/** How the agent signs in: the stored API key when there is one, the key, or its own account. */
export type AgentAuth = 'auto' | 'api_key' | 'account';

/** An ACP agent added by command. */
export interface CustomAgent {
  /** `custom-` followed by lowercase letters, digits and `-`. */
  id: string;
  name: string;
  command: string;
  args: string[];
}

/** ACP agents on this device. */
export interface AgentsPrefs {
  /** Ids of detected agents turned off. */
  disabled: string[];
  custom: CustomAgent[];
  auth: AgentAuth;
  /** Agents the user runs on their own may use the saved hosts through RoveTerm. */
  expose: boolean;
}

export interface DevicePrefs {
  /** Absolute path; empty uses the system download directory. */
  download_dir: string;
  max_transfers: number;
  agents: AgentsPrefs;
  connection: ConnectionPrefs;
  /** Device-private executable and argument vector; an empty program is unconfigured. */
  external_editor: { program: string; args: string[] };
  tailscale: TailscalePrefs;
  updates: UpdatePrefs;
  vault_biometric: boolean;
}

export interface UpdatePrefs {
  auto_check: boolean;
  skipped_version: string;
}

export interface TailscalePrefs {
  configured: boolean;
  auto_connect: boolean;
  control_url: string;
  hostname: string;
}

/** SSH connection timing on this device; applies to new connections. */
export interface ConnectionPrefs {
  connect_timeout_secs: number;
  keepalive_secs: number;
  keepalive_max: number;
}

export function devicePrefsGet(): Promise<DevicePrefs> {
  return invoke('device_prefs_get');
}

export function devicePrefsSet(prefs: DevicePrefs): Promise<DevicePrefs> {
  return invoke('device_prefs_set', { prefs });
}

/** Installed monospaced font families; empty where they cannot be listed. */
export function fontsMonospace(): Promise<string[]> {
  return invoke('fonts_monospace');
}

export function onVaultState(handler: (state: LockState) => void): Promise<UnlistenFn> {
  return listen<LockState>('vault-state', (event) => handler(event.payload));
}

export interface HostKeyPrompt {
  request_id: string;
  hostname: string;
  port: number;
  /** How the server is reached; empty for a direct connection. */
  route: string;
  key: { algorithm: string; public_key: string; fingerprint: string };
}

export function onHostKeyPrompt(handler: (prompt: HostKeyPrompt) => void): Promise<UnlistenFn> {
  return listen<HostKeyPrompt>('hostkey-prompt', (event) => handler(event.payload));
}

export function hostkeyRespond(requestId: string, answer: 'trust' | 'reject'): Promise<boolean> {
  return invoke('hostkey_respond', { requestId, answer });
}

export type FsId = string;
export type TransferId = string;

export type EntryKind = 'file' | 'dir' | 'symlink' | 'other';

export interface DirEntry {
  name: string;
  kind: EntryKind;
  size: number;
  modified: number | null;
  permissions: number | null;
}

/** Where a remote pane connects: a saved host or a quick-connect address. */
export type RemoteSource = { hostId: HostId } | { address: string };
export type StatsSource = RemoteSource | { kind: 'local' };

export interface SftpOpened {
  id: FsId;
  label: string;
  home: string;
}

export function sftpOpen(
  source: RemoteSource,
  password: string | null,
  remember: boolean,
  passwordKind: PasswordKind = 'natural',
): Promise<SftpOpened> {
  return invoke('sftp_open', {
    hostId: 'hostId' in source ? source.hostId : null,
    address: 'address' in source ? source.address : null,
    password,
    passwordKind,
    remember,
  });
}

export function sftpList(id: FsId, path: string): Promise<DirEntry[]> {
  return invoke('sftp_list', { id, path });
}

export function sftpMkdir(id: FsId, dir: string, name: string): Promise<string> {
  return invoke('sftp_mkdir', { id, dir, name });
}

export function sftpRename(id: FsId, path: string, name: string): Promise<string> {
  return invoke('sftp_rename', { id, path, name });
}

export function sftpRemove(id: FsId, paths: string[]): Promise<void> {
  return invoke('sftp_remove', { id, paths });
}

export function sftpClose(id: FsId): Promise<boolean> {
  return invoke('sftp_close', { id });
}

/** Takes a new reference to the exact filesystem already used by the browser. */
export function sftpRetain(id: FsId): Promise<FsId> {
  return invoke('sftp_retain', { id });
}

export interface TextSnapshot { text: string; version: string }
export function sftpReadText(id: FsId, path: string): Promise<TextSnapshot> {
  return invoke('sftp_read_text', { id, path });
}
export function sftpSaveText(id: FsId, path: string, version: string, text: string): Promise<string> {
  return invoke('sftp_save_text', { id, path, version, text });
}

export interface ExternalEditUpdate {
  id: string;
  label: string;
  path: string;
  local_path: string;
  state: 'watching' | 'uploading' | 'saved' | 'conflict' | 'failed' | 'recovered' | 'closed';
  error: CommandError | null;
  revision: number;
}
export function externalEditStart(id: FsId, path: string): Promise<ExternalEditUpdate> {
  return invoke('external_edit_start', { id, path });
}
export function externalEditList(): Promise<ExternalEditUpdate[]> { return invoke('external_edit_list'); }
export function externalEditOpenCopy(editId: string): Promise<void> {
  return invoke('external_edit_open_copy', { editId });
}
export function externalEditRemoteVersion(editId: string, fsId: FsId | null): Promise<string> {
  return invoke('external_edit_remote_version', { editId, fsId });
}
export function externalEditRetry(editId: string, fsId: FsId | null, approvedVersion: string | null = null): Promise<void> {
  return invoke('external_edit_retry', { editId, fsId, approvedVersion });
}
/** The backend permits and completes application exit after the UI protects drafts. */
export function externalEditShutdown(): Promise<void> { return invoke('external_edit_shutdown'); }
export function externalEditSetDirty(dirty: boolean): Promise<void> { return invoke('external_edit_set_dirty', { dirty }); }
export function onExternalEditExitRequest(handler: () => void): Promise<UnlistenFn> {
  return listen('external-edit-exit-request', handler);
}
export function externalEditFinish(editId: string, fsId: FsId | null): Promise<void> {
  return invoke('external_edit_finish', { editId, fsId });
}
export function externalEditDiscard(editId: string): Promise<void> { return invoke('external_edit_discard', { editId }); }
export function externalEditReveal(editId: string): Promise<void> { return invoke('external_edit_reveal', { editId }); }
export function onExternalEditUpdate(handler: (update: ExternalEditUpdate) => void): Promise<UnlistenFn> {
  return listen<ExternalEditUpdate>('external-edit-update', (event) => handler(event.payload));
}

export type Conflict = 'overwrite' | 'rename' | 'skip';

export type TransferState = 'queued' | 'running' | 'done' | 'failed' | 'cancelled';

export interface StagedUpload {
  path: string;
  name: string;
}

export interface TransferUpdate {
  id: TransferId;
  tag: 'upload' | 'download';
  name: string;
  src_label: string;
  src_path: string;
  dst_label: string;
  dst_path: string;
  state: TransferState;
  bytes_done: number;
  bytes_total: number;
  files_done: number;
  files_total: number;
  skipped: boolean;
  error: { kind: string; detail?: string } | null;
}

export function transferPrepareUpload(paths: string[]): Promise<StagedUpload[]> {
  return invoke('transfer_prepare_upload', { paths });
}

export function transferDiscardStaged(paths: string[]): Promise<void> {
  return invoke('transfer_discard_staged', { paths });
}

export function transferUpload(id: FsId, localPaths: string[], remoteDir: string, conflict: Conflict): Promise<TransferId[]> {
  return invoke('transfer_upload', { id, localPaths, remoteDir, conflict });
}

/** Downloads into `localDir`, or into the system download directory when it is null. */
export function transferDownload(id: FsId, remotePaths: string[], localDir: string | null, conflict: Conflict): Promise<TransferId[]> {
  return invoke('transfer_download', { id, remotePaths, localDir, conflict });
}

/** Which of `names` already exist in `localDir` (or the system download directory). */
export function transferLocalClashes(names: string[], localDir: string | null): Promise<string[]> {
  return invoke('transfer_local_clashes', { names, localDir });
}

export function transferList(): Promise<TransferUpdate[]> {
  return invoke('transfer_list');
}

export function transferCancel(id: TransferId): Promise<boolean> {
  return invoke('transfer_cancel', { id });
}

/** Writes a warning or error of the interface to the application log. */
export function logWrite(level: 'warn' | 'error', message: string): Promise<void> {
  return invoke('log_write', { level, message });
}

/** Whether RoveTerm starts (minimized) when the user logs in; `null` where it cannot. */
export function autostartGet(): Promise<boolean | null> {
  return invoke('autostart_get');
}

export function autostartSet(enabled: boolean): Promise<void> {
  return invoke('autostart_set', { enabled });
}

/** Opens the folder of the log files in the system file manager. */
export function logsReveal(): Promise<void> {
  return invoke('logs_reveal');
}

export interface LogFileInfo {
  name: string;
  size_bytes: number;
  modified_at: number;
}

export interface LogReadResult {
  file: string;
  content: string;
  truncated: boolean;
  total_bytes: number;
}

/** Lists available daily log files in the log directory, newest first. */
export function logsList(): Promise<LogFileInfo[]> {
  return invoke('logs_list');
}

/** Reads trailing lines of a log file (or the newest file when `file` is omitted). */
export function logsRead(file: string | null = null, tailLines: number | null = null): Promise<LogReadResult> {
  return invoke('logs_read', { file, tailLines });
}

/** Shows finished download `id` in the system file manager. */
export function transferReveal(id: TransferId): Promise<void> {
  return invoke('transfer_reveal', { id });
}

export function transferExport(id: TransferId): Promise<boolean> {
  return invoke('transfer_export', { id });
}

export function transferClear(): Promise<void> {
  return invoke('transfer_clear');
}

export function onTransferUpdate(handler: (update: TransferUpdate) => void): Promise<UnlistenFn> {
  return listen<TransferUpdate>('transfer-update', (event) => handler(event.payload));
}

export type ForwardId = string;

export type ForwardSpec =
  | { kind: 'local'; bind_host: string; bind_port: number; dest_host: string; dest_port: number }
  | { kind: 'remote'; bind_host: string; bind_port: number; dest_host: string; dest_port: number }
  | { kind: 'dynamic'; bind_host: string; bind_port: number };

export interface ForwardDraft {
  label: string;
  spec: ForwardSpec;
  auto_start: boolean;
}

export interface ForwardRule extends ForwardDraft {
  id: ForwardId;
  host_id: HostId;
  created_at: number;
  updated_at: number;
}

export interface ForwardStatus {
  id: ForwardId;
  host_id: HostId;
  spec: ForwardSpec;
  state: 'running' | 'stopped' | 'failed';
  bound_port: number;
  active: number;
  total: number;
  error: string | null;
}

export interface ForwardEntry extends ForwardRule {
  status: ForwardStatus | null;
  /** The local listener is reachable from other machines. */
  public: boolean;
}

export function forwardList(hostId: HostId | null): Promise<ForwardEntry[]> {
  return invoke('forward_list', { hostId });
}

export function forwardCreate(hostId: HostId, draft: ForwardDraft): Promise<ForwardEntry> {
  return invoke('forward_create', { hostId, draft });
}

export function forwardUpdate(id: ForwardId, draft: ForwardDraft): Promise<ForwardEntry> {
  return invoke('forward_update', { id, draft });
}

export function forwardDelete(id: ForwardId): Promise<void> {
  return invoke('forward_delete', { id });
}

export function forwardStart(id: ForwardId, password: string | null, remember: boolean): Promise<ForwardStatus> {
  return invoke('forward_start', { id, password, remember });
}

export function forwardStop(id: ForwardId): Promise<boolean> {
  return invoke('forward_stop', { id });
}

export function onForwardUpdate(handler: (status: ForwardStatus) => void): Promise<UnlistenFn> {
  return listen<ForwardStatus>('forward-update', (event) => handler(event.payload));
}

// Key library

export type KeyId = string;

export interface SshKey {
  id: KeyId;
  name: string;
  algorithm: string;
  public_key: string;
  fingerprint: string;
  /** `false` after a credential reset: the private key must be imported again. */
  has_private: boolean;
  created_at: number;
  updated_at: number;
}

export type KeyType = 'ed25519' | 'ecdsa_p256' | 'rsa3072';

export function keyList(): Promise<SshKey[]> {
  return invoke('key_list');
}

export function keyGenerate(name: string, kind: KeyType, comment: string, passphrase: string | null): Promise<SshKey> {
  return invoke('key_generate', { name, kind, comment, passphrase });
}

/** Imports a private key; `publicKey` (optional) is kept as written once it matches. */
export function keyImport(name: string, privateKey: string, passphrase: string | null, publicKey: string | null = null): Promise<SshKey> {
  return invoke('key_import', { name, privateKey, passphrase, publicKey });
}

/** Both halves found from one chosen key file; see `key_load_file`. */
export interface LoadedKey {
  private_key: string | null;
  public_key: string | null;
  /** File name without `.pub`, as a suggested key name. */
  name: string;
  /** The private key file, or the chosen file when no private key was found. */
  path: string;
}

/** Reads a private or public key file (detected from its text) and the other half next to it. */
export function keyLoadFile(path: string): Promise<LoadedKey> {
  return invoke('key_load_file', { path });
}

/** Default key files in `~/.ssh` on this device, most preferred first; empty on mobile. */
export function keyDefaultFiles(): Promise<string[]> {
  return invoke('key_default_files');
}

/** Renames a key; with `publicKey` replaces its public key; with `privateKey` replaces both halves. */
export function keyUpdate(
  id: KeyId,
  name: string,
  privateKey: string | null,
  passphrase: string | null,
  publicKey: string | null = null,
): Promise<SshKey> {
  return invoke('key_update', { id, name, privateKey, passphrase, publicKey });
}

export function keyDelete(id: KeyId): Promise<void> {
  return invoke('key_delete', { id });
}

export function keyReveal(id: KeyId): Promise<{ private_key: string; passphrase: string | null }> {
  return invoke('key_reveal', { id });
}

export function keyInstall(id: KeyId, hostId: HostId, password: string | null, passwordKind: PasswordKind): Promise<void> {
  return invoke('key_install', { id, hostId, password, passwordKind });
}

// Server status

export interface CpuTimes {
  total: number;
  idle: number;
}

export interface Disk {
  filesystem: string;
  mount: string;
  total: number;
  used: number;
}

export interface NetInterface {
  name: string;
  rx: number;
  tx: number;
}

/** One status sample; counters are cumulative. See `ServerStats` in rove-ssh. */
export interface ServerStats {
  load: [number, number, number];
  cpu: CpuTimes;
  cpus: CpuTimes[];
  cores: number;
  mem_total: number;
  mem_available: number;
  swap_total: number;
  swap_free: number;
  disks: Disk[];
  uptime_secs: number;
  interfaces: NetInterface[];
  kernel: string;
  hostname: string;
  os: string;
  /** Latency of each path segment in microseconds: one per jump host plus the final hop. */
  latency_us: number[];
  tailscale_path?: TailnetPath | null;
}

export function serverStats(source: StatsSource): Promise<ServerStats> {
  if ('kind' in source && source.kind === 'local') {
    return invoke('server_stats', { hostId: null, address: null, local: true });
  }
  return invoke('server_stats', {
    hostId: 'hostId' in source ? source.hostId : null,
    address: 'address' in source ? source.address : null,
    local: false,
  });
}

// Snippets

export interface Snippet {
  id: string;
  name: string;
  command: string;
  created_at: number;
  updated_at: number;
}

export interface SnippetDraft {
  name: string;
  command: string;
}

export function snippetList(): Promise<Snippet[]> {
  return invoke('snippet_list');
}

export function snippetCreate(draft: SnippetDraft): Promise<Snippet> {
  return invoke('snippet_create', { draft });
}

export function snippetUpdate(id: string, draft: SnippetDraft): Promise<Snippet> {
  return invoke('snippet_update', { id, draft });
}

export function snippetDelete(id: string): Promise<void> {
  return invoke('snippet_delete', { id });
}

// Session restore

export function workspaceSave(snapshot: object): Promise<void> {
  return invoke('workspace_save', { snapshot });
}

export function workspaceLoad(): Promise<object | null> {
  return invoke('workspace_load');
}

/** A menu as sent to the macOS menu bar; see `MenuSpec` in src-tauri. */
export interface NativeMenu {
  label: string;
  /** `accelerator`: the key equivalent the menu shows and handles, such as `Cmd+Shift+I`. */
  items: ({ kind: 'action'; id: string; label: string; accelerator: string | null } | { kind: 'separator' })[];
}

/** Texts of the system-provided macOS menu items, in the interface language. */
export interface NativeMenuLabels {
  about: string;
  settings: string;
  hide: string;
  hide_others: string;
  show_all: string;
  quit: string;
  edit: string;
  undo: string;
  redo: string;
  cut: string;
  copy: string;
  paste: string;
  select_all: string;
  window: string;
  minimize: string;
  zoom: string;
}

/** Installs the macOS menu bar; a no-op elsewhere. */
export function menuSet(menus: NativeMenu[], labels: NativeMenuLabels): Promise<void> {
  return invoke('menu_set', { menus, labels });
}

/** Action ids of chosen macOS menu items. */
export function onMenuAction(handler: (id: string) => void): Promise<UnlistenFn> {
  return listen<string>('menu-action', (event) => handler(event.payload));
}

/** Where a sync space lives; see `SyncTarget` in src-tauri. */
export type SyncTarget = { kind: 'folder'; path: string } | { kind: 'webdav'; url: string; username: string };

export interface SyncStatus {
  /** Storage is chosen on this device. */
  configured: boolean;
  /** The storage can be reached: a folder, or WebDAV with its password on this device. */
  connected: boolean;
  target: SyncTarget | null;
  /** The sync key is on this device and the vault is unlocked. */
  ready: boolean;
  device: string | null;
  /** Records this device has synced. */
  records: number;
}

/** What one sync round did. */
export interface SyncReport {
  segments: number;
  applied: number;
  pushed: number;
}

export function syncStatus(): Promise<SyncStatus> {
  return invoke('sync_status');
}

/** What connecting to the storage found. */
export interface SyncProbe {
  /** A sync space exists there already: the next step joins it. */
  exists: boolean;
}

/** Checks and stores the storage (`storagePassword` is the WebDAV password, sealed on this device). */
export function syncConnect(target: SyncTarget, storagePassword: string | null): Promise<SyncProbe> {
  return invoke('sync_connect', { target, storagePassword });
}

/** Looks again whether a space exists at the connected storage. */
export function syncProbe(): Promise<SyncProbe> {
  return invoke('sync_probe');
}

/** Joins the space at the connected storage, or creates one when there is none; then syncs once. */
export function syncSetup(password: string): Promise<SyncReport> {
  return invoke('sync_setup', { password });
}

export function syncNow(): Promise<SyncReport> {
  return invoke('sync_now');
}

/** Returns the saved WebDAV password on this device, or null when none is stored. */
export function syncRevealSecret(): Promise<string | null> {
  return invoke('sync_reveal_secret');
}

export function syncDisable(): Promise<void> {
  return invoke('sync_disable');
}

// ---- AI ------------------------------------------------------------------------------------

/** Where an AI session works. */
export type AiContext = { kind: 'local' } | { kind: 'host'; host_id: HostId } | { kind: 'quick'; address: string } | { kind: 'fleet' };

/** An agent the user can talk to. */
export interface AgentChoice {
  /** `builtin:<provider id>` or `acp:<agent id>`. */
  agent: string;
  kind: 'builtin' | 'acp';
  name: string;
  /** The model, or the program that runs. */
  detail: string;
  /** Runs through npx (downloaded on first use). */
  via_npx: boolean;
  has_key: boolean;
  /** Offered in the panel; detected agents the user turned off are listed for the settings. */
  enabled: boolean;
  /** Added by the user's own command. */
  custom: boolean;
}

export interface AgentList {
  agents: AgentChoice[];
  default: string | null;
  /** The Jev reviewer's key is stored. */
  jev_key: boolean;
}

/** Stores the Jev reviewer's API key sealed on this device; `null` removes it. */
export function aiSetReviewKey(key: string | null): Promise<void> {
  return invoke('ai_set_review_key', { key });
}

export function aiRevealReviewKey(): Promise<string | null> {
  return invoke('ai_reveal_review_key');
}

/** A mode the agent itself offers (such as read-only). */
export interface AgentMode {
  id: string;
  name: string;
  description: string | null;
}

export interface AiStarted {
  id: string;
  /** The host the session works on. */
  host: string;
  agent: string | null;
  agent_version: string | null;
  modes: AgentMode[];
  current_mode: string | null;
  /** Settings the user can change in the conversation (model, reasoning effort, …). */
  config_options: ConfigOption[];
  /** An earlier conversation was continued. */
  resumed: boolean;
}

/** A setting of a conversation (ACP session config option, or the built-in assistant's model). */
export interface ConfigOption {
  id: string;
  name: string;
  /** `model`, `thought_level`, or another category the agent names. */
  category: string | null;
  kind: 'select' | 'boolean';
  /** `true` / `false` for a boolean setting. */
  current: string;
  choices: { value: string; name: string; description: string | null; group?: string }[];
}

export type Risk = 'read_only' | 'modify' | 'dangerous';

export interface RiskReason {
  code: string;
  detail: string;
}

export interface Assessment {
  risk: Risk;
  programs: string[];
  opaque: boolean;
  reasons: RiskReason[];
}

export type AiOperation =
  | { op: 'command'; command: string }
  | { op: 'read_file'; path: string }
  | { op: 'write_file'; path: string }
  /** `diff`: removed lines after `- `, added after `+ `. */
  | { op: 'edit_file'; path: string; diff: string }
  | { op: 'list_dir'; path: string }
  | { op: 'read_terminal' }
  | { op: 'read_notes' }
  /** `diff`: removed lines after `- `, added after `+ `. */
  | { op: 'update_notes'; diff: string }
  | {
      op: 'agent_tool';
      kind: string;
      title: string;
      command: string | null;
      /** The first file the tool touches. */
      path?: string;
      /** The agent's reason, or the arguments of a tool it calls. */
      detail?: string;
    };

export type ToolContent =
  | { type: 'text'; text: string }
  | { type: 'diff'; path: string; old_text: string | null; new_text: string }
  | { type: 'terminal'; terminal_id: string };

export interface ToolCall {
  id: string;
  title: string | null;
  kind: string | null;
  status: string | null;
  content: ToolContent[];
  locations: string[];
  raw_input: unknown;
}

export interface PlanEntry {
  content: string;
  priority: string;
  status: string;
}

export type SessionUpdate =
  | { kind: 'message'; text: string; message_id: string | null }
  | { kind: 'thought'; text: string }
  | { kind: 'tool_call'; call: ToolCall; changed: boolean }
  | { kind: 'plan'; entries: PlanEntry[] }
  | { kind: 'mode_changed'; mode_id: string }
  | { kind: 'config_options'; options: ConfigOption[] }
  /** How much of the model's context window the conversation fills, in tokens. */
  | { kind: 'usage'; used: number; size: number }
  | { kind: 'commands'; commands: AgentCommand[] }
  | { kind: 'compacted'; summary: string }
  | { kind: 'stats'; input_tokens: number | null; cached_tokens: number | null; output_tokens: number | null; first_token_ms: number | null; tokens_per_second: number | null };

/** A command the agent offers, run by sending `/<name>` as a prompt. */
export interface AgentCommand {
  name: string;
  description: string;
  /** What to type after the name, when it takes input. */
  hint: string | null;
}

/** The built-in assistant's last request: tokens in and out, and how fast they came. */
export type RequestStats = Omit<Extract<SessionUpdate, { kind: 'stats' }>, 'kind'>;

export interface CommandOutput {
  stdout: string;
  stderr: string;
  exit_code: number | null;
  truncated: boolean;
}

export type AiOutcome =
  | { result: 'command'; output: CommandOutput }
  | { result: 'agent_tool' }
  | { result: 'read'; bytes: number; sha256: string; truncated: boolean }
  | { result: 'written'; bytes: number; sha256: string; previous_sha256: string | null; lines_before: number; lines_after: number }
  | { result: 'listed'; entries: number }
  | { result: 'denied' }
  | { result: 'failed'; error: string };

export type AiAnswer = 'allow_once' | 'allow_session' | 'deny';

/** Why the auto mode's judge did not let an operation run, so the user is asked. */
export type ReviewNote =
  | { kind: 'declined'; reviewer: string; confidence: number }
  | { kind: 'no_judge' }
  | { kind: 'failed'; reviewer: string }
  | { kind: 'opaque' }
  | { kind: 'dangerous' };

export type AiEvent =
  | { event: 'update'; update: SessionUpdate }
  | { event: 'approval'; request_id: number; operation: AiOperation; assessment: Assessment; rememberable: boolean; agent_tool: boolean; review: ReviewNote | null }
  | { event: 'approved'; request_id: number; answer: AiAnswer | null }
  /** A reviewer looked at an operation; `allowed` when it runs on the reviewer's word. */
  | { event: 'reviewed'; operation: AiOperation; reviewer: string; allowed: boolean; confidence: number }
  | { event: 'executed'; operation: AiOperation; assessment: Assessment; approved: boolean; outcome: AiOutcome };

/** The agents of this device: the built-in assistant with each provider, then ACP agents. */
export function aiAgents(): Promise<AgentList> {
  return invoke('ai_agents');
}

/** Checks a provider with a tiny request; resolves with the model's reply. `key` defaults to
 *  the stored key of a saved provider. */
export function aiProviderTest(provider: ProviderPrefs, key: string | null): Promise<string> {
  return invoke('ai_provider_test', { provider, key });
}

/** Deletes conversation `id` here and, through sync, on the other devices; stops it first when
 *  it runs. */
export function aiDelete(id: string): Promise<void> {
  return invoke('ai_delete', { id });
}

/** The models `provider` offers; `key` is the typed key, else the stored one. */
export function aiProviderModels(provider: ProviderPrefs, key: string | null): Promise<ListedModel[]> {
  return invoke('ai_provider_models', { provider, key });
}

/** Stores the API key of `agent` sealed on this device; `null` removes it. */
export function aiSetKey(agent: string, key: string | null): Promise<void> {
  return invoke('ai_set_key', { agent, key });
}

export function aiRevealKey(agent: string): Promise<string | null> {
  return invoke('ai_reveal_key', { agent });
}

/** One sign-in method reported for a local ACP agent. */
export interface AcpAuthMethod {
  id: string;
  name: string;
  description: string | null;
  kind: 'agent' | 'terminal' | 'api_key';
  command_preview: string | null;
  fallback: boolean;
}

/** Sign-in options probed for an ACP agent. */
export interface AcpAuthInfo {
  agent: string;
  agent_name: string;
  methods: AcpAuthMethod[];
  probe_error: string | null;
}

/** Probes an ACP agent for sign-in methods (`authMethods` and local CLI fallback). */
export function aiAcpAuthMethods(agent: string): Promise<AcpAuthInfo> {
  return invoke('ai_acp_auth_methods', { agent });
}

/** Runs an ACP agent's protocol-driven `authenticate(methodId)` flow. */
export function aiAcpAuthenticate(agent: string, methodId: string): Promise<void> {
  return invoke('ai_acp_authenticate', { agent, methodId });
}

/** Cancels any in-flight ACP sign-in probe or protocol authentication. */
export function aiAcpCancelAuth(): Promise<void> {
  return invoke('ai_acp_cancel_auth');
}

/** Opens an interactive local PTY running a verified ACP terminal sign-in method. */
export function aiAcpLoginTerminal(
  agent: string,
  methodId: string,
  size: TermSize,
  onOutput: OutputHandler,
): Promise<SessionId> {
  return invoke('ai_acp_login_terminal', { agent, methodId, onOutput: outputChannel(onOutput), ...size });
}

/** Starts `agent` on `context`; `terminal` is the user's terminal session there. */
/** Starts `agent` on `context`; `terminal` is the user's terminal session there. `resume`
 *  continues an earlier conversation of that agent (same id). */
export function aiStart(context: AiContext, agent: string, terminal: SessionId | null, resume: string | null = null, model: string | null = null): Promise<AiStarted> {
  return invoke('ai_start', { context, agent, terminal, resume, model });
}

/** Sets setting `option` (such as `model`) of session `id`; returns the settings as they now are. */
export function aiSelectModel(id: string, provider: string, model: string): Promise<ConfigOption[]> {
  return invoke('ai_select_model', { id, provider, model });
}

export function aiSetOption(id: string, option: string, value: string): Promise<ConfigOption[]> {
  return invoke('ai_set_option', { id, option, value });
}

/** One rate-limit window of a local agent's plan. */
export interface QuotaWindow {
  /** The agent's own name for it; empty when `minutes` describes it. */
  label: string;
  minutes: number | null;
  used_percent: number;
  /** Unix milliseconds. */
  resets_at: number | null;
  resets_text: string | null;
}

export interface AiQuota {
  windows: QuotaWindow[];
  /** How full the context of the agent's latest session is, when its logs say (Codex). */
  context: { used: number; size: number } | null;
  as_of: number;
}

/** Plan usage of local agent `agent` (Codex, Claude Code); `null` for agents that report none. */
export function aiQuota(agent: string, refresh: boolean): Promise<AiQuota | null> {
  return invoke('ai_quota', { agent, refresh });
}

/** Names AI session `id`; returns the title as stored. */
export function aiRename(id: string, title: string): Promise<string> {
  return invoke('ai_rename', { id, title });
}

/** Sends a prompt and resolves with the stop reason when the turn ends. */
export function aiPrompt(id: string, text: string, requestId?: string): Promise<string> {
  return invoke('ai_prompt', { id, text, requestId });
}

export function aiCancel(id: string): Promise<void> {
  return invoke('ai_cancel', { id });
}

export function aiAnswer(id: string, requestId: number, answer: AiAnswer): Promise<boolean> {
  return invoke('ai_answer', { id, requestId, answer });
}

export function aiSetMode(id: string, mode: AiMode): Promise<void> {
  return invoke('ai_set_mode', { id, mode });
}

export function aiStop(id: string): Promise<void> {
  return invoke('ai_stop', { id });
}

/** Lifecycle of an automatic fleet lead summary. */
export type TurnEvent = {
  id: string;
  active: boolean;
  kind: 'ops_wakeup';
  stop_reason?: string;
  error?: { code: string; message: string };
};

export function onAiTurn(handler: (event: TurnEvent) => void): Promise<UnlistenFn> {
  return listen<TurnEvent>('ai-turn', (event) => handler(event.payload));
}

/** An agent the user runs started a conversation on a host, or one ended. */
export type ExternalEvent = { id: string; context: AiContext; agent: string; host: string } | { id: string; ended: true };

/** A sub-agent of the fleet lead started on a host, or one ended. */
export type SubtaskEndState = 'completed' | 'failed' | 'blocked' | 'cancelled' | 'interrupted';
export type SubtaskEvent =
  | { parent: string; id: string; host: string; host_id: string; task: string; task_id: string; subtask_id: string }
  | { id: string; parent?: string | null; ended: true; state?: SubtaskEndState; summary?: string | null };

export function onAiSubtask(handler: (event: SubtaskEvent) => void): Promise<UnlistenFn> {
  return listen<SubtaskEvent>('ai-subtask', (event) => handler(event.payload));
}

export function onAiExternal(handler: (event: ExternalEvent) => void): Promise<UnlistenFn> {
  return listen<ExternalEvent>('ai-external', (event) => handler(event.payload));
}

/** Whether agents the user runs may use the saved hosts, and the command they start. */
export interface ExternalStatus {
  enabled: boolean;
  config: string;
  command: string[];
}

export function externalStatus(): Promise<ExternalStatus> {
  return invoke('external_status');
}

export function externalSet(enabled: boolean): Promise<ExternalStatus> {
  return invoke('external_set', { enabled });
}

/** Adds RoveTerm's MCP server to `client` (`claude` or `codex`); resolves with its output. */
export function externalInstall(client: 'claude' | 'codex'): Promise<string> {
  return invoke('external_install', { client });
}

export function onAiUpdate(handler: (id: string, event: AiEvent) => void): Promise<UnlistenFn> {
  return listen<{ id: string; event: AiEvent }>('ai-update', (event) => handler(event.payload.id, event.payload.event));
}

/** A past AI session. */
export type HistoryRole = 'fleet' | 'assistant' | 'subagent' | 'unknown';
export interface AiSessionSummary {
  role: HistoryRole;
  children: AiSessionSummary[];
  model: string | null;
  id: string;
  /** Unix milliseconds. */
  started: number;
  host: string | null;
  agent: string | null;
  /** Given by the user, or the first question. */
  title: string | null;
  /** The agent to resume with; absent for conversations recorded before resuming existed. */
  agent_ref: string | null;
  context: AiContext | null;
}

/** One audit record of an AI session. */
export interface AiRecord {
  ts: number;
  kind: string;
  payload: Record<string, unknown>;
}

/** Recent AI sessions, newest first. */
export function aiSessions(role: HistoryRole | null = null): Promise<AiSessionSummary[]> {
  return invoke('ai_sessions', { role });
}

/** The audit records of AI session `id`, oldest first. */
export function aiAuditDetail(id: string, reference: string): Promise<Record<string, unknown>> {
  return invoke('ai_audit_detail', { id, reference });
}

export function aiRecords(id: string): Promise<AiRecord[]> {
  return invoke('ai_records', { id });
}

export type UpdateInstallMode = 'in_app' | 'external';

export interface UpdateInfo {
  current_version: string;
  available: boolean;
  version: string | null;
  notes: string;
  published_at: string | null;
  release_url: string;
  install_mode: UpdateInstallMode;
  asset_sha256?: string | null;
  asset_size?: number | null;
}

export type UpdateProgress =
  | { stage: 'started'; total: number | null }
  | { stage: 'downloading'; downloaded: number; total: number | null }
  | { stage: 'installing' }
  | { stage: 'finished' };

export function updateCheck(): Promise<UpdateInfo> {
  return invoke('update_check');
}

export function updateInstall(version: string, onProgress: (progress: UpdateProgress) => void): Promise<void> {
  const channel = new Channel<UpdateProgress>();
  channel.onmessage = onProgress;
  return invoke('update_install', { version, onProgress: channel });
}

export function updateRestart(): Promise<void> {
  return invoke('update_restart');
}

export function updateOpenRelease(url: string): Promise<void> {
  return invoke('update_open_release', { url });
}
