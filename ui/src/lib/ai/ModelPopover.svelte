<script lang="ts">
  import { tick } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import Switch from '../ui/Switch.svelte';
  import { t, type MessageKey } from '../i18n';
  import type { ConfigOption } from '../ipc';
  import { viewport } from '../layout/viewport.svelte';
  import { pushBackHandler } from '../ui/backStack';
  import { currentChoice, effortPill, fastIndicator, fastValue, modelGroups, optionKey, speedToggle } from './composer';

  interface Props {
    model: ConfigOption | null;
    effort: ConfigOption | null;
    toggles: ConfigOption[];
    speed: ConfigOption | null;
    title: (option: ConfigOption) => string;
    value: (option: ConfigOption) => string;
    label: (option: ConfigOption, choice: { value: string; name: string }) => string;
    onset: (option: string, value: string) => void;
    onopen: () => Promise<boolean>;
    starting: boolean;
    busy: boolean;
  }

  let { model, effort, toggles, speed, title, value, label, onset, onopen, starting, busy }: Props = $props();
  let open = $state(false);
  let opening = false;
  let view = $state<'main' | 'models'>('main');
  let query = $state('');
  let root = $state<HTMLDivElement>();
  let trigger = $state<HTMLButtonElement>();
  let selector = $state<HTMLButtonElement>();
  let back = $state<HTMLButtonElement>();
  let search = $state<HTMLInputElement>();

  $effect(() => {
    if (!open) return;
    const page = view;
    return pushBackHandler(() => {
      if (page === 'models') void showMain();
      else open = false;
    });
  });

  const chosenModel = $derived(currentChoice(model, value));
  const modelName = $derived(model && chosenModel ? label(model, chosenModel) : t('ai.model'));
  const chosenEffort = $derived(currentChoice(effort, value));
  const effortLabel = $derived(effort && chosenEffort ? effortPill(shortEffort(effort, chosenEffort)) : '');
  const fast = $derived(fastIndicator(speed, toggles, value));
  const groups = $derived(modelGroups(model?.choices ?? [], query));
  const triggerHint = $derived(
    starting
      ? t('ai.loadingModels')
      : [modelName, effortLabel, fast === 'on' ? t('ai.fastMode') : ''].filter(Boolean).join(' '),
  );

  const SHORT_EFFORT: Record<string, MessageKey> = {
    minimal: 'ai.sidebar.effort.minimal',
    medium: 'ai.sidebar.effort.medium',
  };

  function shortEffort(option: ConfigOption, choice: { value: string; name: string }): string {
    const key = SHORT_EFFORT[choice.value.toLowerCase()];
    return key ? t(key) : label(option, choice);
  }

  async function toggle(): Promise<void> {
    if (open) {
      open = false;
      return;
    }
    opening = true;
    const ready = await onopen();
    if (!opening) return;
    opening = false;
    if (!ready) return;
    view = 'main';
    open = true;
  }

  async function showModels(): Promise<void> {
    query = '';
    view = 'models';
    await tick();
    (viewport.coarse ? back : (search ?? back))?.focus();
  }

  async function showMain(): Promise<void> {
    view = 'main';
    await tick();
    selector?.focus();
  }

  function pickModel(choice: string): void {
    if (model && choice !== value(model)) onset(model.id, choice);
    void showMain();
  }

  function onWindowPointer(event: PointerEvent): void {
    if (root && !root.contains(event.target as Node)) {
      opening = false;
      open = false;
    }
  }

  function onKey(event: KeyboardEvent): void {
    if (event.key !== 'Escape' || event.defaultPrevented) return;
    opening = false;
    if (!open) return;
    event.preventDefault();
    if (view === 'models' && query) query = '';
    else if (view === 'models') void showMain();
    else {
      open = false;
      trigger?.focus();
    }
  }
</script>

<svelte:window onpointerdown={onWindowPointer} onkeydown={onKey} />

