import type { HostEntry } from '../ipc';

/** Whether `host` reaches its destination over Tailscale, directly or through a jump host chain. */
export function usesTailscaleRoute(host: HostEntry | null | undefined, findHost: (id: string) => HostEntry | undefined): boolean {
  let current: HostEntry | undefined = host ?? undefined;
  const seen = new Set<string>();
  while (current && !seen.has(current.id)) {
    seen.add(current.id);
    if (current.route.kind === 'tailscale') return true;
    if (current.route.kind !== 'jump') return false;
    current = findHost(current.route.host);
  }
  return false;
}

/** Whether `code` is a Tailscale transport or startup error code. */
export function isTailscaleFailureCode(code: string | null | undefined): boolean {
  return typeof code === 'string' && code.startsWith('tailscale.');
}

/** Whether a failed pane should offer "Open Tailscale settings" instead of password fallback. */
export function showTailscaleSettingsAction(params: {
  isMobile: boolean;
  isTailscaleHost: boolean;
  tailscaleConnected: boolean;
  failureCode: string;
}): boolean {
  if (params.isMobile) return false;
  if (isTailscaleFailureCode(params.failureCode)) return true;
  if (!params.isTailscaleHost) return false;
  return !params.tailscaleConnected || params.failureCode === 'ssh.connect' || params.failureCode === 'ssh.timeout';
}

/** Whether a failed pane should offer the "Sign in with password" fallback button. */
export function showPasswordFallbackAction(params: {
  isRemote: boolean;
  isTailscaleHost: boolean;
  tailscaleConnected: boolean;
  failureCode: string;
}): boolean {
  if (!params.isRemote) return false;
  if (isTailscaleFailureCode(params.failureCode)) return false;
  if (params.isTailscaleHost && !params.tailscaleConnected) return false;
  return params.failureCode !== 'ssh.connect' && params.failureCode !== 'ssh.timeout';
}

/**
 * Decides whether a failed pane should automatically retry now that Tailscale is connected.
 *
 * Retries once when Tailscale transitions to `running` after a Tailscale/connect failure, and
 * also covers the startup race where `tailscale.not_connected` arrives just after Tailscale
 * already reported `running` (guarded by `alreadyRetriedForEpoch` so it can never loop).
 */
export function shouldAutoRetryTailscale(params: {
  isTailscaleHost: boolean;
  failureCode: string | null;
  connected: boolean;
  wasConnected: boolean;
  alreadyRetriedForEpoch: boolean;
}): boolean {
  const { isTailscaleHost, failureCode, connected, wasConnected, alreadyRetriedForEpoch } = params;
  if (!connected || !failureCode || alreadyRetriedForEpoch) return false;
  if (!wasConnected) {
    if (isTailscaleFailureCode(failureCode)) return true;
    return isTailscaleHost && (failureCode === 'ssh.connect' || failureCode === 'ssh.timeout');
  }
  return failureCode === 'tailscale.not_connected';
}
