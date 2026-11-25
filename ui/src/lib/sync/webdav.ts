// WebDAV addresses as the sync settings build them.

export const MASKED_PASSWORD_PLACEHOLDER = '••••••••';

export function webdavPasswordPlaceholder({ editing, isWebdav, connected }: {
  editing: boolean;
  isWebdav: boolean;
  connected: boolean;
}): string {
  return editing && isWebdav && connected ? MASKED_PASSWORD_PLACEHOLDER : '';
}

export function shouldClearHiddenWebdavPassword(loadedSavedPassword: boolean): boolean {
  return loadedSavedPassword;
}

/** The WebDAV directory URL for a server address and a folder name on it. */
export function webdavUrl(server: string, folder: string): string {
  const base = server.trim().replace(/\/+$/, '');
  const path = folder
    .trim()
    .split('/')
    .filter((part) => part.length > 0)
    .map((part) => encodeURIComponent(part))
    .join('/');
  return path ? `${base}/${path}` : base;
}

/** Splits a stored WebDAV directory URL back into server base URL and remote folder name
 *  so editing an existing WebDAV configuration prefills both fields cleanly. */
export function splitWebdavUrl(fullUrl: string): { server: string; folder: string } {
  const trimmed = fullUrl.trim().replace(/\/+$/, '');
  try {
    const parsed = new URL(trimmed);
    const segments = parsed.pathname.split('/').filter((part) => part.length > 0);
    if (segments.length === 0) {
      return { server: `${parsed.origin}/`, folder: '' };
    }
    const last = segments.pop() ?? '';
    let folder = last;
    try {
      folder = decodeURIComponent(last);
    } catch {
      folder = last;
    }
    const prefix = segments.length > 0 ? `/${segments.join('/')}/` : '/';
    return { server: `${parsed.origin}${prefix}`, folder };
  } catch {
    return { server: trimmed, folder: '' };
  }
}
