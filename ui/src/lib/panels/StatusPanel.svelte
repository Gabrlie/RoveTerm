<script lang="ts">
  // Server status of the focused pane's remote, sampled every few seconds over its live
  // connection. CPU usage and throughput come from the difference between two samples.
  import { onDestroy } from 'svelte';
  import { isCommandError, serverStats, type RemoteSource, type ServerStats } from '../ipc';
  import { errorText, t } from '../i18n';
  import { formatSize } from '../files/format';

  let { source, visible }: { source: RemoteSource; visible: boolean } = $props();

  /** Time between samples. */
  const INTERVAL_MS = 3000;

  let stats = $state<ServerStats | null>(null);
  let previous: { stats: ServerStats; at: number } | null = null;
  let cpu = $state<number | null>(null);
  let rates = $state<{ rx: number; tx: number } | null>(null);
  let problem = $state<string | null>(null);
  let timer = 0;
  let stopped = false;

  async function sample(): Promise<void> {
    try {
      const next = await serverStats(source);
      if (stopped) return;
      const now = Date.now();
      if (previous) {
        const total = next.cpu_total - previous.stats.cpu_total;
        const idle = next.cpu_idle - previous.stats.cpu_idle;
        cpu = total > 0 ? Math.min(100, Math.max(0, ((total - idle) / total) * 100)) : null;
        const seconds = (now - previous.at) / 1000;
        rates = seconds > 0 ? { rx: Math.max(0, (next.net_rx - previous.stats.net_rx) / seconds), tx: Math.max(0, (next.net_tx - previous.stats.net_tx) / seconds) } : null;
      }
      previous = { stats: next, at: now };
      stats = next;
      problem = null;
    } catch (error) {
      if (stopped) return;
      problem =
        isCommandError(error) && error.code === 'ssh.not_connected'
          ? t('status.notConnected')
          : isCommandError(error) && error.code === 'monitor.unsupported'
            ? t('status.unsupported')
            : errorText(error);
    }
  }

  function schedule(): void {
    clearTimeout(timer);
    if (stopped || !visible) return;
    timer = window.setTimeout(async () => {
      await sample();
      schedule();
    }, INTERVAL_MS);
  }

  // Sample immediately when shown, then on a timer while visible.
  $effect(() => {
    if (visible) {
      void sample().then(schedule);
    } else {
      clearTimeout(timer);
    }
  });

  onDestroy(() => {
    stopped = true;
    clearTimeout(timer);
  });

  function percent(used: number, total: number): number {
    return total > 0 ? Math.min(100, (used / total) * 100) : 0;
  }

  function uptime(seconds: number): string {
    const days = Math.floor(seconds / 86400);
    const hours = Math.floor((seconds % 86400) / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    return days > 0 ? t('status.uptimeDays', { days, hours }) : t('status.uptimeHours', { hours, minutes });
  }
</script>

<div class="status">
  {#if problem && !stats}
    <p class="muted">{problem}</p>
  {:else if !stats}
    <p class="muted">{t('status.loading')}</p>
  {:else}
    <div class="head">
      <strong>{stats.hostname}</strong>
      <span class="muted">{stats.kernel}</span>
      <span class="muted">{t('status.uptime', { value: uptime(stats.uptime_secs) })}</span>
    </div>

    {@const mem = stats.mem_total - stats.mem_available}
    {@const swap = stats.swap_total - stats.swap_free}
    <section>
      <div class="row"><span>{t('status.cpu')}</span><span>{cpu === null ? '…' : `${cpu.toFixed(0)}%`} · {t('status.cores', { n: stats.cores })}</span></div>
      <div class="bar"><div class="fill" class:hot={(cpu ?? 0) > 85} style:width="{cpu ?? 0}%"></div></div>
      <div class="row muted"><span>{t('status.load')}</span><span>{stats.load.map((value) => value.toFixed(2)).join('  ')}</span></div>
    </section>
    <section>
      <div class="row"><span>{t('status.memory')}</span><span>{formatSize(mem)} / {formatSize(stats.mem_total)}</span></div>
      <div class="bar"><div class="fill" class:hot={percent(mem, stats.mem_total) > 90} style:width="{percent(mem, stats.mem_total)}%"></div></div>
      {#if stats.swap_total > 0}
        <div class="row muted"><span>{t('status.swap')}</span><span>{formatSize(swap)} / {formatSize(stats.swap_total)}</span></div>
      {/if}
    </section>
    <section>
      <div class="row"><span>{t('status.disk')}</span><span>{formatSize(stats.disk_used)} / {formatSize(stats.disk_total)}</span></div>
      <div class="bar"><div class="fill" class:hot={percent(stats.disk_used, stats.disk_total) > 90} style:width="{percent(stats.disk_used, stats.disk_total)}%"></div></div>
    </section>
    <section>
      <div class="row"><span>{t('status.network')}</span><span>{rates ? `↓ ${formatSize(rates.rx)}/s  ↑ ${formatSize(rates.tx)}/s` : '…'}</span></div>
    </section>
    {#if problem}<p class="muted">{problem}</p>{/if}
  {/if}
</div>

<style>
  .status {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-3);
    font-size: var(--text-footnote);
    color: var(--fg);
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-wrap: anywhere;
  }
  .head strong {
    font-size: var(--text-body);
  }
  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .row {
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .muted {
    margin: 0;
    color: var(--fg-muted);
  }
  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--bg-sunken);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    border-radius: 3px;
    background: var(--accent);
    transition: width 0.4s ease;
  }
  .fill.hot {
    background: var(--danger);
  }
</style>
