<script lang="ts">
  // The window's top bar. Windows and Linux draw it instead of the system title bar: text menus on
  // the left, layout toggles and window buttons on the right. On macOS the menus live in the
  // system menu bar and the window buttons are the traffic lights drawn over the left corner.
  // Empty space drags the window; double-clicking it maximizes.
  import { onDestroy, onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import IconButton from '../ui/IconButton.svelte';
  import Icon from '../ui/Icon.svelte';
  import ContextMenu, { type MenuItem } from '../ui/ContextMenu.svelte';
  import { actions, bindingsOf, displayBinding, isMac, type ActionId } from '../actions';
  import { MENUS, type Menu } from '../menu';
  import { t } from '../i18n';

  interface Props {
    sidebarOpen: boolean;
    rightOpen: boolean;
    bottomOpen: boolean;
    onaction: (id: ActionId) => void;
  }

  let { sidebarOpen, rightOpen, bottomOpen, onaction }: Props = $props();

  let bar = $state<HTMLElement>();
  let open = $state<{ id: string; x: number; y: number } | null>(null);
  let maximized = $state(false);
  let unlistenResize: (() => void) | undefined;

  function items(menu: Menu): MenuItem[] {
    const result: MenuItem[] = [];
    let separated = false;
    for (const entry of menu.items) {
      if (entry === '-') {
        separated = result.length > 0;
        continue;
      }
      const action = actions.find((candidate) => candidate.id === entry);
      if (!action) continue;
      const binding = bindingsOf(action)[0];
      result.push({ label: t(action.label), hint: binding ? displayBinding(binding) : undefined, separated, onselect: () => onaction(entry) });
      separated = false;
    }
    return result;
  }

  function show(menu: Menu, target: HTMLElement): void {
    const rect = target.getBoundingClientRect();
    open = { id: menu.id, x: rect.left, y: rect.bottom + 4 };
  }

  function toggle(menu: Menu, event: PointerEvent): void {
    if (event.button !== 0) return;
    if (open?.id === menu.id) open = null;
    else show(menu, event.currentTarget as HTMLElement);
  }

  /** While a menu is open, pointing at another menu title opens that one instead. */
  function hover(menu: Menu, event: PointerEvent): void {
    if (open && open.id !== menu.id) show(menu, event.currentTarget as HTMLElement);
  }

  function step(direction: -1 | 1): void {
    if (!open || !bar) return;
    const index = MENUS.findIndex((menu) => menu.id === open?.id);
    const next = MENUS[(index + direction + MENUS.length) % MENUS.length];
    const title = next && bar.querySelector<HTMLElement>(`[data-menu="${next.id}"]`);
    if (next && title) show(next, title);
  }

  const appWindow = isMac ? null : getCurrentWindow();

  async function syncMaximized(): Promise<void> {
    maximized = (await appWindow?.isMaximized().catch(() => false)) ?? false;
  }

  onMount(async () => {
    if (!appWindow) return;
    await syncMaximized();
    unlistenResize = await appWindow.onResized(() => void syncMaximized());
  });

  onDestroy(() => unlistenResize?.());
</script>

<header class="titlebar" class:mac={isMac} bind:this={bar} data-tauri-drag-region>
  {#if !isMac}
    <span class="mark" aria-hidden="true"><Icon name="terminal" size={15} /></span>
    <nav class="menus" aria-label={t('menu.bar')}>
      {#each MENUS as menu (menu.id)}
        <button
          type="button"
          class="menu-title"
          class:open={open?.id === menu.id}
          data-menu={menu.id}
          aria-haspopup="menu"
          aria-expanded={open?.id === menu.id}
          onpointerdown={(event) => toggle(menu, event)}
          onpointerenter={(event) => hover(menu, event)}
          onkeydown={(event) => (event.key === 'Enter' || event.key === ' ' || event.key === 'ArrowDown') && (event.preventDefault(), show(menu, event.currentTarget))}
        >
          {t(menu.label)}
        </button>
      {/each}
    </nav>
  {/if}
  <span class="drag" data-tauri-drag-region></span>
  <div class="layout">
    <IconButton icon="sidebar" small label={t('action.sidebar.toggle')} active={sidebarOpen} onclick={() => onaction('sidebar.toggle')} />
    <IconButton icon="panelBottom" small label={t('action.panel.bottom')} active={bottomOpen} onclick={() => onaction('panel.bottom')} />
    <IconButton icon="panelRight" small label={t('action.panel.right')} active={rightOpen} onclick={() => onaction('panel.right')} />
  </div>
  {#if appWindow}
    <div class="window">
      <button type="button" class="window-button" aria-label={t('window.minimize')} title={t('window.minimize')} onclick={() => void appWindow.minimize()}>
        <Icon name="minimize" size={14} />
      </button>
      <button
        type="button"
        class="window-button"
        aria-label={maximized ? t('window.restore') : t('window.maximize')}
        title={maximized ? t('window.restore') : t('window.maximize')}
        onclick={() => void appWindow.toggleMaximize()}
      >
        <Icon name={maximized ? 'restore' : 'maximize'} size={12} />
      </button>
      <button type="button" class="window-button close" aria-label={t('window.close')} title={t('window.close')} onclick={() => void appWindow.close()}>
        <Icon name="winClose" size={15} />
      </button>
    </div>
  {/if}
</header>

{#if open}
  {@const menu = MENUS.find((candidate) => candidate.id === open?.id)}
  {#if menu}
    {#key menu.id}
      <ContextMenu x={open.x} y={open.y} items={items(menu)} owner={bar} onstep={step} onclose={() => (open = null)} />
    {/key}
  {/if}
{/if}

<style>
  .titlebar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
    height: 38px;
    padding-left: var(--space-3);
    user-select: none;
  }
  /* Room for the traffic lights. */
  .titlebar.mac {
    padding-left: 80px;
    padding-right: var(--space-2);
  }
  .mark {
    display: inline-flex;
    margin-right: var(--space-2);
    color: var(--accent);
  }
  .menus {
    display: flex;
    gap: 1px;
  }
  .menu-title {
    height: 26px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: var(--text-body);
    cursor: default;
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .menu-title:hover,
  .menu-title.open {
    background: var(--fill-hover);
  }
  .menu-title:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .drag {
    flex: 1;
    align-self: stretch;
  }
  .layout {
    display: flex;
    gap: 2px;
    margin-right: var(--space-2);
  }
  .window {
    display: flex;
    align-self: stretch;
  }
  .window-button {
    display: grid;
    place-items: center;
    width: 46px;
    border: none;
    background: transparent;
    color: var(--fg);
    cursor: default;
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .window-button:hover {
    background: var(--fill-hover);
  }
  .window-button.close:hover {
    background: var(--sys-red);
    color: #fff;
  }
</style>
