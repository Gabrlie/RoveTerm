// How a saved host is reached, in words, for the line under its name. The address itself stays
// out of the list (it shows on hover) so screenshots and screen sharing do not expose it.
import { t } from '../i18n';
import type { HostEntry } from '../ipc';
import { hosts } from './hosts.svelte';

/** `SSH`, `via bastion` or `SOCKS5 proxy`. */
export function routeLabel(host: HostEntry): string {
  const { route } = host;
  if (route.kind === 'jump') return t('host.via', { name: hosts.find(route.host)?.name ?? '?' });
  if (route.kind === 'proxy') return t('host.viaProxy', { protocol: route.protocol.toUpperCase() });
  return 'SSH';
}

/** `SSH · root`: the route and the login user. */
export function hostSubtitle(host: HostEntry): string {
  return `${routeLabel(host)} · ${host.username}`;
}

/** The full address with the route, for the hover text. */
export function hostAddress(host: HostEntry): string {
  const address = `${host.username}@${host.hostname}:${host.port}`;
  return host.route.kind === 'direct' ? address : `${address} (${routeLabel(host)})`;
}
