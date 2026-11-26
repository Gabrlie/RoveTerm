<script lang="ts">
  import Icon from '../ui/Icon.svelte';
  import CoreChart from './CoreChart.svelte';
  import type { SessionId, StatsSource } from '../ipc';
  import { errorText, t } from '../i18n';
  import { formatSize } from '../files/format';
  import { prefs } from '../settings/prefs.svelte';
  import { describeTailscalePath, formatLatencyMs, formatNetworkRate, HISTORY, observeServerStats } from './serverStats.svelte';

  let { source, visible, session = null }: { source: StatsSource | null; visible: boolean; session?: SessionId | null } = $props();

  const INTERFACES_SHOWN = 6;
  const isLocal = $derived(!!source && 'kind' in source && source.kind === 'local');
  const settings = $derived(prefs.value.status);
  const showCards = $derived(settings.show_system || settings.show_cpu || settings.show_memory || settings.show_network || settings.show_disks);
  const sampling = observeServerStats(() => visible && showCards ? source : null, () => session);
  const { stats, cpu, coreHistory, rates } = $derived(sampling.state);
  const problem = $derived(sampling.state.problem === 'monitor.unsupported'
    ? t('status.unsupported')
    : sampling.state.problem ? errorText(sampling.state.error) : null);
  const badge = $derived(describeTailscalePath(stats?.tailscale_path));
  const showCores = $derived(settings.cpu_display === 'per_core' ||
    (settings.cpu_display === 'auto' && (stats?.cpus.length ?? 0) <= 8));
  let showAllInterfaces = $state(false);

  function percent(used: number, total: number): number {
    return total > 0 ? Math.min(100, (used / total) * 100) : 0;
  }

  function uptime(seconds: number): string {
    const days = Math.floor(seconds / 86400);
    const hours = Math.floor((seconds % 86400) / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    return days > 0 ? t('status.uptimeDays', { days, hours }) : t('status.uptimeHours', { hours, minutes });
  }

  /** Names of the segments in the order of the latency values: one per jump host, then the target. */
  function segmentNames(count: number): string {
    return [...Array<string>(count - 1).fill(t('status.segmentJump')), t('status.segmentTarget')].join(' + ');
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
  {#if !showCards}
    <p class="muted">{t('status.cardsHidden')}</p>
  {:else if problem && !stats}
    <p class="muted">{problem}</p>
  {:else if !stats}
    <!-- Skeleton: the cards' layout with placeholders where values will appear. -->
    <div class="loading" aria-busy="true" aria-label={t('status.loading')}>
      {#if settings.show_system}
        <section class="card">
          <h4><Icon name="server" size={14} />{t('status.system')}</h4>
          <dl class="facts">
            {#each ['status.hostname', 'status.os', ...(isLocal ? [] : ['status.response'] as const), 'status.uptimeLabel'] as const as label (label)}
              <div><dt>{t(label)}</dt><dd><span class="skeleton" style:width="70%"></span></dd></div>
            {/each}
            <div class="wide"><dt>{t('status.kernel')}</dt><dd><span class="skeleton" style:width="55%"></span></dd></div>
          </dl>
        </section>
      {/if}
      {#if settings.show_cpu}
        <section class="card">
          <h4><Icon name="terminal" size={14} />{t('status.cpu')}</h4>
          <div class="headline"><span class="skeleton" style:width="64px" style:height="24px"></span></div>
          {#if showCores}
            <div class="cores skeleton-cores">
              {#each [0, 1, 2, 3] as index (index)}<span class="skeleton"></span>{/each}
            </div>
          {/if}
          <div class="row muted"><span>{t('status.load')}</span><span class="skeleton" style:width="90px"></span></div>
        </section>
      {/if}
      {#if settings.show_memory}
        <section class="card">
          <h4><Icon name="transfer" size={14} />{t('status.memory')}</h4>
          {#each ['status.physical', ...(settings.show_swap ? ['status.swap'] as const : [])] as const as label (label)}
            <div class="row"><span>{t(label)}</span><span class="skeleton" style:width="32px"></span></div>
            <div class="bar"></div>
            <span class="skeleton" style:width="45%"></span>
          {/each}
        </section>
      {/if}
      {#if settings.show_network}
        <section class="card">
          <h4><Icon name="forwards" size={14} />{t('status.network')}</h4>
          {#each [0, 1, 2] as index (index)}<span class="skeleton net-skeleton"></span>{/each}
        </section>
      {/if}
      {#if settings.show_disks}
        <section class="card">
          <h4><Icon name="folder" size={14} />{t('status.disks')}</h4>
          <div class="row"><span class="skeleton" style:width="30%"></span><span class="skeleton" style:width="32px"></span></div>
          <div class="bar"></div>
          <span class="skeleton" style:width="45%"></span>
        </section>
      {/if}
    </div>
  {:else}
    {@const mem = stats.mem_total - stats.mem_available}
    {@const swap = stats.swap_total - stats.swap_free}
    {#if settings.show_system}
      <section class="card">
        <h4><Icon name="server" size={14} />{t('status.system')}</h4>
        <dl class="facts">
          <div><dt>{t('status.hostname')}</dt><dd>{stats.hostname}</dd></div>
          <div><dt>{t('status.os')}</dt><dd>{stats.os || stats.kernel.split(' ')[0]}</dd></div>
          {#if stats.latency_us.length > 0}
            <div title={stats.latency_us.length > 1 ? t('status.latencySegments') : undefined}>
              <dt>
                {t('status.response')}
                {#if stats.latency_us.length > 1}
                  <small class="parts">{t('status.segments', { names: segmentNames(stats.latency_us.length) })}</small>
                {/if}
              </dt>
              <dd>
                {t('status.ms', { n: formatLatencyMs(stats.latency_us) })}
                {#if badge}
                  <span class="path-badge" class:relay={badge.kind === 'relay'} title={badge.title}>
                    {badge.kind === 'direct' ? t('status.pathDirect') : badge.region ? t('status.pathRelayRegion', { region: badge.region }) : t('status.pathRelay')}
                  </span>
                {/if}
              </dd>
            </div>
          {/if}
          <div><dt>{t('status.uptimeLabel')}</dt><dd>{uptime(stats.uptime_secs)}</dd></div>
          <div class="wide"><dt>{t('status.kernel')}</dt><dd>{stats.kernel}</dd></div>
        </dl>
      </section>
    {/if}

    {#if settings.show_cpu}
      <section class="card">
        <h4><Icon name="terminal" size={14} />{t('status.cpu')} <span class="aside">{t('status.cores', { n: stats.cores })}</span></h4>
        {#if cpu === null}
          <div class="headline"><span class="skeleton" style:width="64px" style:height="24px"></span></div>
        {:else}
          <div class="headline">
            <strong class:hot={cpu > 85}>{`${cpu.toFixed(0)}%`}</strong>
            <span class="muted">{t('status.average')}</span>
          </div>
        {/if}
        {#if showCores}
          {#if coreHistory.some((points) => points.length > 0)}
            <div class="cores" bind:clientWidth={coresWidth} style:grid-template-columns="repeat({coreColumns}, minmax(0, 1fr))" aria-label={t('status.perCore')}>
              {#each coreHistory as points, index (index)}
                <CoreChart label={t('status.core', { n: index })} {points} capacity={HISTORY} compact={coreMinWidth < 110} />
              {/each}
            </div>
          {:else}
            <div class="cores skeleton-cores">
              {#each [0, 1, 2, 3] as index (index)}<span class="skeleton"></span>{/each}
            </div>
          {/if}
        {/if}
        <div class="row muted"><span>{t('status.load')}</span><span class="mono">{stats.load.map((value) => value.toFixed(2)).join('  ')}</span></div>
      </section>
    {/if}

    {#if settings.show_memory}
      <section class="card">
        <h4><Icon name="transfer" size={14} />{t('status.memory')}</h4>
        <div class="row"><span>{t('status.physical')}</span><span class="value">{percent(mem, stats.mem_total).toFixed(0)}%</span></div>
        <div class="bar"><div class="fill" class:hot={percent(mem, stats.mem_total) > 90} style:width="{percent(mem, stats.mem_total)}%"></div></div>
        <div class="muted mono">{formatSize(mem)} / {formatSize(stats.mem_total)}</div>
        {#if settings.show_swap}
          <div class="row"><span>{t('status.swap')}</span><span class="value">{percent(swap, stats.swap_total).toFixed(0)}%</span></div>
          <div class="bar"><div class="fill" style:width="{percent(swap, stats.swap_total)}%"></div></div>
          <div class="muted mono">{formatSize(swap)} / {formatSize(stats.swap_total)}</div>
        {/if}
      </section>
    {/if}

    {#if settings.show_network}
      <section class="card">
        <h4><Icon name="forwards" size={14} />{t('status.network')}</h4>
        {#each showAllInterfaces ? interfaces : interfaces.slice(0, INTERFACES_SHOWN) as net (net.name)}
          <div class="net">
            <span class="name mono" title={net.name}>{net.name}</span>
            <span class="up mono">↑ {formatNetworkRate(rates[net.name]?.tx, settings.network_unit)}</span>
            <span class="down mono">↓ {formatNetworkRate(rates[net.name]?.rx, settings.network_unit)}</span>
          </div>
        {:else}
          <p class="muted">{t('status.noInterfaces')}</p>
        {/each}
        {#if interfaces.length > INTERFACES_SHOWN}
          <button type="button" class="more" aria-expanded={showAllInterfaces} onclick={() => (showAllInterfaces = !showAllInterfaces)}>
            {showAllInterfaces ? t('status.showLess') : t('status.showAll', { n: interfaces.length })}
          </button>
        {/if}
      </section>
    {/if}

    {#if settings.show_disks}
      <section class="card">
        <h4><Icon name="folder" size={14} />{t('status.disks')}</h4>
        {#each stats.disks as disk (`${disk.filesystem}\0${disk.mount}`)}
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
    {/if}
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
    min-width: 0;
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
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-2);
    margin: 0;
    overflow-wrap: anywhere;
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
  }
  .path-badge {
    display: inline-block;
    max-width: 100%;
    padding: 0 var(--space-1);
    border-radius: var(--radius-pill);
    background: var(--success-soft);
    color: var(--success);
    font-size: var(--text-caption);
    font-weight: 600;
  }
  .path-badge.relay {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .headline {
    display: flex;
    flex-wrap: wrap;
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
    grid-template-columns: minmax(0, 1fr) repeat(2, minmax(0, max-content));
    gap: var(--space-2);
    align-items: center;
    padding: 3px var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .up {
    color: var(--success);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .down {
    color: var(--accent);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    text-align: right;
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
  @media (max-width: 767px) {
    .status {
      gap: 12px;
      padding: 12px var(--m-page-x, 16px) 24px;
      font-size: calc(13.5px * var(--ui-scale));
    }
    .card {
      gap: 8px;
      padding: 14px 16px;
      border-radius: var(--m-card-radius, 12px);
    }
    h4 {
      gap: 6px;
      margin: 0 0 4px;
      font-size: calc(15px * var(--ui-scale));
    }
    h4 :global(svg) {
      width: 16px;
      height: 16px;
    }
    .aside {
      font-size: calc(13px * var(--ui-scale));
    }
    .facts {
      gap: 10px 14px;
    }
    .facts dt {
      margin-bottom: 2px;
      font-size: calc(12px * var(--ui-scale));
    }
    .facts dd {
      font-size: calc(14px * var(--ui-scale));
      line-height: 1.35;
    }
    .headline strong {
      font-size: 28px;
    }
    .mono {
      font-size: calc(12.5px * var(--ui-scale));
    }
    .bar {
      height: 7px;
      border-radius: 4px;
    }
    .cores {
      gap: 6px;
      margin: 4px 0;
    }
    .net {
      gap: 10px;
      padding: 8px 10px;
      border-radius: 8px;
    }
    .net .name {
      font-size: calc(13px * var(--ui-scale));
      font-weight: 500;
    }
    .disk {
      gap: 4px;
    }
    .disk + .disk {
      margin-top: 10px;
    }
    .more {
      font-size: calc(13px * var(--ui-scale));
      font-weight: 500;
    }
  }
  @media (pointer: coarse) {
    .more {
      min-height: var(--touch-target);
      padding: 0 var(--space-2);
    }
  }
</style>
