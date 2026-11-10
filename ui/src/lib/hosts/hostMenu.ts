// The menu of a saved host, shared by the sidebar and the home page.
import type { MenuItem } from '../ui/ContextMenu.svelte';
import { ask } from '../ui/confirm.svelte';
import { errorText, t } from '../i18n';
import { hostCreate, type HostEntry } from '../ipc';
import { hosts } from './hosts.svelte';
import { workspace } from '../workspace/workspace.svelte';

export interface HostMenuHandlers {
  onconnect: (host: HostEntry) => void;
  onfiles: (host: HostEntry) => void;
  onforwards: (host: HostEntry) => void;
  onedit: (host: HostEntry) => void;
  /** Reports a failure to the user. */
  onnotice: (message: string) => void;
}

/** Saves a copy of `host` without its stored secret. */
async function duplicate(host: HostEntry, onnotice: (message: string) => void): Promise<void> {
  try {
    const { group, hostname, port, username, auth, route, tags } = host;
    await hostCreate({ name: t('host.copyName', { name: host.name }), group, hostname, port, username, auth, route, tags }, null);
    await hosts.refresh();
  } catch (error) {
    onnotice(errorText(error));
  }
}

/** Deletes `host` after confirmation and closes its tabs, which could no longer reconnect. */
async function remove(host: HostEntry, onnotice: (message: string) => void): Promise<void> {
  const open = workspace.tabsOfHost(host.id).length;
  const message = t('host.confirmDelete', { name: host.name }) + (open > 0 ? ` ${t('host.confirmDeleteTabs', { count: open })}` : '');
  const confirmed = await ask({ title: t('host.delete'), message, confirmLabel: t('common.delete'), danger: true });
  if (!confirmed) return;
  try {
    await hosts.remove(host.id);
  } catch (error) {
    onnotice(errorText(error));
    return;
  }
  for (const paneId of workspace.tabsOfHost(host.id)) workspace.closePane(paneId);
}

export function hostMenuItems(host: HostEntry, handlers: HostMenuHandlers): MenuItem[] {
  return [
    { label: t('host.connect'), icon: 'terminal', onselect: () => handlers.onconnect(host) },
    { label: t('host.openFiles'), icon: 'files', onselect: () => handlers.onfiles(host) },
    { label: t('host.forwards'), icon: 'forwards', onselect: () => handlers.onforwards(host) },
    { label: t('host.edit'), icon: 'edit', separated: true, onselect: () => handlers.onedit(host) },
    { label: t('host.duplicate'), icon: 'copy', onselect: () => void duplicate(host, handlers.onnotice) },
    { label: t('common.delete'), icon: 'remove', danger: true, separated: true, onselect: () => void remove(host, handlers.onnotice) },
  ];
}
