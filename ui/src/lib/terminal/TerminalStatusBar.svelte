<script lang="ts">
  import type { RemoteSource, SessionId } from '../ipc';
  import { t } from '../i18n';
  import { formatSize } from '../files/format';
  import { describeTailscalePath, formatLatencyMs, formatNetworkRate, observeServerStats } from '../panels/serverStats.svelte';
  import { prefs } from '../settings/prefs.svelte';

  let { source, session, visible }: { source: RemoteSource; session: SessionId | null; visible: boolean } = $props();

  const settings = $derived(prefs.value.status);
  const enabled = $derived(settings.terminal_bar &&
    (settings.terminal_bar_cpu || settings.terminal_bar_memory || settings.terminal_bar_network || settings.terminal_bar_latency));
  const sampling = observeServerStats(() => visible && enabled ? source : null, () => session);
  const { stats, cpu, totalRate, problem } = $derived(sampling.state);
  const memUsed = $derived(stats ? Math.max(0, stats.mem_total - stats.mem_available) : 0);
  const memPercent = $derived(stats && stats.mem_total > 0 ? (memUsed / stats.mem_total) * 100 : 0);
  const badge = $derived(describeTailscalePath(stats?.tailscale_path));
</script>

{#if enabled}
  <div class="status-bar" role="group" aria-label={t('settings.statusTerminalBar')}>
    {#if problem !== 'monitor.unsupported'}
      {#if stats}
        {#if settings.terminal_bar_cpu}
          <span class="metric" class:warning={cpu !== null && cpu > 85} title={t('status.cpu')}>
            <span class="label">{t('status.cpu')}</span>
            {cpu === null ? '…' : t('status.percent', { n: cpu.toFixed(0) })}
          </span>
        {/if}
        {#if settings.terminal_bar_memory}
          <span class="metric" class:danger={memPercent > 90} title={t('status.memory')}>
            <span class="label">{t('status.memoryShort')}</span>
            {t('status.memoryUsage', { used: formatSize(memUsed), total: formatSize(stats.mem_total), percent: memPercent.toFixed(0) })}
          </span>
        {/if}
        {#if settings.terminal_bar_network}
          <span class="metric" title={t('status.network')}>
            <span class="label">↑</span> {formatNetworkRate(totalRate?.tx, settings.network_unit)}
            <span class="label">↓</span> {formatNetworkRate(totalRate?.rx, settings.network_unit)}
          </span>
        {/if}
        {#if settings.terminal_bar_latency && stats.latency_us.length}
          <span class="metric" title={t('status.response')}>{t('status.ms', { n: formatLatencyMs(stats.latency_us) })}</span>
          {#if badge}
            <span class="path-badge" class:relay={badge.kind === 'relay'} title={badge.title}>
              {badge.kind === 'direct' ? t('status.pathDirect') : badge.region ? t('status.pathRelayRegion', { region: badge.region }) : t('status.pathRelay')}
            </span>
          {/if}
        {/if}
      {:else if !problem}
        <span class="label" aria-label={t('status.loading')}>…</span>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .status-bar {
    flex: 0 0 var(--space-6);
    display: flex;
    gap: var(--space-3);
    align-items: center;
    overflow: hidden;
    padding: 0 var(--space-2);
    background: var(--material-bar);
    box-shadow: inset 0 1px 0 var(--separator);
    color: var(--fg);
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .metric,
  .path-badge {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .label {
    color: var(--fg-muted);
  }
  .warning {
    color: var(--warning);
  }
  .danger {
    color: var(--danger);
  }
  .path-badge {
    padding: 0 var(--space-1);
    border-radius: var(--radius-pill);
    background: var(--success-soft);
    color: var(--success);
    font-family: var(--font-ui);
    font-weight: 600;
  }
  .path-badge.relay {
    background: var(--warning-soft);
    color: var(--warning);
  }
</style>
