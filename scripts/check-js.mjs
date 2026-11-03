#!/usr/bin/env node

import { readdir } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const uiRoot = path.join(projectRoot, 'ui');

async function collectJavaScriptFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];

  for (const entry of entries) {
    const fullPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...await collectJavaScriptFiles(fullPath));
    } else if (entry.isFile() && entry.name.endsWith('.js')) {
      files.push(fullPath);
    }
  }

  return files.sort((left, right) => left.localeCompare(right));
}

try {
  const files = await collectJavaScriptFiles(uiRoot);
  if (files.length === 0) {
    throw new Error(`No JavaScript files found under ${uiRoot}`);
  }

  const failures = [];
  for (const file of files) {
    const result = spawnSync(process.execPath, ['--check', file], {
      cwd: projectRoot,
      encoding: 'utf8',
      windowsHide: true,
    });

    if (result.error || result.status !== 0) {
      failures.push({ file, result });
      process.stderr.write(`\n[FAIL] ${path.relative(projectRoot, file)}\n`);
      if (result.error) process.stderr.write(`${result.error.message}\n`);
      if (result.stderr) process.stderr.write(result.stderr);
    } else {
      process.stdout.write(`[OK]   ${path.relative(projectRoot, file)}\n`);
    }
  }

  if (failures.length > 0) {
    process.stderr.write(`\n${failures.length} of ${files.length} JavaScript files failed syntax validation.\n`);
    process.exitCode = 1;
  } else {
    process.stdout.write(`\nAll ${files.length} JavaScript files passed syntax validation.\n`);
  }
} catch (error) {
  process.stderr.write(`${error.stack || error.message || String(error)}\n`);
  process.exitCode = 1;
}
