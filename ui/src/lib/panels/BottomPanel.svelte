<script lang="ts">
  // The bottom panel: files of the focused pane's remote, over the same shared connection.
  import IconButton from '../ui/IconButton.svelte';
  import FileBrowser from '../files/FileBrowser.svelte';
  import { t } from '../i18n';
  import { remoteSourceOf, sourceKey, workspace } from '../workspace/workspace.svelte';

  let { onclose }: { onclose: () => void } = $props();

  const pane = $derived(workspace.focusedPane);
  // A file pane already shows these files; the panel follows terminals only, once they are
  // connected, so it reuses their connection instead of starting (and prompting for) its own.
  const source = $derived(pane && pane.target.kind !== 'sftp' && pane.sessionId ? remoteSourceOf(pane.target) : null);
</script>

<section class="panel">
  <div class="head">
    <span class="title">{t('files.panelTitle')}{pane && source ? ` · ${pane.title}` : ''}</span>
    <IconButton icon="close" label={t('common.close')} small onclick={onclose} />
  </div>
  <div class="body">
    {#if source}
      {#key sourceKey(source)}
        <FileBrowser {source} password={null} remember={false} active={false} visible={true} />
      {/key}
    {:else}
      <p class="muted">{t('status.noRemote')}</p>
    {/if}
  </div>
</section>

<style>
  .panel {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg-elevated);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 30px;
    padding: 0 var(--space-2) 0 var(--space-3);
    border-bottom: 1px solid var(--border);
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .body {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  .muted {
    margin: 0;
    padding: var(--space-3);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
</style>
