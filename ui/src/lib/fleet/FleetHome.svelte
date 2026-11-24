<script lang="ts">
  // The fleet lead on the home page: tell it what to do across all servers, or go back to a
  // recent conversation with it. Both open its tab.
  import { onMount } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import { t } from '../i18n';
  import { aiRecords, aiSessions, type AiRecord, type AiSessionSummary } from '../ipc';
  import { ai } from '../ai/ai.svelte';
  import { capabilities } from '../platform';

  interface Props {
    /** Opens the fleet lead's tab, sending `text` when given. */
    onopen: (text: string | null) => void;
    /** Continues an earlier conversation with the fleet lead. */
    onresume: (summary: AiSessionSummary, records: AiRecord[]) => void;
    /** Opens the scheduled tasks. */
    onschedules: () => void;
  }

  let { onopen, onresume, onschedules }: Props = $props();
  let draft = $state('');
  let recent = $state<AiSessionSummary[]>([]);
  let recentRevision = 0;

  function send(): void {
    const text = draft.trim();
    if (!text) return;
    draft = '';
    onopen(text);
  }

  async function resume(summary: AiSessionSummary): Promise<void> {
    try {
      onresume(summary, await aiRecords(summary.id));
    } catch (error) {
      console.warn('fleet conversation unavailable', error);
    }
  }

  async function loadRecent(): Promise<void> {
    const revision = ++recentRevision;
    try {
      const sessions = await aiSessions('fleet');
      if (revision === recentRevision) recent = sessions.slice(0, 5);
    } catch (error) {
      if (revision === recentRevision) console.warn('AI history unavailable', error);
    }
  }

  $effect(() => {
    void ai.sessionsVersion;
    void loadRecent();
  });

  onMount(() => {
    if (capabilities.schedules) return ai.onReport(() => void loadRecent());
  });
</script>

<section class="fleet">
  <div class="head">
    <span class="mark"><Icon name="bot" size={16} /></span>
    <div class="titles">
      <h2>{t('fleet.home')}</h2>
      <p>{t('fleet.homeHint')}</p>
    </div>
    {#if capabilities.schedules}
      <button type="button" class="secondary" onclick={onschedules}>
        {t('fleet.schedules')}{#if ai.unreadReports > 0}<span class="badge">{t('fleet.newReports', { n: ai.unreadReports })}</span>{/if}
      </button>
    {/if}
    <button type="button" class="secondary" onclick={() => onopen(null)}>{t('fleet.openAll')}</button>
  </div>
  <div class="input">
    <textarea
      bind:value={draft}
      rows="2"
      placeholder={t('fleet.placeholder')}
      aria-label={t('fleet.placeholder')}
      onkeydown={(event) => {
        if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
          event.preventDefault();
          send();
        }
      }}
    ></textarea>
    <button type="button" class="send" disabled={!draft.trim()} title={t('ai.send')} aria-label={t('ai.send')} onclick={send}><Icon name="up" size={14} /></button>
  </div>
  {#if recent.length > 0}
    <div class="recent">
      <span class="recent-label">{t('fleet.recent')}</span>
      {#each recent as session (session.id)}
        <button type="button" class="chip" title={new Date(session.started).toLocaleString()} onclick={() => void resume(session)}>{session.title ?? t('fleet.title')}</button>
      {/each}
    </div>
  {/if}
</section>

<style>
  .fleet {
    max-width: 1100px;
    margin: 0 auto var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow:
      0 0 0 0.5px var(--separator),
      var(--shadow-sm);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .mark {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 9px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .titles {
    flex: 1;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: var(--text-body);
    font-weight: 600;
  }
  .titles p {
    margin: 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .badge {
    margin-left: 6px;
    padding: 0 6px;
    border-radius: var(--radius-pill);
    background: var(--accent);
    color: var(--accent-fg);
    font-size: 10.5px;
  }
  .input {
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--fill);
  }
  textarea {
    flex: 1;
    min-height: 40px;
    resize: none;
    padding: 2px;
    border: none;
    background: transparent;
    box-shadow: none;
    color: var(--fg);
    font: inherit;
    outline: none;
  }
  .send {
    display: grid;
    place-items: center;
    flex: none;
    width: 28px;
    height: 28px;
    border: none;
    border-radius: 50%;
    background: var(--accent);
    color: var(--accent-fg);
    cursor: pointer;
  }
  .send:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .recent {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .recent-label {
    color: var(--fg-muted);
    font-size: 11px;
  }
  .chip {
    max-width: 240px;
    padding: 2px 10px;
    border: none;
    border-radius: var(--radius-pill);
    background: var(--fill);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-footnote);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .chip:hover {
    background: var(--fill-hover);
  }
</style>
