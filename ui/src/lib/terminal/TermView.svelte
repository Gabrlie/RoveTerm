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

  interface Props {
    open: (size: TermSize, onOutput: (message: OutputMessage) => void) => Promise<SessionId>;
    active?: boolean;
    onsession?: (id: SessionId) => void;
    onexit?: (status: ExitStatus) => void;
    onerror?: (error: unknown) => void;
    ontitle?: (title: string) => void;
  }

  let { open, active = true, onsession, onexit, onerror, ontitle }: Props = $props();

  let container: HTMLDivElement;
  let term: Terminal | undefined;
  let fit: FitAddon | undefined;
  let sessionId: SessionId | undefined;
  let observer: ResizeObserver | undefined;
  let frame = 0;
  let destroyed = false;
  let lastSize: TermSize = { cols: 0, rows: 0 };
  const media = matchMedia('(prefers-color-scheme: dark)');

  /** Moves keyboard focus into the terminal. */
  export function focus(): void {
    term?.focus();
  }

  /** Returns the current selection, or an empty string. */
  export function selection(): string {
    return term?.getSelection() ?? '';
  }

  /** Sends text as if typed, honouring bracketed paste mode. */
  export function paste(text: string): void {
    term?.paste(text);
  }

  function handleOutput(message: OutputMessage): void {
    if (!term || destroyed) return;
    if (message instanceof ArrayBuffer) {
      term.write(new Uint8Array(message));
      return;
    }
    if ('progress' in message) {
      term.write(formatStep(message.progress));
      return;
    }
    const { exit } = message;
    const notice = exit.signal ? t('terminal.exitedSignal', { signal: exit.signal }) : t('terminal.exited', { code: exit.code ?? '?' });
    term.write(`\r\n\x1b[2m[${notice}]\x1b[0m\r\n`);
    onexit?.(exit);
  }

  function refit(): void {
    if (!term || !fit) return;
    // Hidden views (inactive tabs) have no size; fitting them would shrink the shell to nothing.
    if (container.clientWidth === 0 || container.clientHeight === 0) return;
    fit.fit();
    // Resizing the WebGL canvas clears it; repaint the viewport without waiting for new output.
    term.refresh(0, term.rows - 1);
    const size = { cols: term.cols, rows: term.rows };
    if (sessionId && (size.cols !== lastSize.cols || size.rows !== lastSize.rows)) {
      lastSize = size;
      sessionResize(sessionId, size).catch((error: unknown) => console.warn('resize failed', error));
    }
  }

  function scheduleRefit(): void {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(refit);
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

  // Applies preference changes to a running terminal.
  $effect(() => {
    const { font_size: fontSize, line_height: lineHeight } = prefs.value.terminal;
    const fontFamily = prefs.terminalFontFamily;
    const target = term;
    if (!target) return;
    void loadFonts(fontSize).then(() => {
      if (target !== term) return;
      target.options.fontFamily = fontFamily;
      target.options.fontSize = fontSize;
      target.options.lineHeight = lineHeight;
      scheduleRefit();
    });
  });

  onMount(async () => {
    const { font_size: fontSize, line_height: lineHeight } = prefs.value.terminal;
    await loadFonts(fontSize);
    if (destroyed) return;
    term = new Terminal({
      allowProposedApi: true,
      cursorBlink: true,
      fontFamily: prefs.terminalFontFamily,
      fontSize,
      lineHeight,
      scrollback: 5000,
      theme: currentTheme(),
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = '11';
    term.loadAddon(new WebLinksAddon());
    term.open(container);
    if (prefersWebgl()) void loadWebgl(term);
    fit.fit();
    lastSize = { cols: term.cols, rows: term.rows };

    // App shortcuts are handled by the window; the terminal must not also send them to the shell.
    term.attachCustomKeyEventHandler((event) => !isAppShortcut(event));
    term.onData((data) => {
      if (sessionId) sessionWrite(sessionId, data).catch((error: unknown) => console.warn('write failed', error));
    });
    term.onTitleChange((title) => ontitle?.(title));
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
      refit();
      if (active) term.focus();
    } catch (error) {
      if (destroyed) return;
      term.write(`\x1b[31m${t('terminal.openFailed', { message: errorText(error) })}\x1b[0m\r\n`);
      onerror?.(error);
    }
  });

  $effect(() => {
    if (active) {
      scheduleRefit();
      term?.focus();
    }
  });

  onDestroy(() => {
    destroyed = true;
    cancelAnimationFrame(frame);
    observer?.disconnect();
    media.removeEventListener('change', applyTheme);
    term?.dispose();
  });

</script>

<div class="term" bind:this={container}></div>

<style>
  .term {
    width: 100%;
    height: 100%;
    padding: var(--space-1) 0 0 var(--space-2);
    background: var(--term-bg);
    overflow: hidden;
  }
  .term :global(.xterm) {
    height: 100%;
  }
  .term :global(.xterm-viewport) {
    background: transparent !important;
  }
</style>
