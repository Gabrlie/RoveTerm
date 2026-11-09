<script lang="ts">
  import { icons, type IconName } from './icons';

  let { name, size = 16 }: { name: IconName; size?: number } = $props();
</script>

<span class="icon" style:width="{size}px" style:height="{size}px" aria-hidden="true">{@html icons[name]}</span>

<style>
  .icon {
    display: inline-flex;
    flex: none;
  }
  /* Lucide icons are strokes; a slightly lighter line suits the interface text. */
  .icon :global(svg) {
    width: 100%;
    height: 100%;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.75;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
