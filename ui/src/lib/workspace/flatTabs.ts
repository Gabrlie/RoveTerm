export interface FlatTab {
  groupId: string;
  paneId: string;
}

export function flattenTabs(groupIds: readonly string[], groups: Record<string, { tabs: readonly string[] }>): FlatTab[] {
  return groupIds.flatMap((groupId) => groups[groupId]?.tabs?.map((paneId) => ({ groupId, paneId })) ?? []);
}

/** Cycles through Home followed by the tabs in reading order. */
export function cycleFlatTarget(flatTabs: readonly FlatTab[], activePaneId: string | null, offset: number): FlatTab | null {
  const index = flatTabs.findIndex(({ paneId }) => paneId === activePaneId) + 1;
  const count = flatTabs.length + 1;
  const next = ((index + offset) % count + count) % count;
  return flatTabAt(flatTabs, next - 1);
}

export function flatTabAt(flatTabs: readonly FlatTab[], index: number): FlatTab | null {
  return flatTabs[index] ?? null;
}
