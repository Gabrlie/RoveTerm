import { parseKeyBarLayout, serializeKeyBarLayout, type KeyBarLayout } from './keybar';

const STORAGE_KEY = 'ui.keybar.layout.v1';

function loadLayout(): KeyBarLayout {
  try {
    return parseKeyBarLayout(localStorage.getItem(STORAGE_KEY));
  } catch {
    return parseKeyBarLayout(null);
  }
}

class KeyBarPrefs {
  #layout = $state.raw<KeyBarLayout>(loadLayout());

  get layout(): KeyBarLayout {
    return this.#layout;
  }

  setExpanded(expanded: boolean): void {
    this.save({ ...this.#layout, expanded });
  }

  save(next: KeyBarLayout): void {
    this.#layout = parseKeyBarLayout(next);
    try {
      localStorage.setItem(STORAGE_KEY, serializeKeyBarLayout(this.#layout));
    } catch {
      // Keep the current layout usable when device storage is unavailable.
    }
  }

  reset(): void {
    this.save(parseKeyBarLayout(null));
  }
}

export const keybarPrefs = new KeyBarPrefs();
