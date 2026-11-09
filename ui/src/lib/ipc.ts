// Typed wrappers around the Tauri commands. Components call these instead of `invoke` directly.
import { Channel, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

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
export type OutputMessage = ArrayBuffer | { exit: ExitStatus };

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
  remember: boolean,
  size: TermSize,
  onOutput: OutputHandler,
): Promise<SessionId> {
  return invoke('session_open_ssh', { onOutput: outputChannel(onOutput), hostId, password, remember, ...size });
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
  | { method: 'key_file'; path: string }
  | { method: 'agent' }
  | { method: 'interactive' };

export type ProxyProtocol = 'http' | 'socks5';

/** How the connection to a host is made; see `HostRoute` in rove-vault. */
export type HostRoute =
  | { kind: 'direct' }
  | { kind: 'jump'; host: HostId }
  | { kind: 'proxy'; protocol: ProxyProtocol; host: string; port: number };

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

export function hostImportSshConfig(): Promise<ImportReport> {
  return invoke('host_import_ssh_config');
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

/** Terminal appearance; see `TerminalPrefs` in rove-vault. */
export interface TerminalPrefs {
  /** Empty uses the built-in font stack. */
  font_family: string;
  font_size: number;
  line_height: number;
}

export interface Preferences {
  terminal: TerminalPrefs;
}

export function prefsGet(): Promise<Preferences> {
  return invoke('prefs_get');
}

export function prefsSet(prefs: Preferences): Promise<Preferences> {
  return invoke('prefs_set', { prefs });
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

export interface SftpOpened {
  id: FsId;
  label: string;
  home: string;
}

export function sftpOpen(source: RemoteSource, password: string | null, remember: boolean): Promise<SftpOpened> {
  return invoke('sftp_open', {
    hostId: 'hostId' in source ? source.hostId : null,
    address: 'address' in source ? source.address : null,
    password,
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

export type Conflict = 'overwrite' | 'rename' | 'skip';

export type TransferState = 'queued' | 'running' | 'done' | 'failed' | 'cancelled';

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

export function transferUpload(id: FsId, localPaths: string[], remoteDir: string, conflict: Conflict): Promise<TransferId[]> {
  return invoke('transfer_upload', { id, localPaths, remoteDir, conflict });
}

/** Downloads into `localDir`, or into the system download directory when it is null. */
export function transferDownload(id: FsId, remotePaths: string[], localDir: string | null, conflict: Conflict): Promise<TransferId[]> {
  return invoke('transfer_download', { id, remotePaths, localDir, conflict });
}

export function transferList(): Promise<TransferUpdate[]> {
  return invoke('transfer_list');
}

export function transferCancel(id: TransferId): Promise<boolean> {
  return invoke('transfer_cancel', { id });
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
