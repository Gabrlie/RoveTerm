import { isMobile } from '../platform.ts';

export interface MobileInsets {
  top: number;
  right: number;
  bottom: number;
  left: number;
  imeBottom: number;
}

interface InsetsBridge {
  fetch?: () => Promise<MobileInsets>;
  listen?: (cb: (insets: MobileInsets) => void) => Promise<() => void>;
}

function insetPixels(value: number | undefined): number {
  return value !== undefined && Number.isFinite(value) && value > 0 ? Math.round(value) : 0;
}

export function sanitizeInsets(input: Partial<MobileInsets> | null | undefined): MobileInsets {
  return {
    top: insetPixels(input?.top),
    right: insetPixels(input?.right),
    bottom: insetPixels(input?.bottom),
    left: insetPixels(input?.left),
    imeBottom: insetPixels(input?.imeBottom),
  };
}

export function computeVisualViewportIme(innerHeight: number, visualHeight: number, offsetTop: number): number {
  return insetPixels(innerHeight - visualHeight - offsetTop);
}

export function applyInsets(insets: MobileInsets, root = document.documentElement): void {
  root.style.setProperty('--safe-top', `${insets.top}px`);
  root.style.setProperty('--safe-right', `${insets.right}px`);
  root.style.setProperty('--safe-bottom', `${insets.bottom}px`);
  root.style.setProperty('--safe-left', `${insets.left}px`);
  root.style.setProperty('--ime-bottom', `${insets.imeBottom}px`);
}

/** Uses CSS safe areas until the bridge supplies native insets in CSS pixels. */
export function initInsets(bridge?: InsetsBridge): () => void {
  if (!isMobile || typeof window === 'undefined') return () => {};

  const root = document.documentElement;
  const viewport = window.visualViewport;
  let native: MobileInsets | undefined;
  let disposed = false;
  let receivedEvent = false;
  let unlisten: (() => void) | undefined;

  function update(): void {
    const imeBottom = native?.imeBottom || (viewport
      ? computeVisualViewportIme(window.innerHeight, viewport.height, viewport.offsetTop)
      : 0);
    if (native) applyInsets({ ...native, imeBottom }, root);
    else root.style.setProperty('--ime-bottom', `${imeBottom}px`);
  }

  function updateNative(insets: MobileInsets): void {
    native = sanitizeInsets(insets);
    update();
  }

  viewport?.addEventListener('resize', update);
  viewport?.addEventListener('scroll', update);
  update();

  void bridge?.listen?.((insets) => {
    if (disposed) return;
    receivedEvent = true;
    updateNative(insets);
  }).then((stop) => {
    if (disposed) stop();
    else unlisten = stop;
  }).catch(() => { /* The browser insets remain available. */ });

  void bridge?.fetch?.().then((insets) => {
    if (!disposed && !receivedEvent) updateNative(insets);
  }).catch(() => { /* The browser insets remain available. */ });

  return () => {
    disposed = true;
    viewport?.removeEventListener('resize', update);
    viewport?.removeEventListener('scroll', update);
    unlisten?.();
  };
}
