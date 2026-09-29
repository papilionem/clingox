// Runs wasm32-unknown-emscripten test executables in one headless browser.
//
//   node run.mjs --browser <chromium|firefox|webkit> --timeout <seconds>
//                --result <file> [--arg <test argument>]... <executable.js>...
//
// Each --arg is passed to every executable as a command-line argument, for
// example `--arg --skip --arg <test name>`.
//
// Test output goes to this process's stdout and stderr. The verdict goes to the
// result file as JSON, which `cargo xtask test wasm --browser` checks against
// the executables it passed in, so a runner that stops early or loses an
// executable cannot be read as a pass.

import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, firefox, webkit } from 'playwright';

const ENGINES = { chromium, firefox, webkit };
const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.wasm': 'application/wasm',
};
// Output that arrives after the first terminal event (a trailing "Aborted(...)"
// line, a late page error) still belongs to the executable, and a late error
// must still turn a pass into a failure.
const SETTLE_MS = 250;
// Closing a page whose main thread is stuck in wasm can itself stall.
const CLOSE_MS = 10_000;

const PAGE = path.join(path.dirname(fileURLToPath(import.meta.url)), 'index.html');

function parseArgs(argv) {
  const opts = { executables: [], args: [] };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--arg') {
      if (i + 1 >= argv.length) throw new Error('--arg needs a value');
      opts.args.push(argv[++i]);
    } else if (arg === '--browser' || arg === '--timeout' || arg === '--result') {
      if (i + 1 >= argv.length) throw new Error(`${arg} needs a value`);
      opts[arg.slice(2)] = argv[++i];
    } else if (arg.startsWith('--')) {
      throw new Error(`unknown option ${arg}`);
    } else {
      opts.executables.push(path.resolve(arg));
    }
  }
  if (!ENGINES[opts.browser]) throw new Error(`--browser must be one of ${Object.keys(ENGINES).join(', ')}`);
  opts.timeout = Number(opts.timeout ?? 120);
  if (!(opts.timeout > 0)) throw new Error('--timeout must be a positive number of seconds');
  if (!opts.result) throw new Error('--result is required');
  if (opts.executables.length === 0) throw new Error('no test executables given');
  return opts;
}

// Serves the page and, under /exe/<index>/, only the listed executables and the
// .wasm files next to them. Nothing else on disk is reachable.
function serve(executables) {
  const files = new Map([['/', PAGE], ['/index.html', PAGE]]);
  executables.forEach((js, i) => {
    const base = path.basename(js, '.js');
    files.set(`/exe/${i}/${base}.js`, js);
    files.set(`/exe/${i}/${base}.wasm`, path.join(path.dirname(js), `${base}.wasm`));
  });
  const server = http.createServer((req, res) => {
    const file = files.get(new URL(req.url, 'http://127.0.0.1').pathname);
    if (!file || !fs.existsSync(file)) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, {
      'Content-Type': MIME[path.extname(file)] ?? 'application/octet-stream',
      'Cache-Control': 'no-store',
    });
    fs.createReadStream(file).pipe(res);
  });
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => resolve(server));
  });
}

function withTimeout(promise, ms) {
  let timer;
  const timeout = new Promise((resolve) => { timer = setTimeout(resolve, ms); });
  return Promise.race([promise.catch(() => {}), timeout]).finally(() => clearTimeout(timer));
}

// Runs one executable in a fresh browser context and returns null on success or
// the reason it failed.
async function runOne(browser, origin, index, js, timeoutSecs, args) {
  // The page's own error listener and Playwright's pageerror event see the
  // same uncaught exception; it is printed and counted once.
  const problems = new Set();
  let exitStatus;
  let finish;
  const finished = new Promise((resolve) => { finish = resolve; });
  const terminal = (problem) => {
    if (problem && !problems.has(problem)) {
      problems.add(problem);
      process.stderr.write(`${problem}\n`);
    }
    setTimeout(finish, SETTLE_MS);
  };

  const context = await browser.newContext();
  try {
    const page = await context.newPage();
    await page.exposeFunction('xtaskReport', (kind, data) => {
      switch (kind) {
        case 'stdout': process.stdout.write(`${data}\n`); break;
        case 'stderr': process.stderr.write(`${data}\n`); break;
        case 'exit':
          exitStatus = data;
          terminal(data === '0' ? null : `exited with status ${data}`);
          break;
        case 'abort': terminal(data ? `aborted: ${data}` : 'aborted'); break;
        default: terminal(`uncaught exception: ${data}`);
      }
    });
    page.on('pageerror', (err) => terminal(`uncaught exception: ${err.name}: ${err.message}`));
    page.on('crash', () => terminal('the page crashed'));
    page.on('console', (msg) => process.stderr.write(`[console.${msg.type()}] ${msg.text()}\n`));

    const query = new URLSearchParams({ exe: `${index}/${path.basename(js)}` });
    for (const arg of args) query.append('arg', arg);
    const url = `${origin}/index.html?${query}`;
    // The executable starts during page load; waiting for the load event would
    // let a hanging test stall here instead of under the timer below.
    await page.goto(url, { waitUntil: 'commit', timeout: timeoutSecs * 1000 });
    let timer;
    const timedOut = new Promise((resolve) => {
      timer = setTimeout(() => resolve(true), timeoutSecs * 1000);
    });
    if (await Promise.race([finished.then(() => false), timedOut])) {
      problems.add(`did not finish within ${timeoutSecs} s`);
    }
    clearTimeout(timer);
  } catch (err) {
    problems.add(`runner error: ${err.message}`);
  } finally {
    await withTimeout(context.close(), CLOSE_MS);
  }

  if (problems.size === 0 && exitStatus !== '0') problems.add('reported no exit status');
  return problems.size ? [...problems].join('; ') : null;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const result = { engine: opts.browser, total: opts.executables.length, passed: [], failed: [] };
  const writeResult = () => fs.writeFileSync(opts.result, JSON.stringify(result, null, 2));

  let browser;
  try {
    browser = await ENGINES[opts.browser].launch({ headless: true });
  } catch (err) {
    result.launchError = err.message;
    writeResult();
    process.stderr.write(`\n${opts.browser}: cannot launch the browser, so no test ran.\n${err.message}\n`);
    if (opts.browser === 'webkit') {
      process.stderr.write('Playwright supports WebKit only on the Linux distributions it builds for; the message above names what is missing on this one.\n');
    }
    return 1;
  }

  const server = await serve(opts.executables);
  const origin = `http://127.0.0.1:${server.address().port}`;
  process.stderr.write(`${opts.browser} ${browser.version()}, serving on ${origin}\n`);
  try {
    for (const [i, js] of opts.executables.entries()) {
      process.stderr.write(`\n     Running ${js} in ${opts.browser}\n`);
      const problem = await runOne(browser, origin, i, js, opts.timeout, opts.args);
      if (problem) {
        process.stderr.write(`${opts.browser}: FAILED ${path.basename(js)}: ${problem}\n`);
        result.failed.push({ executable: js, reason: problem });
      } else {
        result.passed.push(js);
      }
    }
  } finally {
    server.close();
    await withTimeout(browser.close(), CLOSE_MS);
  }
  writeResult();
  return result.failed.length === 0 && result.passed.length === result.total ? 0 : 1;
}

main().then(
  (code) => process.exit(code),
  (err) => {
    process.stderr.write(`run.mjs: ${err.stack ?? err}\n`);
    process.exit(2);
  },
);
