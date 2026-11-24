<script lang="ts">
  // Saved commands. "Insert" types a snippet into the focused terminal; "Run" also presses Enter.
  import { sync } from '../sync/sync.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import Icon from '../ui/Icon.svelte';
  import { ask } from '../ui/confirm.svelte';
  import { sessionWrite, snippetCreate, snippetDelete, snippetList, snippetUpdate, type Snippet } from '../ipc';
  import { errorText, t } from '../i18n';
  import { terminals } from '../terminal/registry';
  import { workspace } from '../workspace/workspace.svelte';

  let { onsent }: { onsent?: () => void } = $props();

  let panel: HTMLDivElement;
  let items = $state<Snippet[]>([]);
  let query = $state('');
  /** `null` while no editor is open; `''` for a new snippet. */
  let editing = $state<string | null>(null);
  let name = $state('');
  let command = $state('');
  let error = $state('');

  const filtered = $derived(
    items.filter((snippet) => {
      const needle = query.trim().toLowerCase();
      return !needle || snippet.name.toLowerCase().includes(needle) || snippet.command.toLowerCase().includes(needle);
    }),
  );

  /** The focused pane when it is a terminal with a running session. */
  const target = $derived.by(() => {
    const pane = workspace.focusedPane;
    return pane && pane.target.kind !== 'sftp' && pane.sessionId ? pane : null;
  });

  async function load(): Promise<void> {
    try {
      items = await snippetList();
    } catch (caught) {
      error = errorText(caught);
    }
  }

  function edit(snippet: Snippet | null): void {
    error = '';
    editing = snippet?.id ?? '';
    name = snippet?.name ?? '';
    command = snippet?.command ?? '';
  }

  async function save(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (editing === null) return;
    try {
      const draft = { name, command };
      if (editing) await snippetUpdate(editing, draft);
      else await snippetCreate(draft);
      editing = null;
      await load();
    } catch (caught) {
      error = errorText(caught);
    }
  }

  async function remove(snippet: Snippet): Promise<void> {
    const confirmed = await ask({
      title: t('snippets.delete'),
      message: t('snippets.confirmDelete', { name: snippet.name }),
      confirmLabel: t('common.delete'),
      danger: true,
    });
    if (!confirmed) return;
    await snippetDelete(snippet.id).catch((caught: unknown) => (error = errorText(caught)));
    await load();
  }

  /**
   * Insert pastes the snippet (bracketed paste, so the shell does not run it). Run types it as
   * keystrokes ending in Enter, in one write so the Enter cannot overtake the text; each line of
   * a multi-line snippet runs in turn.
   */
  async function send(snippet: Snippet, run: boolean): Promise<void> {
    const pane = target;
    if (!pane?.sessionId) return;
    const view = terminals.get(pane.id);
    const text = snippet.command.replace(/\r?\n$/, '');
    if (run) {
      try {
        await sessionWrite(pane.sessionId, `${text.replace(/\r?\n/g, '\r')}\r`);
      } catch (caught) {
        error = errorText(caught);
        return;
      }
      if (!panel?.isConnected || target?.id !== pane.id) return;
    } else {
      if (!view) return;
      view.paste(text);
    }
    view?.focus();
    onsent?.();
  }

  /** Focuses the editor's first field when it appears. */
  function focusOnMount(node: HTMLElement): void {
    queueMicrotask(() => node.focus());
  }

  // Reload after a sync round brought in changes (and on first show).
  $effect(() => {
    void sync.generation;
    void load();
  });
</script>

<div class="snippets" bind:this={panel}>
  <div class="tools">
    <div class="search">
      <Icon name="search" size={13} />
      <input class="bare" bind:value={query} placeholder={t('snippets.search')} spellcheck="false" />
    </div>
    <IconButton icon="add" label={t('snippets.add')} small onclick={() => edit(null)} />
  </div>

  {#if editing !== null}
    <form class="editor" onsubmit={save}>
      <input bind:value={name} placeholder={t('snippets.name')} autocomplete="off" use:focusOnMount />
      <textarea bind:value={command} rows="4" placeholder={t('snippets.command')} spellcheck="false" required></textarea>
      <div class="buttons">
        <button type="button" class="secondary" onclick={() => (editing = null)}>{t('common.cancel')}</button>
        <button type="submit" class="primary">{t('common.save')}</button>
      </div>
    </form>
  {/if}

  {#if !target}
    <p class="muted">{t('snippets.noTerminal')}</p>
  {/if}

  <ul>
    {#each filtered as snippet (snippet.id)}
      <li>
        <div class="text">
          <span class="name">{snippet.name}</span>
          <code>{snippet.command}</code>
        </div>
        <div class="actions">
          <IconButton icon="play" label={t('snippets.run')} small disabled={!target} onclick={() => void send(snippet, true)} />
          <IconButton icon="terminal" label={t('snippets.insert')} small disabled={!target} onclick={() => void send(snippet, false)} />
          <IconButton icon="edit" label={t('snippets.edit')} small onclick={() => edit(snippet)} />
          <IconButton icon="remove" label={t('snippets.delete')} small danger onclick={() => void remove(snippet)} />
        </div>
      </li>
    {:else}
      {#if editing === null}<p class="muted">{query ? t('host.noMatch') : t('snippets.empty')}</p>{/if}
    {/each}
  </ul>
  {#if error}<p class="error">{error}</p>{/if}
</div>

<style>
  .snippets {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    font-size: var(--text-footnote);
  }
  .tools {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .search {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: 0 var(--space-2);
    height: var(--control-height);
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg-muted);
  }
  .search:focus-within {
    background: var(--surface);
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--accent) 70%, transparent),
      var(--focus-ring);
  }
  .search input {
    flex: 1;
    min-width: 0;
    width: 100%;
    color: var(--fg);
    outline: none;
  }
  .editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .editor textarea {
    font-family: var(--font-mono);
    resize: vertical;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-1);
  }
  .buttons button {
    min-width: 0;
    height: 26px;
    padding: 0 var(--space-3);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  li {
    position: relative;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow:
      0 0 0 0.5px var(--separator),
      var(--shadow-sm);
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .name {
    color: var(--fg);
    font-weight: 600;
  }
  code {
    color: var(--fg-muted);
    font-family: var(--font-mono);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 4.5em;
    overflow: hidden;
  }
  .actions {
    position: absolute;
    top: 2px;
    right: 2px;
    display: none;
    gap: 1px;
    border-radius: var(--radius-sm);
    background: var(--material-popover);
    box-shadow: var(--glass-edge);
  }
  li:hover .actions,
  li:focus-within .actions {
    display: flex;
  }
  .muted,
  .error {
    margin: 0;
    color: var(--fg-muted);
  }
  .error {
    color: var(--danger);
  }
  @media (pointer: coarse) {
    .actions {
      position: static;
      display: flex;
      flex-wrap: wrap;
      justify-content: flex-end;
      margin-top: var(--space-2);
      background: transparent;
      box-shadow: none;
    }
    .buttons button {
      min-height: var(--control-height);
    }
  }
</style>
