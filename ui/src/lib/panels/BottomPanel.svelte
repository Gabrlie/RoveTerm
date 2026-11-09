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
  {#if source}
    {#key sourceKey(source)}
      <FileBrowser {source} password={null} remember={false} active={false} visible={true} {onclose} />
    {/key}
  {:else}
    <p class="muted">{t('status.noRemote')}</p>
    <span class="close"><IconButton icon="close" label={t('common.close')} small onclick={onclose} /></span>
  {/if}
</section>

<style>
  .panel {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface);
  }
  .close {
    position: absolute;
    top: 7px;
    right: var(--space-2);
  }
  .muted {
    margin: 0;
    padding: var(--space-6) var(--space-4);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    text-align: center;
  }
</style>
