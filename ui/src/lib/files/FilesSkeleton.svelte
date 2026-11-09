<script lang="ts">
  // Placeholder for a file list that has nothing to show yet: rows shaped like the real list, and
  // optionally the toolbar above them (when no file browser is mounted at all).
  import IconButton from '../ui/IconButton.svelte';
  import { t } from '../i18n';

  let { toolbar = false, onclose }: { toolbar?: boolean; onclose?: () => void } = $props();

  /** Name widths, so the rows do not look stamped out. */
  const WIDTHS = [46, 30, 58, 38, 50, 26, 42];
</script>

<div class="files-skeleton" aria-busy="true" aria-label={t('status.loading')}>
  {#if toolbar}
    <div class="toolbar">
      <span class="skeleton tool"></span>
      <span class="skeleton tool"></span>
      <span class="skeleton path"></span>
      {#each [0, 1, 2, 3] as index (index)}<span class="skeleton tool"></span>{/each}
      {#if onclose}
        <IconButton icon="close" label={t('common.close')} small onclick={onclose} />
      {/if}
    </div>
  {/if}
  <div class="rows">
    {#each WIDTHS as width, index (index)}
      <div class="row">
        <span class="skeleton glyph"></span>
        <span class="skeleton" style:width="{width}%"></span>
        <span class="skeleton meta"></span>
      </div>
    {/each}
  </div>
</div>

<style>
  .files-skeleton {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    overflow: hidden;
  }
  .toolbar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 38px;
    padding: 0 var(--space-3);
    background: var(--material-bar);
    box-shadow: inset 0 -1px 0 var(--separator);
  }
  .tool {
    width: 18px;
    height: 18px;
    border-radius: var(--radius-xs);
  }
  .path {
    flex: 1;
    height: 22px;
    border-radius: var(--radius-sm);
  }
  .rows {
    display: flex;
    flex-direction: column;
    padding: var(--space-2) var(--space-3);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 28px;
  }
  .glyph {
    flex: none;
    width: 15px;
    height: 15px;
    border-radius: var(--radius-xs);
  }
  .meta {
    width: 110px;
    margin-left: auto;
  }
</style>
