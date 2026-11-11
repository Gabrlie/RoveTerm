<script lang="ts">
  // The model button of the input card (a brain) and its popover, after Codex: switches such as
  // fast mode at the top, the current model (which opens the list of models), and a slider for
  // the reasoning effort. The list of models hides the slider until a model is picked.
  import Icon from '../ui/Icon.svelte';
  import { t } from '../i18n';
  import type { ConfigOption } from '../ipc';
  import { effortIndex } from './composer';

  interface Props {
    model: ConfigOption | null;
    effort: ConfigOption | null;
    toggles: ConfigOption[];
    /** The value in effect for `option` (changes the user made before the agent started too). */
    value: (option: ConfigOption) => string;
    /** A choice's name as shown. */
    label: (option: ConfigOption, choice: { value: string; name: string }) => string;
    onset: (option: string, value: string) => void;
    /** Runs before the popover opens (an agent that has not started starts, to list its models). */
    onopen: () => Promise<boolean>;
    busy: boolean;
  }

  let { model, effort, toggles, value, label, onset, onopen, busy }: Props = $props();
  let open = $state(false);
  let view = $state<'main' | 'models'>('main');
  let root = $state<HTMLDivElement>();
  /** The slider's position while it is dragged. */
  let dragging = $state<number | null>(null);

  const modelName = $derived.by(() => {
    if (!model) return '';
    const current = value(model);
    return label(model, model.choices.find((choice) => choice.value === current) ?? { value: current, name: current });
  });
  const effortAt = $derived(effort ? (dragging ?? effortIndex(effort, value(effort))) : 0);
  const effortChoice = $derived(effort?.choices[effortAt] ?? null);

  async function toggle(): Promise<void> {
    if (open) {
      open = false;
      return;
    }
    if (!(await onopen())) return;
    view = 'main';
    open = true;
  }

  function pickModel(choice: string): void {
    if (model) onset(model.id, choice);
    view = 'main';
  }

  function commitEffort(): void {
    if (!effort || dragging === null) return;
    const choice = effort.choices[dragging];
    dragging = null;
    if (choice && choice.value !== value(effort)) onset(effort.id, choice.value);
  }

  function onWindowPointer(event: PointerEvent): void {
    if (open && root && !root.contains(event.target as Node)) open = false;
  }

  function onKey(event: KeyboardEvent): void {
    if (event.key !== 'Escape' || !open) return;
    event.stopPropagation();
    if (view === 'models') view = 'main';
    else open = false;
  }
</script>

<svelte:window onpointerdown={onWindowPointer} />

<div class="holder" bind:this={root} onkeydown={onKey} role="presentation">
  <button type="button" class="icon-button" class:active={open} onclick={() => void toggle()} disabled={busy} aria-expanded={open} title={busy ? t('ai.loadingModels') : modelName ? `${t('ai.model')}: ${modelName}` : t('ai.modelSettings')} aria-label={t('ai.modelSettings')}>
    <Icon name="brain" size={14} />
  </button>
  {#if open}
    <div class="popover" role="dialog" aria-label={t('ai.modelSettings')}>
      {#if view === 'main'}
        {#if !model && !effort && toggles.length === 0}
          <p class="empty">{t('ai.noModelSettings')}</p>
        {/if}
        {#if toggles.length > 0}
          <div class="toggles">
            {#each toggles as option (option.id)}
              {@const on = value(option) === 'true'}
              <button type="button" class="chip" class:on role="switch" aria-checked={on} title={option.name} onclick={() => onset(option.id, on ? 'false' : 'true')}>
                <Icon name="quickConnect" size={11} />{option.name}
              </button>
            {/each}
          </div>
        {/if}
        {#if model}
          <button type="button" class="model" onclick={() => (view = 'models')} disabled={model.choices.length === 0} title={t('ai.chooseModel')}>
            <span class="model-name">{modelName}</span>
            {#if model.choices.length > 0}<Icon name="chevron" size={12} />{/if}
          </button>
        {/if}
        {#if effort && effort.choices.length > 1}
          <div class="effort">
            <div class="effort-head">
              <span>{t('ai.effort')}</span>
              <span class="effort-value">{effortChoice ? label(effort, effortChoice) : ''}</span>
            </div>
            <input
              type="range"
              min="0"
              max={effort.choices.length - 1}
              step="1"
              value={effortAt}
              aria-label={t('ai.effort')}
              aria-valuetext={effortChoice ? label(effort, effortChoice) : ''}
              oninput={(event) => (dragging = Number(event.currentTarget.value))}
              onchange={commitEffort}
            />
            <div class="ends">
              <span>{label(effort, effort.choices[0]!)}</span>
              <span>{label(effort, effort.choices[effort.choices.length - 1]!)}</span>
            </div>
          </div>
        {/if}
      {:else if model}
        <div class="list-head">
          <button type="button" class="back" onclick={() => (view = 'main')} aria-label={t('ai.back')} title={t('ai.back')}><Icon name="back" size={13} /></button>
          <span>{t('ai.model')}</span>
        </div>
        <div class="models" role="listbox" aria-label={t('ai.model')}>
          {#each model.choices as choice (choice.value)}
            {@const current = choice.value === value(model)}
            <button type="button" role="option" aria-selected={current} class="choice" class:current onclick={() => pickModel(choice.value)}>
              <span class="choice-name">{label(model, choice)}{#if current}<Icon name="check" size={12} />{/if}</span>
              {#if choice.description}<span class="choice-detail">{choice.description}</span>{/if}
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  /* The popover spans the input card, which is the positioned ancestor. */
  .holder {
    flex: none;
  }
  .icon-button {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    cursor: pointer;
  }
  .icon-button:hover:not(:disabled),
  .icon-button.active {
    background: var(--fill-hover);
    color: var(--fg);
  }
  .icon-button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .popover {
    position: absolute;
    left: 0;
    right: 0;
    bottom: calc(100% + 6px);
    z-index: 20;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-height: 340px;
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--material-popover);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: var(--glass-edge), var(--shadow-md);
    font-size: var(--text-footnote);
  }
  .empty {
    margin: var(--space-1);
    color: var(--fg-muted);
  }
  .toggles {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 8px;
    border: none;
    border-radius: var(--radius-pill);
    background: var(--fill);
    color: var(--fg-muted);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }
  .chip.on {
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }
  .model {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    min-height: 30px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg);
    font: inherit;
    font-weight: 600;
    text-align: left;
    cursor: pointer;
  }
  .model:hover:not(:disabled) {
    background: var(--fill-hover);
  }
  .model:disabled {
    cursor: default;
  }
  .model-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .effort {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 0 var(--space-1);
  }
  .effort-head,
  .ends {
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .effort-head {
    color: var(--fg-muted);
  }
  .effort-value {
    color: var(--fg);
    font-weight: 600;
  }
  .ends {
    color: var(--fg-subtle);
    font-size: 10.5px;
  }
  input[type='range'] {
    width: 100%;
    margin: 0;
    accent-color: var(--accent);
    cursor: pointer;
  }
  .list-head {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--fg-muted);
    font-size: 11px;
    font-weight: 600;
  }
  .back {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    cursor: pointer;
  }
  .back:hover {
    background: var(--fill-hover);
    color: var(--fg);
  }
  .models {
    min-height: 0;
    overflow-y: auto;
  }
  .choice {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    width: 100%;
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .choice:hover {
    background: var(--accent);
    color: var(--accent-fg);
  }
  .choice-name {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .choice.current .choice-name {
    font-weight: 600;
  }
  .choice-detail {
    opacity: 0.7;
    font-size: 11px;
  }
</style>
