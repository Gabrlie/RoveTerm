// Arguments are a vector, never a shell command. Blank rows are omitted; spaces within a row
// remain literal. macOS application bundles are opened by the system and accept no argv here.
export function editorArgs(value: string): string[] {
  return value.split(/\r?\n/).filter((line) => line.trim() !== '');
}
export function editorProgram(program: string, args: string[]): { program: string; args: string[] } {
  const path = program.trim();
  return { program: path, args: /\.app\/?$/i.test(path) ? [] : args };
}
