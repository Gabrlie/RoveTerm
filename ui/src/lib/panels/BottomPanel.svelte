<script lang="ts">
  // The bottom panel: files of the focused pane's remote, over the same shared connection.
  import { untrack } from 'svelte';
  import FileBrowser from '../files/FileBrowser.svelte';
  import FilesSkeleton from '../files/FilesSkeleton.svelte';
  import { remoteSourceOf, sourceKey, workspace } from '../workspace/workspace.svelte';
  import type { RemoteSource } from '../ipc';

  let { onclose }: { onclose: () => void } = $props();

  const pane = $derived(workspace.focusedPane);
  // A file pane already shows these files; the panel follows terminals only, once they are
  // connected, so it reuses their connection instead of starting (and prompting for) its own.
  const source = $derived(pane && pane.target.kind !== 'sftp' && pane.sessionId ? remoteSourceOf(pane.target) : null);
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
    <FilesSkeleton toolbar {onclose} />
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
</style>
