<script lang="ts">
  // The bottom panel: files of the focused pane's remote, over the same shared connection.
  import { untrack } from 'svelte';
  import FileBrowser from '../files/FileBrowser.svelte';
  import FilesSkeleton from '../files/FilesSkeleton.svelte';
  import { t } from '../i18n';
  import { viewport } from '../layout/viewport.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { remoteSourceOf, sourceKey, workspace } from '../workspace/workspace.svelte';
  import type { RemoteSource } from '../ipc';

  let { onclose }: { onclose: () => void } = $props();

  const pane = $derived(workspace.focusedPane);
  // A file pane already shows these files; the panel follows connected terminals and editor tabs
  // opened from a remote host, reusing their shared connection instead of prompting for its own.
  const source = $derived(
    pane && pane.target.kind !== 'sftp' && (pane.sessionId || pane.target.kind === 'editor')
      ? remoteSourceOf(pane.target)
      : null,
  );
  const current = $derived(source ? sourceKey(source) : null);

  /** File browsers kept open, most recently shown last. Switching back to a host shows its
   *  browser as it was (folder, list, selection); beyond this many the oldest is closed. */
  const KEEP = 4;
  let kept = $state<{ key: string; source: RemoteSource }[]>([]);

  /** The remote a key belongs to, without how it is reached. */
  const remoteOf = (key: string): string => key.split('=')[0] ?? key;

  $effect(() => {
    if (!source || !current) return;
    // A browser of the same host reached another way (its address or route was edited) is
    // closed: it still works on the old connection.
    const others = untrack(() => kept).filter((entry) => entry.key !== current && remoteOf(entry.key) !== remoteOf(current));
    kept = [...others.slice(Math.max(0, others.length - (KEEP - 1))), { key: current, source }];
  });
</script>

<section class="panel">
  {#each kept as entry (entry.key)}
    <div class="browser" hidden={entry.key !== current}>
      <FileBrowser source={entry.source} password={null} remember={false} active={false} visible={entry.key === current} {onclose} />
    </div>
  {/each}
  {#if !source}
    {#if viewport.compact}
      <header class="mobile-nav">
        <IconButton icon="back" label={t('mobile.back')} onclick={onclose} />
        <strong>{t('action.panel.bottom')}</strong>
      </header>
      <FilesSkeleton />
    {:else}
      <FilesSkeleton toolbar {onclose} />
    {/if}
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
  .browser {
    flex: 1;
    min-height: 0;
  }
  .browser[hidden] {
    display: none;
  }
  .mobile-nav {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--m-header-nav, 48px);
    padding: 0 var(--space-3);
    background: var(--material-bar);
    box-shadow: inset 0 -0.5px 0 var(--separator);
  }
  .mobile-nav strong {
    font-size: var(--m-text-nav, 16px);
    font-weight: 600;
  }
</style>
