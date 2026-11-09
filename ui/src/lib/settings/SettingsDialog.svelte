<script lang="ts">
  // Preferences editor. Changes apply when saved; the preview shows the draft font.
  import { onMount } from 'svelte';
  import Dialog from '../ui/Dialog.svelte';
  import Select from '../ui/Select.svelte';
  import { fontsMonospace } from '../ipc';
  import { errorText, t } from '../i18n';
  import { prefs, terminalFontStack } from './prefs.svelte';

  let { onclose }: { onclose: () => void } = $props();

  /** Powerline and Nerd Font code points shown in the preview. */
  const GLYPH = {
    arrow: '\ue0b0',
    branch: '\ue0a0',
    folder: '\uf07c',
    check: '\uf00c',
    apple: '\uf179',
    linux: '\uf17c',
    windows: '\uf17a',
  };

  const current = prefs.value.terminal;
  let fontFamily = $state(current.font_family);
  let fontSize = $state(current.font_size);
  let lineHeight = $state(current.line_height);
  let installed = $state<string[]>([]);
  let error = $state('');
  let saving = $state(false);

  const fontOptions = $derived([
    { value: '', label: t('settings.fontDefault') },
    // Keep a saved family selectable even when it is no longer installed.
    ...[...new Set([...(fontFamily ? [fontFamily] : []), ...installed])]
      .sort((a, b) => a.localeCompare(b))
      .map((family) => ({ value: family, label: family })),
  ]);

  async function save(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    saving = true;
    error = '';
    try {
      await prefs.save({
        ...prefs.value,
        terminal: { font_family: fontFamily, font_size: Number(fontSize), line_height: Number(lineHeight) },
      });
      onclose();
    } catch (caught) {
      error = errorText(caught);
    } finally {
      saving = false;
    }
  }

  onMount(() => {
    fontsMonospace()
      .then((families) => (installed = families))
      .catch((caught: unknown) => console.warn('font list unavailable', caught));
  });
</script>

<Dialog title={t('settings.title')} {onclose} width={480}>
  <form id="settings-form" onsubmit={save}>
    <h3>{t('settings.terminal')}</h3>
    <label>{t('settings.fontFamily')}<Select bind:value={fontFamily} options={fontOptions} /></label>
    <div class="row">
      <label>{t('settings.fontSize')}<input type="number" min="8" max="32" step="1" bind:value={fontSize} required /></label>
      <label>{t('settings.lineHeight')}<input type="number" min="1" max="2" step="0.05" bind:value={lineHeight} required /></label>
    </div>
    <p class="hint">{t('settings.fontHint')}</p>
    <div class="preview" aria-label={t('settings.preview')} style:font-family={terminalFontStack(fontFamily)} style:font-size="{fontSize}px" style:line-height={lineHeight}>
      <div><span class="seg a"> user@host </span><span class="sep a">{GLYPH.arrow}</span><span class="seg b"> {GLYPH.folder} ~/projects </span><span class="sep b">{GLYPH.arrow}</span> {GLYPH.branch} main {GLYPH.check}</div>
      <div>0O 1lI {'{}'} [] =&gt; 中文 {GLYPH.apple} {GLYPH.linux} {GLYPH.windows}</div>
    </div>
    {#if error}<p class="error">{error}</p>{/if}
  </form>
  {#snippet actions()}
    <button type="button" class="secondary" onclick={onclose}>{t('common.cancel')}</button>
    <button type="submit" class="primary" form="settings-form" disabled={saving}>{t('common.save')}</button>
  {/snippet}
</Dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  h3 {
    margin: 0;
    color: var(--fg);
    font-size: var(--text-body);
    font-weight: 600;
  }
  .hint {
    margin: calc(-1 * var(--space-1)) 0 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .preview {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--term-bg);
    color: var(--term-fg);
    white-space: pre;
    overflow: hidden;
  }
  .seg.a {
    background: var(--accent);
    color: #fff;
  }
  .sep.a {
    color: var(--accent);
    background: var(--fg-muted);
  }
  .seg.b {
    background: var(--fg-muted);
    color: var(--bg);
  }
  .sep.b {
    color: var(--fg-muted);
  }
</style>
