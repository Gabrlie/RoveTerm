<script lang="ts">
  // One transfer: direction, name and where it goes, progress, state, and what can be done
  // with it (cancel while it runs, show a finished download in the file manager).
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { errorText, t } from '../i18n';
  import type { TransferUpdate } from '../ipc';
  import { notify } from '../ui/toast.svelte';
  import { formatSize } from './format';
  import { isFinal, progress } from './transferview';
  import { transfers } from './transfers.svelte';

  let { row }: { row: TransferUpdate } = $props();

  const failure = $derived(row.error ? errorText({ code: `fs.${row.error.kind}`, message: row.error.detail ?? '' }) : '');
  const status = $derived.by(() => {
    if (row.state === 'running') return `${formatSize(row.bytes_done)} / ${formatSize(row.bytes_total)}`;
    if (row.state === 'done' && row.skipped) return t('transfer.skipped');
    if (row.state === 'done') return row.bytes_total > 0 ? formatSize(row.bytes_total) : t('transfer.done');
    return t(`transfer.${row.state}`);
  });
  const where = $derived(row.tag === 'download' ? row.src_label : row.dst_label);

  function reveal(): void {
    transfers.reveal(row.id).catch((error: unknown) => notify(errorText(error), 'error'));
  }
</script>

<div class="row" class:failed={row.state === 'failed'} title={failure || row.dst_path}>
  <span class="icon" class:upload={row.tag === 'upload'}><Icon name={row.tag === 'upload' ? 'upload' : 'download'} size={14} /></span>
  <div class="main">
    <div class="line">
      <span class="name">{row.name}</span>
      <span class="status">{failure || status}</span>
    </div>
    {#if !isFinal(row)}
      <span class="bar"><span style:width="{progress(row)}%"></span></span>
    {:else}
      <span class="where">{where}</span>
    {/if}
  </div>
  {#if !isFinal(row)}
    <IconButton icon="close" small label={t('files.cancelTransfer')} onclick={() => void transfers.cancel(row.id)} />
  {:else if row.tag === 'download' && row.state === 'done' && !row.skipped}
    <IconButton icon="folder" small label={t('transfer.reveal')} onclick={reveal} />
  {/if}
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    padding: 6px var(--space-2);
    border-radius: var(--radius-sm);
  }
  .row:hover {
    background: var(--fill-hover);
  }
  .icon {
    display: grid;
    place-items: center;
    flex: none;
    width: 26px;
    height: 26px;
    border-radius: var(--radius-sm);
    background: var(--accent-soft);
    color: var(--accent);
  }
  .icon.upload {
    background: var(--fill);
    color: var(--fg-muted);
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    min-width: 0;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-footnote);
  }
  .status,
  .where {
    flex: none;
    color: var(--fg-muted);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .where {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .failed .status {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--danger);
  }
  .bar {
    display: block;
    height: 4px;
    border-radius: 2px;
    background: var(--fill-hover);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width var(--dur-fast) linear;
  }
</style>
