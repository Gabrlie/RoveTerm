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

export interface StatusPrefs {
  interval_secs: number;
}

/** Preferences that follow the user; see `Preferences` in rove-vault. */
export interface Preferences {
  appearance: AppearancePrefs;
  terminal: TerminalPrefs;
  files: FilePrefs;
  status: StatusPrefs;
}

export function prefsGet(): Promise<Preferences> {
  return invoke('prefs_get');
}

export function prefsSet(prefs: Preferences): Promise<Preferences> {
  return invoke('prefs_set', { prefs });
}

/** Preferences of this device only; see `DevicePrefs` in rove-vault. */
export interface DevicePrefs {
  window_material: boolean;
  /** Absolute path; empty uses the system download directory. */
  download_dir: string;
  max_transfers: number;
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

/** A menu as sent to the macOS menu bar; see `MenuSpec` in src-tauri. */
export interface NativeMenu {
  label: string;
  items: ({ kind: 'action'; id: string; label: string } | { kind: 'separator' })[];
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
  configured: boolean;
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

/** Creates a space at `target` (`create`) or joins the one there, then syncs once.
 *  `storagePassword` is the WebDAV password, stored sealed on this device. */
export function syncSetup(target: SyncTarget, password: string, create: boolean, storagePassword: string | null = null): Promise<SyncReport> {
  return invoke('sync_setup', { target, password, create, storagePassword });
}

export function syncNow(): Promise<SyncReport> {
  return invoke('sync_now');
}

export function syncDisable(): Promise<void> {
  return invoke('sync_disable');
}
