<script lang="ts">
  import type { Snippet } from 'svelte';
  import { t } from '../i18n';
  import IconButton from '../ui/IconButton.svelte';

  let { title, large = false, onback, actions }: {
    title: string;
    large?: boolean;
    onback?: () => void;
    actions?: Snippet;
  } = $props();
</script>

<header class:large>
  {#if onback}
    <IconButton icon="back" label={t('mobile.back')} onclick={onback} />
  {/if}
  <h1>{title}</h1>
  {#if actions}
    <div class="actions">{@render actions()}</div>
  {/if}
</header>

<style>
  header {
    position: sticky;
    top: 0;
    z-index: 1;
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--m-header-nav, 48px);
    padding: 0 var(--space-3);
    background: var(--material-bar);
    box-shadow: inset 0 -0.5px 0 var(--separator);
  }
  h1 {
    flex: 1;
    min-width: 0;
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--m-text-nav, 16px);
    font-weight: 600;
  }
  .large {
    min-height: var(--m-header-large, 52px);
    padding: 0 var(--m-page-x, 16px);
  }
  .large h1 {
    font-size: var(--m-text-page, 22px);
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .actions {
    display: flex;
    flex: none;
    gap: var(--space-1);
  }
</style>
