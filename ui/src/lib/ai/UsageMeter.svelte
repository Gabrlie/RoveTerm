<script lang="ts">
  // Context use of the conversation (as the agent reports it) and, for Codex and Claude Code,
  // how much of the plan's limits is used, behind a small ring in the input card.
  import { t, type MessageKey } from '../i18n';
  import { aiQuota, type AiQuota, type QuotaWindow } from '../ipc';
  import { errorText } from '../i18n';
  import IconButton from '../ui/IconButton.svelte';
  import { contextPercent, tokens, windowName } from './quota';

  interface Props {
    /** The chat's agent (`acp:codex`…), or null. */
    agent: string | null;
    usage: { used: number; size: number } | null;
  }

  let { agent, usage }: Props = $props();
  let open = $state(false);
  let root = $state<HTMLDivElement>();
  let quota = $state<AiQuota | null>(null);
  let loading = $state(false);
  let problem = $state('');

  /** Agents whose plan limits can be read. */
  const QUOTA_AGENTS = new Set(['acp:codex', 'acp:claude']);
  const hasQuota = $derived(!!agent && QUOTA_AGENTS.has(agent));
  const percent = $derived(contextPercent(usage));
  const shown = $derived(percent !== null || hasQuota);

  async function load(refresh: boolean): Promise<void> {
    if (!agent || !hasQuota) return;
    loading = true;
    problem = '';
    try {
      quota = await aiQuota(agent, refresh);
    } catch (error) {
      problem = errorText(error);
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
</script>

<svelte:window onpointerdown={onWindowPointer} />

{#if shown}
  <div class="meter" bind:this={root}>
    <button type="button" class="ring" onclick={toggle} aria-expanded={open} title={percent !== null ? t('ai.quota.context', { percent: Math.round(percent) }) : t('ai.quota.title')} aria-label={t('ai.quota.title')}>
      <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true">
        <circle cx="10" cy="10" r="7" class="track" />
        <circle cx="10" cy="10" r="7" class="fill" class:high={(percent ?? 0) >= 80} stroke-dasharray={`${((percent ?? 0) / 100) * 44} 44`} />
      </svg>
    </button>
    {#if open}
      <div class="popover" role="dialog" aria-label={t('ai.quota.title')}>
        <div class="row head">
          <span class="title">{t('ai.quota.title')}</span>
          {#if hasQuota}<IconButton icon="refresh" small label={t('ai.quota.refresh')} disabled={loading} onclick={() => void load(true)} />{/if}
        </div>
        <div class="item">
          <div class="row"><span>{t('ai.quota.contextLabel')}</span><span class="value">{percent !== null && usage ? `${Math.round(percent)}% · ${tokens(usage.used)} / ${tokens(usage.size)}` : t('ai.quota.contextUnknown')}</span></div>
          <div class="bar"><span style:width={`${percent ?? 0}%`} class:high={(percent ?? 0) >= 80}></span></div>
        </div>
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
