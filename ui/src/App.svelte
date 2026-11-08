<script lang="ts">
  import { onMount } from 'svelte';
  import TabBar from './lib/tabs/TabBar.svelte';
  import PaneGrid from './lib/layout/PaneGrid.svelte';
  import CommandPalette from './lib/ui/CommandPalette.svelte';
  import { actionFor, type ActionId } from './lib/actions';
  import { terminals } from './lib/terminal/registry';
  import { workspace } from './lib/workspace/workspace.svelte';
  import { t } from './lib/i18n';

  let paletteOpen = $state(false);

  function focusedTerminal() {
    const pane = workspace.focusedPane;
    return pane ? terminals.get(pane.id) : undefined;
  }

  function run(id: ActionId): void {
    switch (id) {
      case 'tab.newLocal':
        workspace.openTab({ kind: 'local' });
        break;
      case 'tab.close':
        workspace.closeFocused();
        break;
      case 'tab.next':
        workspace.cycleTab(1);
        break;
      case 'tab.prev':
        workspace.cycleTab(-1);
        break;
      case 'pane.splitRight':
        workspace.splitFocused('row');
        break;
      case 'pane.splitDown':
        workspace.splitFocused('column');
        break;
      case 'pane.focusNext':
        workspace.cyclePane(1);
        break;
      case 'pane.focusPrev':
        workspace.cyclePane(-1);
        break;
      case 'terminal.copy': {
        const text = focusedTerminal()?.selection();
        if (text) navigator.clipboard.writeText(text).catch((error: unknown) => console.warn('copy failed', error));
        break;
      }
      case 'terminal.paste':
        navigator.clipboard
          .readText()
          .then((text) => focusedTerminal()?.paste(text))
          .catch((error: unknown) => console.warn('paste failed', error));
        break;
      case 'palette.open':
        paletteOpen = true;
        break;
      default:
        if (id.startsWith('tab.goto')) workspace.gotoTab(Number(id.slice('tab.goto'.length)) - 1);
    }
  }

  function onKeydown(event: KeyboardEvent): void {
    const id = actionFor(event);
    if (!id) return;
    event.preventDefault();
    event.stopPropagation();
    run(id);
  }

  onMount(() => {
    workspace.openTab({ kind: 'local' });
  });
</script>

<svelte:window onkeydowncapture={onKeydown} />

<div class="app">
  <TabBar onnew={() => run('tab.newLocal')} />
  <main class="stage">
    {#each workspace.tabs as tab (tab.id)}
      <PaneGrid {tab} visible={tab.id === workspace.activeTabId} />
    {:else}
      <div class="empty">{t('workspace.empty')}</div>
    {/each}
  </main>
</div>

{#if paletteOpen}
  <CommandPalette onrun={run} onclose={() => (paletteOpen = false)} />
{/if}

<style>
  .app {
    height: 100vh;
    display: flex;
    flex-direction: column;
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    background: var(--term-bg);
  }
  .empty {
    height: 100%;
    display: grid;
    place-items: center;
    color: var(--fg-muted);
  }
</style>
