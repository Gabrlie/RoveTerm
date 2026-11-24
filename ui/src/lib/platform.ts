export interface PlatformInfo {
  isAndroid: boolean;
  isIos: boolean;
  isMobile: boolean;
  isApple: boolean;
  isMacDesktop: boolean;
}

export function detectPlatform(
  { userAgent = '', platform = '', maxTouchPoints = 0 }: { userAgent?: string; platform?: string; maxTouchPoints?: number } = {},
): PlatformInfo {
  const isAndroid = /Android/i.test(userAgent);
  const isIos = /iPhone|iPad|iPod/i.test(userAgent) || (platform === 'MacIntel' && maxTouchPoints > 1);
  const isMobile = isAndroid || isIos;
  const isApple = isIos || /Mac/i.test(platform) || userAgent.includes('Mac OS');
  return { isAndroid, isIos, isMobile, isApple, isMacDesktop: isApple && !isMobile };
}

export function capabilitiesFor(info: PlatformInfo) {
  return {
    localPty: !info.isMobile,
    schedules: !info.isMobile,
    localAgents: !info.isMobile,
    externalEditor: !info.isMobile,
    autostart: !info.isMobile,
    biometricVault: info.isMobile,
  };
}

export const platform = detectPlatform(typeof navigator === 'undefined' ? undefined : navigator);
export const { isMobile, isAndroid, isIos, isApple, isMacDesktop } = platform;
export const capabilities = capabilitiesFor(platform);
