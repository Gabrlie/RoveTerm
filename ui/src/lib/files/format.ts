// Pure helpers for the file browser: POSIX paths, sizes, permissions and ordering.
import type { DirEntry } from '../ipc';

// Matches rove_fs::TEXT_FILE_LIMIT.
const TEXT_FILE_LIMIT = 2 * 1024 * 1024;
const BINARY_EXTENSIONS = new Set(`
  zip tar gz tgz bz2 xz zst 7z rar deb rpm apk dmg iso img pkg
  png jpg jpeg gif webp bmp ico heic avif tiff psd
  mp3 wav flac ogg aac m4a mp4 mkv avi mov webm
  pdf doc docx xls xlsx ppt pptx exe dll so dylib bin o a wasm class jar pyc
  sqlite sqlite3 db woff woff2 ttf otf eot
`.trim().split(/\s+/));
const TEXT_EXTENSIONS = new Set(`
  txt md markdown rst adoc org log out csv tsv json jsonc json5 jsonl yaml yml toml ini
  cfg conf cnf properties prop env xml plist svg html htm xhtml css scss sass less
  js mjs cjs jsx ts mts cts tsx svelte vue astro sh bash zsh fish ksh csh ps1 bat cmd
  py pyi rb pl pm php lua rs go mod sum c h cc cpp cxx hpp hh java kt kts swift cs fs
  scala zig nim ex exs erl hs ml clj r sql graphql gql proto tf tfvars hcl nix vim el
  service socket timer target mount path slice rules repo list sources diff patch tex bib
  gitignore gitattributes gitmodules dockerignore editorconfig npmrc yarnrc eslintrc prettierrc
  htaccess htpasswd bashrc bash_profile zshrc zprofile profile vimrc inputrc ssh_config
  authorized_keys known_hosts pem pub crt csr key
`.trim().split(/\s+/));
const TEXT_FILENAMES = new Set(`
  dockerfile containerfile makefile gnumakefile rakefile gemfile procfile caddyfile justfile
  vagrantfile jenkinsfile cmakelists.txt readme license licence copying changelog authors notice
  hosts hostname fstab crontab passwd group resolv.conf issue motd sudoers exports interfaces
  profile environment authorized_keys known_hosts config id_rsa.pub id_ed25519.pub
`.trim().split(/\s+/));

/** Selects regular files up to 2 MiB with a recognized text name for the default Open action.
 * The server validates the size and text contents when the editor reads the file. */
export function isEditableTextFile(entry: DirEntry): boolean {
  if (entry.kind !== 'file' || entry.size > TEXT_FILE_LIMIT) return false;
  const name = entry.name.toLowerCase();
  const dot = name.lastIndexOf('.');
  const extension = dot < 0 ? '' : name.slice(dot + 1);
  if (BINARY_EXTENSIONS.has(extension)) return false;
  return TEXT_EXTENSIONS.has(extension) || TEXT_FILENAMES.has(name) || name.startsWith('.env.');
}

/** Joins a remote directory and a name: `('/', 'a')` → `/a`. */
export function joinPath(dir: string, name: string): string {
  return dir.endsWith('/') ? `${dir}${name}` : `${dir}/${name}`;
}

/** Parent of a remote path, or null at the root: `/a/b` → `/a`, `/a` → `/`. */
export function parentPath(path: string): string | null {
  const trimmed = path.replace(/\/+$/, '');
  if (trimmed === '') return null;
  const index = trimmed.lastIndexOf('/');
  if (index < 0) return null;
  return index === 0 ? '/' : trimmed.slice(0, index);
}

/** Normalizes a typed path: collapses repeated slashes, resolves `.` and `..`. */
export function normalizePath(path: string): string {
  const parts: string[] = [];
  for (const part of path.split('/')) {
    if (part === '' || part === '.') continue;
    if (part === '..') parts.pop();
    else parts.push(part);
  }
  return `/${parts.join('/')}`;
}

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

/** Formats a byte count with one decimal above 1 KB, such as `1.5 MB`. */
export function formatSize(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${value} B` : `${value.toFixed(1)} ${UNITS[unit]}`;
}

/** Formats permission bits like `ls -l`: `rwxr-xr-x`. */
export function formatMode(mode: number | null): string {
  if (mode === null) return '';
  const bits = 'rwxrwxrwx';
  let text = '';
  for (let i = 0; i < 9; i += 1) text += mode & (1 << (8 - i)) ? bits[i] : '-';
  return text;
}

/** Formats Unix seconds in the user's locale. */
export function formatTime(seconds: number | null): string {
  if (seconds === null) return '';
  return new Date(seconds * 1000).toLocaleString(undefined, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

/** Directories first, then names in natural, case-insensitive order. */
export function sortEntries(entries: DirEntry[]): DirEntry[] {
  const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });
  return [...entries].sort((a, b) => {
    const dirA = a.kind === 'dir' ? 0 : 1;
    const dirB = b.kind === 'dir' ? 0 : 1;
    return dirA - dirB || collator.compare(a.name, b.name);
  });
}

/** Last component of a local or remote path. */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '');
  const index = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  return index < 0 ? trimmed : trimmed.slice(index + 1);
}
