<script lang="ts" module>
  export type { SettingsSection } from './sections';
</script>

<script lang="ts">
  // Preference changes apply and are stored immediately.
  import Dialog from '../ui/Dialog.svelte';
  import Icon from '../ui/Icon.svelte';
  import { t } from '../i18n';
  import { settingsError } from './edit.svelte';
  import { settingsSections, type SettingsSection } from './sections';
  import SettingsSectionContent from './SettingsSectionContent.svelte';
  import { capabilities, isMobile } from '../platform';
  import { viewport } from '../layout/viewport.svelte';
  import { pushBackHandler } from '../ui/backStack';
  import { initialCompactDetail, masterDetailView } from '../ui/masterDetail';

  let { initial, onclose, onopentailscale }: { initial?: SettingsSection; onclose: () => void; onopentailscale?: () => void } = $props();

  const entries = settingsSections({ isMobile, biometricVault: capabilities.biometricVault, includeShortcuts: true });

  // svelte-ignore state_referenced_locally
  let section = $state<SettingsSection>(initial ?? 'appearance');
  // svelte-ignore state_referenced_locally
  let compactDetail = $state(initialCompactDetail(initial !== undefined));
  const layoutMode = $derived(masterDetailView(viewport.compact, compactDetail));
  const activeSection = $derived(entries.find((entry) => entry.id === section));

  $effect(() => {
    if (layoutMode === 'detail') return pushBackHandler(() => { compactDetail = false; });
  });

  settingsError.text = '';
</script>

<Dialog title={t('settings.title')} {onclose} width={780}>
  <div class="settings" class:compact={viewport.compact}>
    {#if layoutMode !== 'detail'}
      <nav aria-label={t('settings.title')}>
        {#each entries as entry (entry.id)}
          {@const active = layoutMode === 'both' && section === entry.id}
          <button type="button" class:active aria-current={active ? 'page' : undefined} onclick={() => {
            section = entry.id;
            compactDetail = true;
          }}>
            <span class="glyph" style:--tint="var(--sys-{entry.tint})"><Icon name={entry.icon} size={13} /></span>
            {t(entry.label)}
            {#if layoutMode === 'list'}
              <span class="nav-chevron"><Icon name="chevron" size={14} /></span>
            {/if}
          </button>
        {/each}
      </nav>
    {/if}
    {#if layoutMode !== 'list'}
      <div class="pane">
        {#if layoutMode === 'detail'}
          <div class="compact-bar">
            <button type="button" class="plain back-btn" onclick={() => (compactDetail = false)}>
              <Icon name="back" size={16} />{t('settings.backToSections')}
            </button>
          </div>
        {/if}
        <h2>{t(activeSection?.label ?? 'settings.title')}</h2>
        <SettingsSectionContent
          section={activeSection?.id ?? 'about'}
          {onopentailscale}
          onopenlogs={() => {
            section = 'logs';
            compactDetail = true;
          }}
        />
      </div>
    {/if}
  </div>
  {#snippet actions()}
    {#if settingsError.text}<p class="error problem">{settingsError.text}</p>{/if}
    <button type="button" class="primary" onclick={onclose}>{t('settings.done')}</button>
  {/snippet}
</Dialog>

<style>
  .settings {
    display: flex;
    height: min(560px, 68vh);
    margin: 0 calc(-1 * var(--space-5));
    box-shadow:
      inset 0 1px 0 var(--separator),
      inset 0 -1px 0 var(--separator);
  }
  nav {
    flex: 0 0 184px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-3) var(--space-2);
    box-shadow: inset -1px 0 0 var(--separator);
    overflow-y: auto;
  }
  nav button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 30px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: left;
    cursor: default;
  }
  nav button:hover:not(.active) {
    background: var(--fill-hover);
  }
  nav button.active {
    background: var(--accent);
    color: var(--accent-fg);
  }
  .glyph {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 5px;
    background: linear-gradient(to bottom, color-mix(in srgb, var(--tint) 80%, #fff), var(--tint));
    color: #fff;
    box-shadow: 0 0.5px 1px rgb(0 0 0 / 0.15);
  }
  .pane {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4) var(--space-5) var(--space-6);
    overflow-y: auto;
    background: color-mix(in srgb, var(--fill) 50%, transparent);
  }
  h2 {
    margin: 0 0 var(--space-2);
    font-size: var(--text-large);
    font-weight: 700;
    letter-spacing: -0.01em;
  }
  /* Groups keep their height; the pane scrolls instead. */
  .pane > :global(*) {
    flex-shrink: 0;
  }
  .pane :global(.section-label) {
    margin-top: var(--space-3);
  }
  .problem {
    flex: 1;
    margin-right: auto;
  }
  .settings.compact {
    flex-direction: column;
    height: min(72dvh, 620px);
  }
  .compact nav {
    flex: 1;
    min-height: 0;
    box-shadow: none;
  }
  .compact nav button {
    flex-shrink: 0;
    min-height: max(44px, var(--row-height));
    cursor: pointer;
  }
  .compact .glyph,
  .nav-chevron {
    flex: none;
  }
  .nav-chevron {
    display: inline-flex;
    margin-inline-start: auto;
    color: var(--fg-muted);
  }
  .compact .pane {
    min-height: 0;
    padding: var(--space-3) var(--space-3) var(--space-5);
  }
  .back-btn {
    min-height: var(--control-height);
  }
</style>
