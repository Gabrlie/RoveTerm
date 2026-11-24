<script lang="ts" module>
  // Font size is shared by every terminal. Save successive pinches in gesture order.
  let fontSizeSave = Promise.resolve();
</script>

<script lang="ts">
  // One xterm.js view bound to one backend session. The parent supplies `open`, which starts the
  // session with the view's initial size; output arrives on the channel passed to it.
  import { onDestroy, onMount } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { Unicode11Addon } from '@xterm/addon-unicode11';
  import { WebLinksAddon } from '@xterm/addon-web-links';
  import {
    sessionClose,
    sessionResize,
    sessionWrite,
    type ExitStatus,
    type OutputMessage,
    type SessionId,
    type TermSize,
  } from '../ipc';
  import { errorText, t } from '../i18n';
  import { currentTheme } from './theme';
  import { formatStep } from './connectLog';
  import { isAppShortcut } from '../actions';
  import { prefs, SYMBOL_FONT } from '../settings/prefs.svelte';
  import { editPrefs } from '../settings/edit.svelte';
  import { readClipboard, writeClipboard } from '../ui/clipboard';
  import { osc52Text } from './osc52';
  import ContextMenu, { type MenuItem } from '../ui/ContextMenu.svelte';
  import { applyModifiers, type ModifierState } from './keybar';
  import { accumulateScrollLines, alternateScrollSequence, exceededTouchSlop, LONG_PRESS_MS, pinchFontSize, touchDistance } from './touch';

  interface Props {
    open: (size: TermSize, onOutput: (message: OutputMessage) => void) => Promise<SessionId>;
    active?: boolean;
    coarse?: boolean;
    ctrl?: ModifierState;
    alt?: ModifierState;
    onmodifiers?: (ctrl: ModifierState, alt: ModifierState) => void;
    onsession?: (id: SessionId) => void;
    onexit?: (status: ExitStatus) => void;
    onerror?: (error: unknown) => void;
    ontitle?: (title: string) => void;
    /** The session is remote: an end without an exit code means the connection dropped. */
    remote?: boolean;
    /** Enter pressed after the session ended: start it again. */
    onrestart?: () => void;
  }

  let {
    open, active = true, coarse = false, ctrl = 'off', alt = 'off', onmodifiers,
    onsession, onexit, onerror, ontitle, remote = false, onrestart,
  }: Props = $props();
  /** The session ended; keys no longer go anywhere. */
  let ended = false;

  let container: HTMLDivElement;
  let term: Terminal | undefined;
  let fit: FitAddon | undefined;
  let sessionId: SessionId | undefined;
  let observer: ResizeObserver | undefined;
  let frame = 0;
  let destroyed = false;
  /** Connection steps are on screen; the first terminal output replaces them. */
  let showingLog = false;
  let lastSize: TermSize = { cols: 0, rows: 0 };
  /** Where the right-click menu is open, if it is. */
  let menu = $state<{ x: number; y: number } | null>(null);
  /** The bell flashes the view. */
  let ringing = $state(false);
  let bellTimer = 0;
  const media = matchMedia('(prefers-color-scheme: dark)');

  type TouchGesture = {
    kind: 'single';
    id: number;
    x: number;
    y: number;
    lastY: number;
    remainderPx: number;
    scrolling: boolean;
  } | PinchGesture | { kind: 'blocked' };

  interface PinchGesture {
    kind: 'pinch';
    first: number;
    second: number;
    distance: number;
    initialSize: number;
    size: number;
  }

  let gesture: TouchGesture | undefined;
  let longPress = 0;
  let screen: HTMLElement | null = null;
  let pendingPinch: PinchGesture | undefined;

  /** Moves keyboard focus into the terminal. */
  export function focus(): void {
    term?.focus();
  }

  export function blur(): void {
    term?.blur();
  }

  export function isFocused(): boolean {
    return !!container?.contains(document.activeElement);
  }

  /** Sends an already encoded key or sequence. Enter reconnects an ended session. */
  export function send(data: string): void {
    if (ended) {
      if (data === '\r') onrestart?.();
      return;
    }
    if (sessionId && data) sessionWrite(sessionId, data).catch((error: unknown) => console.warn('write failed', error));
  }

  export function refresh(): void {
    refit();
    // Resizing the WebGL canvas clears it. Repaint without waiting for new output.
    if (term) term.refresh(0, term.rows - 1);
  }

  /** Returns the current selection, or an empty string. */
  export function selection(): string {
    return term?.getSelection() ?? '';
  }

  /** Sends text as if typed, honouring bracketed paste mode. */
  export function paste(text: string): void {
    term?.paste(text);
  }

  function copySelection(): void {
    const text = term?.getSelection();
    if (text) writeClipboard(text).catch((error: unknown) => console.warn('copy failed', error));
  }

  function pasteClipboard(): void {
    void readClipboard().then((text) => text && term?.paste(text));
  }

  const menuItems = (): MenuItem[] => [
    { label: t('terminal.menu.copy'), icon: 'copy', onselect: copySelection },
    { label: t('terminal.menu.paste'), icon: 'paste', onselect: pasteClipboard },
    { label: t('terminal.menu.selectAll'), separated: true, onselect: () => term?.selectAll() },
    { label: t('terminal.menu.clear'), icon: 'clear', onselect: () => term?.clear() },
  ];

  /** The right button opens RoveTerm's menu and is not reported to the program in the terminal,
   *  which would otherwise clear its own selection (Claude Code) before the menu can use it. */
  function holdRightButton(event: MouseEvent): void {
    if (event.button === 2) event.stopPropagation();
  }

  function onContextMenu(event: MouseEvent): void {
    event.preventDefault();
    if (gesture || (event instanceof PointerEvent && event.pointerType === 'touch')) return;
    if (prefs.value.terminal.right_click === 'paste') pasteClipboard();
    else menu = { x: event.clientX, y: event.clientY };
  }

  function fingerDistance(first: Touch, second: Touch): number {
    return touchDistance({ x: first.clientX, y: first.clientY }, { x: second.clientX, y: second.clientY });
  }

  function visibleFontSize(saved: number): number {
    return (gesture?.kind === 'pinch' ? gesture : pendingPinch)?.size ?? saved;
  }

  function resizeFont(size: number): void {
    if (!term || term.options.fontSize === size) return;
    term.options.fontSize = size;
    scheduleRefit();
  }

  function savePinch(pinch: PinchGesture): void {
    if (pinch.size === pinch.initialSize) return;
    pendingPinch = pinch;
    fontSizeSave = fontSizeSave.then(async () => {
      if (pinch.size !== prefs.value.terminal.font_size) {
        await editPrefs((draft) => (draft.terminal.font_size = pinch.size));
      }
    });
    void fontSizeSave.then(() => {
      if (pendingPinch !== pinch) return;
      pendingPinch = undefined;
      resizeFont(visibleFontSize(prefs.value.terminal.font_size));
    });
  }

  function onTouchStart(event: TouchEvent): void {
    event.preventDefault();
    if (!term || gesture?.kind === 'pinch' || gesture?.kind === 'blocked') return;
    const [first, second] = Array.from(event.touches).filter((touch) => container.contains(touch.target as Node));
    if (!first) return;
    if (second) {
      clearTimeout(longPress);
      const size = term.options.fontSize ?? prefs.value.terminal.font_size;
      gesture = {
        kind: 'pinch', first: first.identifier, second: second.identifier,
        distance: fingerDistance(first, second), initialSize: size, size,
      };
      return;
    }
    if (gesture) return;
    const touch: TouchGesture = {
      kind: 'single', id: first.identifier, x: first.clientX, y: first.clientY,
      lastY: first.clientY, remainderPx: 0, scrolling: false,
    };
    gesture = touch;
    longPress = window.setTimeout(() => {
      if (gesture !== touch) return;
      gesture = { kind: 'blocked' };
      menu = { x: touch.x, y: touch.y };
    }, LONG_PRESS_MS);
  }

  function onTouchMove(event: TouchEvent): void {
    event.preventDefault();
    if (!term || !gesture || gesture.kind === 'blocked') return;
    const touches = Array.from(event.touches);
    if (gesture.kind === 'pinch') {
      const { first, second, initialSize, distance } = gesture;
      const a = touches.find((touch) => touch.identifier === first);
      const b = touches.find((touch) => touch.identifier === second);
      if (a && b) {
        gesture.size = pinchFontSize(initialSize, distance, fingerDistance(a, b));
        resizeFont(gesture.size);
      }
      return;
    }

    const single = gesture;
    const touch = touches.find((touch) => touch.identifier === single.id);
    if (!touch) return;
    if (!single.scrolling) {
      if (!exceededTouchSlop(single.x, single.y, touch.clientX, touch.clientY)) return;
      clearTimeout(longPress);
      if (Math.abs(touch.clientX - single.x) > Math.abs(touch.clientY - single.y)) {
        gesture = { kind: 'blocked' };
        return;
      }
      single.scrolling = true;
    }
    const lineHeight = (screen?.clientHeight ?? 0) / term.rows
      || prefs.value.terminal.font_size * prefs.value.terminal.line_height;
    const scroll = accumulateScrollLines(single.remainderPx, touch.clientY - single.lastY, lineHeight);
    single.lastY = touch.clientY;
    single.remainderPx = scroll.remainderPx;
    if (!scroll.lines) return;
    if (term.buffer.active.type === 'normal') term.scrollLines(scroll.lines);
    else send(alternateScrollSequence(scroll.lines));
  }

  function onTouchEnd(event: TouchEvent): void {
    event.preventDefault();
    clearTimeout(longPress);
    const touches = Array.from(event.touches).filter((touch) => container.contains(touch.target as Node));
    const previous = gesture;
    if (previous?.kind === 'pinch') {
      const { first, second } = previous;
      if (touches.some((touch) => touch.identifier === first) && touches.some((touch) => touch.identifier === second)) return;
      savePinch(previous);
    } else if (previous?.kind === 'single' && !previous.scrolling) {
      const touch = Array.from(event.changedTouches).find((touch) => touch.identifier === previous.id);
      if (touch && !exceededTouchSlop(previous.x, previous.y, touch.clientX, touch.clientY)) focus();
    }
    // A remaining pinch finger cannot become a tap, long press or scroll.
    gesture = touches.length ? { kind: 'blocked' } : undefined;
  }

  function cancelTouch(): void {
    clearTimeout(longPress);
    const wasPinching = gesture?.kind === 'pinch';
    gesture = undefined;
    if (wasPinching) resizeFont(visibleFontSize(prefs.value.terminal.font_size));
  }

  function onForeground(): void {
    if (document.visibilityState === 'hidden') {
      cancelTouch();
    } else if (getComputedStyle(container).visibility === 'visible') {
      scheduleRefit();
    }
  }

  function ring(): void {
    if (prefs.value.terminal.bell !== 'visual') return;
    ringing = true;
    clearTimeout(bellTimer);
    bellTimer = window.setTimeout(() => (ringing = false), 160);
  }

  function handleOutput(message: OutputMessage): void {
    if (!term || destroyed) return;
    if (message instanceof ArrayBuffer) {
      if (showingLog) {
        // Connected: the shell's own output starts on a clean screen.
        showingLog = false;
        term.reset();
      }
      term.write(new Uint8Array(message));
      return;
    }
    if ('progress' in message) {
      showingLog = true;
      term.write(formatStep(message.progress));
      return;
    }
    const { exit } = message;
    ended = true;
    const notice = exit.signal
      ? t('terminal.exitedSignal', { signal: exit.signal })
      : exit.code === null
        ? t(remote ? 'terminal.disconnected' : 'terminal.ended')
        : t('terminal.exited', { code: exit.code });
    term.write(`\r\n\x1b[2m[${notice}]\x1b[0m\r\n`);
    onexit?.(exit);
  }

  function refit(): void {
    if (!term || !fit) return;
    // Hidden views (inactive tabs) have no size; fitting them would shrink the shell to nothing.
    if (container.clientWidth === 0 || container.clientHeight === 0) return;
    fit.fit();
    const size = { cols: term.cols, rows: term.rows };
    if (sessionId && (size.cols !== lastSize.cols || size.rows !== lastSize.rows)) {
      lastSize = size;
      sessionResize(sessionId, size).catch((error: unknown) => console.warn('resize failed', error));
    }
  }

  function scheduleRefit(): void {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(refresh);
  }

  /**
   * WebKitGTK (Linux desktop) stops repainting older WebGL terminals once several GL contexts
   * coexist, so Linux desktop keeps xterm's DOM renderer. WKWebView, WebView2 and Android WebView
   * use WebGL.
   */
  function prefersWebgl(): boolean {
    const agent = navigator.userAgent;
    return !(agent.includes('Linux') && !agent.includes('Android'));
  }

  /** Loads the WebGL renderer lazily; xterm keeps its DOM renderer when WebGL is unavailable. */
  async function loadWebgl(target: Terminal): Promise<void> {
    try {
      const { WebglAddon } = await import('@xterm/addon-webgl');
      if (target !== term) return;
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        // The browser reclaimed the GL context (WebViews cap live contexts); fall back to the DOM
        // renderer and repaint, since nothing else triggers a redraw until new output arrives.
        console.info('WebGL context lost, using DOM renderer');
        webgl.dispose();
        target.refresh(0, target.rows - 1);
      });
      target.loadAddon(webgl);
    } catch (error) {
      console.info('WebGL renderer unavailable, using DOM renderer', error);
    }
  }

  function applyTheme(): void {
    if (term) term.options.theme = currentTheme();
  }

  /** Waits for the fonts the terminal draws with, so glyphs are not measured or cached before they load. */
  async function loadFonts(size: number): Promise<void> {
    const families = [SYMBOL_FONT, prefs.value.terminal.font_family].filter(Boolean);
    try {
      await Promise.all(families.map((family) => document.fonts.load(`${size}px "${family}"`, '\ue0b0a')));
    } catch (error) {
      console.info('terminal fonts not preloaded', error);
    }
  }

  // Applies a changed colour scheme to a running terminal.
  $effect(() => {
    void prefs.value.terminal.scheme_light;
    void prefs.value.terminal.scheme_dark;
    void prefs.value.appearance.theme;
    applyTheme();
  });

  // Applies cursor and scrollback preferences to a running terminal.
  $effect(() => {
    const { cursor_style: cursorStyle, cursor_blink: cursorBlink, scrollback } = prefs.value.terminal;
    if (!term) return;
    term.options.cursorStyle = cursorStyle;
    term.options.cursorBlink = cursorBlink;
    term.options.scrollback = scrollback;
  });

  // Applies preference changes to a running terminal.
  $effect(() => {
    const { font_size: fontSize, line_height: lineHeight } = prefs.value.terminal;
    const fontFamily = prefs.terminalFontFamily;
    const target = term;
    if (!target) return;
    let superseded = false;
    void loadFonts(fontSize).then(() => {
      if (superseded || target !== term) return;
      target.options.fontFamily = fontFamily;
      target.options.fontSize = visibleFontSize(fontSize);
      target.options.lineHeight = lineHeight;
      scheduleRefit();
    });
    return () => (superseded = true);
  });

  onMount(async () => {
    const { font_size: fontSize, line_height: lineHeight, cursor_style: cursorStyle, cursor_blink: cursorBlink, scrollback } = prefs.value.terminal;
    await loadFonts(fontSize);
    if (destroyed) return;
    term = new Terminal({
      allowProposedApi: true,
      cursorBlink,
      cursorStyle,
      fontFamily: prefs.terminalFontFamily,
      fontSize,
      lineHeight,
      scrollback,
      theme: currentTheme(),
      // Programs that assume a dark terminal print light text; keep it readable on any background.
      minimumContrastRatio: 4.5,
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = '11';
    term.loadAddon(new WebLinksAddon());
    // Programs that keep their own selection (Claude Code, tmux) copy it with OSC 52.
    term.parser.registerOscHandler(52, (data) => {
      const text = osc52Text(data);
      if (text) void writeClipboard(text).catch((error: unknown) => console.warn('OSC 52 copy failed', error));
      return true;
    });
    term.open(container);
    screen = container.querySelector('.xterm-screen');
    container.addEventListener('touchstart', onTouchStart, { passive: false });
    container.addEventListener('touchmove', onTouchMove, { passive: false });
    container.addEventListener('touchend', onTouchEnd, { passive: false });
    container.addEventListener('touchcancel', cancelTouch);
    document.addEventListener('visibilitychange', onForeground);
    window.addEventListener('pageshow', onForeground);
    window.addEventListener('focus', onForeground);
    if (prefersWebgl()) void loadWebgl(term);
    fit.fit();
    lastSize = { cols: term.cols, rows: term.rows };

    // App shortcuts are handled by the window; the terminal must not also send them to the shell.
    term.attachCustomKeyEventHandler((event) => !isAppShortcut(event));
    term.onData((data) => {
      if (ended) return send(data);
      if (!sessionId) return;
      const next = applyModifiers(data, ctrl, alt);
      if (next.ctrl !== ctrl || next.alt !== alt) onmodifiers?.(next.ctrl, next.alt);
      send(next.data);
    });
    term.onTitleChange((title) => ontitle?.(title));
    term.onBell(ring);
    term.onSelectionChange(() => {
      if (prefs.value.terminal.copy_on_select) copySelection();
    });
    observer = new ResizeObserver(scheduleRefit);
    observer.observe(container);
    media.addEventListener('change', applyTheme);

    try {
      const opened = await open(lastSize, handleOutput);
      // The pane was closed while connecting: nothing owns this session, so end it.
      if (destroyed) {
        sessionClose(opened).catch((error: unknown) => console.warn('closing orphaned session failed', error));
        return;
      }
      sessionId = opened;
      onsession?.(sessionId);
      // The container may have changed size while the session was opening.
      refresh();
      if (active && !coarse) term.focus();
    } catch (error) {
      if (destroyed) return;
      term.write(`\x1b[31m${t('terminal.openFailed', { message: errorText(error) })}\x1b[0m\r\n`);
      onerror?.(error);
    }
  });

  $effect(() => {
    if (active) {
      scheduleRefit();
      if (!coarse) term?.focus();
    }
  });

  onDestroy(() => {
    destroyed = true;
    cancelAnimationFrame(frame);
    clearTimeout(bellTimer);
    clearTimeout(longPress);
    container?.removeEventListener('touchstart', onTouchStart);
    container?.removeEventListener('touchmove', onTouchMove);
    container?.removeEventListener('touchend', onTouchEnd);
    container?.removeEventListener('touchcancel', cancelTouch);
    document.removeEventListener('visibilitychange', onForeground);
    window.removeEventListener('pageshow', onForeground);
    window.removeEventListener('focus', onForeground);
    observer?.disconnect();
    media.removeEventListener('change', applyTheme);
    term?.dispose();
    term = undefined;
  });

</script>

<div class="term" class:ringing bind:this={container} oncontextmenu={onContextMenu} onmousedowncapture={holdRightButton} onmouseupcapture={holdRightButton} role="presentation"></div>
{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menuItems()} onclose={() => (menu = null)} />
{/if}

<style>
  .term {
    position: relative;
    width: 100%;
    height: 100%;
    padding: var(--space-1) 0 0 var(--space-2);
    background: var(--term-bg);
    overflow: hidden;
    touch-action: none;
    -webkit-touch-callout: none;
  }
  .term :global(.xterm) {
    height: 100%;
  }
  /* Visual bell: a brief accent wash over the view. */
  .term.ringing::after {
    content: '';
    position: absolute;
    inset: 0;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    pointer-events: none;
  }
  .term :global(.xterm-viewport) {
    background: transparent !important;
  }
</style>
