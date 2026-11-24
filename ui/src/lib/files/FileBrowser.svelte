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
    transferDiscardStaged,
    transferDownload,
    transferLocalClashes,
    transferPrepareUpload,
    transferUpload,
    type Conflict,
    type DirEntry,
    type ExternalEditUpdate,
    type FsId,
    type RemoteSource,
    type StagedUpload,
    type TransferUpdate,
  } from '../ipc';
  import { errorText, t } from '../i18n';
  import { baseName, formatMode, formatSize, formatTime, isEditableTextFile, joinPath, normalizePath, parentPath, sortEntries } from './format';
  import { allSelected, exceededMoveThreshold, formatCompactMeta, LONG_PRESS_MS, toggleSelectAll, toggleSelection, touchRowAction } from './fileSelection';
  import { isFinal, transfers } from './transfers.svelte';
  import { prefs } from '../settings/prefs.svelte';
  import FilesSkeleton from './FilesSkeleton.svelte';
  import ContextMenu, { type MenuItem } from '../ui/ContextMenu.svelte';
  import { editPrefs } from '../settings/edit.svelte';
  import { editors } from './editors.svelte';
  import { workspace } from '../workspace/workspace.svelte';
  import { externalEdits } from './external.svelte';
  import { isMobile } from '../platform';
  import { viewport } from '../layout/viewport.svelte';
  import { notify } from '../ui/toast.svelte';

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
    | { kind: 'conflict'; fs: FsId; paths: string[]; dir: string; clashes: string[]; stagedPaths: string[] }
    | { kind: 'downloadConflict'; fs: FsId; paths: string[]; dir: string | null; clashes: string[] };

  let fsId = $state<FsId | null>(null);
  let label = $state('');
  let cwd = $state('');
  let pathInput = $state('');
  let entries = $state<DirEntry[]>([]);
  let loading = $state(true);
  let loadingPath = $state<string | null>(null);
  let navSeq = 0;
  let status = $state<{ error: boolean; text: string } | null>(null);
  let selected = $state<Set<string>>(new Set());
  let selectMode = $state(false);
  let anchor = -1;
  const touchUi = $derived(isMobile || viewport.coarse);
  const compactList = $derived(viewport.compact);
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
  let unlistenSaves: (() => void) | undefined;
  let unlistenDrop: UnlistenFn | undefined;
  let unbindExternal: (() => void) | undefined;
  let openingEditor = $state(false);
  let press: { id: number; x: number; y: number; timer: number; fired: boolean } | undefined;
  let suppressNextClick = false;
  let suppressClickTimer = 0;

  async function editFile(external = false, targetEntry?: DirEntry): Promise<void> {
    const entry = targetEntry ?? (selection.length === 1 ? selection[0] : undefined);
    if (!fsId || entry?.kind !== 'file' || openingEditor) return;
    const path = joinPath(cwd, entry.name);
    openingEditor = true;
    try {
      if (external) { await externalEdits.open(fsId, path); return; }
      const opened = await editors.open(fsId, label, source, path);
      if (!destroyed && editors.find(opened.id)) workspace.openEditor(opened);
      else editors.detached(opened.id);
    } catch (error) {
      if (!destroyed) fail(error);
      notify(errorText(error), 'error');
    }
    finally { openingEditor = false; }
  }

  const shown = $derived(sortEntries(showHidden ? entries : entries.filter((entry) => !entry.name.startsWith('.'))));
  const shownNames = $derived(shown.map((entry) => entry.name));
  const selection = $derived(shown.filter((entry) => selected.has(entry.name)));
  const editable = $derived(selection.length === 1 && selection[0]?.kind === 'file');

  function exitSelectMode(): void {
    selectMode = false;
    selected = new Set();
    anchor = -1;
  }

  function toggleSelectMode(): void {
    if (selectMode) exitSelectMode();
    else selectMode = true;
  }

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
    if (!fsId || destroyed) return;
    const seq = ++navSeq;
    loading = true;
    loadingPath = path;
    try {
      const listing = await sftpList(fsId, path);
      if (seq !== navSeq || destroyed) return;
      if (path !== cwd) {
        exitSelectMode();
        cancelLongPress();
      }
      entries = listing;
      cwd = path;
      status = null;
    } catch (error) {
      if (seq === navSeq && !destroyed) fail(error);
    } finally {
      if (seq === navSeq && !destroyed) {
        pathInput = cwd;
        loadingPath = null;
        loading = false;
        // Re-rendered rows drop focus; keep keyboard navigation on the list.
        if (!viewport.coarse && focusList && active && !pending) list?.focus({ preventScroll: true });
      }
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
    if (touchUi) {
      if (suppressNextClick) {
        clearClickSuppression();
      } else if (selectMode) {
        selected = toggleSelection(selected, entry.name);
      } else {
        switch (touchRowAction(entry)) {
          case 'open': void navigate(joinPath(cwd, entry.name)); break;
          case 'edit': void editFile(false, entry); break;
          case 'menu': rowMenu(event.clientX, event.clientY, entry, index); break;
        }
      }
      return;
    }
    if (!viewport.coarse) list?.focus({ preventScroll: true });
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

  function clearClickSuppression(): void {
    clearTimeout(suppressClickTimer);
    suppressNextClick = false;
  }

  function suppressPressClick(): void {
    clearTimeout(suppressClickTimer);
    suppressNextClick = true;
    suppressClickTimer = window.setTimeout(clearClickSuppression, 700);
  }

  function cancelLongPress(): void {
    if (press) clearTimeout(press.timer);
    press = undefined;
  }

  function startLongPress(event: PointerEvent, entry: DirEntry, index: number): void {
    if (!touchUi || !event.isPrimary) return;
    clearClickSuppression();
    cancelLongPress();
    if ((event.pointerType !== 'touch' && event.pointerType !== 'pen') ||
        (event.target instanceof Element && event.target.closest('.row-more'))) return;
    const { pointerId: id, clientX: x, clientY: y } = event;
    const candidate = { id, x, y, timer: 0, fired: false };
    candidate.timer = window.setTimeout(() => {
      candidate.fired = true;
      suppressPressClick();
      rowMenu(x, y, entry, index);
    }, LONG_PRESS_MS);
    press = candidate;
  }

  function moveLongPress(event: PointerEvent): void {
    if (press?.id === event.pointerId && !press.fired &&
        exceededMoveThreshold(press.x, press.y, event.clientX, event.clientY)) {
      cancelLongPress();
    }
  }

  function endLongPress(event: PointerEvent): void {
    if (press?.id !== event.pointerId) return;
    if (press.fired) suppressPressClick();
    cancelLongPress();
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
  function onRowContextMenu(event: MouseEvent, entry: DirEntry, index: number): void {
    event.preventDefault();
    event.stopPropagation();
    if (touchUi && (press || suppressNextClick ||
        (event instanceof PointerEvent && (event.pointerType === 'touch' || event.pointerType === 'pen')))) return;
    rowMenu(event.clientX, event.clientY, entry, index);
  }

  function openRowMenuAtButton(event: MouseEvent, entry: DirEntry, index: number): void {
    const button = event.currentTarget as HTMLButtonElement;
    const { left, bottom } = button.getBoundingClientRect();
    rowMenu(left, bottom, entry, index);
  }

  function rowMenu(x: number, y: number, entry: DirEntry, index: number): void {
    if (!selected.has(entry.name)) {
      selected = new Set([entry.name]);
      anchor = index;
    }
    if (!viewport.coarse) list?.focus({ preventScroll: true });
    const names = selection.map((row) => row.name);
    const single = names.length === 1;
    const items: MenuItem[] = [];
    if (single && (entry.kind === 'dir' || entry.kind === 'symlink')) {
      items.push({ label: t('files.open'), icon: 'folder', onselect: () => openEntry(entry) });
    }
    items.push({ label: t('files.download'), icon: 'download', onselect: () => void download(names, false) });
    if (!isMobile) items.push({ label: t('files.downloadTo'), icon: 'folderDownload', onselect: () => void download(names, true) });
    if (single && entry.kind === 'file') {
      items.push({ label: t('files.editBuiltin'), icon: 'fileCode', separated: true, onselect: () => void editFile() });
      if (!isMobile) items.push({ label: t('files.editExternal'), icon: 'externalEdit', onselect: () => void editFile(true) });
    }
    if (single) items.push({ label: t('files.rename'), icon: 'edit', separated: true, onselect: askRename });
    items.push({ label: t('files.copyPath'), icon: 'copy', separated: !single, onselect: () => copyPath(entry.name) });
    items.push({ label: t('common.delete'), icon: 'remove', danger: true, separated: true, onselect: askDelete });
    menu = { x, y, items };
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
    const items: MenuItem[] = [
      { label: t('files.newFolder'), icon: 'folderAdd', onselect: askMkdir },
      { label: t('files.upload'), icon: 'upload', onselect: () => void pickUpload(false) },
    ];
    if (!isMobile) items.push({ label: t('files.uploadFolder'), icon: 'folderUpload', onselect: () => void pickUpload(true) });
    items.push(
      { label: t('files.refresh'), icon: 'refresh', separated: true, onselect: () => void refresh() },
      { label: t('files.showHidden'), icon: showHidden ? 'eye' : 'eyeOff', onselect: toggleHidden },
    );
    menu = { x: event.clientX, y: event.clientY, items };
  }

  function openEntry(entry: DirEntry): void {
    if (entry.kind === 'dir' || entry.kind === 'symlink') void navigate(joinPath(cwd, entry.name));
    else if (isEditableTextFile(entry)) void editFile(false, entry);
    else void download([entry.name], false);
  }

  /** Remote paths and the file session are fixed when the action starts, so navigating while a
   *  dialog is open cannot change what is transferred. */
  async function download(names: string[], ask: boolean): Promise<void> {
    if (!fsId || names.length === 0 || (isMobile && ask)) return;
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
      if (destroyed) return;
      if (clashes.length > 0 && !defaultConflict()) {
        cancelPending();
        pending = { kind: 'downloadConflict', fs, paths, dir, clashes };
      } else {
        await startDownload(fs, paths, dir, defaultConflict() ?? 'rename');
      }
    } catch (error) {
      fail(error);
    }
  }

  async function startDownload(fs: FsId, paths: string[], dir: string | null, conflict: Conflict): Promise<void> {
    cancelPending();
    try {
      await transferDownload(fs, paths, dir, conflict);
      info(t('files.downloading', { count: paths.length }));
      transfers.popover = true;
    } catch (error) {
      fail(error);
    }
  }

  async function pickUpload(directory: boolean): Promise<void> {
    if (isMobile && directory) return;
    try {
      const picked = await openDialog({ directory, multiple: true });
      if (picked === null || destroyed) return;
      const rawPaths = Array.isArray(picked) ? picked : [picked];
      const staged: StagedUpload[] = await transferPrepareUpload(rawPaths);
      if (destroyed) {
        discardStaged(staged.map((item) => item.path));
        return;
      }
      startPreparedUpload(staged);
    } catch (error) {
      if (!destroyed) fail(error);
    }
  }

  function discardStaged(paths: string[]): void {
    if (paths.length === 0) return;
    void transferDiscardStaged(paths).catch((error: unknown) => console.warn('staged upload cleanup failed', error));
  }

  function cancelPending(): void {
    if (pending?.kind === 'conflict') discardStaged(pending.stagedPaths);
    pending = null;
  }

  function startPreparedUpload(staged: StagedUpload[]): void {
    const paths = staged.map((item) => item.path);
    startUpload(paths, staged.map((item) => item.name), paths);
  }

  function startUpload(paths: string[], names = paths.map(baseName), stagedPaths: string[] = []): void {
    if (!fsId || paths.length === 0) return;
    const fs = fsId;
    const dir = cwd;
    const existing = new Set(entries.map((entry) => entry.name));
    const clashes = names.filter((name) => existing.has(name));
    const conflict = defaultConflict();
    if (clashes.length > 0 && !conflict) {
      cancelPending();
      pending = { kind: 'conflict', fs, paths, dir, clashes, stagedPaths };
    } else {
      void upload(fs, paths, dir, conflict ?? 'rename', stagedPaths);
    }
  }

  function confirmUpload(conflict: Conflict): void {
    if (pending?.kind !== 'conflict') return;
    const { fs, paths, dir, stagedPaths } = pending;
    pending = null;
    void upload(fs, paths, dir, conflict, stagedPaths);
  }

  async function upload(fs: FsId, paths: string[], dir: string, conflict: Conflict, stagedPaths: string[] = []): Promise<void> {
    try {
      await transferUpload(fs, paths, dir, conflict);
      info(t('files.uploading', { count: paths.length }));
      transfers.popover = true;
    } catch (error) {
      discardStaged(stagedPaths);
      fail(error);
    }
  }

  function askMkdir(): void {
    nameInput = '';
    cancelPending();
    pending = { kind: 'mkdir' };
  }

  function askRename(): void {
    const entry = selection[0];
    if (!entry) return;
    nameInput = entry.name;
    cancelPending();
    pending = { kind: 'rename', entry };
  }

  function askDelete(): void {
    if (selection.length === 0) return;
    const request: Pending = { kind: 'delete', names: selection.map((entry) => entry.name) };
    if (prefs.value.files.confirm_delete) {
      cancelPending();
      pending = request;
    } else {
      void confirmPending(undefined, request);
    }
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
      if (pending === action) pending = null;
      await refresh();
    } catch (error) {
      if (pending === action) pending = null;
      fail(error);
    }
  }

  function onListKeydown(event: KeyboardEvent): void {
    if (pending || event.target instanceof HTMLButtonElement) return;
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
    if (!visible || !root) return false;
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
    unlistenSaves = editors.onSaved((draft) => {
      if (draft.label === label) refreshAfterSave(draft.path);
    });
    if (!isMobile) unlistenEdits = externalEdits.listen(onExternalEdit);
    transfers.start().catch((error: unknown) => console.warn('transfer events unavailable', error));
    if (!isMobile) {
      try {
        const unlisten = await getCurrentWebview().onDragDropEvent(onDragDrop);
        if (destroyed) {
          unlisten();
          return;
        }
        unlistenDrop = unlisten;
      } catch (error) {
        console.info('drag and drop unavailable', error);
      }
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
      if (destroyed) return;
      loading = false;
      fail(error);
      onerror?.(error);
    }
  });

  $effect(() => {
    if (!viewport.coarse && active && !pending) list?.focus({ preventScroll: true });
  });

  $effect(() => {
    if (!touchUi) {
      if (selectMode) exitSelectMode();
      cancelLongPress();
      clearClickSuppression();
    }
  });

  onDestroy(() => {
    destroyed = true;
    clearTimeout(refreshTimer);
    cancelLongPress();
    clearClickSuppression();
    if (pending?.kind === 'conflict') discardStaged(pending.stagedPaths);
    unsubscribe?.();
    unlistenEdits?.();
    unlistenSaves?.();
    unlistenDrop?.();
    unbindExternal?.();
    if (fsId) sftpClose(fsId).catch((error: unknown) => console.warn('sftp close failed', error));
  });
