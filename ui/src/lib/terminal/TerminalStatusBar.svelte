<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import type { SessionId, StatsSource } from '../ipc';
  import { t } from '../i18n';
  import { formatSize } from '../files/format';
  import Icon from '../ui/Icon.svelte';
  import {
    combinedNetworkRate,
    describeTailscalePath,
    formatLatencyMs,
    formatMemoryBar,
    formatNetworkRate,
    observeServerStats,
  } from '../panels/serverStats.svelte';
  import { prefs } from '../settings/prefs.svelte';
  import { currentScheme } from './theme';

  let { source, session, visible }: { source: StatsSource; session: SessionId | null; visible: boolean } = $props();

  const media = matchMedia('(prefers-color-scheme: dark)');
  let systemDark = $state(media.matches);
  function onMediaChange(): void {
    systemDark = media.matches;
  }
  onMount(() => media.addEventListener('change', onMediaChange));
  onDestroy(() => media.removeEventListener('change', onMediaChange));

  const scheme = $derived(currentScheme(systemDark));
  const barStyle = $derived.by(() => {
    const theme = scheme.theme;
    const bg = theme.background ?? (scheme.dark ? '#1a1a1c' : '#ffffff');
    const fg = theme.foreground ?? (scheme.dark ? '#f2f2f4' : '#1d1d1f');
    const success = theme.green ?? (scheme.dark ? '#32d74b' : '#248a3d');
    const warning = theme.yellow ?? (scheme.dark ? '#ffd60a' : '#b25000');
    const danger = theme.red ?? (scheme.dark ? '#ff453a' : '#d70015');
    return [
      `--bar-term-bg: ${bg}`,
      `--bar-fg: ${fg}`,
      `--bar-success: ${success}`,
      `--bar-warning: ${warning}`,
      `--bar-danger: ${danger}`,
    ].join('; ');
  });

  const settings = $derived(prefs.value.status);
  const enabled = $derived(
    settings.terminal_bar &&
      (settings.terminal_bar_cpu ||
        settings.terminal_bar_memory ||
        settings.terminal_bar_network ||
        settings.terminal_bar_latency ||
        settings.terminal_bar_tailscale_path),
  );
  const sampling = observeServerStats(() => (visible && enabled ? source : null), () => session);
  const { stats, cpu, totalRate, problem } = $derived(sampling.state);
  const memUsed = $derived(stats ? Math.max(0, stats.mem_total - stats.mem_available) : 0);
  const memPercent = $derived(stats && stats.mem_total > 0 ? (memUsed / stats.mem_total) * 100 : 0);
  const badge = $derived(describeTailscalePath(stats?.tailscale_path));
  const hasVisibleMetric = $derived(
    Boolean(
      stats &&
        (settings.terminal_bar_cpu ||
          settings.terminal_bar_memory ||
          settings.terminal_bar_network ||
          (settings.terminal_bar_latency && stats.latency_us.length > 0) ||
          (settings.terminal_bar_tailscale_path && badge !== null)),
    ),
  );
</script>

{#if enabled && problem !== 'monitor.unsupported' && (!stats || hasVisibleMetric)}
  <div class="status-bar" style={barStyle} role="group" aria-label={t('settings.statusTerminalBar')}>
    {#if stats}
      {#if settings.terminal_bar_cpu}
        <span class="metric" class:warning={cpu !== null && cpu > 85} title={t('status.cpu')} aria-label={t('status.cpu')}>
          <span class="icon"><Icon name="cpu" size={12} /></span>
          <span>{cpu === null ? '…' : t('status.percent', { n: cpu.toFixed(0) })}</span>
        </span>
      {/if}
      {#if settings.terminal_bar_memory}
        {@const fullMemory = formatMemoryBar(memUsed, stats.mem_total, 'full_percent', formatSize)}
        <span class="metric" class:danger={memPercent > 90} title={`${t('status.memory')}: ${fullMemory}`} aria-label={t('status.memory')}>
          <span class="icon"><Icon name="memory" size={12} /></span>
          <span>{formatMemoryBar(memUsed, stats.mem_total, settings.terminal_bar_memory_display, formatSize)}</span>
        </span>
      {/if}
      {#if settings.terminal_bar_network}
        <span class="metric" title={t('status.network')} aria-label={t('status.network')}>
          <span class="icon"><Icon name="transfer" size={12} /></span>
          {#if settings.terminal_bar_network_display === 'total'}
            <span>{formatNetworkRate(combinedNetworkRate(totalRate), settings.network_unit)}</span>
          {:else}
            <span class="net-pair">
              <span class="dir">↑</span>{formatNetworkRate(totalRate?.tx, settings.network_unit)}
              <span class="dir">↓</span>{formatNetworkRate(totalRate?.rx, settings.network_unit)}
            </span>
          {/if}
        </span>
      {/if}
      {#if settings.terminal_bar_latency && stats.latency_us.length}
        <span class="metric" title={t('status.response')} aria-label={t('status.response')}>
          <span class="icon"><Icon name="gauge" size={12} /></span>
          <span>{t('status.ms', { n: formatLatencyMs(stats.latency_us) })}</span>
        </span>
      {/if}
      {#if settings.terminal_bar_tailscale_path && badge}
        <span class="path-badge" class:relay={badge.kind === 'relay'} title={badge.title}>
          <Icon name="route" size={11} />
          <span>{badge.kind === 'direct' ? t('status.pathDirect') : badge.region ? t('status.pathRelayRegion', { region: badge.region }) : t('status.pathRelay')}</span>
        </span>
      {/if}
    {:else if !problem}
      <span class="muted" aria-label={t('status.loading')}>…</span>
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
    background: color-mix(in srgb, var(--bar-fg) 4%, var(--bar-term-bg));
    box-shadow: inset 0 1px 0 color-mix(in srgb, var(--bar-fg) 14%, var(--bar-term-bg));
    color: var(--bar-fg);
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .metric,
  .path-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .icon,
  .dir,
  .muted {
    color: color-mix(in srgb, var(--bar-fg) 62%, var(--bar-term-bg));
  }
  .icon {
    display: inline-flex;
    flex: none;
    align-items: center;
  }
  .net-pair {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .warning {
    color: var(--bar-warning);
  }
  .warning .icon {
    color: var(--bar-warning);
  }
  .danger {
    color: var(--bar-danger);
  }
  .danger .icon {
    color: var(--bar-danger);
  }
  .path-badge {
    flex-shrink: 0;
    padding: 1px var(--space-2);
    border-radius: var(--radius-pill);
    background: color-mix(in srgb, var(--bar-success) 16%, transparent);
    color: var(--bar-success);
    font-family: var(--font-ui);
    font-weight: 600;
    line-height: 1.3;
  }
  .path-badge.relay {
    background: color-mix(in srgb, var(--bar-warning) 16%, transparent);
    color: var(--bar-warning);
  }
  @media (max-width: 767px) {
    .status-bar {
      min-width: 0;
      overflow-x: auto;
      scrollbar-width: none;
      touch-action: pan-x;
    }
    .metric,
    .path-badge {
      flex: none;
    }
  }
</style>
