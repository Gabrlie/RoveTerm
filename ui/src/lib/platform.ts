/** Desktop-only integrations are hidden on mobile builds. */
export const isMobile = /Android|iPhone|iPad/i.test(navigator.userAgent);
