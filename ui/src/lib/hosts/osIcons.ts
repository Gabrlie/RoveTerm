// Logos of operating systems for host icons, from Simple Icons (CC0; all 24 × 24). Looked up by
// the os-release `ID`, then by each `ID_LIKE` entry, so derivatives fall back to their parent.
import {
  siAlibabacloud,
  siAlmalinux,
  siAlpinelinux,
  siApple,
  siArchlinux,
  siArtixlinux,
  siCentos,
  siDebian,
  siDeepin,
  siElementary,
  siEndeavouros,
  siFedora,
  siFreebsd,
  siGarudalinux,
  siGentoo,
  siKalilinux,
  siLinux,
  siLinuxmint,
  siManjaro,
  siMxlinux,
  siNetbsd,
  siNixos,
  siOpenbsd,
  siOpensuse,
  siOpenwrt,
  siPopos,
  siProxmox,
  siRaspberrypi,
  siRedhat,
  siRockylinux,
  siSlackware,
  siSolus,
  siUbuntu,
  siVoidlinux,
  siZorin,
} from 'simple-icons';
import type { HostOs } from '../ipc';

export interface OsIcon {
  /** SVG path in a 24 × 24 box. */
  path: string;
  /** Brand colour, `#rrggbb`. */
  color: string;
  /** Whether the brand colour is light enough to need a dark logo. */
  light: boolean;
}

const FNOS = {
  hex: '1677ff',
  path: 'M4 3h16a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Zm2 3v2h8V6Zm11 0v2h2V6ZM4 13h16a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-4a2 2 0 0 1 2-2Zm2 3v2h8v-2Zm11 0v2h2v-2Z',
};

const BY_ID: Record<string, { path: string; hex: string }> = {
  ubuntu: siUbuntu,
  debian: siDebian,
  proxmox: siProxmox,
  pve: siProxmox,
  fnos: FNOS,
  trim: FNOS,
  centos: siCentos,
  fedora: siFedora,
  rhel: siRedhat,
  rocky: siRockylinux,
  almalinux: siAlmalinux,
  arch: siArchlinux,
  artix: siArtixlinux,
  alpine: siAlpinelinux,
  opensuse: siOpensuse,
  'opensuse-leap': siOpensuse,
  'opensuse-tumbleweed': siOpensuse,
  suse: siOpensuse,
  sles: siOpensuse,
  linuxmint: siLinuxmint,
  kali: siKalilinux,
  gentoo: siGentoo,
  nixos: siNixos,
  manjaro: siManjaro,
  raspbian: siRaspberrypi,
  pop: siPopos,
  elementary: siElementary,
  endeavouros: siEndeavouros,
  garuda: siGarudalinux,
  void: siVoidlinux,
  slackware: siSlackware,
  zorin: siZorin,
  mx: siMxlinux,
  solus: siSolus,
  deepin: siDeepin,
  openwrt: siOpenwrt,
  alinux: siAlibabacloud,
  freebsd: siFreebsd,
  openbsd: siOpenbsd,
  netbsd: siNetbsd,
  macos: siApple,
  linux: siLinux,
};

/** Relative luminance of `#rrggbb` above which white would not stand out. */
function isLight(hex: string): boolean {
  const [r, g, b] = [0, 2, 4].map((offset) => parseInt(hex.slice(offset, offset + 2), 16) / 255);
  return 0.2126 * (r ?? 0) + 0.7152 * (g ?? 0) + 0.0722 * (b ?? 0) > 0.62;
}

/** The logo for `os`, or null for an unknown or undetected system. */
export function osIcon(os: HostOs | null): OsIcon | null {
  if (!os) return null;
  for (const id of [os.id, ...os.like]) {
    if (!Object.hasOwn(BY_ID, id)) continue;
    const icon = BY_ID[id];
    if (icon) return { path: icon.path, color: `#${icon.hex}`, light: isLight(icon.hex) };
  }
  return null;
}
