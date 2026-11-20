<script lang="ts">
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { externalEdits } from './external.svelte';
  import { externalEditReveal, type ExternalEditUpdate } from '../ipc';
  import { errorText, t, type MessageKey } from '../i18n';
  import { writeClipboard } from '../ui/clipboard';
  import { notify } from '../ui/toast.svelte';
  const states: Record<ExternalEditUpdate['state'], MessageKey> = {
    watching: 'external.watching', uploading: 'external.uploading', saved: 'external.saved', conflict: 'external.conflict',
    failed: 'external.failed', recovered: 'external.recovered', closed: 'external.closed',
  };
  async function copy(path: string): Promise<void> {
    try { await writeClipboard(path); } catch (error) { notify(errorText(error), 'error'); }
  }
  async function reveal(id: string): Promise<void> {
    try { await externalEditReveal(id); } catch (error) { notify(errorText(error), 'error'); }
  }
</script>

<div class="external-edits">
  {#if !externalEdits.all.length}<p class="empty">{t('external.empty')}</p>{/if}
  {#each externalEdits.all as row (row.id)}
    <div class="edit-row" class:attention={['conflict', 'failed', 'recovered'].includes(row.state)}>
      <div class="title"><Icon name="externalEdit" size={16} /><strong title={row.path}>{row.path.split('/').at(-1)}</strong><span class="state">{t(states[row.state])}</span></div>
      <p class="path" title={row.label + ' · ' + row.path}>{row.label} · {row.path}</p>
      <p class="local selectable" title={row.local_path}>{row.local_path}</p>
      {#if row.error}<p class="error selectable">{errorText(row.error)}</p>{/if}
      {#if row.state === 'recovered'}<p class="hint">{t('external.recoveredHint')}</p>{/if}
      <div class="buttons">
        <IconButton icon="externalEdit" small disabled={row.state === 'uploading'} label={t('external.openCopy')} onclick={() => void externalEdits.reopen(row)} />
        <IconButton icon="copy" small label={t('external.copyLocation')} onclick={() => void copy(row.local_path)} />
        <IconButton icon="folder" small label={t('external.reveal')} onclick={() => void reveal(row.id)} />
        <span class="gap"></span>
        {#if row.state === 'conflict'}
          <button class="secondary danger" onclick={() => void externalEdits.retry(row, true)}>{t('external.overwrite')}</button>
        {:else if row.state === 'failed' || row.state === 'recovered'}
          <button class="secondary" onclick={() => void externalEdits.retry(row)}>{t('external.retry')}</button>
        {/if}
        <button class="secondary" disabled={row.state === 'uploading'} onclick={() => void externalEdits.finish(row)}>{t('external.finish')}</button>
        <IconButton icon="remove" small danger disabled={row.state === 'uploading'} label={t('external.discard')} onclick={() => void externalEdits.discard(row)} />
      </div>
    </div>
  {/each}
</div>

<style>
  .external-edits { display: flex; flex-direction: column; gap: var(--space-2); min-width: 0; }
  .edit-row { border-radius: var(--radius-sm); border: 1px solid var(--separator); padding: var(--space-3); background: var(--surface); }
  .edit-row.attention { border-color: color-mix(in srgb, var(--warning) 40%, var(--separator)); }
  .title { display: flex; align-items: center; gap: var(--space-2); color: var(--fg); }
  strong { font-size: var(--text-footnote); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .state { font-size: var(--text-caption); color: var(--fg-muted); margin-left: auto; white-space: nowrap; }
  .attention .state { color: var(--warning); }
  p { margin: var(--space-1) 0 0; font-size: var(--text-caption); }
  .path, .local { font-family: var(--font-mono); color: var(--fg-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .error { color: var(--danger); overflow-wrap: anywhere; }
  .hint { color: var(--warning); }
  .buttons { display: flex; align-items: center; flex-wrap: wrap; gap: var(--space-1); margin-top: var(--space-2); }
  .buttons button { font-size: var(--text-caption); }
  .gap { flex: 1; }
  .empty { color: var(--fg-muted); text-align: center; font-size: var(--text-footnote); padding: var(--space-4); }
</style>
