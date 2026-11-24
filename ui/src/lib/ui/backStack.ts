const stack: { handler: () => void }[] = [];

export function pushBackHandler(handler: () => void): () => void {
  const entry = { handler };
  stack.push(entry);
  return () => {
    const index = stack.indexOf(entry);
    if (index !== -1) stack.splice(index, 1);
  };
}

export function handleBack(): boolean {
  const entry = stack.pop();
  if (!entry) return false;
  entry.handler();
  return true;
}

export function clearBackStack(): void {
  stack.length = 0;
}
