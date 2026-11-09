<script lang="ts">
  // Lists the actions table; Enter runs the highlighted action.
  import { actions, bindingsOf, displayBinding, type ActionId } from '../actions';
  import { t } from '../i18n';
  import Icon from './Icon.svelte';

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
    <div class="field">
    <Icon name="search" size={18} />
    <input class="bare"
      bind:this={input}
      bind:value={query}
      oninput={() => (selected = 0)}
      onkeydown={onKeydown}
      placeholder={t('palette.placeholder')}
      spellcheck="false"
    />
    </div>
    <ul>
      {#each matches as action, index (action.id)}
        <li class:selected={index === selected}>
          <button type="button" onclick={() => { onclose(); onrun(action.id); }} onpointerenter={() => (selected = index)}>
            <span>{t(action.label)}</span>
            {#if bindingsOf(action)[0]}<kbd>{displayBinding(bindingsOf(action)[0] ?? '')}</kbd>{/if}
          </button>
        </li>
      {:else}
        <li class="empty">{t('palette.empty')}</li>
      {/each}
    </ul>
  </div>
</div>

<style>
  /* Spotlight-like: a large field over a glass list. */
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: flex;
    justify-content: center;
    padding-top: 14vh;
    background: color-mix(in srgb, var(--overlay) 50%, transparent);
    animation: fade var(--dur) var(--ease-out);
  }
  .palette {
    width: min(600px, 90vw);
    max-height: 62vh;
    align-self: flex-start;
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-lg);
    background: var(--material-popover);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: var(--glass-edge), var(--shadow-lg);
    overflow: hidden;
    animation: drop var(--dur-slow) var(--ease);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes drop {
    from {
      opacity: 0;
      transform: translateY(-8px) scale(0.98);
    }
  }
  .field {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    color: var(--fg-muted);
    box-shadow: inset 0 -1px 0 var(--separator);
  }
  input {
    flex: 1;
    min-width: 0;
    color: var(--fg);
    font-size: var(--text-large);
    font-weight: 300;
    outline: none;
  }
  ul {
    margin: 0;
    padding: 6px;
    list-style: none;
    overflow-y: auto;
  }
  li button {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    height: 34px;
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: left;
    cursor: default;
  }
  li.selected button {
    background: var(--accent);
    color: var(--accent-fg);
  }
  kbd {
    padding: 1px 6px;
    border-radius: var(--radius-xs);
    background: var(--fill);
    color: var(--fg-muted);
    font-family: var(--font-ui);
    font-size: var(--text-caption);
    font-weight: 500;
  }
  li.selected kbd {
    background: rgb(255 255 255 / 0.2);
    color: var(--accent-fg);
  }
  .empty {
    padding: var(--space-4);
    color: var(--fg-muted);
    text-align: center;
  }
</style>
