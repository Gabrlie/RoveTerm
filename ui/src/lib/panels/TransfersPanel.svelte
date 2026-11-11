<script lang="ts">
  // The download center: every upload and download of this run, the ones still going first.
  import IconButton from '../ui/IconButton.svelte';
  import { t } from '../i18n';
  import TransferRow from '../files/TransferRow.svelte';
  import { transfers } from '../files/transfers.svelte';
  import { isFinal, newestFirst } from '../files/transferview';

  const going = $derived(newestFirst(transfers.all.filter((row) => !isFinal(row))));
  const done = $derived(newestFirst(transfers.all.filter(isFinal)));
</script>

<div class="center">
  {#if going.length === 0 && done.length === 0}
    <p class="empty">{t('transfer.empty')}</p>
  {/if}
  {#if going.length > 0}
    <p class="section">{t('transfer.going', { n: going.length })}</p>
    {#each going as row (row.id)}<TransferRow {row} />{/each}
  {/if}
  {#if done.length > 0}
    <div class="section-row">
      <p class="section">{t('transfer.finished', { n: done.length })}</p>
      <IconButton icon="clear" small label={t('files.clearFinished')} onclick={() => void transfers.clearFinished()} />
    </div>
    {#each done as row (row.id)}<TransferRow {row} />{/each}
  {/if}
</div>

<style>
  .center {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 var(--space-2) var(--space-3);
  }
  .section-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .section {
    margin: var(--space-2) var(--space-2) var(--space-1);
    color: var(--fg-muted);
    font-size: 11px;
    font-weight: 600;
  }
  .empty {
    margin: var(--space-6) 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    text-align: center;
  }
</style>
