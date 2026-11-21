<script lang="ts">
  import Select from '../../ui/Select.svelte';
  import Switch from '../../ui/Switch.svelte';
  import { t, type MessageKey } from '../../i18n';
  import type { StatusPrefs } from '../../ipc';
  import { prefs } from '../prefs.svelte';
  import { editPrefs } from '../edit.svelte';

  const INTERVALS = [2, 3, 5, 10, 30];
  const CPU_MODES = ['auto', 'per_core', 'summary'] as const;
  const NETWORK_UNITS = ['bytes', 'bits'] as const;
  const status = $derived(prefs.value.status);
  const intervals = $derived([...new Set([...INTERVALS, status.interval_secs])].sort((a, b) => a - b));

  type Toggle = { [K in keyof StatusPrefs]: StatusPrefs[K] extends boolean ? K : never }[keyof StatusPrefs];
  const CARDS = [
    ['show_system', 'status.system'],
    ['show_cpu', 'status.cpu'],
    ['show_memory', 'status.memory'],
    ['show_swap', 'status.swap'],
    ['show_network', 'status.network'],
    ['show_disks', 'status.disks'],
  ] as const satisfies readonly [Toggle, MessageKey][];
  const METRICS = [
    ['terminal_bar_cpu', 'status.cpu'],
    ['terminal_bar_memory', 'status.memory'],
    ['terminal_bar_network', 'status.network'],
    ['terminal_bar_latency', 'settings.statusLatencyPath'],
  ] as const satisfies readonly [Toggle, MessageKey][];

  function update<K extends keyof StatusPrefs>(key: K, value: StatusPrefs[K]): void {
    void editPrefs((draft) => { draft.status[key] = value; });
  }
</script>

{#snippet toggle(key: Toggle, label: MessageKey, disabled = false)}
  <div class="form-row">
    <span class="form-label" class:nested={key === 'show_swap'}>{t(label)}</span>
    <Switch label={t(label)} checked={status[key]} {disabled} onchange={(checked) => update(key, checked)} />
  </div>
{/snippet}

<p class="section-label">{t('settings.statusSampling')}</p>
<div class="form-group">
  <div class="form-row">
    <span class="form-label">{t('settings.statusInterval')}</span>
    <div class="control">
      <Select
        label={t('settings.statusInterval')}
        value={String(status.interval_secs)}
        options={intervals.map((n) => ({ value: String(n), label: t('settings.seconds', { n }) }))}
        onchange={(value) => update('interval_secs', Number(value))}
      />
    </div>
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.statusCpuDisplay')}</span>
    <div class="control">
      <Select
        label={t('settings.statusCpuDisplay')}
        value={status.cpu_display}
        options={CPU_MODES.map((value) => ({ value, label: t(`settings.statusCpuDisplay.${value}`) }))}
        onchange={(value) => update('cpu_display', value)}
      />
    </div>
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.statusNetworkUnit')}</span>
    <div class="control">
      <Select
        label={t('settings.statusNetworkUnit')}
        value={status.network_unit}
        options={NETWORK_UNITS.map((value) => ({ value, label: t(`settings.statusNetworkUnit.${value}`) }))}
        onchange={(value) => update('network_unit', value)}
      />
    </div>
  </div>
</div>

<p class="section-label">{t('settings.statusCards')}</p>
<div class="form-group">
  {#each CARDS as [key, label] (key)}
    {@render toggle(key, label, key === 'show_swap' && !status.show_memory)}
  {/each}
</div>

<p class="section-label">{t('settings.statusTerminalBar')}</p>
<div class="form-group">
  {@render toggle('terminal_bar', 'settings.statusShowTerminalBar')}
  {#if status.terminal_bar}
    {#each METRICS as [key, label] (key)}
      {@render toggle(key, label)}
    {/each}
  {/if}
</div>

<style>
  .control {
    width: 16em;
    max-width: 62%;
  }
  .nested {
    padding-left: var(--space-3);
  }
</style>
