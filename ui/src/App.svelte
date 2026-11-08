<script lang="ts">
  import TermView from './lib/terminal/TermView.svelte';
  import { sessionOpenLocal } from './lib/ipc';
</script>

<main class="shell">
  <TermView open={sessionOpenLocal} />
</main>

<style>
  .shell {
    height: 100vh;
    display: flex;
  }
</style>
