<script lang="ts">
  // Lists the actions table; Enter runs the highlighted action.
  import { actions, bindingsOf, displayBinding, type ActionId } from '../actions';
  import { t } from '../i18n';

  let { onrun, onclose }: { onrun: (id: ActionId) => void; onclose: () => void } = $props();

  let query = $state('');
  let selected = $state(0);
  let input: HTMLInputElement;

  const matches = $derived(
    actions
      .filter((action) => !action.hidden)
      .filter((action) => t(action.label).toLowerCase().includes(query.trim().toLowerCase())),
  );

  $effect(() => {
    input.focus();
  });

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      onclose();
    } else if (event.key === 'ArrowDown') {
      selected = Math.min(selected + 1, matches.length - 1);
      event.preventDefault();
    } else if (event.key === 'ArrowUp') {
      selected = Math.max(selected - 1, 0);
      event.preventDefault();
    } else if (event.key === 'Enter') {
      const action = matches[selected];
      if (action) {
        onclose();
        onrun(action.id);
      }
    }
  }
</script>

<div class="backdrop" role="presentation" onpointerdown={onclose}>
  <div class="palette" role="dialog" tabindex="-1" aria-label={t('action.palette.open')} onpointerdown={(event) => event.stopPropagation()}>
    <input
      bind:this={input}
      bind:value={query}
      oninput={() => (selected = 0)}
      onkeydown={onKeydown}
      placeholder={t('palette.placeholder')}
      spellcheck="false"
    />
    <ul>
      {#each matches as action, index (action.id)}
        <li class:selected={index === selected}>
          <button type="button" onclick={() => { onclose(); onrun(action.id); }} onpointerenter={() => (selected = index)}>
            <span>{t(action.label)}</span>
            <kbd>{bindingsOf(action).map(displayBinding)[0] ?? ''}</kbd>
          </button>
        </li>
      {:else}
        <li class="empty">{t('palette.empty')}</li>
      {/each}
    </ul>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: flex;
    justify-content: center;
    padding-top: 12vh;
    background: color-mix(in srgb, var(--bg-sunken) 40%, transparent);
  }
  .palette {
    width: min(520px, 90vw);
    max-height: 60vh;
    display: flex;
    flex-direction: column;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: 0 12px 32px rgb(0 0 0 / 0.18);
    overflow: hidden;
  }
  input {
    padding: var(--space-3) var(--space-4);
    border: none;
    border-bottom: 1px solid var(--border);
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: var(--text-title);
    outline: none;
  }
  ul {
    margin: 0;
    padding: var(--space-1);
    list-style: none;
    overflow-y: auto;
  }
  li button {
    width: 100%;
    display: flex;
    justify-content: space-between;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  li.selected button {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  kbd {
    color: var(--fg-muted);
    font-family: var(--font-ui);
    font-size: var(--text-footnote);
  }
  .empty {
    padding: var(--space-3);
    color: var(--fg-muted);
  }
</style>
