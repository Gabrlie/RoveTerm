// Interface icons, named by meaning. Only Lucide 1.61.0 (ISC) icons are used: rounded stroke
// caps and joins, drawn with `currentColor`.
import add from 'lucide-static/icons/plus.svg?raw';
import close from 'lucide-static/icons/x.svg?raw';
import splitRight from 'lucide-static/icons/columns-2.svg?raw';
import splitDown from 'lucide-static/icons/rows-2.svg?raw';
import terminal from 'lucide-static/icons/square-terminal.svg?raw';
import server from 'lucide-static/icons/server.svg?raw';
import search from 'lucide-static/icons/search.svg?raw';
import lock from 'lucide-static/icons/lock.svg?raw';
import unlock from 'lucide-static/icons/lock-open.svg?raw';
import key from 'lucide-static/icons/key-round.svg?raw';
import edit from 'lucide-static/icons/pencil.svg?raw';
import remove from 'lucide-static/icons/trash-2.svg?raw';
import importConfig from 'lucide-static/icons/file-down.svg?raw';
import quickConnect from 'lucide-static/icons/zap.svg?raw';
import command from 'lucide-static/icons/command.svg?raw';
import more from 'lucide-static/icons/ellipsis.svg?raw';
import chevron from 'lucide-static/icons/chevron-right.svg?raw';
import chevronDown from 'lucide-static/icons/chevron-down.svg?raw';
import chevronsUpDown from 'lucide-static/icons/chevrons-up-down.svg?raw';
import check from 'lucide-static/icons/check.svg?raw';
import folder from 'lucide-static/icons/folder.svg?raw';
import file from 'lucide-static/icons/file.svg?raw';
import folderAdd from 'lucide-static/icons/folder-plus.svg?raw';
import folderUpload from 'lucide-static/icons/folder-up.svg?raw';
import folderDownload from 'lucide-static/icons/folder-down.svg?raw';
import files from 'lucide-static/icons/folder-sync.svg?raw';
import transfer from 'lucide-static/icons/arrow-down-up.svg?raw';
import upload from 'lucide-static/icons/upload.svg?raw';
import download from 'lucide-static/icons/download.svg?raw';
import refresh from 'lucide-static/icons/refresh-cw.svg?raw';
import up from 'lucide-static/icons/arrow-up.svg?raw';
import eye from 'lucide-static/icons/eye.svg?raw';
import eyeOff from 'lucide-static/icons/eye-off.svg?raw';
import link from 'lucide-static/icons/link.svg?raw';
import clear from 'lucide-static/icons/eraser.svg?raw';
import forwards from 'lucide-static/icons/arrow-left-right.svg?raw';
import play from 'lucide-static/icons/play.svg?raw';
import stop from 'lucide-static/icons/square.svg?raw';
import warning from 'lucide-static/icons/triangle-alert.svg?raw';
import settings from 'lucide-static/icons/settings.svg?raw';
import panelRight from 'lucide-static/icons/panel-right.svg?raw';
import panelBottom from 'lucide-static/icons/panel-bottom.svg?raw';
import home from 'lucide-static/icons/house.svg?raw';
import sidebar from 'lucide-static/icons/panel-left.svg?raw';
import copy from 'lucide-static/icons/copy.svg?raw';
import palette from 'lucide-static/icons/palette.svg?raw';
import keyboard from 'lucide-static/icons/keyboard.svg?raw';
import info from 'lucide-static/icons/info.svg?raw';
import pulse from 'lucide-static/icons/activity.svg?raw';
import paste from 'lucide-static/icons/clipboard.svg?raw';
import cloud from 'lucide-static/icons/cloud.svg?raw';
import minimize from 'lucide-static/icons/minus.svg?raw';
import maximize from 'lucide-static/icons/square.svg?raw';
import restore from 'lucide-static/icons/copy.svg?raw';
import winClose from 'lucide-static/icons/x.svg?raw';

import history from 'lucide-static/icons/history.svg?raw';
import bot from 'lucide-static/icons/bot-message-square.svg?raw';
import brain from 'lucide-static/icons/brain.svg?raw';
import back from 'lucide-static/icons/chevron-left.svg?raw';
import slash from 'lucide-static/icons/square-slash.svg?raw';
import plug from 'lucide-static/icons/plug.svg?raw';
import snippets from 'lucide-static/icons/scroll-text.svg?raw';
import sun from 'lucide-static/icons/sun.svg?raw';
import moon from 'lucide-static/icons/moon.svg?raw';
import sunMoon from 'lucide-static/icons/sun-moon.svg?raw';
import clock from 'lucide-static/icons/clock.svg?raw';
import cloudCheck from 'lucide-static/icons/cloud-check.svg?raw';
export const icons = {
  sun,
  moon,
  sunMoon,
  clock,
  cloudCheck,
  bot,
  brain,
  back,
  slash,
  plug,
  snippets,
  history,
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
  chevronDown,
  chevronsUpDown,
  check,
  folder,
  file,
  folderAdd,
  folderUpload,
  folderDownload,
  files,
  transfer,
  upload,
  download,
  refresh,
  up,
  eye,
  eyeOff,
  link,
  clear,
  forwards,
  play,
  stop,
  warning,
  settings,
  panelRight,
  panelBottom,
  home,
  sidebar,
  copy,
  palette,
  keyboard,
  info,
  pulse,
  paste,
  cloud,
  minimize,
  maximize,
  restore,
  winClose,
} as const;

export type IconName = keyof typeof icons;
