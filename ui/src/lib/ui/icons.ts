// Interface icons, named by meaning. Only Remix Icon 4.8.0 (Apache-2.0) line icons are used.
import add from 'remixicon/icons/System/add-line.svg?raw';
import close from 'remixicon/icons/System/close-line.svg?raw';
import splitRight from 'remixicon/icons/Design/layout-column-line.svg?raw';
import splitDown from 'remixicon/icons/Design/layout-row-line.svg?raw';
import terminal from 'remixicon/icons/Development/terminal-box-line.svg?raw';
import server from 'remixicon/icons/Device/server-line.svg?raw';
import search from 'remixicon/icons/System/search-line.svg?raw';
import lock from 'remixicon/icons/System/lock-line.svg?raw';
import unlock from 'remixicon/icons/System/lock-unlock-line.svg?raw';
import key from 'remixicon/icons/Others/key-2-line.svg?raw';
import edit from 'remixicon/icons/Design/pencil-line.svg?raw';
import remove from 'remixicon/icons/System/delete-bin-line.svg?raw';
import importConfig from 'remixicon/icons/System/download-2-line.svg?raw';
import quickConnect from 'remixicon/icons/Weather/flashlight-line.svg?raw';
import command from 'remixicon/icons/Development/command-line.svg?raw';
import more from 'remixicon/icons/System/more-2-line.svg?raw';
import chevron from 'remixicon/icons/Arrows/arrow-right-s-line.svg?raw';

export const icons = {
  add,
  close,
  splitRight,
  splitDown,
  terminal,
  server,
  search,
  lock,
  unlock,
  key,
  edit,
  remove,
  importConfig,
  quickConnect,
  command,
  more,
  chevron,
} as const;

export type IconName = keyof typeof icons;
