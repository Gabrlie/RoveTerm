<script lang="ts">
  // Presents available update details, signed download progress and restart confirmation.
  import Dialog from '../ui/Dialog.svelte';
  import Icon from '../ui/Icon.svelte';
  import Markdown from '../ai/Markdown.svelte';
  import { t } from '../i18n';
  import { isAndroid } from '../platform';
  import { updater } from './updater.svelte';
  import { formatReleaseDate, formatUpdateBytes } from './updaterLogic';

  interface Props {
    onclose: () => void;
    onrestart: () => Promise<void>;
  }

  let { onclose, onrestart }: Props = $props();
  let restarting = $state(false);

  const info = $derived(updater.info);
  const releaseDate = $derived(formatReleaseDate(info?.published_at));
  const busy = $derived(updater.phase === 'downloading' || updater.phase === 'installing');
  const progressLabel = $derived.by(() => {
    if (updater.phase === 'installing') return t(isAndroid ? 'update.androidInstalling' : 'update.installing');
    if (updater.total && updater.total > 0) {
      return t('update.downloadingProgress', {
        done: formatUpdateBytes(updater.downloaded),
        total: formatUpdateBytes(updater.total),
        percent: updater.percent ?? 0,
      });
    }
    if (updater.downloaded > 0) {
      return t('update.downloadingBytes', { done: formatUpdateBytes(updater.downloaded) });
    }
    return t('update.downloading');
  });

  async function restart(): Promise<void> {
    if (restarting) return;
    restarting = true;
    try {
      await onrestart();
    } finally {
      restarting = false;
    }
  }
</script>

