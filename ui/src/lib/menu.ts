// The application menus: the title bar menus on Windows and Linux and the menu bar on macOS.
// Items name actions; their labels and shortcuts come from the actions table.
import type { ActionId } from './actions';
import type { MessageKey } from './i18n';

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
