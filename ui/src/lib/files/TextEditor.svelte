<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { t } from '../i18n';
  import { editors, type EditDraft, type EditorCommand } from './editors.svelte';
  import type { EditorController } from './codemirror';

  let { draft, active = true, onclose }: { draft: EditDraft; active?: boolean; onclose: () => void } = $props();
  let host: HTMLDivElement;
  let controller = $state<EditorController | null>(null);
  let loadError = $state(false);
  const dirty = $derived(draft.text !== draft.original);
  const locked = $derived(draft.busy || editors.exiting || editors.closingIds.includes(draft.id));

  async function close(): Promise<void> {
    if (await editors.close(draft.id)) onclose();
  }
  function command(action: EditorCommand): void {
    if (locked && action !== 'editor.find') return;
    if (action === 'editor.save') void editors.save(draft);
    if (action === 'editor.close') void close();
    if (action === 'editor.find') controller?.find();
    if (action === 'editor.undo') controller?.undo();
    if (action === 'editor.redo') controller?.redo();
  }
  onMount(() => {
    let disposed = false;
    const unregister = editors.register(draft.id, command);
    void import('./codemirror').then(async ({ createEditor }) => {
      if (disposed) return;
      const created = await createEditor(host, draft.text, draft.path, {
        'Find': t('editor.find'), 'Replace': t('editor.replace'), 'next': t('editor.next'), 'previous': t('editor.previous'),
        'replace': t('editor.replace'), 'replace all': t('editor.replaceAll'), 'all': t('editor.all'),
        'match case': t('editor.matchCase'), 'regexp': t('editor.regexp'), 'by word': t('editor.byWord'),
        'close': t('common.close'), 'No matches found': t('editor.noMatches'),
      }, (text) => { draft.text = text; });
      if (disposed) created.destroy();
      else controller = created;
    }).catch(() => { loadError = true; });
    return () => { disposed = true; controller?.destroy(); unregister(); };
  });
  $effect(() => { controller?.sync(draft.text); });
  $effect(() => { controller?.setEditable(!locked); });
  $effect(() => { if (active) controller?.focus(); });
</script>

<section class="text-editor" onfocusin={() => editors.focus(draft.id, command)}>
  <header>
    <Icon name="fileCode" size={15} />
    <div class="identity"><strong title={draft.path}>{draft.path.split('/').at(-1)}</strong><span title={draft.label}>{draft.label}</span></div>
    {#if dirty}<span class="dot" title={t('editor.unsaved')} aria-label={t('editor.unsaved')}></span>{/if}
    <IconButton icon="undo" small label={t('editor.undo')} disabled={locked || !controller} onclick={() => controller?.undo()} />
    <IconButton icon="redo" small label={t('editor.redo')} disabled={locked || !controller} onclick={() => controller?.redo()} />
    <IconButton icon="search" small label={t('editor.find')} disabled={!controller} onclick={() => controller?.find()} />
    <button class="secondary save" disabled={locked || !dirty} onclick={() => void editors.save(draft)}>{t(draft.busy ? 'editor.saving' : 'common.save')}</button>
    <IconButton icon="refresh" small label={t('editor.reload')} disabled={locked} onclick={() => void editors.reload(draft)} />
    <IconButton icon="close" small label={t('editor.close')} disabled={locked} onclick={() => void close()} />
  </header>
  <div class="location" title={draft.path}>{draft.path}</div>
  {#if draft.conflict}
    <div class="notice conflict" role="alert"><Icon name="warning" size={16} /><span>{t('editor.conflict')}</span>
      <button class="plain" disabled={locked} onclick={() => void editors.reload(draft)}>{t('editor.reload')}</button>
      <button class="plain danger" disabled={locked} onclick={() => void editors.overwrite(draft)}>{t('editor.overwrite')}</button>
    </div>
  {/if}
  {#if draft.error}
    <div class="notice error" role="alert"><span class="selectable">{draft.error}</span><button class="plain" disabled={locked} onclick={() => void editors.reconnect(draft)}>{t('editor.reconnect')}</button></div>
  {/if}
  <div class="code" bind:this={host}></div>
  {#if loadError}<p class="notice error">{t('editor.loadError')}</p>{/if}
  <footer><span>{dirty ? t('editor.unsaved') : t('editor.saved')}</span><span>UTF-8{draft.text.startsWith('\uFEFF') ? ' · BOM' : ''} · {draft.text.includes('\r\n') ? 'CRLF' : draft.text.includes('\r') ? 'CR' : 'LF'}</span></footer>
</section>

<style>
  .text-editor { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--surface); color: var(--fg); }
  header { display: flex; align-items: center; flex-wrap: wrap; gap: var(--space-1); padding: var(--space-1) var(--space-2); min-height: 38px; background: var(--material-bar); box-shadow: inset 0 -1px 0 var(--separator); }
  .identity { display: flex; align-items: baseline; flex: 1; min-width: 0; gap: var(--space-2); }
  .identity strong { font-size: var(--text-body); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .identity span { color: var(--fg-muted); font-size: var(--text-caption); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dot { width: 7px; height: 7px; border-radius: var(--radius-pill); background: var(--accent); flex: none; }
  .location { padding: var(--space-1) var(--space-3); font: var(--text-caption) var(--font-mono); color: var(--fg-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; box-shadow: inset 0 -1px 0 var(--separator); }
  .code { flex: 1; min-height: 0; overflow: hidden; text-align: left; -webkit-user-select: text; user-select: text; }
  .notice { display: flex; align-items: center; flex-wrap: wrap; gap: var(--space-2); padding: var(--space-2) var(--space-3); font-size: var(--text-footnote); }
  .notice span { flex: 1; min-width: 100px; }
  .conflict { background: var(--warning-soft); color: var(--warning); }
  .error { background: var(--danger-soft); color: var(--danger); }
  footer { display: flex; justify-content: space-between; padding: var(--space-1) var(--space-3); font-size: var(--text-caption); color: var(--fg-muted); box-shadow: inset 0 1px 0 var(--separator); }
  .save { height: 26px; padding: 0 var(--space-2); font-size: var(--text-footnote); }
</style>
