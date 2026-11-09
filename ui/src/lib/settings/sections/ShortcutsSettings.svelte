<script lang="ts">
  import Icon from '../../ui/Icon.svelte';
  import { actions, bindingsOf, displayBinding } from '../../actions';
  import { t } from '../../i18n';

  let query = $state('');
  const listed = $derived(
    actions
      .filter((action) => !action.hidden)
      .filter((action) => {
        const needle = query.trim().toLowerCase();
        return !needle || t(action.label).toLowerCase().includes(needle) || bindingsOf(action).some((binding) => binding.toLowerCase().includes(needle));
      }),
  );
</script>

<div class="search">
  <Icon name="search" size={14} />
  <input class="bare" bind:value={query} placeholder={t('settings.shortcutSearch')} spellcheck="false" />
</div>
<div class="form-group">
  {#each listed as action (action.id)}
    <div class="form-row compact">
      <span class="form-label">{t(action.label)}</span>
      <span class="keys">
        {#each bindingsOf(action) as binding (binding)}<kbd>{displayBinding(binding)}</kbd>{:else}<span class="none">{t('settings.noShortcut')}</span>{/each}
      </span>
    </div>
  {/each}
</div>
<p class="form-note">{t('settings.shortcutsHint')}</p>

<style>
  .search {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--control-height);
    padding: 0 var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg-muted);
  }
  .search:focus-within {
    background: var(--surface);
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--accent) 70%, transparent),
      var(--focus-ring);
  }
  .search input {
    flex: 1;
    min-width: 0;
    color: var(--fg);
    outline: none;
  }
  .compact {
    min-height: 34px;
  }
  .keys {
    display: flex;
    gap: var(--space-1);
  }
  kbd {
    padding: 1px 7px;
    border-radius: var(--radius-xs);
    background: var(--fill);
    box-shadow: inset 0 -0.5px 0 var(--border);
    color: var(--fg);
    font-family: var(--font-ui);
    font-size: var(--text-caption);
    font-weight: 500;
  }
  .none {
    color: var(--fg-subtle);
    font-size: var(--text-footnote);
  }
</style>
