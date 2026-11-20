<script lang="ts">
  import { writeClipboard } from '../ui/clipboard';
  // Remote file browser bound to one SFTP session. The session opens on mount and closes on
  // destroy; transfers started here keep running after the browser closes.
  import { onDestroy, onMount } from 'svelte';
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import { getCurrentWebview, type DragDropEvent } from '@tauri-apps/api/webview';
  import type { Event as TauriEvent, UnlistenFn } from '@tauri-apps/api/event';
  import Dialog from '../ui/Dialog.svelte';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import {
    sftpClose,
    sftpList,
    sftpMkdir,
    sftpOpen,
    sftpRemove,
    sftpRename,
    transferDownload,
    transferLocalClashes,
    transferUpload,
    type Conflict,
    type DirEntry,
    type ExternalEditUpdate,
    type FsId,
    type RemoteSource,
    type TransferUpdate,
  } from '../ipc';
  import { errorText, t } from '../i18n';
  import { baseName, formatMode, formatSize, formatTime, joinPath, normalizePath, parentPath, sortEntries } from './format';
  import { isFinal, transfers } from './transfers.svelte';
  import { prefs } from '../settings/prefs.svelte';
  import FilesSkeleton from './FilesSkeleton.svelte';
  import ContextMenu, { type MenuItem } from '../ui/ContextMenu.svelte';
  import { editPrefs } from '../settings/edit.svelte';
  import TextEditor from './TextEditor.svelte';
  import { editors } from './editors.svelte';
  import { externalEdits } from './external.svelte';
  import { isMobile } from '../platform';

  interface Props {
    source: RemoteSource;
    /** One-shot password for the first open. */
    password: string | null;
    remember: boolean;
    /** The pane is focused in the visible tab. */
    active: boolean;
    /** The pane's tab is visible. */
    visible: boolean;
    onopened?: () => void;
    onerror?: (error: unknown) => void;
    /** Shows a close button at the right end of the toolbar (the bottom panel). */
    onclose?: () => void;
  }

  let { source, password, remember, active, visible, onopened, onerror, onclose }: Props = $props();

  type Pending =
    | { kind: 'mkdir' }
    | { kind: 'rename'; entry: DirEntry }
    | { kind: 'delete'; names: string[] }
    | { kind: 'conflict'; fs: FsId; paths: string[]; dir: string; clashes: string[] }
    | { kind: 'downloadConflict'; fs: FsId; paths: string[]; dir: string | null; clashes: string[] };

  let fsId = $state<FsId | null>(null);
  let label = $state('');
  let cwd = $state('');
  let pathInput = $state('');
  let entries = $state<DirEntry[]>([]);
  let loading = $state(true);
  let status = $state<{ error: boolean; text: string } | null>(null);
  let selected = $state<Set<string>>(new Set());
  let anchor = -1;
  const showHidden = $derived(prefs.value.files.show_hidden);
  let dropping = $state(false);
  let pending = $state<Pending | null>(null);
  let nameInput = $state('');
  let root: HTMLDivElement;
  let list = $state<HTMLDivElement>();
  let destroyed = false;
  let refreshTimer = 0;
  let unsubscribe: (() => void) | undefined;
  let unlistenEdits: (() => void) | undefined;
  let unlistenDrop: UnlistenFn | undefined;
  let unbindExternal: (() => void) | undefined;
  let editingId = $state<string | null>(null);
  let openingEditor = $state(false);
  const draft = $derived(editingId ? editors.find(editingId) : undefined);

  async function editFile(external = false): Promise<void> {
    const entry = selection[0];
    if (!fsId || selection.length !== 1 || entry?.kind !== 'file' || openingEditor) return;
    const path = joinPath(cwd, entry.name);
    openingEditor = true;
    try {
      if (external) { await externalEdits.open(fsId, path); return; }
      const opened = await editors.open(fsId, label, source, path);
      if (!destroyed) editingId = opened.id;
      else editors.detached(opened.id);
    } catch (error) { fail(error); }
    finally { openingEditor = false; }
  }

  const shown = $derived(sortEntries(showHidden ? entries : entries.filter((entry) => !entry.name.startsWith('.'))));
  const selection = $derived(shown.filter((entry) => selected.has(entry.name)));
  const editable = $derived(selection.length === 1 && selection[0]?.kind === 'file');

  function toggleHidden(): void {
    void editPrefs((draft) => (draft.files.show_hidden = !draft.files.show_hidden));
  }

  /** The stored answer to a name clash, or `null` to ask. */
  function defaultConflict(): Conflict | null {
    const choice = prefs.value.files.default_conflict;
    return choice === 'ask' ? null : choice;
  }

  function fail(error: unknown): void {
    status = { error: true, text: errorText(error) };
  }

  function info(text: string): void {
    status = { error: false, text };
  }

  async function navigate(path: string, focusList = true): Promise<void> {
    if (!fsId) return;
    loading = true;
    try {
      const listing = await sftpList(fsId, path);
      if (path !== cwd) {
        selected = new Set();
        anchor = -1;
      }
      entries = listing;
      cwd = path;
      status = null;
    } catch (error) {
      fail(error);
    } finally {
      pathInput = cwd;
      loading = false;
      // Re-rendered rows drop focus; keep keyboard navigation on the list.
      if (focusList && active && !pending && !draft) list?.focus({ preventScroll: true });
    }
  }

  function refresh(): Promise<void> {
    return navigate(cwd);
  }

  function goUp(): void {
    const parent = parentPath(cwd);
    if (parent) void navigate(parent);
  }

  function submitPath(event: SubmitEvent): void {
    event.preventDefault();
    const target = pathInput.trim();
    if (target) void navigate(normalizePath(target.startsWith('/') ? target : joinPath(cwd, target)));
  }

  function clickRow(event: MouseEvent, index: number): void {
    const entry = shown[index];
    if (!entry) return;
    list?.focus({ preventScroll: true });
    if (event.shiftKey && anchor >= 0) {
      const [from, to] = anchor < index ? [anchor, index] : [index, anchor];
      selected = new Set(shown.slice(from, to + 1).map((row) => row.name));
      return;
    }
    if (event.ctrlKey || event.metaKey) {
      const next = new Set(selected);
      if (next.has(entry.name)) next.delete(entry.name);
      else next.add(entry.name);
      selected = next;
    } else {
      selected = new Set([entry.name]);
    }
    anchor = index;
  }

  /** Extension groups and the system colour their icons use. */
  const FILE_TINTS: [RegExp, string][] = [
    [/\.(zip|tar|gz|tgz|bz2|xz|zst|7z|rar)$/i, 'orange'],
    [/\.(png|jpe?g|gif|webp|svg|bmp|ico|heic)$/i, 'purple'],
    [/\.(sh|bash|zsh|py|rb|pl|js|ts|go|rs|c|h|cpp|java)$/i, 'green'],
    [/\.(conf|cfg|ini|toml|ya?ml|json|xml|env)$/i, 'teal'],
    [/\.(log|txt|md)$/i, 'graphite'],
  ];

  function fileTint(entry: DirEntry): string {
    if (entry.kind === 'dir') return 'var(--accent)';
    if (entry.kind === 'symlink') return 'var(--fg-muted)';
    const tint = FILE_TINTS.find(([pattern]) => pattern.test(entry.name))?.[1];
    return tint ? `var(--sys-${tint})` : 'var(--fg-muted)';
  }

  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  function copyPath(name: string): void {
    writeClipboard(joinPath(cwd, name)).catch((error: unknown) => console.warn('copy failed', error));
  }

  /** Right-click on a row: acts on the selection, selecting the row first when it is not in it. */
  function rowMenu(event: MouseEvent, entry: DirEntry, index: number): void {
    event.preventDefault();
    event.stopPropagation();
    if (!selected.has(entry.name)) {
      selected = new Set([entry.name]);
      anchor = index;
    }
    list?.focus({ preventScroll: true });
    const names = selection.map((row) => row.name);
    const single = names.length === 1;
    const items: MenuItem[] = [];
    if (single && (entry.kind === 'dir' || entry.kind === 'symlink')) {
      items.push({ label: t('files.open'), icon: 'folder', onselect: () => openEntry(entry) });
    }
    items.push(
      { label: t('files.download'), icon: 'download', onselect: () => void download(names, false) },
      { label: t('files.downloadTo'), icon: 'folderDownload', onselect: () => void download(names, true) },
    );
    if (single && entry.kind === 'file') {
      items.push({ label: t('files.editBuiltin'), icon: 'fileCode', separated: true, onselect: () => void editFile() });
      if (!isMobile) items.push({ label: t('files.editExternal'), icon: 'externalEdit', onselect: () => void editFile(true) });
    }
    if (single) items.push({ label: t('files.rename'), icon: 'edit', separated: true, onselect: askRename });
    items.push({ label: t('files.copyPath'), icon: 'copy', separated: !single, onselect: () => copyPath(entry.name) });
    items.push({ label: t('common.delete'), icon: 'remove', danger: true, separated: true, onselect: askDelete });
    menu = { x: event.clientX, y: event.clientY, items };
  }

  /** A click on empty space in the list clears the selection, as in a file manager. */
  function clickBlank(event: MouseEvent): void {
    if ((event.target as HTMLElement).closest('.entry')) return;
    selected = new Set();
    anchor = -1;
  }

  /** Right-click on empty space in the list. */
  function listMenu(event: MouseEvent): void {
    event.preventDefault();
    if (!fsId) return;
    selected = new Set();
    anchor = -1;
    menu = {
      x: event.clientX,
      y: event.clientY,
      items: [
        { label: t('files.newFolder'), icon: 'folderAdd', onselect: askMkdir },
        { label: t('files.upload'), icon: 'upload', onselect: () => void pickUpload(false) },
        { label: t('files.uploadFolder'), icon: 'folderUpload', onselect: () => void pickUpload(true) },
        { label: t('files.refresh'), icon: 'refresh', separated: true, onselect: () => void refresh() },
        { label: t('files.showHidden'), icon: showHidden ? 'eye' : 'eyeOff', onselect: toggleHidden },
      ],
    };
  }

  function openEntry(entry: DirEntry): void {
    if (entry.kind === 'dir' || entry.kind === 'symlink') void navigate(joinPath(cwd, entry.name));
    else void download([entry.name], false);
  }

  /** Remote paths and the file session are fixed when the action starts, so navigating while a
   *  dialog is open cannot change what is transferred. */
  async function download(names: string[], ask: boolean): Promise<void> {
    if (!fsId || names.length === 0) return;
    const fs = fsId;
    const paths = names.map((name) => joinPath(cwd, name));
    let dir: string | null = prefs.device.download_dir || null;
    if (ask) {
      const picked = await openDialog({ directory: true, multiple: false });
      if (typeof picked !== 'string') return;
      dir = picked;
    }
    try {
      const clashes = await transferLocalClashes(names, dir);
      if (clashes.length > 0 && !defaultConflict()) pending = { kind: 'downloadConflict', fs, paths, dir, clashes };
      else await startDownload(fs, paths, dir, defaultConflict() ?? 'rename');
    } catch (error) {
      fail(error);
    }
  }

  async function startDownload(fs: FsId, paths: string[], dir: string | null, conflict: Conflict): Promise<void> {
    pending = null;
    try {
      await transferDownload(fs, paths, dir, conflict);
      info(t('files.downloading', { count: paths.length }));
      transfers.popover = true;
    } catch (error) {
      fail(error);
    }
  }

  async function pickUpload(directory: boolean): Promise<void> {
    const picked = await openDialog({ directory, multiple: true });
    if (picked === null) return;
    startUpload(Array.isArray(picked) ? picked : [picked]);
  }

  function startUpload(paths: string[]): void {
    if (!fsId || paths.length === 0) return;
    const fs = fsId;
    const dir = cwd;
    const existing = new Set(entries.map((entry) => entry.name));
    const clashes = paths.map(baseName).filter((name) => existing.has(name));
    if (clashes.length > 0 && !defaultConflict()) pending = { kind: 'conflict', fs, paths, dir, clashes };
    else void upload(fs, paths, dir, defaultConflict() ?? 'rename');
  }

  async function upload(fs: FsId, paths: string[], dir: string, conflict: Conflict): Promise<void> {
    pending = null;
    try {
      await transferUpload(fs, paths, dir, conflict);
      info(t('files.uploading', { count: paths.length }));
      transfers.popover = true;
    } catch (error) {
      fail(error);
    }
  }

  function askMkdir(): void {
    nameInput = '';
    pending = { kind: 'mkdir' };
  }

  function askRename(): void {
    const entry = selection[0];
    if (!entry) return;
    nameInput = entry.name;
    pending = { kind: 'rename', entry };
  }

  function askDelete(): void {
    if (selection.length === 0) return;
    const request: Pending = { kind: 'delete', names: selection.map((entry) => entry.name) };
    if (prefs.value.files.confirm_delete) pending = request;
    else void confirmPending(undefined, request);
  }

  /** Carries out `action` (the open dialog's by default). */
  async function confirmPending(event?: SubmitEvent, action: Pending | null = pending): Promise<void> {
    event?.preventDefault();
    if (!fsId || !action) return;
    const name = nameInput.trim();
    try {
      if (action.kind === 'mkdir') {
        if (!name) return;
        await sftpMkdir(fsId, cwd, name);
        selected = new Set([name]);
      } else if (action.kind === 'rename') {
        if (!name || name === action.entry.name) {
          pending = null;
          return;
        }
        await sftpRename(fsId, joinPath(cwd, action.entry.name), name);
        selected = new Set([name]);
      } else if (action.kind === 'delete') {
        await sftpRemove(fsId, action.names.map((entry) => joinPath(cwd, entry)));
        selected = new Set();
      }
      pending = null;
      await refresh();
    } catch (error) {
      pending = null;
      fail(error);
    }
  }

  function onListKeydown(event: KeyboardEvent): void {
    if (pending) return;
    const index = shown.findIndex((entry) => entry.name === [...selected].at(-1));
    switch (event.key) {
      case 'ArrowDown':
      case 'ArrowUp': {
        event.preventDefault();
        const next = Math.min(Math.max(index + (event.key === 'ArrowDown' ? 1 : -1), 0), shown.length - 1);
        const entry = shown[next];
        if (entry) {
          selected = new Set([entry.name]);
          anchor = next;
          list?.querySelector<HTMLElement>(`[data-index="${next}"]`)?.scrollIntoView({ block: 'nearest' });
        }
        break;
      }
      case 'Enter': {
        const entry = shown[index];
        if (entry) openEntry(entry);
        break;
      }
      case 'Backspace':
        goUp();
        break;
      case 'Delete':
        askDelete();
        break;
      case 'F2':
        askRename();
        break;
      case 'a':
        if (event.ctrlKey || event.metaKey) {
          event.preventDefault();
          selected = new Set(shown.map((entry) => entry.name));
        }
        break;
    }
  }

  function onTransfer(update: TransferUpdate): void {
    if (!isFinal(update) || update.dst_label !== label) return;
    if (update.state === 'failed') fail({ code: update.error?.kind ? `fs.${update.error.kind}` : 'fs.io', message: '' });
    if (update.state === 'done') refreshAfterSave(update.dst_path);
  }

  function onExternalEdit(update: ExternalEditUpdate): void {
    if (update.label === label && (update.state === 'saved' || update.state === 'closed')) {
      refreshAfterSave(update.path);
    }
  }

  function refreshAfterSave(path: string): void {
    if (!cwd || parentPath(path) !== cwd) return;
    clearTimeout(refreshTimer);
    refreshTimer = window.setTimeout(() => {
      if (!loading && parentPath(path) === cwd) void navigate(cwd, false);
    }, 150);
  }

  function inside(position: { x: number; y: number }): boolean {
    if (!visible || !root || draft) return false;
    const scale = window.devicePixelRatio || 1;
    const x = position.x / scale;
    const y = position.y / scale;
    const rect = root.getBoundingClientRect();
    return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
  }

  function onDragDrop(event: TauriEvent<DragDropEvent>): void {
    const payload = event.payload;
    if (payload.type === 'enter' || payload.type === 'over') {
      dropping = fsId !== null && inside(payload.position);
    } else if (payload.type === 'drop') {
      const hit = dropping && inside(payload.position);
      dropping = false;
      if (hit) startUpload(payload.paths);
    } else {
      dropping = false;
    }
  }

  onMount(async () => {
    unsubscribe = transfers.subscribe(onTransfer);
    if (!isMobile) unlistenEdits = externalEdits.listen(onExternalEdit);
    transfers.start().catch((error: unknown) => console.warn('transfer events unavailable', error));
    try {
      unlistenDrop = await getCurrentWebview().onDragDropEvent(onDragDrop);
    } catch (error) {
      console.info('drag and drop unavailable', error);
    }
    try {
      const opened = await sftpOpen(source, password, remember);
      if (destroyed) {
        void sftpClose(opened.id);
        return;
      }
      fsId = opened.id;
      label = opened.label;
      unbindExternal = externalEdits.bind(label, fsId);
      onopened?.();
      await navigate(opened.home);
    } catch (error) {
      loading = false;
      fail(error);
      onerror?.(error);
    }
  });

  $effect(() => {
    if (active && !pending && !draft) list?.focus({ preventScroll: true });
  });

  onDestroy(() => {
    destroyed = true;
    clearTimeout(refreshTimer);
    unsubscribe?.();
    unlistenEdits?.();
    unlistenDrop?.();
    unbindExternal?.();
    if (editingId) editors.detached(editingId);
    if (fsId) sftpClose(fsId).catch((error: unknown) => console.warn('sftp close failed', error));
  });
