<script lang="ts">
  import { onMount } from 'svelte';
  import Select from '../../ui/Select.svelte';
  import Switch from '../../ui/Switch.svelte';
  import { fontsMonospace, type TerminalPrefs } from '../../ipc';
  import { t, type MessageKey } from '../../i18n';
  import { viewport } from '../../layout/viewport.svelte';
  import { isMobile } from '../../platform';
  import KeyBarCustomizeDialog from '../../terminal/KeyBarCustomizeDialog.svelte';
  import { SCHEMES, schemeById } from '../../terminal/schemes';
  import { prefs, terminalFontStack } from '../prefs.svelte';
  import { editDevice, editPrefs } from '../edit.svelte';

  /** Powerline and Nerd Font code points shown in the preview. */
  const GLYPH = {
    arrow: '',
    branch: '',
    folder: '',
    check: '',
    apple: '',
    linux: '',
    windows: '',
  };
  const SCROLLBACK = [1_000, 5_000, 10_000, 50_000, 100_000];
  /** ANSI colours shown as chips under a scheme picker. */
  const CHIPS = ['red', 'green', 'yellow', 'blue', 'magenta', 'cyan'] as const;

  const terminal = $derived(prefs.value.terminal);
  let installed = $state<string[]>([]);
  let customizingKeyBar = $state(false);

  const fontOptions = $derived([
    { value: '', label: t('settings.fontDefault') },
    // Keep a saved family selectable even when it is no longer installed.
    ...[...new Set([...(terminal.font_family ? [terminal.font_family] : []), ...installed])]
      .sort((a, b) => a.localeCompare(b))
      .map((family) => ({ value: family, label: family })),
  ]);
  const schemeOptions = SCHEMES.map((scheme) => ({ value: scheme.id, label: scheme.name }));

  function set<K extends keyof TerminalPrefs>(key: K, value: TerminalPrefs[K]): void {
    void editPrefs((draft) => {
      draft.terminal[key] = value;
    });
  }

  /** Commits a number field when it holds a number; the backend checks the range. */
  function setNumber(key: 'font_size' | 'line_height', event: Event): void {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    if (Number.isFinite(value)) set(key, value);
  }

  onMount(() => {
    fontsMonospace()
      .then((families) => (installed = families))
      .catch((caught: unknown) => console.warn('font list unavailable', caught));
  });

  /** SSH connection timing, kept per device; applies to new connections. */
  const CONNECTION: {
    key: 'connect_timeout_secs' | 'keepalive_secs' | 'keepalive_max';
    label: MessageKey;
    hint: MessageKey;
    unit: MessageKey | null;
    values: number[];
  }[] = [
    { key: 'connect_timeout_secs', label: 'settings.connectTimeout', hint: 'settings.connectTimeoutHint', unit: 'settings.seconds', values: [5, 10, 15, 30, 60, 120] },
    { key: 'keepalive_secs', label: 'settings.keepalive', hint: 'settings.keepaliveHint', unit: 'settings.seconds', values: [10, 15, 30, 60, 120, 300] },
    { key: 'keepalive_max', label: 'settings.keepaliveMax', hint: 'settings.keepaliveMaxHint', unit: null, values: [1, 2, 3, 5, 10] },
  ];
</script>

