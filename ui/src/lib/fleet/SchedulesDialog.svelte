<script lang="ts">
  // The fleet lead's scheduled tasks: a list with each task's switch, last run and "run now",
  // and an editor for one task. Tasks are preferences (they follow the user); runs happen on
  // the device where the app is open.
  import { onMount } from 'svelte';
  import Dialog from '../ui/Dialog.svelte';
  import Select from '../ui/Select.svelte';
  import Switch from '../ui/Switch.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { errorText, t, type MessageKey } from '../i18n';
  import { scheduleClaim, scheduleDevice, scheduleRunNow, scheduleStatus, type Repeat, type Schedule, type ScheduleStatus } from '../ipc';
  import { prefs } from '../settings/prefs.svelte';
  import { editPrefs, settingsError } from '../settings/edit.svelte';
  import { notify } from '../ui/toast.svelte';
  import { ask } from '../ui/confirm.svelte';
  import { clock, parseClock, repeatText } from './schedule';
  import AutostartRow from '../settings/AutostartRow.svelte';

  let { onclose, onopenreport }: { onclose: () => void; onopenreport: (conversation: string) => void } = $props();

  const schedules = $derived(prefs.value.ai.schedules);
  let statuses = $state<Record<string, ScheduleStatus>>({});
  /** The task being edited (a copy), with its time as text; `null` when only the list shows. */
  let editing = $state<{ original: string | null; draft: Schedule; time: string; once: string } | null>(null);

  const KINDS: { value: Repeat['kind']; label: string }[] = (['daily', 'weekly', 'hourly', 'once'] as const).map((kind) => ({ value: kind, label: t(`schedule.kind.${kind}`) }));
  const WEEKDAYS = [0, 1, 2, 3, 4, 5, 6].map((day) => ({ value: String(day), label: t(`schedule.day.${day}` as MessageKey) }));

  let device = $state('');
  async function claimTask(id: string): Promise<void> {
    try { await scheduleClaim(id); await prefs.current(); await refresh(); }
    catch (error) { notify(errorText(error), 'error'); }
  }
  async function refresh(): Promise<void> {
    try {
      device = await scheduleDevice();
      statuses = Object.fromEntries((await scheduleStatus()).map((status) => [status.schedule, status]));
    } catch (error) {
      console.warn('scheduled task status unavailable', error);
    }
  }

  function describe(schedule: Schedule): string {
    const { key, values } = repeatText(schedule.repeat);
    const day = typeof values.day === 'number' ? t(`schedule.day.${values.day}` as MessageKey) : '';
    return t(`schedule.repeat.${key}`, { ...values, day });
  }

  function lastRun(schedule: Schedule): string {
    const status = statuses[schedule.id];
    if (status?.running) return t('schedule.running');
    if (!status) return t('schedule.never');
    return t(`schedule.status.${status.status}`, { time: new Date(status.due).toLocaleString([], { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' }) });
  }

  function local(ms: number): string {
    const date = new Date(ms - new Date(ms).getTimezoneOffset() * 60_000);
    return date.toISOString().slice(0, 16);
  }

  function startNew(): void {
    const now = Date.now();
    editing = {
      original: null,
      draft: { owner: null, id: '', name: '', instruction: '', repeat: { kind: 'daily', hour: 9, minute: 0 }, allow_changes: false, catch_up: true, enabled: true, created_at: now },
      time: '09:00',
      once: local(now + 3_600_000),
    };
  }

  function startEdit(schedule: Schedule): void {
    const repeat = schedule.repeat;
    editing = {
      original: schedule.id,
      draft: structuredClone($state.snapshot(schedule)) as Schedule,
      time: repeat.kind === 'daily' || repeat.kind === 'weekly' ? clock(repeat.hour, repeat.minute) : '09:00',
      once: local(repeat.kind === 'once' ? repeat.at : Date.now() + 3_600_000),
    };
  }

  /** The edited repeat from the form, or why it is not valid. */
  function formRepeat(): Repeat | string {
    if (!editing) return '';
    const kind = editing.draft.repeat.kind;
    if (kind === 'once') {
      const at = new Date(editing.once).getTime();
      return Number.isFinite(at) ? { kind, at } : t('schedule.badTime');
    }
    if (kind === 'hourly') {
      const minute = editing.draft.repeat.kind === 'hourly' ? editing.draft.repeat.minute : 0;
      return { kind, minute: Math.min(59, Math.max(0, Math.round(minute))) };
    }
    const time = parseClock(editing.time);
    if (!time) return t('schedule.badTime');
    const weekday = editing.draft.repeat.kind === 'weekly' ? editing.draft.repeat.weekday : 0;
    return kind === 'daily' ? { kind, ...time } : { kind, weekday, ...time };
  }

  function setKind(kind: Repeat['kind']): void {
    if (!editing) return;
    editing.draft.repeat =
      kind === 'once' ? { kind, at: 0 } : kind === 'hourly' ? { kind, minute: 0 } : kind === 'daily' ? { kind, hour: 9, minute: 0 } : { kind, weekday: 0, hour: 9, minute: 0 };
  }

  async function save(): Promise<void> {
    if (!editing) return;
    const repeat = formRepeat();
    if (typeof repeat === 'string') {
      notify(repeat, 'error');
      return;
    }
    const { original } = editing;
    const draft: Schedule = { ...$state.snapshot(editing.draft), repeat, name: editing.draft.name.trim(), instruction: editing.draft.instruction.trim() };
    // A changed time starts counting from now: earlier times are not caught up.
    if (!original || JSON.stringify(schedules.find((entry) => entry.id === original)?.repeat) !== JSON.stringify(repeat)) draft.created_at = Date.now();
    await editPrefs((prefsDraft) => {
      if (!original) draft.id = crypto.randomUUID();
      const at = prefsDraft.ai.schedules.findIndex((entry) => entry.id === original);
      if (at >= 0) prefsDraft.ai.schedules[at] = draft;
      else prefsDraft.ai.schedules.push(draft);
    });
    if (!settingsError.text) editing = null;
  }

  async function remove(schedule: Schedule): Promise<void> {
    const confirmed = await ask({ title: t('schedule.deleteTitle'), message: t('schedule.deleteMessage', { name: schedule.name }), confirmLabel: t('schedule.delete'), danger: true });
    if (!confirmed) return;
    await editPrefs((draft) => (draft.ai.schedules = draft.ai.schedules.filter((entry) => entry.id !== schedule.id)));
  }

  async function runNow(schedule: Schedule): Promise<void> {
    try {
      const started = await scheduleRunNow(schedule.id);
      notify(started ? t('schedule.started', { name: schedule.name }) : t('schedule.alreadyRunning'), 'info');
      await refresh();
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  onMount(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 5000);
    return () => clearInterval(timer);
  });
</script>

<Dialog title={t('schedule.title')} {onclose} width={720}>
  {#if editing}
    <div class="form-group">
      <div class="form-row">
        <span class="form-label">{t('schedule.name')}</span>
        <input class="field" bind:value={editing.draft.name} maxlength="60" aria-label={t('schedule.name')} />
      </div>
      <div class="form-row stacked">
        <span class="form-label">{t('schedule.instruction')}<small>{t('schedule.instructionHint')}</small></span>
        <textarea rows="4" bind:value={editing.draft.instruction} aria-label={t('schedule.instruction')}></textarea>
      </div>
      <div class="form-row">
        <span class="form-label">{t('schedule.when')}</span>
        <div class="when">
          <Select label={t('schedule.when')} value={editing.draft.repeat.kind} options={KINDS} onchange={setKind} />
          {#if editing.draft.repeat.kind === 'weekly'}
            <Select label={t('schedule.weekday')} value={String(editing.draft.repeat.weekday)} options={WEEKDAYS} onchange={(value) => editing && editing.draft.repeat.kind === 'weekly' && (editing.draft.repeat.weekday = Number(value))} />
          {/if}
          {#if editing.draft.repeat.kind === 'daily' || editing.draft.repeat.kind === 'weekly'}
            <input class="field time" type="time" bind:value={editing.time} aria-label={t('schedule.time')} />
          {:else if editing.draft.repeat.kind === 'hourly'}
            <input class="field time" type="number" min="0" max="59" bind:value={editing.draft.repeat.minute} aria-label={t('schedule.minute')} />
          {:else}
            <input class="field" type="datetime-local" bind:value={editing.once} aria-label={t('schedule.time')} />
          {/if}
        </div>
      </div>
      <div class="form-row">
        <span class="form-label">{t('schedule.allowChanges')}<small>{t('schedule.allowChangesHint')}</small></span>
        <Switch label={t('schedule.allowChanges')} checked={editing.draft.allow_changes} onchange={(checked) => editing && (editing.draft.allow_changes = checked)} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('schedule.catchUp')}<small>{t('schedule.catchUpHint')}</small></span>
        <Switch label={t('schedule.catchUp')} checked={editing.draft.catch_up} onchange={(checked) => editing && (editing.draft.catch_up = checked)} />
      </div>
    </div>
    {#if settingsError.text}<p class="form-note failed">{settingsError.text}</p>{/if}
  {:else}
    <p class="intro">{t('schedule.intro')}</p>
    <div class="form-group">
      {#each schedules as schedule (schedule.id)}
        {@const status = statuses[schedule.id]}
        <div class="form-row task">
          <span class="form-label">
            {schedule.name}
            <small>{schedule.owner === device ? t('schedule.thisDevice') : schedule.owner ?? t('schedule.unassigned')} · {describe(schedule)} · {schedule.allow_changes ? t('schedule.mayChange') : t('schedule.checkOnly')} · {lastRun(schedule)}</small>
          </span>
          {#if status?.conversation && status.status !== 'missed'}
            <button type="button" class="plain" onclick={() => status.conversation && onopenreport(status.conversation)}>{t('schedule.report')}</button>
          {/if}
          {#if !schedule.owner}<button type="button" class="plain" onclick={() => void claimTask(schedule.id)}>{t('schedule.claim')}</button>{/if}
          <IconButton icon="play" small label={t('schedule.runNow')} disabled={status?.running || schedule.owner !== device} onclick={() => void runNow(schedule)} />
          <IconButton icon="edit" small label={t('schedule.edit')} onclick={() => startEdit(schedule)} />
          <IconButton icon="remove" small label={t('schedule.delete')} onclick={() => void remove(schedule)} />
          <Switch label={t('schedule.enabled')} checked={schedule.enabled} onchange={(checked) => void editPrefs((draft) => { const entry = draft.ai.schedules.find((item) => item.id === schedule.id); if (entry) entry.enabled = checked; })} />
        </div>
      {:else}
        <p class="empty">{t('schedule.empty')}</p>
      {/each}
    </div>
    <div class="form-group">
      <AutostartRow />
    </div>
  {/if}
  {#snippet actions()}
    {#if editing}
      <button type="button" class="secondary" onclick={() => (editing = null)}>{t('common.cancel')}</button>
      <button type="button" class="primary" disabled={!editing.draft.name.trim() || !editing.draft.instruction.trim()} onclick={() => void save()}>{t('schedule.save')}</button>
    {:else}
      <button type="button" class="secondary add" onclick={startNew}>{t('schedule.new')}</button>
      <button type="button" class="primary" onclick={onclose}>{t('settings.done')}</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .intro {
    margin: 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .task .form-label small {
    white-space: normal;
  }
  .when {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }
  .time {
    width: 140px;
  }
  .stacked {
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-2);
  }
  .stacked textarea {
    width: 100%;
    box-sizing: border-box;
  }
  textarea {
    resize: vertical;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--separator);
    color: var(--fg);
    font: inherit;
  }
  .empty {
    margin: 0;
    padding: var(--space-4);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    text-align: center;
  }
  .add {
    margin-right: auto;
  }
</style>
