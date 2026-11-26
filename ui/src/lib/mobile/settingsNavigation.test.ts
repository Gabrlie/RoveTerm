import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  emptyMobileSettingsNav,
  openMobileSettings,
  selectMobileSettingsSection,
  stepBackMobileSettings,
} from './settingsNavigation.ts';

describe('mobile settingsNavigation', () => {
  it('returns straight to the caller pane when opened from Fleet Lead or an active panel', () => {
    const entered = openMobileSettings('ai', { tab: 'hosts', paneId: 'pane-fleet', rightPage: null });
    assert.equal(entered.section, 'ai');

    const back = stepBackMobileSettings(entered);
    assert.equal(back.kind, 'restore');
    if (back.kind === 'restore') {
      assert.deepEqual(back.target, { tab: 'hosts', paneId: 'pane-fleet', rightPage: null });
    }
    assert.deepEqual(back.state, emptyMobileSettingsNav());
  });

  it('restores an active right panel opened over a terminal pane', () => {
    const entered = openMobileSettings('ai', { tab: 'hosts', paneId: 'pane-ssh', rightPage: 'ai' });
    const back = stepBackMobileSettings(entered);
    assert.equal(back.kind, 'restore');
    if (back.kind === 'restore') {
      assert.deepEqual(back.target, { tab: 'hosts', paneId: 'pane-ssh', rightPage: 'ai' });
    }
  });

  it('unwinds nested settings navigation (About -> Logs) before returning to root or caller', () => {
    const root = emptyMobileSettingsNav();
    const about = selectMobileSettingsSection(root, 'about');
    const logs = selectMobileSettingsSection(about, 'logs', true);

    assert.equal(logs.section, 'logs');
    assert.deepEqual(logs.history, ['about']);

    const backToAbout = stepBackMobileSettings(logs);
    assert.equal(backToAbout.kind, 'section');
    assert.equal(backToAbout.state.section, 'about');

    const backToRoot = stepBackMobileSettings(backToAbout.state);
    assert.equal(backToRoot.kind, 'root');
    assert.equal(backToRoot.state.section, null);
  });

  it('preserves caller returnTarget across nested pushes and clears it on direct root picks', () => {
    const entered = openMobileSettings('about', { tab: 'hosts', paneId: 'pane-1', rightPage: null });
    const logs = selectMobileSettingsSection(entered, 'logs', true);

    const firstBack = stepBackMobileSettings(logs);
    assert.equal(firstBack.kind, 'section');
    assert.equal(firstBack.state.section, 'about');

    const secondBack = stepBackMobileSettings(firstBack.state);
    assert.equal(secondBack.kind, 'restore');
    if (secondBack.kind === 'restore') {
      assert.equal(secondBack.target.paneId, 'pane-1');
    }

    const jumped = selectMobileSettingsSection(entered, 'terminal', false);
    assert.equal(jumped.returnTarget, null);
    assert.equal(stepBackMobileSettings(jumped).kind, 'root');
  });
});
