// Real HTTP smoke test for the local web server.
//
// The Rust unit tests build the router in-process. This script starts the
// actual `hesabdari-web` executable, waits for the loopback listener, and then
// drives the request guard, command API, one-time downloads, and restore
// upload over real TCP connections. It uses an isolated data folder so the
// developer's accounting database is never opened.

import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, readdir, rm, stat } from 'node:fs/promises';
import http from 'node:http';
import net from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';
import process from 'node:process';

const DEFAULT_EXE = 'src-tauri/target/debug/hesabdari-web.exe';
const SQLITE_HEADER = 'SQLite format 3\u0000';
const ZIP_HEADER = 'PK\u0003\u0004';
const STARTUP_TIMEOUT_MS = 30000;
const POLL_INTERVAL_MS = 200;

function readArg(name, fallback) {
  const index = process.argv.indexOf(name);
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback;
}

function delay(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

function expect(condition, message) {
  if (!condition) throw new Error(message);
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

async function freePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  const port = typeof address === 'object' && address ? address.port : 0;
  await new Promise((resolve) => {
    server.close(resolve);
  });
  return port;
}

function request(port, { method = 'GET', path: requestPath, headers = {}, body }) {
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        host: '127.0.0.1',
        port,
        method,
        path: requestPath,
        headers,
      },
      (res) => {
        const chunks = [];
        res.on('data', (chunk) => chunks.push(chunk));
        res.on('end', () => {
          resolve({
            status: res.statusCode ?? 0,
            headers: res.headers,
            body: Buffer.concat(chunks),
          });
        });
      },
    );
    req.setTimeout(15000, () => {
      req.destroy(new Error(`request timed out: ${method} ${requestPath}`));
    });
    req.on('error', reject);
    if (body) req.write(body);
    req.end();
  });
}

function parseJson(response, context) {
  const text = response.body.toString('utf8');
  try {
    return JSON.parse(text);
  } catch {
    throw new Error(
      `${context}: expected JSON (status ${response.status}), got ${JSON.stringify(text.slice(0, 200))}`,
    );
  }
}

async function countFiles(directory) {
  try {
    return (await readdir(directory)).length;
  } catch {
    return 0;
  }
}

