<script lang="ts">
  import { editors, type CloseChoice } from './editors.svelte';
  import TextEditor from './TextEditor.svelte';
  import Dialog from '../ui/Dialog.svelte';
  import Icon from '../ui/Icon.svelte';
  import { t } from '../i18n';
  import { workspace } from '../workspace/workspace.svelte';

  const selected = $derived(editors.selected ? editors.find(editors.selected) : undefined);
  function answer(choice: CloseChoice): void { editors.closing?.resolve(choice); }
  function openTab(): void {
    if (!selected) return;
    editors.showDrafts = false;
    workspace.openEditor(selected);
  }
</script>

{#if editors.showDrafts}
  <Dialog title={t('editor.drafts')} width={1080} onclose={() => (editors.showDrafts = false)}>
    <div class="drafts">
      <nav aria-label={t('editor.drafts')}>
        {#if !editors.drafts.length}<p class="empty">{t('editor.noDrafts')}</p>{/if}
        {#each editors.drafts as draft (draft.id)}
          <button class:active={selected?.id === draft.id} onclick={() => (editors.selected = draft.id)}>
            <Icon name="fileCode" size={15} /><span><strong>{draft.path.split('/').at(-1)}</strong><small>{draft.label}</small></span>
            {#if draft.text !== draft.original}<span class="dirty" aria-label={t('editor.unsaved')}>●</span>{/if}
          </button>
        {/each}
      </nav>
      <div class="editor">
        {#if selected}
          {#key selected.id}<TextEditor draft={selected} onclose={() => (editors.selected = null)} />{/key}
        {:else}<p class="empty">{t('editor.selectDraft')}</p>{/if}
      </div>
    </div>
    <p class="note">{t('editor.memoryNote')}</p>
    {#snippet actions()}
      <button type="button" class="secondary" onclick={() => (editors.showDrafts = false)}>{t('common.close')}</button>
      {#if selected}
        <button type="button" class="primary" onclick={openTab}>{t('editor.openTab')}</button>
      {/if}
    {/snippet}
  </Dialog>
{/if}

{#if editors.closing}
  <Dialog title={t('editor.closeTitle')} width={480} onclose={() => answer('cancel')}>
    <p class="message">{t('editor.closeConfirm')}</p>
    <ul class="paths">{#each editors.closing.ids as id}{@const draft = editors.find(id)}{#if draft}<li>{draft.path}</li>{/if}{/each}</ul>
    {#snippet actions()}
      <button class="secondary" onclick={() => answer('cancel')}>{t('common.cancel')}</button>
      <button class="secondary danger" onclick={() => answer('discard')}>{t('editor.discard')}</button>
      <button class="primary" onclick={() => answer('save')}>{t('editor.saveAndClose')}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .drafts { display: flex; gap: var(--space-3); height: min(64vh, 640px); min-height: 240px; }
  nav { width: 180px; flex: none; overflow-y: auto; }
  nav button { display: flex; width: 100%; align-items: center; gap: var(--space-2); border: 0; border-radius: var(--radius-sm); background: transparent; text-align: left; padding: var(--space-2); color: var(--fg); }
  nav button.active { background: var(--accent-soft); color: var(--accent); }
  nav button > span { min-width: 0; }
  strong, small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  strong { font-size: var(--text-footnote); font-weight: 500; }
  small { color: var(--fg-muted); font-size: var(--text-caption); }
  .dirty { color: var(--accent); font-size: 8px; margin-left: auto; }
  .editor { flex: 1; min-width: 0; min-height: 0; border: 1px solid var(--separator); border-radius: var(--radius-sm); overflow: hidden; }
  @media (max-width: 600px) {
    .drafts { flex-direction: column; }
    nav { width: auto; max-height: 100px; }
  }
  .empty { text-align: center; padding: var(--space-4); color: var(--fg-muted); font-size: var(--text-footnote); }
  .note { margin: 0; color: var(--fg-muted); font-size: var(--text-caption); }
  .message { margin: 0; }
  .paths { margin: 0; padding-left: var(--space-4); font-family: var(--font-mono); overflow-wrap: anywhere; }
</style>
