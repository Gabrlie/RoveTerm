<script lang="ts">
  // Context use of the conversation (as the agent reports it) and, for Codex and Claude Code,
  // how much of the plan's limits is used, behind a small ring in the input card.
  import { errorText, t, type MessageKey } from '../i18n';
  import { aiQuota, type AiQuota, type QuotaWindow, type RequestStats } from '../ipc';
  import IconButton from '../ui/IconButton.svelte';
  import { cacheRate, contextMeter, speed, tokens, usageTone, wait, windowName, type CacheTotals } from './quota';

  interface Props {
    /** The chat's agent (`acp:codex`…), or null. */
    agent: string | null;
    usage: { used: number; size: number } | null;
    windowSize: number | null;
    /** The built-in assistant's last request. */
    stats?: RequestStats | null;
    /** The conversation's input and cached tokens so far. */
    cache?: CacheTotals | null;
    /** A turn is running; the context is read again when it ends. */
    busy?: boolean;
  }

  let { agent, usage, windowSize, stats = null, cache = null, busy = false }: Props = $props();
  const lastHit = $derived(cacheRate(stats?.cached_tokens ?? null, stats?.input_tokens ?? null));
  const totalHit = $derived(cache ? cacheRate(cache.cached, cache.input) : null);
  let open = $state(false);
  let root = $state<HTMLDivElement>();
  let trigger = $state<HTMLButtonElement>();
  let request = 0;
  let quota = $state<AiQuota | null>(null);
  let loading = $state(false);
  let problem = $state('');

  /** Agents whose plan limits can be read. */
  const QUOTA_AGENTS = new Set(['acp:codex', 'acp:claude']);
  const hasQuota = $derived(!!agent && QUOTA_AGENTS.has(agent));
  const meter = $derived(contextMeter(usage ?? quota?.context ?? null, windowSize, stats));
  const display = $derived(meter.percent !== null ? `${Math.round(meter.percent)}%` : meter.filled !== null ? tokens(meter.filled) : '—');
  const hint = $derived(meter.percent !== null ? t('ai.quota.context', { percent: Math.round(meter.percent) })
    : meter.filled !== null ? t('ai.quota.contextTokens', { tokens: tokens(meter.filled) })
    : t('ai.quota.contextUnknown'));

  async function load(refresh: boolean): Promise<void> {
    if (!agent || !hasQuota) return;
    const asked = agent;
    const ticket = ++request;
    loading = true;
    problem = '';
    try {
      const read = await aiQuota(asked, refresh);
      // An answer for an agent no longer shown is dropped.
      if (ticket === request && agent === asked) quota = read;
    } catch (error) {
      if (ticket === request && agent === asked) problem = errorText(error);
    } finally {
      if (ticket === request) loading = false;
    }
  }

  function toggle(): void {
    open = !open;
    if (open && !quota && !loading) void load(false);
  }

  function name(window: QuotaWindow): string {
    const named = windowName(window);
    return named.key ? t(`ai.quota.${named.key}` as MessageKey, { n: named.count }) : named.label;
  }

  function resets(window: QuotaWindow): string {
    if (window.resets_at) {
      const at = new Date(window.resets_at);
      const sameDay = at.toDateString() === new Date().toDateString();
      return t('ai.quota.resets', { time: sameDay ? at.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : at.toLocaleString([], { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' }) });
    }
    return window.resets_text ?? '';
  }

  function onWindowPointer(event: PointerEvent): void {
    if (open && root && !root.contains(event.target as Node)) open = false;
  }

  function onKey(event: KeyboardEvent): void {
    if (event.key !== 'Escape' || event.defaultPrevented || !open) return;
    event.preventDefault();
    open = false;
    trigger?.focus();
  }

  // A new agent has other limits.
  $effect(() => {
    void agent;
    request += 1;
    quota = null;
    problem = '';
    loading = false;
  });

  // Codex's context is read from its logs, cheaply: at the start and after every turn.
  $effect(() => {
    if (agent === 'acp:codex' && !busy) void load(false);
  });
</script>

<svelte:window onpointerdown={onWindowPointer} onkeydown={onKey} />

<div class="meter" bind:this={root}>
  <button type="button" class="ring {meter.tone}" bind:this={trigger} onclick={toggle} aria-expanded={open} aria-haspopup="dialog" title={hint} aria-label={t('ai.sidebar.usageLabel', { usage: hint })}>
    <svg viewBox="0 0 20 20" width="15" height="15" aria-hidden="true">
      <circle cx="10" cy="10" r="7" class="track" />
      {#if meter.percent !== null && meter.percent > 0}
        <circle cx="10" cy="10" r="7" pathLength="100" class="fill" stroke-dasharray={`${meter.percent} 100`} />
      {:else if meter.filled !== null}
        <circle cx="10" cy="10" r="2" class="dot" />
      {/if}
    </svg>
    <span>{display}</span>
  </button>
  {#if open}
    <div class="popover" role="dialog" aria-label={t('ai.quota.title')}>
      <div class="row head">
        <span class="title">{t('ai.quota.title')}</span>
        {#if hasQuota}<IconButton icon="refresh" small label={t('ai.quota.refresh')} disabled={loading} onclick={() => void load(true)} />{/if}
      </div>
      <div class="item">
        <div class="row"><span>{t('ai.quota.contextLabel')}</span><span class="value">{meter.percent !== null && meter.context ? `${display} · ${tokens(meter.context.used)} / ${tokens(meter.context.size)}` : meter.filled !== null ? t('ai.quota.tokens', { tokens: display }) : t('ai.quota.contextUnknown')}</span></div>
        {#if meter.percent !== null}<div class="bar {meter.tone}"><span style:width={`${meter.percent}%`}></span></div>{/if}
      </div>
      {#if stats}
        <div class="section">
          <p class="note">{t('ai.quota.lastRequest')}</p>
          {#if stats.output_tokens !== null}
            <div class="row"><span>{t('ai.quota.output')}</span><span class="value">{t('ai.quota.tokens', { tokens: tokens(stats.output_tokens) })}</span></div>
          {/if}
          <div class="row"><span>{t('ai.quota.speed')}</span><span class="value">{stats.tokens_per_second !== null ? speed(stats.tokens_per_second) : '—'}</span></div>
          <div class="row" title={t('ai.quota.cacheHint')}><span>{t('ai.quota.cache')}</span><span class="value">{lastHit !== null ? `${Math.round(lastHit)}%` : '—'}{#if totalHit !== null} · {t('ai.quota.cacheTotal', { percent: Math.round(totalHit) })}{/if}</span></div>
          <div class="row"><span>{t('ai.quota.firstToken')}</span><span class="value">{stats.first_token_ms !== null ? wait(stats.first_token_ms) : '—'}</span></div>
        </div>
      {/if}
      {#if hasQuota}
        <div class="section">
          <p class="title">{t('ai.sidebar.planLimits')}</p>
          {#if loading && !quota}
            <p class="note">{t('ai.quota.loading')}</p>
          {:else if problem}
            <p class="note error">{problem}</p>
          {:else if quota && quota.windows.length === 0}
            <p class="note">{t('ai.quota.none')}</p>
          {:else if quota}
            {#each quota.windows as window, index (index)}
              <div class="item">
                <div class="row"><span>{name(window)}</span><span class="value">{Math.round(window.used_percent)}%</span></div>
                <div class="bar {usageTone(window.used_percent)}"><span style:width={`${Math.min(100, Math.max(0, window.used_percent))}%`}></span></div>
                {#if resets(window)}<p class="note">{resets(window)}</p>{/if}
              </div>
            {/each}
            <p class="note">{t('ai.quota.asOf', { time: new Date(quota.as_of).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) })}</p>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .meter {
    flex: none;
    margin-left: auto;
  }
  .ring,
  .bar {
    --meter-color: var(--accent);
  }
  .warning {
    --meter-color: var(--warning);
  }
  .danger {
    --meter-color: var(--danger);
  }
  .ring {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 6px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg-muted);
    font: inherit;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }
  .ring:hover,
  .ring[aria-expanded='true'] {
    background: var(--fill-hover);
  }
  .ring.warning,
  .ring.danger {
    color: var(--meter-color);
  }
  svg {
    transform: rotate(-90deg);
  }
  circle {
    fill: none;
    stroke-width: 3;
  }
  .track {
    stroke: var(--fill-pressed);
  }
  .fill {
    stroke: var(--meter-color);
    stroke-linecap: round;
  }
  .dot {
    fill: var(--fg-muted);
  }
  .popover {
    position: absolute;
    left: 0;
    right: 0;
    bottom: calc(100% + 8px);
    z-index: 20;
    display: grid;
    gap: var(--space-2);
    max-height: min(380px, 55vh);
    overflow-y: auto;
    padding: var(--space-2) var(--space-3) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--material-popover);
    box-shadow: var(--shadow-lg);
    font-size: var(--text-footnote);
    overscroll-behavior: contain;
  }
  .head {
    min-height: 24px;
  }
  .section {
    display: grid;
    gap: 6px;
    padding-top: var(--space-2);
    border-top: 1px solid var(--separator);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .title {
    margin: 0;
    color: var(--fg-muted);
    font-size: 11px;
    font-weight: 600;
  }
  .item {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .value {
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .bar {
    height: 5px;
    border-radius: 3px;
    background: var(--fill-hover);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--meter-color);
  }
  .note {
    margin: 0;
    color: var(--fg-muted);
    font-size: 11px;
  }
  .error {
    color: var(--danger);
  }
</style>