{#snippet schemeRow(label: string, key: 'scheme_light' | 'scheme_dark', dark: boolean)}
  {@const scheme = schemeById(terminal[key], dark)}
  <div class="form-row">
    <span class="form-label">{label}</span>
    <div class="scheme">
      <span class="chips" style:background={scheme.theme.background} aria-hidden="true">
        <span class="fg" style:color={scheme.theme.foreground}>Aa</span>
        {#each CHIPS as chip (chip)}<span class="chip" style:background={scheme.theme[chip]}></span>{/each}
      </span>
      <div class="control"><Select {label} value={scheme.id} options={schemeOptions} onchange={(value) => set(key, value)} /></div>
    </div>
  </div>
{/snippet}

<p class="section-label">{t('settings.group.font')}</p>
<div class="form-group">
  <div class="form-row">
    <span class="form-label">{t('settings.fontFamily')}</span>
    <div class="control wide"><Select label={t('settings.fontFamily')} value={terminal.font_family} options={fontOptions} onchange={(value) => set('font_family', value)} /></div>
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.fontSize')}</span>
    <input class="number" type="number" min="8" max="32" step="1" value={terminal.font_size} onchange={(event) => setNumber('font_size', event)} aria-label={t('settings.fontSize')} />
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.lineHeight')}</span>
    <input class="number" type="number" min="1" max="2" step="0.05" value={terminal.line_height} onchange={(event) => setNumber('line_height', event)} aria-label={t('settings.lineHeight')} />
  </div>
  <div
    class="preview"
    aria-label={t('settings.preview')}
    style:font-family={terminalFontStack(terminal.font_family)}
    style:font-size="{terminal.font_size}px"
    style:line-height={terminal.line_height}
  >
    <div><span class="seg a"> user@host </span><span class="sep a">{GLYPH.arrow}</span><span class="seg b"> {GLYPH.folder} ~/projects </span><span class="sep b">{GLYPH.arrow}</span> {GLYPH.branch} main {GLYPH.check}</div>
    <div>0O 1lI {'{}'} [] =&gt; 中文 {GLYPH.apple} {GLYPH.linux} {GLYPH.windows}</div>
  </div>
</div>
<p class="form-note">{t('settings.fontHint')}</p>

<p class="section-label">{t('settings.group.colors')}</p>
<div class="form-group">
  {@render schemeRow(t('settings.schemeLight'), 'scheme_light', false)}
  {@render schemeRow(t('settings.schemeDark'), 'scheme_dark', true)}
</div>
<p class="form-note">{t('settings.schemeHint')}</p>

<p class="section-label">{t('settings.group.behaviour')}</p>
<div class="form-group">
  <div class="form-row">
    <span class="form-label">{t('settings.cursor')}</span>
    <div class="segmented" role="group" aria-label={t('settings.cursor')}>
      {#each [['block', 'settings.cursorBlock'], ['bar', 'settings.cursorBar'], ['underline', 'settings.cursorUnderline']] as const as [style, label] (style)}
        <button type="button" aria-pressed={terminal.cursor_style === style} onclick={() => set('cursor_style', style)}>{t(label)}</button>
      {/each}
    </div>
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.cursorBlink')}</span>
    <Switch label={t('settings.cursorBlink')} checked={terminal.cursor_blink} onchange={(checked) => set('cursor_blink', checked)} />
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.scrollback')}</span>
    <div class="control">
      <Select
        label={t('settings.scrollback')}
        value={String(terminal.scrollback)}
        options={[...new Set([...SCROLLBACK, terminal.scrollback])].sort((a, b) => a - b).map((n) => ({ value: String(n), label: t('settings.scrollbackLines', { n: n.toLocaleString() }) }))}
        onchange={(value) => set('scrollback', Number(value))}
      />
    </div>
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.copyOnSelect')}</span>
    <Switch label={t('settings.copyOnSelect')} checked={terminal.copy_on_select} onchange={(checked) => set('copy_on_select', checked)} />
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.rightClick')}</span>
    <div class="control">
      <Select
        label={t('settings.rightClick')}
        value={terminal.right_click}
        options={[
          { value: 'menu', label: t('settings.rightClickMenu') },
          { value: 'paste', label: t('settings.rightClickPaste') },
        ]}
        onchange={(value) => set('right_click', value)}
      />
    </div>
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.bell')}</span>
    <div class="control">
      <Select
        label={t('settings.bell')}
        value={terminal.bell}
        options={[
          { value: 'visual', label: t('settings.bellVisual') },
          { value: 'none', label: t('settings.bellNone') },
        ]}
        onchange={(value) => set('bell', value)}
      />
    </div>
  </div>
  {#if isMobile || viewport.coarse}
    <div class="form-row">
      <span class="form-label">{t('keybar.title')}<small>{t('keybar.settingsHint')}</small></span>
      <button type="button" class="secondary" onclick={() => (customizingKeyBar = true)}>{t('keybar.customize')}</button>
    </div>
  {/if}
</div>

{#if customizingKeyBar}
  <KeyBarCustomizeDialog onclose={() => (customizingKeyBar = false)} />
{/if}

<p class="section-label">{t('settings.group.connection')}</p>
<div class="form-group">
  {#each CONNECTION as row (row.key)}
    <div class="form-row">
      <span class="form-label">{t(row.label)}<small>{t(row.hint)}</small></span>
      <div class="control narrow">
        <Select
          label={t(row.label)}
          value={String(prefs.device.connection[row.key])}
          options={row.values.map((value) => ({ value: String(value), label: row.unit ? t(row.unit, { n: value }) : String(value) }))}
          onchange={(value) => void editDevice((draft) => (draft.connection[row.key] = Number(value)))}
        />
      </div>
    </div>
  {/each}
</div>
<p class="form-note">{t('settings.connectionNote')}</p>

<style>
  .control {
    width: 180px;
  }
  .control.wide {
    width: 240px;
  }
  .number {
    width: 80px;
    text-align: right;
  }
  .scheme {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .chips {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    height: 24px;
    padding: 0 6px;
    border-radius: var(--radius-xs);
    box-shadow: inset 0 0 0 0.5px var(--border);
  }
  .fg {
    margin-right: 2px;
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 600;
  }
  .chip {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .preview {
    margin: 0 var(--space-3) var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--term-bg);
    color: var(--term-fg);
    white-space: pre;
    overflow: hidden;
    box-shadow: inset 0 0 0 0.5px var(--separator);
  }
  .seg.a {
    background: var(--accent);
    color: var(--accent-fg);
  }
  .sep.a {
    color: var(--accent);
    background: var(--fg-muted);
  }
  .seg.b {
    background: var(--fg-muted);
    color: var(--term-bg);
  }
  .sep.b {
    color: var(--fg-muted);
  }
  @media (max-width: 560px) {
    .control,
    .control.wide {
      width: 100%;
    }
    .scheme {
      width: 100%;
      flex-wrap: wrap;
    }
    .scheme .control {
      flex: 1 1 160px;
      width: auto;
      min-width: 0;
    }
    .number {
      width: 96px;
    }
    .preview {
      overflow-x: auto;
    }
  }
</style>