async function main() {
  const exe = path.resolve(readArg('--exe', DEFAULT_EXE));
  const uiDir = path.resolve(readArg('--ui', 'ui'));
  await stat(exe).catch(() => {
    throw new Error(
      `Local web server not found at ${exe}. Build it with "cargo build --manifest-path src-tauri/Cargo.toml --features web --bin hesabdari-web".`,
    );
  });

  const root = await mkdtemp(path.join(tmpdir(), 'hesabdari-web-smoke-'));
  const dataDir = path.join(root, 'data');
  await mkdir(dataDir, { recursive: true });

  const port = await freePort();
  const host = `127.0.0.1:${port}`;
  const origin = `http://${host}`;
  const baseHeaders = { Host: host };

  const child = spawn(exe, [], {
    env: {
      ...process.env,
      HESABDARI_DATA_DIR: dataDir,
      HESABDARI_UI_DIR: uiDir,
      HESABDARI_WEB_PORT: String(port),
      HESABDARI_NO_OPEN: '1',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  });
  let output = '';
  child.stdout.on('data', (chunk) => {
    output += chunk.toString();
  });
  child.stderr.on('data', (chunk) => {
    output += chunk.toString();
  });

  try {
    let healthResponse = null;
    const deadline = Date.now() + STARTUP_TIMEOUT_MS;
    while (Date.now() < deadline) {
      if (child.exitCode !== null) {
        throw new Error(`server exited before listening (code ${child.exitCode}).\n${output}`);
      }
      try {
        healthResponse = await request(port, {
          path: '/api/health',
          headers: baseHeaders,
        });
        if (healthResponse.status === 200) break;
      } catch {
        // The listener is not ready yet; keep polling until the deadline.
      }
      await delay(POLL_INTERVAL_MS);
    }
    expect(healthResponse?.status === 200, `server did not answer /api/health.\n${output}`);
    console.log('[OK] loopback server answered /api/health');

    for (const name of [
      'content-security-policy',
      'x-content-type-options',
      'x-frame-options',
      'referrer-policy',
      'permissions-policy',
    ]) {
      expect(
        typeof healthResponse.headers[name] === 'string',
        `missing security header: ${name}`,
      );
    }
    console.log('[OK] security headers are present');

    const health = parseJson(healthResponse, 'health');
    expect(health.ok === true, `health.ok must be true: ${JSON.stringify(health)}`);
    expect(
      health.mode === 'local-web',
      `health.mode must be local-web: ${JSON.stringify(health)}`,
    );
    expect(
      !Object.prototype.hasOwnProperty.call(health, 'databasePath'),
      'health must not expose the database path',
    );
    console.log('[OK] health payload identifies the local web server only');

    const foreignHost = await request(port, {
      path: '/api/health',
      headers: { Host: 'attacker.example' },
    });
    expect(foreignHost.status === 403, `foreign Host must be rejected (got ${foreignHost.status})`);

    const foreignOrigin = await request(port, {
      method: 'POST',
      path: '/api/command',
      headers: {
        ...baseHeaders,
        Origin: 'http://attacker.example',
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ name: 'get_settings', args: {}, window: 'main' }),
    });
    expect(
      foreignOrigin.status === 403,
      `foreign Origin must be rejected (got ${foreignOrigin.status})`,
    );

    const missingOrigin = await request(port, {
      method: 'POST',
      path: '/api/command',
      headers: { ...baseHeaders, 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'get_settings', args: {}, window: 'main' }),
    });
    expect(
      missingOrigin.status === 403,
      `a state-changing request without Origin must be rejected (got ${missingOrigin.status})`,
    );

    const crossSite = await request(port, {
      path: '/api/health',
      headers: { ...baseHeaders, 'Sec-Fetch-Site': 'cross-site' },
    });
    expect(crossSite.status === 403, `cross-site fetch must be rejected (got ${crossSite.status})`);

    const sameOrigin = await request(port, {
      path: '/api/health',
      headers: { ...baseHeaders, 'Sec-Fetch-Site': 'same-origin' },
    });
    expect(sameOrigin.status === 200, `same-origin fetch must pass (got ${sameOrigin.status})`);
    console.log('[OK] Host, Origin, and Sec-Fetch-Site guards behave as documented');

    async function command(name, args = {}, windowName = 'main') {
      return request(port, {
        method: 'POST',
        path: '/api/command',
        headers: {
          ...baseHeaders,
          Origin: origin,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ name, args, window: windowName }),
      });
    }

    const settingsResponse = await command('get_settings');
    expect(settingsResponse.status === 200, `get_settings failed (${settingsResponse.status})`);
    const settings = parseJson(settingsResponse, 'get_settings');
    expect(settings.ok === true, `get_settings returned an error: ${JSON.stringify(settings)}`);
    console.log('[OK] browser command API reads the shared database');

    const backupResponse = await command('backup_database');
    expect(backupResponse.status === 200, `backup_database failed (${backupResponse.status})`);
    const backup = parseJson(backupResponse, 'backup_database');
    expect(backup.ok === true, `backup_database returned an error: ${JSON.stringify(backup)}`);
    const backupLink = backup.data?.webDownload;
    expect(
      typeof backupLink === 'string' && backupLink.startsWith('/api/backup/'),
      `backup download link missing: ${JSON.stringify(backup)}`,
    );

    const backupFile = await request(port, { path: backupLink, headers: baseHeaders });
    expect(backupFile.status === 200, `backup download failed (${backupFile.status})`);
    expect(
      backupFile.body.subarray(0, SQLITE_HEADER.length).toString('latin1') === SQLITE_HEADER,
      'downloaded backup is not a SQLite database',
    );
    await stat(backup.data.path);
    console.log('[OK] backup downloads as a SQLite file and stays in the backups folder');

    const backupAgain = await request(port, { path: backupLink, headers: baseHeaders });
    expect(backupAgain.status === 404, `backup link must be one-time (got ${backupAgain.status})`);
    console.log('[OK] backup download link is rejected after the first request');

    const exportsDir = path.join(dataDir, 'exports');
    const excelResponse = await command('export_account_excel');
    expect(excelResponse.status === 200, `export_account_excel failed (${excelResponse.status})`);
    const excel = parseJson(excelResponse, 'export_account_excel');
    expect(excel.ok === true, `export_account_excel returned an error: ${JSON.stringify(excel)}`);
    const excelLink = excel.data?.webDownload;
    expect(
      typeof excelLink === 'string' && excelLink.startsWith('/api/excel/'),
      `excel download link missing: ${JSON.stringify(excel)}`,
    );
    expect(
      (await countFiles(exportsDir)) === 1,
      'the Excel export should be written to the exports folder',
    );

    const excelFile = await request(port, { path: excelLink, headers: baseHeaders });
    expect(excelFile.status === 200, `excel download failed (${excelFile.status})`);
    expect(
      excelFile.body.subarray(0, ZIP_HEADER.length).toString('latin1') === ZIP_HEADER,
      'downloaded Excel export is not a workbook',
    );
    expect(
      (await countFiles(exportsDir)) === 0,
      'the Excel export file must be deleted after the download',
    );
    const excelAgain = await request(port, { path: excelLink, headers: baseHeaders });
    expect(excelAgain.status === 404, `excel link must be one-time (got ${excelAgain.status})`);
    console.log('[OK] Excel export downloads once and its file is removed');

    const uploadResponse = await request(port, {
      method: 'POST',
      path: '/api/restore/upload',
      headers: {
        ...baseHeaders,
        Origin: origin,
        'Content-Type': 'application/octet-stream',
        'X-File-Name': encodeURIComponent('smoke-backup.sqlite3'),
        'Content-Length': String(backupFile.body.length),
      },
      body: backupFile.body,
    });
    expect(uploadResponse.status === 200, `restore upload failed (${uploadResponse.status})`);
    const upload = parseJson(uploadResponse, 'restore upload');
    expect(upload.ok === true, `restore upload returned an error: ${JSON.stringify(upload)}`);
    expect(upload.data?.encrypted === false, 'a plain backup must be detected as unencrypted');
    expect(
      typeof upload.data?.token === 'string' && upload.data.token.length > 0,
      'restore upload did not return a token',
    );

    const restoreResponse = await command('restore_database_file', {
      token: upload.data.token,
    });
    expect(restoreResponse.status === 200, `restore failed (${restoreResponse.status})`);
    const restore = parseJson(restoreResponse, 'restore_database_file');
    expect(restore.ok === true, `restore returned an error: ${JSON.stringify(restore)}`);
    expect(
      typeof restore.data === 'string' && restore.data.length > 0,
      'restore did not report a safety backup path',
    );
    const restoreUploads = await readdir(path.join(dataDir, 'restore-uploads')).catch(() => []);
    expect(
      !restoreUploads.some((name) => name.endsWith('.upload')),
      `the uploaded restore file must be removed after the restore (found: ${restoreUploads.join(', ')})`,
    );
    if (restoreUploads.length > 0) {
      console.log(`[OK] restore left only SQLite side files: ${restoreUploads.join(', ')}`);
    }

    const afterRestore = await command('get_settings');
    expect(afterRestore.status === 200, `get_settings failed after restore (${afterRestore.status})`);
    const after = parseJson(afterRestore, 'get_settings after restore');
    expect(after.ok === true, `database unusable after restore: ${JSON.stringify(after)}`);
    console.log('[OK] backup upload and restore complete through the browser API');

    expect(child.exitCode === null, `server exited during the smoke test.\n${output}`);
    console.log(`[OK] local web smoke test passed (${exe})`);
  } catch (error) {
    throw new Error(
      `${error instanceof Error ? error.message : String(error)}\nServer output:\n${output}`,
      { cause: error },
    );
  } finally {
    await stopProcess(child);
    await removeWithRetries(root);
  }
}

main().catch((error) => {
  console.error(`[FAIL] ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
