// Real Windows/Tauri/WebView2 startup smoke test.
//
// This is intentionally a process-level smoke test: it launches the actual
// Tauri executable, lets WebView2 create the main window, waits for the real
// SQLite database to be created in an isolated temporary data folder, checks
// the SQLite header and verifies the process is still running.
//
// The data folder is redirected with `HESABDARI_SMOKE_DATA_DIR`, which only
// debug builds honor. A release executable therefore fails this test instead of
// silently opening the user's real database. UI interaction remains covered by
// Playwright's mock E2E suite and the Rust integration tests.

import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import process from 'node:process';

const DEFAULT_EXE = 'src-tauri/target/debug/Hesabdari.exe';
const DB_FILE_NAME = 'hesabdari.sqlite3';
const SQLITE_HEADER = 'SQLite format 3\u0000';
const POLL_INTERVAL_MS = 500;
const WEBVIEW2_POLL_INTERVAL_MS = 1000;
// `db::initialize` mirrors its failure here because a GUI build may not have
// a usable console handle when it is launched by CI.
const STARTUP_LOG = path.join(tmpdir(), 'hesabdari-startup-error.log');

async function readStartupLog() {
  try {
    return (await readFile(STARTUP_LOG, 'utf8')).trim();
  } catch {
    return '';
  }
}

function startupLogSuffix(log) {
  return log ? `\nStartup log:\n${log}` : '';
}

function readArg(name, fallback) {
  const index = process.argv.indexOf(name);
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback;
}