</script>

<div class="browser" class:dropping class:touch={touchUi} class:compact={compactList} bind:this={root}>
  <div class="toolbar">
    <IconButton icon="up" label={t('files.up')} small disabled={!fsId || parentPath(cwd) === null} onclick={goUp} />
    <span class:spinning={loading && entries.length > 0}>
      <IconButton icon="refresh" label={t('files.refresh')} small disabled={!fsId} onclick={() => void refresh()} />
    </span>
    <form class="path" onsubmit={submitPath}>
      <input class="bare" bind:value={pathInput} spellcheck="false" disabled={!fsId} aria-label={t('files.path')} />
    </form>
    <IconButton icon="folderAdd" label={t('files.newFolder')} small disabled={!fsId} onclick={askMkdir} />
    <IconButton icon="upload" label={t('files.upload')} small disabled={!fsId} onclick={() => void pickUpload(false)} />
    {#if !isMobile}
      <IconButton icon="folderUpload" label={t('files.uploadFolder')} small disabled={!fsId} onclick={() => void pickUpload(true)} />
    {/if}
    {#if !selectMode}
      <IconButton icon="download" label={t('files.download')} small disabled={selection.length === 0} onclick={() => void download(selection.map((entry) => entry.name), false)} />
    {/if}
    {#if !isMobile}
      <IconButton icon="folderDownload" label={t('files.downloadTo')} small disabled={selection.length === 0} onclick={() => void download(selection.map((entry) => entry.name), true)} />
    {/if}
    <IconButton icon="fileCode" label={t('files.editBuiltin')} small disabled={!editable || openingEditor} onclick={() => void editFile()} />
    {#if !isMobile}
      <IconButton icon="externalEdit" label={t('files.editExternal')} small disabled={!editable} onclick={() => void editFile(true)} />
      <IconButton icon="externalEdits" label={t('external.title', { n: externalEdits.all.length })} small active={externalEdits.all.length > 0} onclick={() => (externalEdits.dialog = true)} />
    {/if}
    <IconButton icon="drafts" label={t('editor.drafts')} small active={editors.dirty.length > 0} onclick={() => (editors.showDrafts = true)} />
    <IconButton icon="edit" label={t('files.rename')} small disabled={selection.length !== 1} onclick={askRename} />
    {#if !selectMode}
      <IconButton icon="remove" label={t('common.delete')} small danger disabled={selection.length === 0} onclick={askDelete} />
    {/if}
    <IconButton icon={showHidden ? 'eye' : 'eyeOff'} label={t('files.showHidden')} small active={showHidden} onclick={toggleHidden} />
    {#if touchUi}
      <IconButton icon="listChecks" label={t('files.selectMode')} small active={selectMode} disabled={!fsId} onclick={toggleSelectMode} />
    {/if}
    {#if onclose}
      <span class="toolbar-gap"></span>
      <IconButton icon="close" label={t('common.close')} small onclick={onclose} />
    {/if}
  </div>

  {#if selectMode}
    <div class="select-bar">
      <span role="status">{t('files.selectedCount', { count: selection.length })}</span>
      <button type="button" class="plain" disabled={shownNames.length === 0} onclick={() => (selected = toggleSelectAll(shownNames, selected))}>
        {allSelected(shownNames, selected) ? t('files.clearSelection') : t('files.selectAll')}
      </button>
      <button type="button" class="plain" disabled={selection.length === 0} onclick={() => void download(selection.map((entry) => entry.name), false)}>
        {t('files.download')}
      </button>
      <button type="button" class="plain danger" disabled={selection.length === 0} onclick={askDelete}>{t('common.delete')}</button>
      <button type="button" class="plain" onclick={exitSelectMode}>{t('files.exitSelect')}</button>
    </div>
  {/if}

  {#if compactList}
    {#if loading}
      <div class="loading-bar compact-progress" aria-hidden="true"></div>
    {/if}
  {:else}
    <div class="header row" class:with-more={touchUi && !selectMode}>
      <span class="name">{t('files.name')}</span>
      <span class="size">{t('files.size')}</span>
      <span class="time">{t('files.modified')}</span>
      <span class="mode">{t('files.mode')}</span>
      {#if loading && entries.length > 0}
        <div class="loading-bar" aria-hidden="true"></div>
      {/if}
    </div>
  {/if}
  <div class="list" class:loading tabindex="0" role="listbox" aria-multiselectable="true" aria-busy={loading} aria-label={label} bind:this={list} onkeydown={onListKeydown} oncontextmenu={listMenu} onclick={clickBlank} onscroll={cancelLongPress}>
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
        class:with-more={touchUi && !selectMode}
        role="option"
        aria-selected={selected.has(entry.name)}
        tabindex="-1"
        data-index={index}
        onclick={(event) => clickRow(event, index)}
        ondblclick={() => { if (!touchUi) openEntry(entry); }}
        oncontextmenu={(event) => onRowContextMenu(event, entry, index)}
        onpointerdown={(event) => startLongPress(event, entry, index)}
        onpointermove={moveLongPress}
        onpointerup={endLongPress}
        onpointercancel={endLongPress}
        onkeydown={() => {}}
      >
        <span class="name">
          {#if selectMode}
            <span class="selection-check" class:checked={selected.has(entry.name)} aria-hidden="true">
              {#if selected.has(entry.name)}<Icon name="check" size={12} />{/if}
            </span>
          {/if}
          <span class="glyph" style:--tint={fileTint(entry)}><Icon name={entry.kind === 'dir' ? 'folder' : entry.kind === 'symlink' ? 'link' : 'file'} size={15} /></span>
          {#if compactList}
            <span class="compact-content">
              <span class="label">{entry.name}</span>
              <span class="compact-meta">{formatCompactMeta(entry)}</span>
            </span>
          {:else}
            <span class="label">{entry.name}</span>
          {/if}
        </span>
        {#if !compactList}
          <span class="size">{entry.kind === 'dir' ? '' : formatSize(entry.size)}</span>
          <span class="time">{formatTime(entry.modified)}</span>
          <span class="mode">{formatMode(entry.permissions)}</span>
        {/if}
        {#if touchUi && !selectMode}
          <span class="row-more">
            <IconButton icon="more" small label={t('files.moreActions')} onclick={(event) => { event.stopPropagation(); openRowMenuAtButton(event, entry, index); }} />
          </span>
        {/if}
      </div>
    {/each}
  </div>

  <div class="status" class:error={!loading && status?.error} role="status">
    {#if loading && fsId}{t('files.loadingDir', { path: loadingPath ?? cwd })}{:else if status}{status.text}{:else if fsId}{t('files.summary', { count: shown.length, selected: selection.length })}{/if}
  </div>

  {#if dropping}
    <div class="drop-hint"><Icon name="upload" size={20} />{t('files.dropHint', { path: cwd })}</div>
  {/if}
</div>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}

{#if pending?.kind === 'mkdir' || pending?.kind === 'rename'}
  <Dialog title={pending.kind === 'mkdir' ? t('files.newFolder') : t('files.rename')} onclose={cancelPending} width={380} autofocus>
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
  <Dialog title={t('files.conflictTitle')} onclose={cancelPending} width={440}>
    <p class="message">{t('files.conflictBody', { names: pending.clashes.slice(0, 5).join(', ') })}</p>
    {#snippet actions()}
      <button type="button" class="secondary" onclick={cancelPending}>{t('common.cancel')}</button>
      <button type="button" class="secondary" onclick={() => confirmUpload('skip')}>{t('files.skip')}</button>
      <button type="button" class="secondary" onclick={() => confirmUpload('rename')}>{t('files.keepBoth')}</button>
      <button type="button" class="primary" onclick={() => confirmUpload('overwrite')}>{t('files.overwrite')}</button>
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
  .select-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    flex: none;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-3);
    background: var(--material-bar);
    box-shadow: inset 0 -1px 0 var(--separator);
    font-size: var(--text-footnote);
  }
  .select-bar > span {
    margin-inline-end: auto;
    color: var(--fg-muted);
    white-space: nowrap;
  }
  .select-bar button {
    min-height: var(--touch-target, 44px);
  }
  .select-bar .danger {
    color: var(--danger);
  }
  .select-bar .danger:hover:not(:disabled) {
    background: var(--danger-soft);
  }
  .spinning :global(svg) {
    animation: refresh-spin calc(4 * var(--dur-slow)) linear infinite;
  }
  @keyframes refresh-spin {
    to { transform: rotate(1turn); }
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
  .row.with-more {
    grid-template-columns: minmax(120px, 1fr) 80px 136px 88px var(--touch-target, 44px);
  }
  .header {
    position: relative;
    height: 26px;
    margin: 0;
    padding: 0 calc(6px + var(--space-2));
    border-radius: 0;
    color: var(--fg-muted);
    font-size: var(--text-caption);
    font-weight: 600;
    box-shadow: inset 0 -1px 0 var(--separator);
  }
  .loading-bar {
    position: absolute;
    inset: 100% 0 auto;
    z-index: 1;
    height: calc(var(--space-1) / 2);
    overflow: hidden;
    pointer-events: none;
    background: var(--accent-soft);
  }
  .compact-progress {
    position: static;
    flex: none;
  }
  .loading-bar::before {
    content: '';
    display: block;
    width: 35%;
    height: 100%;
    background: var(--accent);
    animation: list-progress calc(5 * var(--dur-slow)) ease-in-out infinite;
  }
  @keyframes list-progress {
    from { transform: translateX(-100%); }
    to { transform: translateX(286%); }
  }
  @media (prefers-reduced-motion: reduce) {
    .loading-bar::before,
    .spinning :global(svg) {
      animation: none;
    }
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
    transition: opacity var(--dur) var(--ease);
  }
  .touch .entry {
    height: auto;
    min-height: var(--touch-target, 44px);
    -webkit-touch-callout: none;
  }
  .compact .entry {
    display: flex;
    height: auto;
    min-height: 52px;
    padding: var(--space-2) var(--space-3);
  }
  .list.loading .entry {
    opacity: 0.55;
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
  .touch .entry.selected,
  .compact .entry.selected {
    background: var(--accent-soft);
  }
  .browser:not(.touch, .compact) .list:focus .entry.selected {
    background: var(--accent);
  }
  .browser:not(.touch, .compact) .list:focus .entry.selected :is(.name, .size, .time, .mode, .glyph) {
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
  .compact .glyph {
    flex: none;
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .compact .name {
    flex: 1;
  }
  .compact-content {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  .compact-meta {
    overflow: hidden;
    color: var(--fg-muted);
    font-size: var(--text-caption);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row-more {
    display: inline-flex;
    flex: none;
  }
  .row-more :global(button) {
    min-width: var(--touch-target, 44px);
    min-height: var(--touch-target, 44px);
  }
  .selection-check {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border: 1px solid var(--fg-muted);
    border-radius: var(--radius-xs);
  }
  .selection-check.checked {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--accent-fg);
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
