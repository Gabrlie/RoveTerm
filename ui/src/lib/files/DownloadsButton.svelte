<script lang="ts">
  // The activity rail's downloads button and its popover, as in a browser: the latest uploads
  // and downloads with their progress, and the way to the download center.
  import { onMount } from 'svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { t } from '../i18n';
  import TransferRow from './TransferRow.svelte';
  import { transfers } from './transfers.svelte';
  import { activity, isFinal, newestFirst } from './transferview';

  /** Opens the download center (the right panel's transfers page). */
  let { onopen }: { onopen: () => void } = $props();

  let root = $state<HTMLDivElement>();
  let place = $state<{ right: number; bottom: number } | null>(null);

  const rows = $derived(newestFirst(transfers.all, 8));
  const going = $derived(activity(transfers.all));
  const finished = $derived(transfers.all.some(isFinal));

  /** Beside the button, left of the rail, its bottom level with the button's. */
  function measure(): void {
    const rect = root?.getBoundingClientRect();
    if (!rect) return;
    place = { right: window.innerWidth - rect.left + 6, bottom: Math.max(8, window.innerHeight - rect.bottom) };
  }

  $effect(() => {
    if (transfers.popover) measure();
  });

  function onWindowPointer(event: PointerEvent): void {
    if (transfers.popover && root && !root.contains(event.target as Node)) transfers.popover = false;
  }

  function onKey(event: KeyboardEvent): void {
    if (event.key === 'Escape' && transfers.popover) transfers.popover = false;
  }

  onMount(() => {
    transfers.start().catch((error: unknown) => console.warn('transfer events unavailable', error));
  });
</script>

<svelte:window onpointerdown={onWindowPointer} onkeydown={onKey} onresize={measure} />

<div class="holder" bind:this={root}>
  <IconButton icon="download" label={t('transfer.title')} active={transfers.popover} onclick={() => (transfers.popover = !transfers.popover)} />
  {#if going.active > 0}
    <span class="badge">{going.active}</span>
    <!-- How far the running transfers are together. -->
    <span class="progress" style:width="{Math.max(8, going.percent ?? 0) * 0.2}px"></span>
  {/if}
  {#if transfers.popover && place}
    <div class="popover" role="dialog" aria-label={t('transfer.title')} style:right="{place.right}px" style:bottom="{place.bottom}px">
      <div class="head">
        <span class="title">{t('transfer.title')}</span>
        <span class="spacer"></span>
        {#if finished}<IconButton icon="clear" small label={t('files.clearFinished')} onclick={() => void transfers.clearFinished()} />{/if}
      </div>
      <div class="list">
        {#each rows as row (row.id)}
          <TransferRow {row} />
        {:else}
          <p class="empty">{t('transfer.empty')}</p>
        {/each}
      </div>
      <button type="button" class="center" onclick={() => ((transfers.popover = false), onopen())}>{t('transfer.openCenter')}</button>
    </div>
  {/if}
</div>

<style>
  .holder {
    position: relative;
  }
  .badge {
    position: absolute;
    top: 0;
    right: -2px;
    min-width: 15px;
    height: 15px;
    padding: 0 4px;
    border-radius: var(--radius-pill);
    background: var(--accent);
    color: var(--accent-fg);
    font-size: 10px;
    font-weight: 600;
    line-height: 15px;
    text-align: center;
    pointer-events: none;
  }
  .progress {
    position: absolute;
    left: 6px;
    bottom: 1px;
    height: 2px;
    border-radius: 1px;
    background: var(--accent);
    pointer-events: none;
  }
  .popover {
    position: fixed;
    z-index: 40;
    width: 340px;
    max-height: min(460px, 70vh);
    display: flex;
    flex-direction: column;
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--material-popover);
    box-shadow: var(--glass-edge), var(--shadow-md);
  }
  .head {
    display: flex;
    align-items: center;
    padding: 0 var(--space-1) var(--space-1) var(--space-2);
  }
  .title {
    font-size: var(--text-footnote);
    font-weight: 600;
  }
  .spacer {
    flex: 1;
  }
  .list {
    min-height: 0;
    overflow-y: auto;
  }
  .empty {
    margin: var(--space-4) 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    text-align: center;
  }
  .center {
    margin-top: var(--space-1);
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    box-shadow: inset 0 1px 0 var(--separator);
    background: transparent;
    color: var(--accent);
    font: inherit;
    font-size: var(--text-footnote);
    cursor: pointer;
  }
  .center:hover {
    background: var(--fill-hover);
  }
</style>
