/// <reference types="svelte" />
/// <reference types="vite/client" />

// Types `.svelte` imports from plain `.ts` modules; imports between components are typed by svelte-check.
declare module '*.svelte' {
  import type { Component } from 'svelte';
  const component: Component<Record<string, unknown>>;
  export default component;
}
