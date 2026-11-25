export type MasterDetailView = 'both' | 'list' | 'detail';

export function masterDetailView(compact: boolean, showingDetail: boolean): MasterDetailView {
  if (!compact) return 'both';
  return showingDetail ? 'detail' : 'list';
}

export function initialCompactDetail(hasExplicitTarget: boolean): boolean {
  return hasExplicitTarget;
}
