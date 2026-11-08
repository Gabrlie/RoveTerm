#!/usr/bin/env node
// Checks Markdown word budgets (CJK characters + Latin words) and local relative links.
import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const skipDirs = new Set(['node_modules', 'target', '.git', 'dist', 'gen']);

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    if (skipDirs.has(name)) continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path, out);
    else if (name.endsWith('.md')) out.push(path);
  }
  return out;
}

function countWords(text) {
  const prose = text.replace(/```[\s\S]*?```/g, '');
  const cjk = prose.match(/[㐀-鿿豈-﫿]/g)?.length ?? 0;
  const latin = prose.replace(/[㐀-鿿豈-﫿]/g, ' ').match(/[A-Za-z0-9_]+/g)?.length ?? 0;
  return cjk + latin;
}

const errors = [];
const budgets = JSON.parse(readFileSync(join(root, 'scripts/doc-budgets.json'), 'utf8'));
for (const [file, limit] of Object.entries(budgets)) {
  const path = join(root, file);
  if (!existsSync(path)) {
    errors.push(`${file}: listed in doc-budgets.json but missing`);
    continue;
  }
  const words = countWords(readFileSync(path, 'utf8'));
  if (words > limit) errors.push(`${file}: ${words} words exceeds budget ${limit}`);
}

const linkPattern = /\[[^\]]*\]\(([^)\s]+)\)/g;
for (const file of walk(root)) {
  const text = readFileSync(file, 'utf8').replace(/```[\s\S]*?```/g, '');
  for (const match of text.matchAll(linkPattern)) {
    const target = match[1];
    if (/^[a-z]+:/i.test(target) || target.startsWith('#')) continue;
    const path = resolve(dirname(file), decodeURIComponent(target.split('#')[0]));
    if (!existsSync(path)) errors.push(`${relative(root, file)}: broken link ${target}`);
  }
}

if (errors.length > 0) {
  for (const error of errors) console.error(error);
  process.exit(1);
}
console.log('docs ok');