</script>

<div class="browser" class:dropping bind:this={root}>
  {#if draft}
    {#key draft.id}<TextEditor {draft} onclose={() => { editingId = null; void refresh(); }} />{/key}
  {:else}
  <div class="toolbar">
    <IconButton icon="up" label={t('files.up')} small disabled={!fsId || parentPath(cwd) === null} onclick={goUp} />
    <IconButton icon="refresh" label={t('files.refresh')} small disabled={!fsId} onclick={() => void refresh()} />
    <form class="path" onsubmit={submitPath}>
      <input class="bare" bind:value={pathInput} spellcheck="false" disabled={!fsId} aria-label={t('files.path')} />
    </form>
    <IconButton icon="folderAdd" label={t('files.newFolder')} small disabled={!fsId} onclick={askMkdir} />
    <IconButton icon="upload" label={t('files.upload')} small disabled={!fsId} onclick={() => void pickUpload(false)} />
    <IconButton icon="folderUpload" label={t('files.uploadFolder')} small disabled={!fsId} onclick={() => void pickUpload(true)} />
    <IconButton icon="download" label={t('files.download')} small disabled={selection.length === 0} onclick={() => void download(selection.map((entry) => entry.name), false)} />
    <IconButton icon="folderDownload" label={t('files.downloadTo')} small disabled={selection.length === 0} onclick={() => void download(selection.map((entry) => entry.name), true)} />
    <IconButton icon="fileCode" label={t('files.editBuiltin')} small disabled={!editable || openingEditor} onclick={() => void editFile()} />
    {#if !isMobile}
      <IconButton icon="externalEdit" label={t('files.editExternal')} small disabled={!editable} onclick={() => void editFile(true)} />
      <IconButton icon="externalEdits" label={t('external.title', { n: externalEdits.all.length })} small active={externalEdits.all.length > 0} onclick={() => (externalEdits.dialog = true)} />
    {/if}
    <IconButton icon="drafts" label={t('editor.drafts')} small active={editors.dirty.length > 0} onclick={() => (editors.showDrafts = true)} />
    <IconButton icon="edit" label={t('files.rename')} small disabled={selection.length !== 1} onclick={askRename} />
    <IconButton icon="remove" label={t('common.delete')} small danger disabled={selection.length === 0} onclick={askDelete} />
    <IconButton icon={showHidden ? 'eye' : 'eyeOff'} label={t('files.showHidden')} small active={showHidden} onclick={toggleHidden} />
    {#if onclose}
      <span class="toolbar-gap"></span>
      <IconButton icon="close" label={t('common.close')} small onclick={onclose} />
    {/if}
  </div>

  <div class="header row">
    <span class="name">{t('files.name')}</span>
    <span class="size">{t('files.size')}</span>
    <span class="time">{t('files.modified')}</span>
    <span class="mode">{t('files.mode')}</span>
  </div>
  <div class="list" tabindex="0" role="listbox" aria-multiselectable="true" aria-label={label} bind:this={list} onkeydown={onListKeydown} oncontextmenu={listMenu} onclick={clickBlank}>
    <!-- Connecting and the first listing both show the skeleton, so no empty list flashes in between. -->
    {#if loading && entries.length === 0}
      <FilesSkeleton />
    {:else if shown.length === 0 && !loading && fsId}
      <p class="placeholder">{t('files.empty')}</p>
    {/if}
    {#each shown as entry, index (entry.name)}
      <div
        class="row entry"
        class:selected={selected.has(entry.name)}
        role="option"
        aria-selected={selected.has(entry.name)}
        tabindex="-1"
        data-index={index}
        onclick={(event) => clickRow(event, index)}
        ondblclick={() => openEntry(entry)}
        oncontextmenu={(event) => rowMenu(event, entry, index)}
        onkeydown={() => {}}
      >
        <span class="name">
          <span class="glyph" style:--tint={fileTint(entry)}><Icon name={entry.kind === 'dir' ? 'folder' : entry.kind === 'symlink' ? 'link' : 'file'} size={15} /></span>
          <span class="label">{entry.name}</span>
        </span>
        <span class="size">{entry.kind === 'dir' ? '' : formatSize(entry.size)}</span>
        <span class="time">{formatTime(entry.modified)}</span>
        <span class="mode">{formatMode(entry.permissions)}</span>
      </div>
    {/each}
  </div>

  <div class="status" class:error={status?.error}>
    {#if status}{status.text}{:else if fsId}{t('files.summary', { count: shown.length, selected: selection.length })}{/if}
  </div>

  {#if dropping}
    <div class="drop-hint"><Icon name="upload" size={20} />{t('files.dropHint', { path: cwd })}</div>
  {/if}
  {/if}
</div>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}

{#if pending?.kind === 'mkdir' || pending?.kind === 'rename'}
  <Dialog title={pending.kind === 'mkdir' ? t('files.newFolder') : t('files.rename')} onclose={() => (pending = null)} width={380}>
    <form id="files-name-form" onsubmit={confirmPending}>
      <input bind:value={nameInput} spellcheck="false" aria-label={t('files.name')} />
    </form>
    {#snippet actions()}
      <button type="button" class="secondary" onclick={() => (pending = null)}>{t('common.cancel')}</button>
      <button type="submit" class="primary" form="files-name-form">{t('common.save')}</button>
    {/snippet}
  </Dialog>
{:else if pending?.kind === 'delete'}
  <Dialog title={t('files.deleteTitle')} onclose={() => (pending = null)} width={400}>
    <p class="message">{t('files.deleteConfirm', { count: pending.names.length, names: pending.names.slice(0, 5).join(', ') })}</p>
    {#snippet actions()}
      <button type="button" class="secondary" onclick={() => (pending = null)}>{t('common.cancel')}</button>
      <button type="button" class="primary danger" onclick={() => void confirmPending()}>{t('common.delete')}</button>
    {/snippet}
  </Dialog>
{:else if pending?.kind === 'conflict'}
  {@const request = pending}
  <Dialog title={t('files.conflictTitle')} onclose={() => (pending = null)} width={440}>
    <p class="message">{t('files.conflictBody', { names: request.clashes.slice(0, 5).join(', ') })}</p>
    {#snippet actions()}
      <button type="button" class="secondary" onclick={() => (pending = null)}>{t('common.cancel')}</button>
      <button type="button" class="secondary" onclick={() => void upload(request.fs, request.paths, request.dir, 'skip')}>{t('files.skip')}</button>
      <button type="button" class="secondary" onclick={() => void upload(request.fs, request.paths, request.dir, 'rename')}>{t('files.keepBoth')}</button>
      <button type="button" class="primary" onclick={() => void upload(request.fs, request.paths, request.dir, 'overwrite')}>{t('files.overwrite')}</button>
    {/snippet}
  </Dialog>
{:else if pending?.kind === 'downloadConflict'}
  {@const request = pending}
  <Dialog title={t('files.conflictTitle')} onclose={() => (pending = null)} width={440}>
    <p class="message">{t('files.downloadConflictBody', { names: request.clashes.slice(0, 5).join(', ') })}</p>
    {#snippet actions()}
      <button type="button" class="secondary" onclick={() => (pending = null)}>{t('common.cancel')}</button>
      <button type="button" class="secondary" onclick={() => void startDownload(request.fs, request.paths, request.dir, 'skip')}>{t('files.skip')}</button>
      <button type="button" class="secondary" onclick={() => void startDownload(request.fs, request.paths, request.dir, 'rename')}>{t('files.keepBoth')}</button>
      <button type="button" class="primary" onclick={() => void startDownload(request.fs, request.paths, request.dir, 'overwrite')}>{t('files.overwrite')}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .browser {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    font-size: var(--text-body);
  }
  .toolbar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    flex: none;
    gap: var(--space-1);
    min-height: 38px;
    padding: var(--space-1) var(--space-2);
    background: var(--material-bar);
    box-shadow: inset 0 -1px 0 var(--separator);
  }
  .toolbar-gap {
    width: 1px;
    height: 16px;
    margin: 0 var(--space-1);
    background: var(--separator);
  }
  .path {
    flex: 1;
    min-width: 80px;
    margin: 0 var(--space-1);
  }
  .path input {
    width: 100%;
    height: 24px;
    padding: 0 var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg);
    font-family: var(--font-mono);
    font-size: var(--text-footnote);
    outline: none;
    transition: box-shadow var(--dur-fast) var(--ease-out);
  }
  .path input:focus {
    background: var(--surface);
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--accent) 70%, transparent),
      var(--focus-ring);
  }
  .row {
    display: grid;
    grid-template-columns: minmax(120px, 1fr) 80px 136px 88px;
    gap: var(--space-2);
    align-items: center;
    margin: 0 6px;
    padding: 0 var(--space-2);
    height: 28px;
    border-radius: var(--radius-sm);
  }
  .header {
    height: 26px;
    margin: 0;
    padding: 0 calc(6px + var(--space-2));
    border-radius: 0;
    color: var(--fg-muted);
    font-size: var(--text-caption);
    font-weight: 600;
    box-shadow: inset 0 -1px 0 var(--separator);
  }
  .list {
    flex: 1;
    min-height: 0;
    padding: 4px 0;
    overflow-y: auto;
    outline: none;
  }
  .entry {
    cursor: default;
    user-select: none;
  }
  /* Stripes only plain rows: `nth-child(… of …)` outranks the hover and selection rules. */
  .entry:nth-child(even of .entry):not(:hover, .selected) {
    background: color-mix(in srgb, var(--fill) 55%, transparent);
  }
  .entry:hover {
    background: var(--fill-hover);
  }
  .entry.selected {
    background: var(--fill-pressed);
  }
  .list:focus .entry.selected {
    background: var(--accent);
  }
  .list:focus .entry.selected .name,
  .list:focus .entry.selected .size,
  .list:focus .entry.selected .time,
  .list:focus .entry.selected .mode,
  .list:focus .entry.selected .glyph {
    color: var(--accent-fg);
  }
  .name {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    color: var(--fg);
  }
  .glyph {
    display: inline-flex;
    color: var(--tint);
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .size,
  .time,
  .mode {
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    white-space: nowrap;
    overflow: hidden;
  }
  .size {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .time {
    font-variant-numeric: tabular-nums;
  }
  .mode {
    font-family: var(--font-mono);
  }
  .placeholder {
    padding: var(--space-6) var(--space-4);
    color: var(--fg-muted);
    text-align: center;
  }
  .status {
    padding: 3px var(--space-3);
    color: var(--fg-muted);
    font-size: var(--text-caption);
    min-height: 22px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    box-shadow: inset 0 1px 0 var(--separator);
  }
  .status.error {
    color: var(--danger);
  }
  .drop-hint {
    position: absolute;
    inset: var(--space-2);
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    border: 2px dashed color-mix(in srgb, var(--accent) 70%, transparent);
    border-radius: var(--radius-lg);
    background: var(--accent-softer);
    backdrop-filter: blur(2px);
    color: var(--accent);
    font-weight: 600;
    pointer-events: none;
  }
  form input {
    width: 100%;
  }
  .message {
    margin: 0;
    word-break: break-all;
  }
</style>
