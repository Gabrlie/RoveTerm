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
  hex: '3374db',
  path: 'M4.65 7.753C4.059 8.409 3.703 9.281 3.703 10.233v0.014H2.977C1.331 10.247 0 8.911 0 7.27 0 5.775 0.619 4.345 1.711 3.328l0.75-0.703s-1.172 2.306-0.633 3.652c0.239 0.605 0.886 1.317 2.822 1.477m4.144 2.48h12.23c1.645 0 2.977-1.331 2.977-2.977 0-1.495-0.619-2.92-1.711-3.942l-0.75-0.703s1.172 2.306 0.633 3.652c-0.277 0.689-1.073 1.509-1.945 1.509h-11.438c-1.645 0-2.977 1.331-2.977 2.977V19.125c0 1.252 1.013 2.264 2.264 2.264h3.113c0.811 0 1.467-0.656 1.467-1.467v-1.927h2.147c0.811 0 1.467-0.656 1.467-1.467v-1.683c1.842 0 3.586-0.858 4.706-2.32l0.108-0.141H15.286c-0.811 0-1.467 0.656-1.467 1.467v1.364c0 0.173-0.141 0.319-0.319 0.319h-1.833c-0.811 0-1.467 0.656-1.467 1.467v1.355c0 0.314-0.258 0.572-0.572 0.572h-0.783c-0.314 0-0.572-0.258-0.572-0.572V10.748c0-0.286 0.234-0.516 0.52-0.516',
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
