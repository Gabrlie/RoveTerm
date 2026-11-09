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
  /** Last successful connection, Unix milliseconds. */
  last_connected_at: number | null;
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

export function keyReadFile(path: string): Promise<string> {
  return invoke('key_read_file', { path });
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
}

export function serverStats(source: RemoteSource): Promise<ServerStats> {
  return invoke('server_stats', {
    hostId: 'hostId' in source ? source.hostId : null,
    address: 'address' in source ? source.address : null,
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

/** Turns the translucent window material on or off; resolves to whether it is now shown. */
export function windowSetMaterial(enabled: boolean): Promise<boolean> {
  return invoke('window_set_material', { enabled });
}
