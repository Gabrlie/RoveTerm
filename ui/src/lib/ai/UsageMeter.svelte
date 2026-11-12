<script lang="ts">
  // Context use of the conversation (as the agent reports it) and, for Codex and Claude Code,
  // how much of the plan's limits is used, behind a small ring in the input card.
  import { t, type MessageKey } from '../i18n';
  import { aiQuota, type AiQuota, type QuotaWindow, type RequestStats } from '../ipc';
  import { errorText } from '../i18n';
  import IconButton from '../ui/IconButton.svelte';
  import { cacheRate, contextPercent, contextTokens, speed, tokens, wait, windowName, type CacheTotals } from './quota';

  interface Props {
    /** The chat's agent (`acp:codex`…), or null. */
    agent: string | null;
    usage: { used: number; size: number } | null;
    /** The built-in assistant's last request. */
    stats?: RequestStats | null;
    /** The conversation's input and cached tokens so far. */
    cache?: CacheTotals | null;
    /** A turn is running; the context is read again when it ends. */
    busy?: boolean;
  }

  let { agent, usage, stats = null, cache = null, busy = false }: Props = $props();
  const lastHit = $derived(cacheRate(stats?.cached_tokens ?? null, stats?.input_tokens ?? null));
  const totalHit = $derived(cache ? cacheRate(cache.cached, cache.input) : null);
  let open = $state(false);
  let root = $state<HTMLDivElement>();
  let quota = $state<AiQuota | null>(null);
  let loading = $state(false);
  let problem = $state('');

  /** Agents whose plan limits can be read. */
  const QUOTA_AGENTS = new Set(['acp:codex', 'acp:claude']);
  const hasQuota = $derived(!!agent && QUOTA_AGENTS.has(agent));
  /** As the agent reports it, else from its logs (Codex). */
  const context = $derived(usage ?? quota?.context ?? null);
  const percent = $derived(contextPercent(context));
  /** Tokens in context when only the request's counts are known (no window size). */
  const filled = $derived(percent === null ? contextTokens(stats) : null);
  const shown = $derived(percent !== null || hasQuota || stats !== null);

  async function load(refresh: boolean): Promise<void> {
    if (!agent || !hasQuota) return;
    const asked = agent;
    loading = true;
    problem = '';
    try {
      const read = await aiQuota(asked, refresh);
      // An answer for an agent no longer shown is dropped.
      if (agent === asked) quota = read;
    } catch (error) {
      if (agent === asked) problem = errorText(error);
    } finally {
      loading = false;
    }
  }

  function toggle(): void {
    open = !open;
    if (open && !quota) void load(false);
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

  // A new agent has other limits.
  $effect(() => {
    void agent;
    quota = null;
  });

  // Codex's context is read from its logs, cheaply: at the start and after every turn.
  $effect(() => {
    if (agent === 'acp:codex' && !busy) void load(false);
  });
</script>

<svelte:window onpointerdown={onWindowPointer} />

{#if shown}
  <div class="meter" bind:this={root}>
    <button type="button" class="ring" onclick={toggle} aria-expanded={open} title={percent !== null ? t('ai.quota.context', { percent: Math.round(percent) }) : filled !== null ? t('ai.quota.contextTokens', { tokens: tokens(filled) }) : t('ai.quota.title')} aria-label={t('ai.quota.title')}>
      <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true">
        <circle cx="10" cy="10" r="7" class="track" />
        {#if percent !== null}
          <circle cx="10" cy="10" r="7" class="fill" class:high={percent >= 80} stroke-dasharray={`${(percent / 100) * 44} 44`} />
        {:else}
          <!-- No window size: a plain gauge, nothing to fill. -->
          <circle cx="10" cy="10" r="2" class="dot" />
        {/if}
      </svg>
    </button>
    {#if open}
      <div class="popover" role="dialog" aria-label={t('ai.quota.title')}>
        <div class="row head">
          <span class="title">{t('ai.quota.title')}</span>
          {#if hasQuota}<IconButton icon="refresh" small label={t('ai.quota.refresh')} disabled={loading} onclick={() => void load(true)} />{/if}
        </div>
        <div class="item">
          <div class="row"><span>{t('ai.quota.contextLabel')}</span><span class="value">{percent !== null && context ? `${Math.round(percent)}% · ${tokens(context.used)} / ${tokens(context.size)}` : filled !== null ? t('ai.quota.tokens', { tokens: tokens(filled) }) : t('ai.quota.contextUnknown')}</span></div>
          {#if percent !== null}<div class="bar"><span style:width={`${percent}%`} class:high={percent >= 80}></span></div>{/if}
        </div>
        {#if stats}
          {#if stats.output_tokens !== null}
            <div class="row"><span>{t('ai.quota.output')}</span><span class="value">{t('ai.quota.tokens', { tokens: tokens(stats.output_tokens) })}</span></div>
          {/if}
          <div class="row"><span>{t('ai.quota.speed')}</span><span class="value">{stats.tokens_per_second !== null ? speed(stats.tokens_per_second) : '—'}</span></div>
          <div class="row" title={t('ai.quota.cacheHint')}><span>{t('ai.quota.cache')}</span><span class="value">{lastHit !== null ? `${Math.round(lastHit)}%` : '—'}{#if totalHit !== null} · {t('ai.quota.cacheTotal', { percent: Math.round(totalHit) })}{/if}</span></div>
          <div class="row"><span>{t('ai.quota.firstToken')}</span><span class="value">{stats.first_token_ms !== null ? wait(stats.first_token_ms) : '—'}</span></div>
          <p class="note">{t('ai.quota.lastRequest')}</p>
        {/if}
        {#if hasQuota}
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
                <div class="bar"><span style:width={`${Math.min(100, window.used_percent)}%`} class:high={window.used_percent >= 80}></span></div>
                {#if resets(window)}<p class="note">{resets(window)}</p>{/if}
              </div>
            {/each}
            <p class="note">{t('ai.quota.asOf', { time: new Date(quota.as_of).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) })}</p>
          {/if}
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .meter {
    position: static;
    flex: none;
  }
  .ring {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    cursor: pointer;
  }
  .ring:hover {
    background: var(--fill-hover);
  }
  svg {
    transform: rotate(-90deg);
  }
  circle {
    fill: none;
    stroke-width: 3;
  }
  .track {
    stroke: var(--fill-hover);
  }
  .fill {
    stroke: var(--accent);
    stroke-linecap: round;
  }
  .fill.high {
    stroke: var(--warning);
  }
  .dot {
    fill: var(--fg-muted);
    stroke: none;
  }
  .popover {
    position: absolute;
    left: 0;
    right: 0;
    bottom: calc(100% + 6px);
    z-index: 20;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--material-popover);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: var(--glass-edge), var(--shadow-md);
    font-size: var(--text-footnote);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .title {
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
    background: var(--accent);
  }
  .bar span.high {
    background: var(--warning);
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