<Dialog title={t('update.dialogTitle')} {onclose} width={540}>
  {#if updater.phase === 'checking'}
    <div class="state-row">
      <span class="spinner"><Icon name="refresh" size={16} /></span>
      <span>{t('update.checking')}</span>
    </div>
  {:else if info?.available && info.version}
    <div class="hero">
      <div class="mark"><Icon name="download" size={20} /></div>
      <div class="meta">
        <div class="versions">
          <span class="current">v{info.current_version}</span>
          <span class="arrow">→</span>
          <span class="latest">v{info.version}</span>
          {#if info.prerelease}
            <span class="channel-tag">{t('update.prereleaseBadge')}</span>
          {/if}
        </div>
        {#if releaseDate}
          <p class="date">{t('update.publishedOn', { date: releaseDate })}</p>
        {/if}
      </div>
    </div>

    <div class="notes-box">
      {#if info.notes.trim()}
        <Markdown text={info.notes} />
      {:else}
        <p class="empty-notes">{t('update.noNotes')}</p>
      {/if}
    </div>

    {#if busy}
      <div class="progress-panel" role="status" aria-live="polite">
        <div class="progress-text">
          <span>{progressLabel}</span>
          {#if updater.phase === 'downloading' && updater.percent !== null}
            <span class="pct">{updater.percent}%</span>
          {/if}
        </div>
        <div class="track">
          {#if updater.phase === 'downloading' && updater.percent !== null}
            <div class="bar" style:width="{updater.percent}%"></div>
          {:else}
            <div class="bar indeterminate"></div>
          {/if}
        </div>
      </div>
    {:else if updater.phase === 'ready'}
      <div class="ready-banner" role="status">
        <Icon name="check" size={15} />
        <span>{t(isAndroid ? 'update.androidReadyBanner' : 'update.readyBanner')}</span>
      </div>
    {/if}
  {:else if updater.phase === 'up_to_date' && info}
    <div class="state-row">
      <Icon name="check" size={16} />
      <span>{t('update.upToDateToast', { version: info.current_version })}</span>
    </div>
  {/if}

  {#if updater.error}
    <p class="error">{updater.error}</p>
  {/if}

  {#snippet actions()}
    {#if updater.phase === 'ready'}
      {#if isAndroid}
        <div class="action-bar">
          <div class="action-links">
            <button type="button" class="plain" onclick={() => void updater.openRelease()}>
              {t('update.openGithubDownload')}
            </button>
          </div>
          <div class="action-buttons">
            <button type="button" class="secondary" onclick={onclose}>{t('update.later')}</button>
            <button type="button" class="primary" onclick={() => void updater.install()}>
              {t('update.androidReopenInstaller')}
            </button>
          </div>
        </div>
      {:else}
        <button type="button" class="secondary" disabled={restarting} onclick={onclose}>
          {t('update.later')}
        </button>
        <button type="button" class="primary" disabled={restarting} onclick={() => void restart()}>
          {restarting ? t('update.restarting') : t('update.restartNow')}
        </button>
      {/if}
    {:else if busy}
      <button type="button" class="secondary" onclick={onclose}>{t('update.hideWhileDownloading')}</button>
    {:else if info?.available && info.version}
      <div class="action-bar">
        <div class="action-links">
          <button type="button" class="plain skip" onclick={() => void updater.skipCurrent()}>
            {t('update.skipVersion')}
          </button>
          {#if info.install_mode === 'in_app'}
            <button type="button" class="plain" onclick={() => void updater.openRelease()}>
              {t('update.openGithubDownload')}
            </button>
          {/if}
        </div>
        <div class="action-buttons">
          <button type="button" class="secondary" onclick={onclose}>{t('update.later')}</button>
          {#if info.install_mode === 'in_app'}
            <button type="button" class="primary" onclick={() => void updater.install()}>
              {updater.phase === 'error' ? t('update.retry') : t('update.downloadAndInstall')}
            </button>
          {:else}
            <button type="button" class="primary" onclick={() => void updater.openRelease()}>
              {t('update.openGithubDownload')}
            </button>
          {/if}
        </div>
      </div>
    {:else}
      {#if updater.phase === 'error'}
        <button type="button" class="secondary" onclick={() => void updater.check({ silent: false, interactive: true })}>
          {t('update.retry')}
        </button>
      {/if}
      <button type="button" class="primary" onclick={onclose}>{t('common.close')}</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .hero {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .mark {
    display: grid;
    place-items: center;
    flex: none;
    width: 40px;
    height: 40px;
    border-radius: var(--radius-md);
    background: var(--accent-soft);
    color: var(--accent);
  }
  .meta {
    min-width: 0;
    flex: 1;
  }
  .versions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--text-body);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .current {
    color: var(--fg-muted);
  }
  .arrow {
    color: var(--fg-muted);
    font-weight: 400;
  }
  .latest {
    color: var(--fg);
  }
  .channel-tag {
    padding: 2px 7px;
    border-radius: var(--radius-pill);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--text-caption);
    font-weight: 600;
  }
  .date {
    margin: 3px 0 0;
    color: var(--fg-muted);
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
  }
  .notes-box {
    max-height: 280px;
    overflow-y: auto;
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--separator);
    font-size: var(--text-footnote);
  }
  .empty-notes {
    margin: 0;
    color: var(--fg-muted);
  }
  .progress-panel {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .progress-text {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    font-variant-numeric: tabular-nums;
  }
  .pct {
    color: var(--fg);
    font-weight: 600;
  }
  .track {
    position: relative;
    height: 6px;
    border-radius: var(--radius-pill);
    background: var(--fill);
    overflow: hidden;
  }
  .bar {
    height: 100%;
    border-radius: var(--radius-pill);
    background: var(--accent);
    transition: width 120ms ease-out;
  }
  .bar.indeterminate {
    width: 36%;
    animation: slide 1.1s ease-in-out infinite;
  }
  @keyframes slide {
    0% {
      transform: translateX(-110%);
    }
    100% {
      transform: translateX(300%);
    }
  }
  .ready-banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--success) 14%, transparent);
    color: var(--success);
    font-size: var(--text-footnote);
    font-weight: 500;
  }
  .state-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) 0;
    color: var(--fg);
    font-size: var(--text-body);
  }
  .spinner {
    display: inline-flex;
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .action-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    width: 100%;
  }
  .action-links,
  .action-buttons {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .skip {
    color: var(--fg-muted);
  }
  @media (max-width: 767px) {
    .mark {
      width: 36px;
      height: 36px;
      border-radius: 9px;
    }
    .notes-box {
      max-height: min(46dvh, 360px);
      padding: 12px 14px;
      font-size: calc(13px * var(--ui-scale));
      line-height: 1.55;
    }
    .action-bar {
      flex-direction: column;
      align-items: stretch;
      gap: 10px;
    }
    .action-links {
      justify-content: space-between;
      gap: var(--space-2);
    }
    .action-links button {
      height: 28px;
      padding: 0 4px;
      font-size: calc(12.5px * var(--ui-scale));
    }
    .action-buttons {
      display: grid;
      grid-template-columns: minmax(0, 1fr) minmax(0, 1.35fr);
      gap: var(--space-2);
    }
    .action-buttons button {
      min-width: 0;
      width: 100%;
    }
  }
</style>
