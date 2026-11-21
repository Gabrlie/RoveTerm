<script lang="ts">
  // In-app viewer for RoveTerm's daily rolling log files (secret-free).
  import { onMount, tick } from 'svelte';
  import Icon from '../../ui/Icon.svelte';
  import Select from '../../ui/Select.svelte';
  import { writeClipboard } from '../../ui/clipboard';
  import { notify } from '../../ui/toast.svelte';
  import { errorText, t } from '../../i18n';
  import { logsList, logsRead, logsReveal, type LogFileInfo } from '../../ipc';
  import { filterLogEntries, formatLogSize, parseLogLines, type LogLevelFilter } from '../logs';

  const TAIL_LINES = 800;
  const AUTO_REFRESH_MS = 3000;

  let files = $state<LogFileInfo[]>([]);
  let selectedFile = $state('');
  let content = $state('');
  let truncated = $state(false);
  let totalBytes = $state(0);
  let loading = $state(true);
  let loadError = $state('');
  let level = $state<LogLevelFilter>('ALL');
  let query = $state('');
  let autoRefresh = $state(false);
  let viewport = $state<HTMLDivElement>();

  const fileOptions = $derived(
    files.map((file) => ({
      value: file.name,
      label: `${file.name} (${formatLogSize(file.size_bytes)})`,
    })),
  );

  const entries = $derived(parseLogLines(content));
  const filtered = $derived(filterLogEntries(entries, level, query));

  async function scrollToBottom(): Promise<void> {
    await tick();
    if (viewport) viewport.scrollTop = viewport.scrollHeight;
  }

  async function loadContent(fileName: string, keepScroll = false): Promise<void> {
    try {
      const result = await logsRead(fileName || null, TAIL_LINES);
      if (result.file) selectedFile = result.file;
      content = result.content;
      truncated = result.truncated;
      totalBytes = result.total_bytes;
      loadError = '';
      if (!keepScroll) await scrollToBottom();
    } catch (error) {
      loadError = errorText(error);
    }
  }

  async function refresh(keepScroll = false): Promise<void> {
    if (!keepScroll) loading = true;
    try {
      const listed = await logsList();
      files = listed;
      const first = listed[0];
      if (!first) {
        selectedFile = '';
        content = '';
        truncated = false;
        totalBytes = 0;
        loadError = '';
        return;
      }
      const nextFile = listed.some((item) => item.name === selectedFile) ? selectedFile : first.name;
      selectedFile = nextFile;
      await loadContent(nextFile, keepScroll);
    } catch (error) {
      loadError = errorText(error);
    } finally {
      loading = false;
    }
  }

  function onSelectFile(name: string): void {
    selectedFile = name;
    void loadContent(name, false);
  }

  async function copyVisible(): Promise<void> {
    const text = filtered.map((entry) => entry.raw).join('\n');
    if (!text) return;
    try {
      await writeClipboard(text);
      notify(t('logs.copied'), 'success');
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  function revealFolder(): void {
    void logsReveal().catch((error: unknown) => notify(errorText(error), 'error'));
  }

  $effect(() => {
    if (!autoRefresh) return;
    const timer = window.setInterval(() => {
      void refresh(true);
    }, AUTO_REFRESH_MS);
    return () => window.clearInterval(timer);
  });

  onMount(() => {
    void refresh(false);
  });
</script>

<div class="logs-section">
  <div class="toolbar">
    <div class="file-picker">
      {#if fileOptions.length > 0}
        <Select value={selectedFile} options={fileOptions} label={t('logs.file')} onchange={onSelectFile} />
      {:else}
        <span class="empty-file">{t('logs.noFiles')}</span>
      {/if}
    </div>
    <div class="toolbar-actions">
      <button type="button" class="secondary compact" onclick={() => void refresh(false)} title={t('logs.refresh')}>
        <Icon name="refresh" size={13} />
        {t('logs.refresh')}
      </button>
      <button
        type="button"
        class="secondary compact"
        class:active-toggle={autoRefresh}
        aria-pressed={autoRefresh}
        onclick={() => (autoRefresh = !autoRefresh)}
      >
        {t('logs.autoRefresh')}
      </button>
      <button type="button" class="secondary compact" disabled={filtered.length === 0} onclick={() => void copyVisible()}>
        <Icon name="copy" size={13} />
        {t('logs.copy')}
      </button>
      <button type="button" class="secondary compact" onclick={revealFolder} title={t('action.help.logs')}>
        <Icon name="folder" size={13} />
      </button>
    </div>
  </div>

  <div class="filter-bar">
    <div class="segmented" role="group" aria-label={t('logs.level.all')}>
      <button type="button" aria-pressed={level === 'ALL'} onclick={() => (level = 'ALL')}>{t('logs.level.all')}</button>
      <button type="button" aria-pressed={level === 'ERROR'} onclick={() => (level = 'ERROR')}>{t('logs.level.error')}</button>
      <button type="button" aria-pressed={level === 'WARN'} onclick={() => (level = 'WARN')}>{t('logs.level.warn')}</button>
      <button type="button" aria-pressed={level === 'INFO'} onclick={() => (level = 'INFO')}>{t('logs.level.info')}</button>
      <button type="button" aria-pressed={level === 'DEBUG'} onclick={() => (level = 'DEBUG')}>{t('logs.level.debug')}</button>
    </div>
    <input
      class="search-input"
      type="search"
      bind:value={query}
      placeholder={t('logs.search')}
      aria-label={t('logs.search')}
      spellcheck="false"
    />
  </div>

  {#if loadError}
    <div class="inline-error" role="alert">
      <Icon name="warning" size={14} />
      <span>{loadError}</span>
    </div>
  {/if}

  <div class="meta-bar">
    <span>{t('logs.summary', { shown: filtered.length, total: entries.length, size: formatLogSize(totalBytes) })}</span>
    {#if truncated}
      <span class="truncated-badge">{t('logs.truncated')}</span>
    {/if}
  </div>

  <div class="log-viewport" bind:this={viewport}>
    {#if loading && entries.length === 0}
      <p class="placeholder">{t('logs.loading')}</p>
    {:else if filtered.length === 0}
      <p class="placeholder">{t('logs.empty')}</p>
    {:else}
      <pre class="log-lines">{#each filtered as entry (entry.index)}<span class="log-line level-{entry.level.toLowerCase()}">{entry.raw}</span>{/each}</pre>
    {/if}
  </div>

  <p class="form-note">{t('settings.logsHint')}</p>
</div>

<style>
  .logs-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .file-picker {
    flex: 1;
    min-width: 190px;
  }
  .empty-file {
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .toolbar-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .compact {
    min-width: 0;
    padding: 0 var(--space-2);
    font-size: var(--text-footnote);
  }
  .active-toggle {
    background: var(--accent-soft);
    color: var(--accent);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .filter-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .search-input {
    flex: 1;
    min-width: 160px;
    font-size: var(--text-footnote);
  }
  .meta-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: 0 2px;
    color: var(--fg-muted);
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
  }
  .truncated-badge {
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--fill);
    color: var(--fg-muted);
  }
  .inline-error {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
    box-shadow: 0 0 0 0.5px color-mix(in srgb, var(--danger) 35%, transparent);
    color: var(--danger);
    font-size: var(--text-footnote);
    line-height: 1.45;
  }
  .log-viewport {
    height: 310px;
    overflow: auto;
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow:
      0 0 0 0.5px var(--separator),
      inset 0 1px 2px rgb(0 0 0 / 0.06);
    padding: var(--space-2) var(--space-3);
  }
  .placeholder {
    margin: var(--space-4) 0;
    text-align: center;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .log-lines {
    margin: 0;
    font-family: var(--font-mono);
    font-size: 11.5px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-all;
    user-select: text;
  }
  .log-line {
    display: block;
    color: var(--fg);
  }
  .log-line.level-error {
    color: var(--danger);
  }
  .log-line.level-warn {
    color: var(--sys-orange);
  }
  .log-line.level-debug,
  .log-line.level-trace {
    color: var(--fg-muted);
  }
</style>
