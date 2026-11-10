// How a saved host is reached, as a string: views of a host's files or state are kept per host,
// and must start over when its address, user, port or route (or a jump host's) changes.
import type { HostEntry } from '../ipc';

/** Jump hosts followed; deeper chains (or loops) stop here. */
const MAX_HOPS = 4;

export function reachKey(id: string, find: (id: string) => HostEntry | undefined, hops = 0): string {
  const host = find(id);
  if (!host) return id;
  const route = host.route;
  let via = '';
  if (route.kind === 'proxy') via = ` via ${route.protocol}://${route.host}:${route.port}`;
  if (route.kind === 'jump') via = hops < MAX_HOPS ? ` via [${reachKey(route.host, find, hops + 1)}]` : ` via ${route.host}`;
  return `${id}=${host.username}@${host.hostname}:${host.port}${via}`;
}
