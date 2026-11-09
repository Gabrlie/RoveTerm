<script lang="ts">
  import Select from '../../ui/Select.svelte';
  import { t } from '../../i18n';
  import { prefs } from '../prefs.svelte';
  import { editPrefs } from '../edit.svelte';

  const INTERVALS = [2, 3, 5, 10, 30];
  const interval = $derived(prefs.value.status.interval_secs);
</script>

<div class="form-group">
  <div class="form-row">
    <span class="form-label">{t('settings.statusInterval')}</span>
    <div class="control">
      <Select
        label={t('settings.statusInterval')}
        value={String(interval)}
        options={[...new Set([...INTERVALS, interval])].sort((a, b) => a - b).map((n) => ({ value: String(n), label: t('settings.seconds', { n }) }))}
        onchange={(value) => void editPrefs((draft) => (draft.status.interval_secs = Number(value)))}
      />
    </div>
  </div>
</div>

<style>
  .control {
    width: 120px;
  }
</style>
