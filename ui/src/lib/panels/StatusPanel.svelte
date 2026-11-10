<script lang="ts" module>
  import type { ServerStats as Stats } from '../ipc';

  /** What a panel showed for one host, kept across remounts (switching panes, closing the panel)
   *  so returning shows it at once while fresh samples arrive. Lives as long as the window. */
  interface Snapshot {
    stats: Stats;
    previous: { stats: Stats; at: number };
    cpu: number | null;
    coreHistory: number[][];
    rates: Record<string, { rx: number; tx: number }>;
  }
  const snapshots = new Map<string, Snapshot>();
</script>

<script lang="ts">
  // Server status of the focused pane's remote, sampled every few seconds over its live
  // connection, shown as cards: system, CPU, memory, network, disks. Usage and rates come from
  // the difference between two samples.
  import { onDestroy } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import CoreChart from './CoreChart.svelte';
  import { isCommandError, serverStats, type CpuTimes, type RemoteSource, type ServerStats } from '../ipc';
  import { errorText, t } from '../i18n';
  import { formatSize } from '../files/format';
  import { prefs } from '../settings/prefs.svelte';
  import { sourceKey } from '../workspace/workspace.svelte';

  /** With no `source` (nothing remote focused) the panel shows its skeleton and samples nothing. */
  let { source, visible }: { source: RemoteSource | null; visible: boolean } = $props();

  /** Samples kept per core: two minutes at the default interval. */
  const HISTORY = 40;
  /** Interfaces shown before the list collapses. */
  const INTERFACES_SHOWN = 6;

  // The panel is keyed by source, so the key never changes for one instance.
  // svelte-ignore state_referenced_locally
  const key = source ? sourceKey(source) : null;
  const cached = key ? snapshots.get(key) : undefined;
  let stats = $state<ServerStats | null>(cached?.stats ?? null);
  let previous: { stats: ServerStats; at: number } | null = cached?.previous ?? null;
  let cpu = $state<number | null>(cached?.cpu ?? null);
  /** Usage history of each core, oldest first. */
  let coreHistory = $state<number[][]>(cached?.coreHistory ?? []);
  let rates = $state<Record<string, { rx: number; tx: number }>>(cached?.rates ?? {});
  let showAllInterfaces = $state(false);
  let problem = $state<string | null>(null);
  let timer = 0;
  let stopped = false;

  function usage(now: CpuTimes, before: CpuTimes | undefined): number | null {
    if (!before) return null;
    const total = now.total - before.total;
    const idle = now.idle - before.idle;
    return total > 0 ? Math.min(100, Math.max(0, ((total - idle) / total) * 100)) : null;
  }

  async function sample(): Promise<void> {
    if (!source || !key) return;
    try {
      const next = await serverStats(source);
      if (stopped) return;
      const now = Date.now();
      if (previous) {
        const before = previous.stats;
        cpu = usage(next.cpu, before.cpu);
        coreHistory = next.cpus.map((core, index) => [
          ...(coreHistory[index] ?? []).slice(1 - HISTORY),
          usage(core, before.cpus[index]) ?? 0,
        ]);
        const seconds = (now - previous.at) / 1000;
        const nextRates: Record<string, { rx: number; tx: number }> = {};
        for (const net of next.interfaces) {
          const old = before.interfaces.find((candidate) => candidate.name === net.name);
          if (old && seconds > 0) {
            nextRates[net.name] = { rx: Math.max(0, (net.rx - old.rx) / seconds), tx: Math.max(0, (net.tx - old.tx) / seconds) };
          }
        }
        rates = nextRates;
      }
      previous = { stats: next, at: now };
      stats = next;
      snapshots.set(key, { stats: next, previous, cpu, coreHistory: $state.snapshot(coreHistory), rates: $state.snapshot(rates) });
      problem = null;
    } catch (error) {
      if (stopped) return;
      // Not connected yet: keep showing the skeleton until the connection is up.
      if (isCommandError(error) && error.code === 'ssh.not_connected') return;
      problem = isCommandError(error) && error.code === 'monitor.unsupported' ? t('status.unsupported') : errorText(error);
    }
  }

  function schedule(): void {
    clearTimeout(timer);
    if (stopped || !visible || !source) return;
    timer = window.setTimeout(async () => {
      await sample();
      schedule();
    }, prefs.value.status.interval_secs * 1000);
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

  /** Names of the segments in the order of {@link latency}: one per jump host, then the target. */
  function segmentNames(count: number): string {
    return [...Array<string>(count - 1).fill(t('status.segmentJump')), t('status.segmentTarget')].join(' + ');
  }

  /** Segment latencies as `12+85`: one decimal below 10 ms. */
  function latency(segments: number[]): string {
    return segments.map((us) => (us < 10_000 ? (us / 1000).toFixed(1) : Math.round(us / 1000).toString())).join('+');
  }

  function rate(bytes: number | undefined): string {
    return bytes === undefined ? '…' : `${formatSize(Math.round(bytes))}/s`;
  }

  let coresWidth = $state(0);
  /** Narrowest chart: labelled charts up to 8 cores, compact (value only) beyond. */
  const coreMinWidth = $derived(coreHistory.length <= 8 ? 110 : coreHistory.length <= 32 ? 64 : 46);
  /** Charts per row: as many as fit, then evened out so the last row is not left mostly empty
   *  (4 cores in a narrow panel become 2 × 2). */
  const coreColumns = $derived.by(() => {
    const count = Math.max(coreHistory.length, 1);
    const fit = Math.max(1, Math.min(count, Math.floor((coresWidth + 4) / (coreMinWidth + 4))));
    return Math.ceil(count / Math.ceil(count / fit));
  });

  /** Interfaces, busiest first. */
  const interfaces = $derived(
    [...(stats?.interfaces ?? [])].sort(
      (a, b) => (rates[b.name]?.rx ?? 0) + (rates[b.name]?.tx ?? 0) - ((rates[a.name]?.rx ?? 0) + (rates[a.name]?.tx ?? 0)),
    ),
  );
</script>

<div class="status">
  {#if problem && !stats}
    <p class="muted">{problem}</p>
  {:else if !stats}
    <!-- Skeleton: the cards' layout with placeholders where values will appear. -->
    <div class="loading" aria-busy="true" aria-label={t('status.loading')}>
      <section class="card">
        <h4><Icon name="server" size={14} />{t('status.system')}</h4>
        <dl class="facts">
          {#each ['status.hostname', 'status.os', 'status.response', 'status.uptimeLabel'] as const as label (label)}
            <div><dt>{t(label)}</dt><dd><span class="skeleton" style:width="70%"></span></dd></div>
          {/each}
          <div class="wide"><dt>{t('status.kernel')}</dt><dd><span class="skeleton" style:width="55%"></span></dd></div>
        </dl>
      </section>
      <section class="card">
        <h4><Icon name="terminal" size={14} />CPU</h4>
        <div class="headline"><span class="skeleton" style:width="64px" style:height="24px"></span></div>
        <div class="cores skeleton-cores">
          {#each [0, 1, 2, 3] as index (index)}<span class="skeleton"></span>{/each}
        </div>
        <div class="row muted"><span>{t('status.load')}</span><span class="skeleton" style:width="90px"></span></div>
      </section>
      <section class="card">
        <h4><Icon name="transfer" size={14} />{t('status.memory')}</h4>
        {#each ['status.physical', 'status.swap'] as const as label (label)}
          <div class="row"><span>{t(label)}</span><span class="skeleton" style:width="32px"></span></div>
          <div class="bar"></div>
          <span class="skeleton" style:width="45%"></span>
        {/each}
      </section>
      <section class="card">
        <h4><Icon name="forwards" size={14} />{t('status.network')}</h4>
        {#each [0, 1, 2] as index (index)}<span class="skeleton net-skeleton"></span>{/each}
      </section>
      <section class="card">
        <h4><Icon name="folder" size={14} />{t('status.disks')}</h4>
        <div class="row"><span class="skeleton" style:width="30%"></span><span class="skeleton" style:width="32px"></span></div>
        <div class="bar"></div>
        <span class="skeleton" style:width="45%"></span>
      </section>
    </div>
  {:else}
    {@const mem = stats.mem_total - stats.mem_available}
    {@const swap = stats.swap_total - stats.swap_free}
    <section class="card">
      <h4><Icon name="server" size={14} />{t('status.system')}</h4>
      <dl class="facts">
        <div><dt>{t('status.hostname')}</dt><dd>{stats.hostname}</dd></div>
        <div><dt>{t('status.os')}</dt><dd>{stats.os || stats.kernel.split(' ')[0]}</dd></div>
        <div title={stats.latency_us.length > 1 ? t('status.latencySegments') : undefined}><dt>{t('status.response')}{#if stats.latency_us.length > 1}<small class="parts">{t('status.segments', { names: segmentNames(stats.latency_us.length) })}</small>{/if}</dt><dd>{t('status.ms', { n: latency(stats.latency_us) })}</dd></div>
        <div><dt>{t('status.uptimeLabel')}</dt><dd>{uptime(stats.uptime_secs)}</dd></div>
        <div class="wide"><dt>{t('status.kernel')}</dt><dd>{stats.kernel}</dd></div>
      </dl>
    </section>

    <section class="card">
      <h4><Icon name="terminal" size={14} />CPU <span class="aside">{t('status.cores', { n: stats.cores })}</span></h4>
      <!-- Usage needs two samples, so it arrives one round after the rest: keep its skeleton
           (the same shape as the panel skeleton) until then. -->
      {#if cpu === null || coreHistory.length === 0}
        <div class="headline"><span class="skeleton" style:width="64px" style:height="24px"></span></div>
        <div class="cores skeleton-cores">
          {#each [0, 1, 2, 3] as index (index)}<span class="skeleton"></span>{/each}
        </div>
      {:else}
        <div class="headline">
          <strong class:hot={cpu > 85}>{`${cpu.toFixed(0)}%`}</strong>
          <span class="muted">{t('status.average')}</span>
        </div>
        <div class="cores" bind:clientWidth={coresWidth} style:grid-template-columns="repeat({coreColumns}, minmax(0, 1fr))" aria-label={t('status.perCore')}>
          {#each coreHistory as points, index (index)}
            <CoreChart label={`CPU ${index}`} {points} capacity={HISTORY} compact={coreMinWidth < 110} />
          {/each}
        </div>
      {/if}
      <div class="row muted"><span>{t('status.load')}</span><span class="mono">{stats.load.map((value) => value.toFixed(2)).join('  ')}</span></div>
    </section>

    <section class="card">
      <h4><Icon name="transfer" size={14} />{t('status.memory')}</h4>
      <div class="row"><span>{t('status.physical')}</span><span class="value">{percent(mem, stats.mem_total).toFixed(0)}%</span></div>
      <div class="bar"><div class="fill" class:hot={percent(mem, stats.mem_total) > 90} style:width="{percent(mem, stats.mem_total)}%"></div></div>
      <div class="muted mono">{formatSize(mem)} / {formatSize(stats.mem_total)}</div>
      <div class="row"><span>{t('status.swap')}</span><span class="value">{percent(swap, stats.swap_total).toFixed(0)}%</span></div>
      <div class="bar"><div class="fill" style:width="{percent(swap, stats.swap_total)}%"></div></div>
      <div class="muted mono">{formatSize(swap)} / {formatSize(stats.swap_total)}</div>
    </section>

    <section class="card">
      <h4><Icon name="forwards" size={14} />{t('status.network')}</h4>
      {#each showAllInterfaces ? interfaces : interfaces.slice(0, INTERFACES_SHOWN) as net (net.name)}
        <div class="net">
          <span class="name mono" title={net.name}>{net.name}</span>
          <span class="up mono">↑ {rate(rates[net.name]?.tx)}</span>
          <span class="down mono">↓ {rate(rates[net.name]?.rx)}</span>
        </div>
      {:else}
        <p class="muted">{t('status.noInterfaces')}</p>
      {/each}
      {#if interfaces.length > INTERFACES_SHOWN}
        <button type="button" class="more" onclick={() => (showAllInterfaces = !showAllInterfaces)}>
          {showAllInterfaces ? t('status.showLess') : t('status.showAll', { n: interfaces.length })}
        </button>
      {/if}
    </section>

    <section class="card">
      <h4><Icon name="folder" size={14} />{t('status.disks')}</h4>
      {#each stats.disks as disk (disk.filesystem)}
        {@const used = percent(disk.used, disk.total)}
        <div class="disk" title={disk.filesystem}>
          <div class="row"><span class="mono name">{disk.mount}</span><span class="value" class:danger={used > 90}>{used.toFixed(0)}%</span></div>
          <div class="bar"><div class="fill" class:hot={used > 90} style:width="{used}%"></div></div>
          <div class="muted mono">{formatSize(disk.used)} / {formatSize(disk.total)}</div>
        </div>
      {:else}
        <p class="muted">{t('status.noDisks')}</p>
      {/each}
    </section>
    {#if problem}<p class="muted">{problem}</p>{/if}
  {/if}
</div>

<style>
  .status {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2);
    font-size: var(--text-footnote);
    color: var(--fg);
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow:
      0 0 0 0.5px var(--separator),
      var(--shadow-sm);
  }
  h4 {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin: 0 0 var(--space-1);
    font-size: var(--text-body);
    font-weight: 600;
  }
  h4 :global(svg) {
    color: var(--accent);
  }
  .aside {
    margin-left: auto;
    color: var(--fg-muted);
    font-weight: normal;
    font-size: var(--text-footnote);
  }
  .facts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
    margin: 0;
  }
  .facts .wide {
    grid-column: 1 / -1;
  }
  .facts dt {
    color: var(--fg-muted);
    font-size: var(--text-caption);
  }
  .parts {
    margin-left: 2px;
    color: var(--fg-subtle);
    font-size: var(--text-caption);
  }
  .facts dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .headline {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
  }
  .headline strong {
    color: var(--accent);
    font-size: 24px;
    font-weight: 600;
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
  }
  .headline strong.hot {
    color: var(--danger);
  }
  .loading {
    display: contents;
  }
  .skeleton-cores {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .skeleton-cores .skeleton {
    width: 100%;
    height: 54px;
    border-radius: var(--radius-sm);
  }
  .net-skeleton {
    width: 100%;
    height: 22px;
    border-radius: var(--radius-sm);
  }
  .cores {
    display: grid;
    gap: 4px;
    margin: var(--space-1) 0;
  }
  .row {
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .value {
    color: var(--accent);
    font-weight: 600;
  }
  .value.danger,
  .hot {
    color: var(--danger);
  }
  .muted {
    margin: 0;
    color: var(--fg-muted);
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
  }
  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--fill-pressed);
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
  .net {
    display: grid;
    grid-template-columns: minmax(4em, 1fr) auto auto;
    gap: var(--space-2);
    align-items: center;
    padding: 3px var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .up {
    color: var(--success);
  }
  .down {
    color: var(--accent);
  }
  .disk {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .disk + .disk {
    margin-top: var(--space-2);
  }
  .more {
    align-self: flex-start;
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
  }
</style>
