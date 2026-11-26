import type { MessageKey } from '../i18n';
import type { IconName } from '../ui/icons';

export type SettingsSection =
  | 'appearance' | 'terminal' | 'files' | 'status' | 'ai' | 'sync'
  | 'security' | 'tailscale' | 'shortcuts' | 'logs' | 'about';

export type SettingsGroupId = 'connect' | 'experience' | 'assistant';

export interface SettingsSectionEntry {
  id: SettingsSection;
  label: MessageKey;
  icon: IconName;
  tint: string;
  group: SettingsGroupId;
}

const SECTIONS: readonly SettingsSectionEntry[] = [
  { id: 'appearance', label: 'settings.section.appearance', icon: 'palette', tint: 'blue', group: 'experience' },
  { id: 'terminal', label: 'settings.section.terminal', icon: 'terminal', tint: 'graphite', group: 'experience' },
  { id: 'files', label: 'settings.section.files', icon: 'folder', tint: 'teal', group: 'experience' },
  { id: 'status', label: 'settings.section.status', icon: 'pulse', tint: 'green', group: 'experience' },
  { id: 'ai', label: 'settings.section.ai', icon: 'command', tint: 'pink', group: 'assistant' },
  { id: 'sync', label: 'settings.section.sync', icon: 'cloud', tint: 'teal', group: 'connect' },
  { id: 'security', label: 'settings.section.security', icon: 'lock', tint: 'green', group: 'connect' },
  { id: 'tailscale', label: 'settings.section.tailscale', icon: 'network', tint: 'blue', group: 'connect' },
  { id: 'shortcuts', label: 'settings.section.shortcuts', icon: 'keyboard', tint: 'orange', group: 'experience' },
  { id: 'logs', label: 'settings.section.logs', icon: 'snippets', tint: 'graphite', group: 'assistant' },
  { id: 'about', label: 'settings.section.about', icon: 'info', tint: 'purple', group: 'assistant' },
];

const GROUPS: readonly { id: SettingsGroupId; label: MessageKey }[] = [
  { id: 'connect', label: 'mobile.settings.groupConnect' },
  { id: 'experience', label: 'mobile.settings.groupExperience' },
  { id: 'assistant', label: 'mobile.settings.groupAssistant' },
];

export function settingsSections({ biometricVault, includeShortcuts = true }: {
  biometricVault: boolean;
  includeShortcuts?: boolean;
}): SettingsSectionEntry[] {
  return SECTIONS.filter(({ id }) => {
    if (id === 'security') return biometricVault;
    if (id === 'shortcuts') return includeShortcuts;
    return true;
  });
}

export function groupSettingsSections(entries: readonly SettingsSectionEntry[]): {
  id: SettingsGroupId;
  label: MessageKey;
  items: SettingsSectionEntry[];
}[] {
  return GROUPS.map((group) => ({
    ...group,
    items: entries.filter((entry) => entry.group === group.id),
  })).filter((group) => group.items.length > 0);
}
