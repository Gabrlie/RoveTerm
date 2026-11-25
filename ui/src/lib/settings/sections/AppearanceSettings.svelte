<script lang="ts">
  import Select from '../../ui/Select.svelte';
  import { t, type MessageKey } from '../../i18n';
  import type { Accent, Language, Theme } from '../../ipc';
  import { prefs } from '../prefs.svelte';
  import { editPrefs } from '../edit.svelte';

  const THEMES: { value: Theme; label: MessageKey }[] = [
    { value: 'system', label: 'settings.themeSystem' },
    { value: 'light', label: 'settings.themeLight' },
    { value: 'dark', label: 'settings.themeDark' },
  ];
  const ACCENTS: Accent[] = ['blue', 'purple', 'pink', 'red', 'orange', 'yellow', 'green', 'graphite'];
  const SCALES = [90, 100, 110, 125];

  const appearance = $derived(prefs.value.appearance);
  const languages = $derived<{ value: Language; label: string }[]>([
    { value: 'auto', label: t('settings.languageAuto') },
    { value: 'zh-CN', label: t('settings.languageZh') },
    { value: 'en', label: t('settings.languageEn') },
  ]);
</script>

<p class="section-label">{t('settings.group.theme')}</p>
<div class="form-group">
  <div class="form-row tall">
    <span class="form-label">{t('settings.theme')}</span>
    <div class="themes" role="radiogroup" aria-label={t('settings.theme')}>
      {#each THEMES as theme (theme.value)}
        <button
          type="button"
          class="theme"
          role="radio"
          aria-checked={appearance.theme === theme.value}
          onclick={() => void editPrefs((draft) => (draft.appearance.theme = theme.value))}
        >
          <span class="thumb {theme.value}"><span class="bar"></span><span class="side"></span></span>
          <span>{t(theme.label)}</span>
        </button>
      {/each}
    </div>
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.accent')}</span>
    <div class="swatches" role="radiogroup" aria-label={t('settings.accent')}>
      {#each ACCENTS as accent (accent)}
        <button
          type="button"
          class="swatch"
          role="radio"
          aria-checked={appearance.accent === accent}
          title={t(`settings.accent.${accent}`)}
          aria-label={t(`settings.accent.${accent}`)}
          style:--tint="var(--sys-{accent})"
          onclick={() => void editPrefs((draft) => (draft.appearance.accent = accent))}
        ></button>
      {/each}
    </div>
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.uiScale')}</span>
    <div class="control">
      <Select
        label={t('settings.uiScale')}
        value={String(appearance.ui_scale)}
        options={SCALES.map((scale) => ({ value: String(scale), label: `${scale}%` }))}
        onchange={(value) => void editPrefs((draft) => (draft.appearance.ui_scale = Number(value)))}
      />
    </div>
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.language')}</span>
    <div class="control">
      <Select
        label={t('settings.language')}
        value={appearance.language}
        options={languages}
        onchange={(value) => void editPrefs((draft) => (draft.appearance.language = value))}
      />
    </div>
  </div>
</div>

<style>
  .tall {
    align-items: flex-start;
    padding-top: var(--space-3);
    padding-bottom: var(--space-3);
  }
  .control {
    width: 180px;
  }
  .themes {
    display: flex;
    gap: var(--space-3);
  }
  .theme {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 0;
    border: none;
    background: none;
    color: var(--fg-muted);
    font: inherit;
    font-size: var(--text-footnote);
    cursor: pointer;
  }
  .theme[aria-checked='true'] {
    color: var(--fg);
    font-weight: 600;
  }
  /* A miniature window: title bar and sidebar over the theme's background. */
  .thumb {
    position: relative;
    width: 68px;
    height: 44px;
    border-radius: 7px;
    overflow: hidden;
    box-shadow:
      0 0 0 0.5px var(--border),
      var(--shadow-sm);
    transition: box-shadow var(--dur-fast) var(--ease-out);
  }
  .theme[aria-checked='true'] .thumb {
    box-shadow:
      0 0 0 2px var(--surface-raised),
      0 0 0 4px var(--accent);
  }
  .thumb.light {
    background: #ffffff;
  }
  .thumb.dark {
    background: #1e1e20;
  }
  .thumb.system {
    background: linear-gradient(135deg, #ffffff 50%, #1e1e20 50%);
  }
  .bar,
  .side {
    position: absolute;
  }
  .bar {
    left: 0;
    right: 0;
    top: 0;
    height: 9px;
    background: rgb(128 128 128 / 0.18);
  }
  .side {
    left: 0;
    top: 9px;
    bottom: 0;
    width: 20px;
    background: rgb(128 128 128 / 0.12);
  }
  .swatches {
    display: flex;
    gap: 8px;
  }
  .swatch {
    width: 18px;
    height: 18px;
    padding: 0;
    border: none;
    border-radius: 50%;
    background: linear-gradient(to bottom, color-mix(in srgb, var(--tint) 80%, #fff), var(--tint));
    box-shadow: inset 0 0 0 0.5px rgb(0 0 0 / 0.15);
    cursor: pointer;
    transition: transform var(--dur-fast) var(--ease-out);
  }
  .swatch:hover {
    transform: scale(1.1);
  }
  .swatch[aria-checked='true'] {
    box-shadow:
      0 0 0 2px var(--surface-raised),
      0 0 0 3.5px var(--tint);
  }
  @media (max-width: 560px) {
    .control {
      width: 100%;
    }
    .themes {
      flex-wrap: wrap;
      width: 100%;
      gap: var(--space-2);
    }
    .swatches {
      flex-wrap: wrap;
      gap: var(--space-2);
    }
  }
  @media (pointer: coarse) {
    .swatch {
      width: 28px;
      height: 28px;
    }
  }
</style>