function delay(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

async function stopProcess(child) {
  if (!child || child.exitCode !== null) return;
  if (process.platform !== 'win32') {
    child.kill('SIGTERM');
    return;
  }
  await new Promise((resolve) => {
    const killer = spawn('taskkill', ['/pid', String(child.pid), '/t', '/f'], {
      stdio: 'ignore',
      windowsHide: true,
    });
    killer.on('exit', resolve);
    killer.on('error', resolve);
  });
}

function runPowerShell(script) {
  return new Promise((resolve) => {
    const child = spawn(
      'powershell.exe',
      ['-NoProfile', '-NonInteractive', '-Command', script],
      { stdio: ['ignore', 'pipe', 'ignore'], windowsHide: true },
    );
    let output = '';
    child.stdout.on('data', (chunk) => {
      output += chunk.toString();
    });
    child.on('error', () => resolve(''));
    child.on('exit', () => resolve(output.trim()));
  });
}

// WebView2 starts `msedgewebview2.exe` as a descendant of the host process.
// Walking the parent chain proves the webview actually initialized instead of
// only proving that the Rust process started.
async function countWebView2Descendants(pid) {
  const script = `
$procs = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name
$byParent = @{}
foreach ($p in $procs) {
  $key = [int]$p.ParentProcessId
  if (-not $byParent.ContainsKey($key)) { $byParent[$key] = @() }
  $byParent[$key] += $p
}
$queue = New-Object System.Collections.Queue
$queue.Enqueue(${Number(pid)})
$count = 0
while ($queue.Count -gt 0) {
  $current = $queue.Dequeue()
  if ($byParent.ContainsKey($current)) {
    foreach ($child in $byParent[$current]) {
      if ($child.Name -eq 'msedgewebview2.exe') { $count++ }
      $queue.Enqueue([int]$child.ProcessId)
    }
  }
}
Write-Output $count
`;
  const output = await runPowerShell(script);
  return Number.parseInt(output, 10) || 0;
}

async function removeWithRetries(target) {
  for (let attempt = 0; attempt < 10; attempt += 1) {
    try {
      await rm(target, { recursive: true, force: true });
      return;
    } catch {
      await delay(300);
    }
  }
}

async function main() {
  const exe = path.resolve(readArg('--exe', DEFAULT_EXE));
  const timeoutMs = Number(readArg('--timeout', '60000'));
  await stat(exe).catch(() => {
    throw new Error(
      `Tauri executable not found at ${exe}. Build a debug executable with "cargo build --manifest-path src-tauri/Cargo.toml".`,
    );
  });

  const root = await mkdtemp(path.join(tmpdir(), 'hesabdari-smoke-'));
  const dataDir = path.join(root, 'data', 'ir.hesabdari.software');
  const appData = path.join(root, 'appdata');
  const localAppData = path.join(root, 'localappdata');
  const userProfile = path.join(root, 'profile');
  await Promise.all([
    mkdir(dataDir, { recursive: true }),
    mkdir(appData, { recursive: true }),
    mkdir(localAppData, { recursive: true }),
    mkdir(userProfile, { recursive: true }),
  ]);

  // Remove any log from an earlier run so a stale message cannot explain a
  // failure in this run.
  await rm(STARTUP_LOG, { force: true });

  const env = {
    ...process.env,
    HESABDARI_SMOKE_DATA_DIR: dataDir,
    APPDATA: appData,
    LOCALAPPDATA: localAppData,
    USERPROFILE: userProfile,
  };
  const child = spawn(exe, [], {
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: false,
  });
  let stderr = '';
  child.stdout.on('data', (chunk) => process.stdout.write(chunk));
  child.stderr.on('data', (chunk) => {
    stderr += chunk.toString();
    process.stderr.write(chunk);
  });

  const databasePath = path.join(dataDir, DB_FILE_NAME);
  const deadline = Date.now() + timeoutMs;
  let databaseReady = false;
  try {
    while (Date.now() < deadline) {
      if (child.exitCode !== null) {
        throw new Error(
          `Tauri app exited before creating the database (code ${child.exitCode}).\n${stderr}` +
            startupLogSuffix(await readStartupLog()),
        );
      }
      try {
        const info = await stat(databasePath);
        if (info.size > 0) {
          databaseReady = true;
          break;
        }
      } catch {
        // The app is still starting; keep polling until the deadline.
      }
      await delay(POLL_INTERVAL_MS);
    }
    if (!databaseReady) {
      throw new Error(
        `Tauri app did not create ${databasePath} within ${timeoutMs}ms.\n` +
          'The executable must be a debug build that honors HESABDARI_SMOKE_DATA_DIR.\n' +
          stderr +
          startupLogSuffix(await readStartupLog()),
      );
    }

    const bytes = await readFile(databasePath);
    const header = bytes.subarray(0, SQLITE_HEADER.length).toString('latin1');
    if (header !== SQLITE_HEADER) {
      throw new Error(`Unexpected SQLite header in ${databasePath}: ${JSON.stringify(header)}`);
    }

    // Give WebView2 a moment to spawn its helper processes.
    let webView2Count = 0;
    const webView2Deadline = Date.now() + timeoutMs;
    while (Date.now() < webView2Deadline) {
      if (child.exitCode !== null) {
        throw new Error(
          `Tauri app exited after database creation (code ${child.exitCode}).\n${stderr}` +
            startupLogSuffix(await readStartupLog()),
        );
      }
      webView2Count = await countWebView2Descendants(child.pid);
      if (webView2Count > 0) break;
      await delay(WEBVIEW2_POLL_INTERVAL_MS);
    }
    if (webView2Count === 0) {
      throw new Error(
        `No msedgewebview2.exe process appeared under the Tauri process (pid ${child.pid}); ` +
          'WebView2 did not initialize.',
      );
    }

    if (child.exitCode !== null) {
      throw new Error(
        `Tauri app exited after WebView2 started (code ${child.exitCode}).\n${stderr}` +
          startupLogSuffix(await readStartupLog()),
      );
    }

    console.log(`[OK] Real Tauri app started: ${exe}`);
    console.log(
      `[OK] WebView2 initialized (${webView2Count} helper process(es), pid ${child.pid}).`,
    );
    console.log(`[OK] SQLite database created and validated: ${databasePath}`);
  } finally {
    await stopProcess(child);
    await removeWithRetries(root);
  }
}

main().catch((error) => {
  console.error(`[FAIL] ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
