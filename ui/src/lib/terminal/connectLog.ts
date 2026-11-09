// Formats connection steps as terminal lines, like a verbose ssh client.
import type { ConnectStep } from '../ipc';
import { t } from '../i18n';

const DIM = '\x1b[2m';
const GREEN = '\x1b[32m';
const RED = '\x1b[31m';
const RESET = '\x1b[0m';

/** One terminal line (with CRLF) describing `step`. */
export function formatStep(step: ConnectStep): string {
  switch (step.step) {
    case 'connecting':
      return line('›', DIM, step.via ? t('connect.connectingVia', { address: step.address, via: step.via }) : t('connect.connecting', { address: step.address }));
    case 'handshake':
      return line('›', DIM, t('connect.handshake'));
    case 'host_key_accepted':
      return line('✓', GREEN, t('connect.hostKey', { algorithm: step.algorithm, fingerprint: step.fingerprint }));
    case 'agent_unavailable':
      return line('!', DIM, t('connect.agentUnavailable', { reason: step.reason }));
    case 'trying':
      return line('›', DIM, t(`connect.trying.${step.method}`, { label: step.label ?? '' }));
    case 'rejected':
      return line('✗', RED, t('connect.rejected'));
    case 'authenticated':
      return line('✓', GREEN, t('connect.authenticated')) + '\r\n';
  }
}

function line(mark: string, color: string, text: string): string {
  return `${color}${mark}${RESET} ${DIM}${text}${RESET}\r\n`;
}
