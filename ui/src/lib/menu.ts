// The application menus: the title bar menus on Windows and Linux and the menu bar on macOS.
// Items name actions; their labels and shortcuts come from the actions table.
import { actions, type ActionId } from './actions';
import { t, type MessageKey } from './i18n';
import type { NativeMenu, NativeMenuLabels } from './ipc';

/** An action, or `-` for a separator. */
export type MenuEntry = ActionId | '-';

export interface Menu {
  id: string;
  label: MessageKey;
  items: MenuEntry[];
}

export const MENUS: Menu[] = [
  {
    id: 'file',
    label: 'menu.file',
    items: ['tab.newLocal', 'host.new', 'host.quickConnect', 'host.import', '-', 'settings.open', '-', 'tab.close'],
  },
  { id: 'edit', label: 'menu.edit', items: ['terminal.copy', 'terminal.paste'] },
  {
    id: 'view',
    label: 'menu.view',
    items: [
      'palette.open',
      '-',
      'sidebar.toggle',
      'panel.right',
      'panel.bottom',
      '-',
      'pane.splitRight',
      'pane.splitDown',
      'pane.focusNext',
      'pane.focusPrev',
      '-',
      'tab.next',
      'tab.prev',
    ],
  },
  { id: 'host', label: 'menu.host', items: ['files.open', 'forward.manage', 'keys.manage'] },
  { id: 'help', label: 'menu.help', items: ['help.shortcuts', 'help.about'] },
];

/** The menus for the macOS menu bar, labelled in the current language. The Edit menu is left
 *  out: the system's own Edit menu keeps native copy and paste in text fields. */
export function nativeMenus(): NativeMenu[] {
  return MENUS.filter((menu) => menu.id !== 'edit').map((menu) => ({
    label: t(menu.label),
    items: menu.items.map((entry) => {
      if (entry === '-') return { kind: 'separator' as const };
      const action = actions.find((candidate) => candidate.id === entry);
      return { kind: 'action' as const, id: entry, label: action ? t(action.label) : entry };
    }),
  }));
}

export function nativeMenuLabels(): NativeMenuLabels {
  return {
    about: t('action.help.about'),
    settings: t('native.settings'),
    hide: t('native.hide'),
    hide_others: t('native.hideOthers'),
    show_all: t('native.showAll'),
    quit: t('native.quit'),
    edit: t('menu.edit'),
    undo: t('native.undo'),
    redo: t('native.redo'),
    cut: t('native.cut'),
    copy: t('native.copy'),
    paste: t('native.paste'),
    select_all: t('native.selectAll'),
    window: t('native.window'),
    minimize: t('window.minimize'),
    zoom: t('native.zoom'),
  };
}

/** Whether `id` names an action, for ids arriving from the menu bar. */
export function isActionId(id: string): id is ActionId {
  return actions.some((action) => action.id === id);
}
