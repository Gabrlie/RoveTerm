// WebDAV addresses as the sync settings build them.

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
