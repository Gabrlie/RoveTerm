<script lang="ts">
  // Usage history of one CPU core as a filled line chart, newest sample at the right edge.
  // `points` holds percentages; the chart always spans `capacity` slots, so a short history
  // starts partway across.

  let { label, points, capacity, compact = false }: { label: string; points: number[]; capacity: number; compact?: boolean } = $props();

  const current = $derived(points.at(-1));
  const line = $derived(
    points
      .map((value, index) => `${capacity - points.length + index},${(100 - Math.min(100, Math.max(0, value))).toFixed(1)}`)
      .join(' '),
  );
  const area = $derived(points.length > 1 ? `${capacity - points.length},100 ${line} ${capacity - 1},100` : '');
</script>

<div class="chart" class:compact class:hot={(current ?? 0) > 85} title={`${label}: ${current === undefined ? '…' : `${current.toFixed(0)}%`}`}>
  <svg viewBox="0 0 {capacity - 1} 100" preserveAspectRatio="none" aria-hidden="true">
    <line class="grid" x1="0" x2={capacity - 1} y1="25" y2="25" />
    <line class="grid" x1="0" x2={capacity - 1} y1="50" y2="50" />
    <line class="grid" x1="0" x2={capacity - 1} y1="75" y2="75" />
    {#if points.length > 1}
      <polygon class="area" points={area} />
      <polyline class="line" points={line} />
    {/if}
  </svg>
  <span class="label">{label}</span>
  <span class="value">{current === undefined ? '…' : `${current.toFixed(0)}%`}</span>
</div>

<style>
  .chart {
    --tone: var(--accent);
    position: relative;
    height: 54px;
    border-radius: var(--radius-sm);
    background: var(--fill);
    box-shadow: inset 0 0 0 0.5px var(--separator);
    overflow: hidden;
  }
  .chart.compact {
    height: 30px;
  }
  .chart.hot {
    --tone: var(--danger);
  }
  /* The plot sits below a band holding the label and value, so a busy core's line never
     crosses them; compact charts overlay the value instead. */
  svg {
    position: absolute;
    top: 15px;
    left: 0;
    width: 100%;
    height: calc(100% - 15px);
  }
  .compact svg {
    top: 0;
    height: 100%;
  }
  .grid {
    stroke: var(--separator);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  .area {
    fill: var(--tone);
    opacity: 0.18;
  }
  .line {
    fill: none;
    stroke: var(--tone);
    stroke-width: 1.5;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
  }
  .label,
  .value {
    position: absolute;
    top: 1px;
    font-family: var(--font-mono);
    font-size: 10px;
    line-height: 1.3;
    color: var(--fg-muted);
  }
  .label {
    left: 4px;
  }
  .value {
    right: 4px;
    color: var(--tone);
  }
  .compact .label {
    display: none;
  }
  @media (max-width: 767px) {
    .chart {
      height: 60px;
      border-radius: 8px;
    }
    .chart.compact {
      height: 36px;
    }
    svg {
      top: 18px;
      height: calc(100% - 18px);
    }
    .label,
    .value {
      top: 2px;
      font-size: 11px;
    }
    .label {
      left: 6px;
    }
    .value {
      right: 6px;
    }
  }
</style>
