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
import chevronDown from 'remixicon/icons/Arrows/arrow-down-s-line.svg?raw';
import check from 'remixicon/icons/System/check-line.svg?raw';
import folder from 'remixicon/icons/Document/folder-line.svg?raw';
import file from 'remixicon/icons/Document/file-line.svg?raw';
import folderAdd from 'remixicon/icons/Document/folder-add-line.svg?raw';
import folderUpload from 'remixicon/icons/Document/folder-upload-line.svg?raw';
import folderDownload from 'remixicon/icons/Document/folder-download-line.svg?raw';
import files from 'remixicon/icons/Document/folder-transfer-line.svg?raw';
import transfer from 'remixicon/icons/Document/file-transfer-line.svg?raw';
import upload from 'remixicon/icons/System/upload-2-line.svg?raw';
import download from 'remixicon/icons/System/download-line.svg?raw';
import refresh from 'remixicon/icons/System/refresh-line.svg?raw';
import up from 'remixicon/icons/Arrows/arrow-up-line.svg?raw';
import eye from 'remixicon/icons/System/eye-line.svg?raw';
import eyeOff from 'remixicon/icons/System/eye-off-line.svg?raw';
import link from 'remixicon/icons/Editor/link.svg?raw';
import clear from 'remixicon/icons/Editor/format-clear.svg?raw';
import forwards from 'remixicon/icons/Arrows/arrow-left-right-line.svg?raw';
import play from 'remixicon/icons/Media/play-line.svg?raw';
import stop from 'remixicon/icons/Media/stop-line.svg?raw';
import warning from 'remixicon/icons/System/error-warning-line.svg?raw';
import settings from 'remixicon/icons/System/settings-3-line.svg?raw';
import sidebar from 'remixicon/icons/Design/layout-left-line.svg?raw';
import copy from 'remixicon/icons/Document/file-copy-line.svg?raw';

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
  chevronDown,
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
  sidebar,
  copy,
} as const;

export type IconName = keyof typeof icons;
