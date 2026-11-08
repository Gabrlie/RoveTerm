<script lang="ts">
  // One xterm.js view bound to one backend session. The parent supplies `open`, which starts the
  // session with the view's initial size; output arrives on the channel passed to it.
  import { onDestroy, onMount } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { Unicode11Addon } from '@xterm/addon-unicode11';
  import { WebLinksAddon } from '@xterm/addon-web-links';
  import {
    sessionResize,
    sessionWrite,
    type ExitStatus,
    type OutputMessage,
    type SessionId,
    type TermSize,
  } from '../ipc';
  import { errorText, t } from '../i18n';
  import { currentTheme } from './theme';

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
    if (!term) return;
    if (message instanceof ArrayBuffer) {
      term.write(new Uint8Array(message));
      return;
    }
    const { exit } = message;
    const notice = exit.signal ? t('terminal.exitedSignal', { signal: exit.signal }) : t('terminal.exited', { code: exit.code ?? '?' });
    term.write(`\r\n\x1b[2m[${notice}]\x1b[0m\r\n`);
    onexit?.(exit);
  }

  function refit(): void {
    if (!term || !fit) return;
    fit.fit();
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

  /** Loads the WebGL renderer lazily; xterm keeps its DOM renderer when WebGL is unavailable. */
  async function loadWebgl(target: Terminal): Promise<void> {
    try {
      const { WebglAddon } = await import('@xterm/addon-webgl');
      if (target !== term) return;
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      target.loadAddon(webgl);
    } catch (error) {
      console.info('WebGL renderer unavailable, using DOM renderer', error);
    }
  }

  function applyTheme(): void {
    if (term) term.options.theme = currentTheme();
  }

  onMount(async () => {
    const styles = getComputedStyle(document.documentElement);
    term = new Terminal({
      allowProposedApi: true,
      cursorBlink: true,
      fontFamily: styles.getPropertyValue('--font-mono').trim() || 'monospace',
      fontSize: 13,
      lineHeight: 1.15,
      scrollback: 5000,
      theme: currentTheme(),
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = '11';
    term.loadAddon(new WebLinksAddon());
    term.open(container);
    void loadWebgl(term);
    fit.fit();
    lastSize = { cols: term.cols, rows: term.rows };

    term.onData((data) => {
      if (sessionId) sessionWrite(sessionId, data).catch((error: unknown) => console.warn('write failed', error));
    });
    term.onTitleChange((title) => ontitle?.(title));
    observer = new ResizeObserver(scheduleRefit);
    observer.observe(container);
    media.addEventListener('change', applyTheme);

    try {
      sessionId = await open(lastSize, handleOutput);
      onsession?.(sessionId);
      // The container may have changed size while the session was opening.
      refit();
      if (active) term.focus();
    } catch (error) {
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
