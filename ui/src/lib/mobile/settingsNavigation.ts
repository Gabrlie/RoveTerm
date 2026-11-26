import type { MobileDestination } from '../layout/MobileNavBar.svelte';
import type { PaneId } from '../layout/layout';
import type { RightPage } from '../panels/RightPanel.svelte';
import type { SettingsSection } from '../settings/sections';

/** Workspace context saved when compact settings are opened from outside the settings tab. */
export interface MobileSettingsReturnTarget {
  tab: MobileDestination;
  paneId: PaneId | null;
  rightPage: RightPage | null;
}

export interface MobileSettingsNavState {
  section: SettingsSection | null;
  history: SettingsSection[];
  returnTarget: MobileSettingsReturnTarget | null;
}

export type MobileSettingsBackOutcome =
  | { kind: 'section'; state: MobileSettingsNavState }
  | { kind: 'restore'; state: MobileSettingsNavState; target: MobileSettingsReturnTarget }
  | { kind: 'root'; state: MobileSettingsNavState };

export function emptyMobileSettingsNav(): MobileSettingsNavState {
  return { section: null, history: [], returnTarget: null };
}

/** Opens a compact settings section, optionally recording where Back should return. */
export function openMobileSettings(
  section: SettingsSection | null,
  returnTarget: MobileSettingsReturnTarget | null,
): MobileSettingsNavState {
  return {
    section,
    history: [],
    returnTarget: section ? returnTarget : null,
  };
}

/** Navigates within compact settings, pushing the previous section for nested links like About -> Logs. */
export function selectMobileSettingsSection(
  state: MobileSettingsNavState,
  next: SettingsSection | null,
  pushCurrent = false,
): MobileSettingsNavState {
  if (next === null) {
    return emptyMobileSettingsNav();
  }
  if (next === state.section) return state;
  const history = pushCurrent && state.section ? [...state.history, state.section] : [];
  const returnTarget = pushCurrent ? state.returnTarget : null;
  return { section: next, history, returnTarget };
}

/** Resolves a Back step from the current compact settings detail page. */
export function stepBackMobileSettings(state: MobileSettingsNavState): MobileSettingsBackOutcome {
  if (state.history.length > 0) {
    const previous = state.history[state.history.length - 1]!;
    return {
      kind: 'section',
      state: {
        section: previous,
        history: state.history.slice(0, -1),
        returnTarget: state.returnTarget,
      },
    };
  }
  if (state.returnTarget) {
    return {
      kind: 'restore',
      state: emptyMobileSettingsNav(),
      target: state.returnTarget,
    };
  }
  return {
    kind: 'root',
    state: emptyMobileSettingsNav(),
  };
}