<div class="holder" bind:this={root}>
  <button type="button" class="trigger" class:starting bind:this={trigger} onclick={() => void toggle()} disabled={starting || busy} aria-expanded={open} aria-haspopup="dialog" title={triggerHint}>
    <span class="trigger-name">{modelName}</span>
    {#if effortLabel}<span class="effort-pill">{effortLabel}</span>{/if}
    {#if fast === 'on'}
      <span class="fast" title={t('ai.fastMode')}><Icon name="quickConnect" size={11} /></span>
    {/if}
    <span class="chevron"><Icon name="chevronDown" size={10} /></span>
  </button>
  {#if open}
    <div class="popover" role="dialog" aria-label={t('ai.sidebar.modelOptions')}>
      {#if view === 'main'}
        <p class="section-title">{t('ai.sidebar.modelOptions')}</p>
        {#if model}
          <button type="button" class="model-row selector" bind:this={selector} onclick={() => void showModels()} disabled={model.choices.length === 0 || starting || busy} title={t('ai.chooseModel')}>
            <span class="row-copy">
              <span class="model-name">{modelName}</span>
              {#if chosenModel?.group}<span class="detail">{chosenModel.group}</span>{/if}
            </span>
            {#if model.choices.length > 0}<Icon name="chevron" size={13} />{/if}
          </button>
        {/if}
        {#if effort && effort.choices.length > 0}
          {@const option = effort}
          <div class="effort">
            <p class="section-title">{title(option)}</p>
            <div class="segmented" role="group" aria-label={title(option)}>
              {#each option.choices as choice (choice.value)}
                {@const selected = choice.value === value(option)}
                <button type="button" class:current={selected} aria-pressed={selected} title={label(option, choice)} disabled={starting || busy} onclick={() => onset(option.id, choice.value)}>{shortEffort(option, choice)}</button>
              {/each}
            </div>
          </div>
        {/if}
        {#if speed || toggles.length > 0}
          <div class="toggles">
            {#if speed}
              {@const option = speed}
              <div class="toggle-row">
                <span><Icon name="quickConnect" size={13} />{t('ai.fastMode')}</span>
                <Switch checked={value(option) === fastValue(option)} disabled={starting || busy} label={t('ai.fastMode')} onchange={() => onset(option.id, speedToggle(option, value(option)))} />
              </div>
            {/if}
            {#each toggles as option (option.id)}
              <div class="toggle-row">
                <span><Icon name={optionKey(option) === 'ai.fastMode' ? 'quickConnect' : 'settings'} size={13} />{title(option)}</span>
                <Switch checked={value(option) === 'true'} disabled={starting || busy} label={title(option)} onchange={(checked) => onset(option.id, String(checked))} />
              </div>
            {/each}
          </div>
        {/if}
        {#if !model && !effort && !speed && toggles.length === 0}
          <p class="empty">{t('ai.noModelSettings')}</p>
        {/if}
      {:else if model}
        <div class="list-head">
          <button type="button" class="back" bind:this={back} onclick={() => void showMain()} aria-label={t('ai.back')} title={t('ai.back')}><Icon name="back" size={14} /></button>
          <span>{t('ai.model')}</span>
        </div>
        {#if model.choices.length > 5}
          <label class="search">
            <Icon name="search" size={12} />
            <input bind:this={search} bind:value={query} placeholder={t('ai.sidebar.searchModels')} aria-label={t('ai.sidebar.searchModels')} />
          </label>
        {/if}
        <div class="models">
          {#each groups as [group, choices] (group)}
            {#if group}<p class="section-title group-title">{group}</p>{/if}
            {#each choices as choice (choice.value)}
              {@const selected = choice.value === value(model)}
              <button type="button" class="model-row" class:current={selected} aria-pressed={selected} disabled={starting || busy} onclick={() => pickModel(choice.value)}>
                <span class="row-copy">
                  <span class="model-name">{label(model, choice)}</span>
                  {#if choice.description}<span class="detail">{choice.description}</span>{/if}
                </span>
                {#if selected}<span class="check"><Icon name="check" size={13} /></span>{/if}
              </button>
            {/each}
          {:else}
            <p class="empty">{t('ai.sidebar.noModelsFound')}</p>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .holder {
    flex: 0 1 auto;
    min-width: 0;
    max-width: 100%;
  }
  .trigger {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 100%;
    min-width: 0;
    height: 24px;
    padding: 0 6px;
    overflow: hidden;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    font-size: var(--text-footnote);
    font-weight: 500;
    cursor: pointer;
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }
  .trigger:not(:disabled):hover {
    background: var(--fill-hover);
    color: var(--fg);
  }
  .trigger[aria-expanded='true'] {
    background: var(--fill);
    color: var(--fg);
  }
  .trigger:disabled {
    opacity: 0.65;
    cursor: default;
  }
  .trigger.starting .trigger-name {
    animation: loading 1.3s ease-in-out infinite;
  }
  @keyframes loading {
    50% { opacity: 0.45; }
  }
  @media (prefers-reduced-motion: reduce) {
    .trigger.starting .trigger-name {
      animation: none;
    }
  }
  .trigger :global(svg),
  .effort-pill,
  .fast,
  .chevron,
  .check {
    flex: none;
  }
  .trigger-name {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg);
    font-weight: 500;
  }
  .effort-pill {
    color: var(--fg-subtle);
    font-size: var(--text-caption);
    font-weight: 400;
    white-space: nowrap;
  }
  .fast,
  .chevron,
  .check {
    display: inline-flex;
    align-items: center;
  }
  .fast {
    color: var(--accent);
  }
  .chevron {
    color: var(--fg-subtle);
    transition:
      transform var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }
  .trigger:not(:disabled):hover .chevron,
  .trigger[aria-expanded='true'] .chevron {
    color: var(--fg-muted);
  }
  .trigger[aria-expanded='true'] .chevron {
    transform: rotate(180deg);
  }
  .check {
    color: var(--accent);
  }
  @container card (max-width: 245px) {
    .trigger {
      padding: 0 5px;
    }
    .effort-pill {
      display: none;
    }
  }
  .popover {
    position: absolute;
    bottom: calc(100% + 8px);
    left: 0;
    z-index: 20;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    width: min(100%, 320px);
    max-height: min(360px, 55vh);
    overflow: auto;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-md);
    background: var(--material-popover);
    box-shadow: var(--glass-edge), var(--shadow-lg);
    font-size: var(--text-footnote);
    overscroll-behavior: contain;
    animation: pop var(--dur-fast) var(--ease-out);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
  .section-title,
  .empty {
    margin: 0;
    padding: 3px var(--space-2);
    color: var(--fg-muted);
    font-size: 11px;
  }
  .section-title {
    font-weight: 600;
  }
  .model-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    width: 100%;
    padding: 7px var(--space-2);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .model-row:hover,
  .model-row:focus-visible,
  .back:hover {
    background: var(--fill-hover);
  }
  .model-row:disabled {
    cursor: default;
  }
  .model-row.selector {
    background: var(--fill);
  }
  .model-row.selector:not(:disabled):hover {
    background: var(--fill-hover);
  }
  .model-row.current {
    background: var(--accent-soft);
  }
  .model-row.current .model-name,
  .model-row.selector .model-name {
    font-weight: 600;
  }
  .model-row :global(svg) {
    flex: none;
  }
  .row-copy {
    display: grid;
    gap: 2px;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .detail {
    color: var(--fg-muted);
    font-size: 11px;
  }
  .effort,
  .toggles {
    padding-top: var(--space-1);
  }
  .segmented {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
    margin: 2px var(--space-1) 0;
    padding: 3px;
    border-radius: var(--radius-sm);
    background: var(--fill);
  }
  .segmented button {
    flex: 1 1 auto;
    min-width: 0;
    max-width: 100%;
    padding: 4px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    font-size: 11px;
    overflow-wrap: anywhere;
    cursor: pointer;
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }
  .segmented button:hover {
    background: var(--fill-hover);
  }
  .segmented button.current {
    background: var(--surface-raised);
    box-shadow: var(--shadow-sm);
    color: var(--fg);
    font-weight: 600;
  }
  .toggle-row,
  .toggle-row > span,
  .list-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .toggle-row {
    justify-content: space-between;
    padding: var(--space-1) var(--space-2);
  }
  .toggle-row > span {
    overflow-wrap: anywhere;
  }
  .list-head {
    color: var(--fg);
    font-weight: 600;
  }
  .back {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    cursor: pointer;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--fg-muted);
  }
  .search:focus-within {
    border-color: var(--accent);
  }
  .search input {
    flex: 1;
    min-width: 0;
    margin: 0;
    padding: 6px 0;
    border: 0;
    background: transparent;
    outline: none;
    box-shadow: none;
    color: var(--fg);
    font: inherit;
  }
  .models {
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .group-title {
    padding-top: var(--space-2);
  }
  @media (pointer: coarse) {
    .trigger {
      min-height: 36px;
      height: auto;
    }
    .model-row, .toggle-row { min-height: var(--row-height); }
    .segmented button {
      min-height: 34px;
      padding: var(--space-1) var(--space-2);
    }
    .back {
      width: 36px;
      height: 36px;
    }
    .popover {
      max-width: calc(100vw - 2 * var(--space-3));
      max-height: min(360px, 52dvh);
    }
  }
</style>
