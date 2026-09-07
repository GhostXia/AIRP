import test from 'node:test';
import assert from 'node:assert/strict';
import { createHook } from 'node:async_hooks';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { execFile, execFileSync } from 'node:child_process';
import { createHash, X509Certificate } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';
import tls from 'node:tls';
import net from 'node:net';
import vm from 'node:vm';
import { performance } from 'node:perf_hooks';
import { navigateInitialGet } from './production-initial-navigation.mjs';

const spki = 'A'.repeat(43) + '=';
const leafSha256 = Array(32).fill('AB').join(':');
const url = 'https://localhost:9443/screens/02-chat-space.html?session=secret-token';
const changed = () => new Error(`page.goto: net::ERR_CERT_VERIFIER_CHANGED at ${url}\nCall log:\n - navigating to "${url}"`);
const identity = { spki, leafSha256 };

function fixture(errors = []) {
  const events = [];
  let calls = 0;
  const response = { status: () => 200 };
  return {
    events, response, calls: () => calls,
    page: {
      async goto(target, options) {
        events.push('goto');
        assert.equal(target, url);
        assert.equal(options.waitUntil, 'commit');
        assert.ok(options.timeout > 0 && options.timeout <= 10_000);
        const error = errors[calls++];
        if (error) throw error;
        return response;
      },
      async waitForLoadState(state) { assert.equal(state, 'domcontentloaded'); events.push('loaded'); },
    },
    options: {
      probe: async target => {
        assert.equal(target.href, 'https://localhost:9443/');
        events.push('probe');
        return identity;
      },
      diagnose: event => events.push(event),
    },
  };
}

test('first GET succeeds without retry and retains response assertions', async () => {
  const f = fixture();
  assert.equal(await navigateInitialGet(f.page, url, spki, f.options), f.response);
  assert.equal(f.calls(), 1);
});

test('exact verifier event recovers once; rechecks identity and redacts URL/query', async () => {
  const f = fixture([changed()]);
  assert.equal(await navigateInitialGet(f.page, url, spki, f.options), f.response);
  assert.equal(f.calls(), 2);
  assert.deepEqual(f.events.filter(e => typeof e === 'string'), ['probe', 'goto', 'probe', 'goto', 'loaded']);
  assert.equal(f.events.find(e => e.attempt === 2 && e.spki)?.leafChanged, false);
  assert.ok(!JSON.stringify(f.events).includes('secret-token'));
  assert.ok(!JSON.stringify(f.events).includes('https:'));
});

test('repeated verifier event exhausts two attempts, without leaking raw error', async () => {
  const f = fixture([changed(), changed(), changed()]);
  await assert.rejects(navigateInitialGet(f.page, url, spki, f.options),
    /^Error: Initial navigation failed: ERR_CERT_VERIFIER_CHANGED \(attempt 2\)$/);
  assert.equal(f.calls(), 2);
  assert.ok(!f.events.includes('loaded'));
});

for (const code of ['ERR_CERT_AUTHORITY_INVALID', 'ERR_CERT_COMMON_NAME_INVALID', 'ERR_CERT_DATE_INVALID',
  'ERR_CERT_REVOKED', 'ERR_SSL_PINNED_KEY_NOT_IN_CERT_CHAIN', 'ERR_CONNECTION_RESET',
  'ERR_CERT_VERIFIER_CHANGED_SUFFIX']) {
  test(`${code} fails on first attempt`, async () => {
    const f = fixture([new Error(`page.goto: net::${code} at ${url}`)]);
    await assert.rejects(navigateInitialGet(f.page, url, spki, f.options), new RegExp(code));
    assert.equal(f.calls(), 1);
  });
}

test('transient text inside an unrelated error is not retry authority', async () => {
  const f = fixture([new Error('timeout navigating to ?error=net::ERR_CERT_VERIFIER_CHANGED')]);
  await assert.rejects(navigateInitialGet(f.page, url, spki, f.options), /NAVIGATION_FAILED/);
  assert.equal(f.calls(), 1);
});

test('SPKI mismatch on retry fails closed before second GET; leaf rotation is visible', async () => {
  const f = fixture([changed()]);
  let probes = 0;
  f.options.probe = async () => ++probes === 1 ? identity : {
    spki: 'B'.repeat(43) + '=', leafSha256: Array(32).fill('CD').join(':'),
  };
  await assert.rejects(navigateInitialGet(f.page, url, spki, f.options), /SPKI identity changed/);
  assert.equal(f.calls(), 1);
  assert.equal(f.events.find(e => e.attempt === 2)?.leafChanged, true);
});

test('certificate probe trust failure is never retried', async () => {
  const f = fixture();
  let probes = 0;
  f.options.probe = async () => { probes++; throw new Error('Gateway verification failed: ERR_TLS_CERT_ALTNAME_INVALID'); };
  await assert.rejects(navigateInitialGet(f.page, url, spki, f.options), /ERR_TLS_CERT_ALTNAME_INVALID/);
  assert.equal(probes, 1);
  assert.equal(f.calls(), 0);
});

for (const phase of ['probe', 'goto', 'load', 'retry-delay']) {
  test(`one deadline includes hanging ${phase}`, async () => {
    const f = fixture(phase === 'retry-delay' ? [changed()] : []);
    const hang = () => new Promise(() => {});
    if (phase === 'probe') f.options.probe = hang;
    if (phase === 'goto') f.page.goto = hang;
    if (phase === 'load') f.page.waitForLoadState = hang;
    const started = performance.now();
    await assert.rejects(navigateInitialGet(f.page, url, spki, { ...f.options, timeoutMs: 40 }), /deadline|after commit/);
    assert.ok(performance.now() - started < 1000, 'driver hangs must not extend the deadline');
    assert.ok(f.calls() <= 1);
  });
}

test('failure after commit cannot retry even with the transient error text', async () => {
  const f = fixture();
  f.page.waitForLoadState = async () => { throw changed(); };
  await assert.rejects(navigateInitialGet(f.page, url, spki, f.options), /after commit \(no retry\)/);
  assert.equal(f.calls(), 1);
});

test('deadline rejection cancels the pending retry-delay timer', async () => {
  const timers = new Set();
  const hook = createHook({
    init(id, type) { if (type === 'Timeout') timers.add(id); },
    destroy(id) { timers.delete(id); },
  });
  const f = fixture([changed()]);
  hook.enable();
  try {
    await assert.rejects(navigateInitialGet(f.page, url, spki, { ...f.options, timeoutMs: 40 }), /deadline/);
    // Timer destruction notifications drain on the following event-loop turn.
    await new Promise(resolve => setImmediate(resolve));
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(timers.size, 0, 'no retry-delay timer survives the rejected operation');
    assert.equal(f.calls(), 1);
  } finally { hook.disable(); }
});

test('retry navigation gets only the remainder of the original deadline, including hangs', async () => {
  const f = fixture();
  const budgets = [];
  f.page.goto = async (_target, options) => {
    budgets.push(options.timeout);
    if (budgets.length === 1) throw changed();
    return new Promise(() => {});
  };
  const started = performance.now();
  await assert.rejects(navigateInitialGet(f.page, url, spki, { ...f.options, timeoutMs: 450 }), /deadline exceeded/);
  assert.equal(budgets.length, 2);
  assert.ok(budgets[1] < budgets[0] - 200);
  assert.ok(performance.now() - started < 1200);
});

test('invalid inputs cannot start network work', async () => {
  for (const timeoutMs of [0, -1, NaN, Infinity]) {
    const f = fixture();
    await assert.rejects(navigateInitialGet(f.page, url, spki, { ...f.options, timeoutMs }), /deadline/);
    assert.deepEqual(f.events, []);
  }
});

test('real TLS probe validates trust, hostname, original SPKI and bounds stalled handshakes', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'airp-navigation-'));
  try {
    const certs = {};
    for (const hostname of ['localhost', 'elsewhere.invalid']) {
      const certFile = join(directory, hostname + '.crt');
      const keyFile = join(directory, hostname + '.key');
      execFileSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
        '-keyout', keyFile, '-out', certFile, '-subj', `/CN=${hostname}`,
        '-addext', `subjectAltName=DNS:${hostname}`], { stdio: 'ignore', timeout: 15_000 });
      const cert = await readFile(certFile);
      certs[hostname] = { certFile, cert, key: await readFile(keyFile),
        pin: createHash('sha256').update(new X509Certificate(cert).publicKey.export({ type: 'spki', format: 'der' })).digest('base64') };
    }
    for (const scenario of ['valid', 'untrusted', 'hostname', 'pin', 'hang']) {
      await t.test(scenario, async () => {
        const served = certs[scenario === 'hostname' ? 'elsewhere.invalid' : 'localhost'];
        const server = scenario === 'hang' ? net.createServer(socket => socket.resume()) : tls.createServer(served);
        const sockets = new Set();
        server.on('connection', socket => { sockets.add(socket); socket.once('close', () => sockets.delete(socket)); });
        await new Promise(resolve => server.listen(0, resolve));
        try {
          const target = `https://localhost:${server.address().port}/?private-token`;
          const pin = scenario === 'pin' ? spki : served.pin;
          const code = `import { navigateInitialGet } from ${JSON.stringify(new URL('./production-initial-navigation.mjs', import.meta.url).href)};
            const page = { goto: async () => { console.log('GET'); return {}; }, waitForLoadState: async () => {} };
            try { await navigateInitialGet(page, ${JSON.stringify(target)}, ${JSON.stringify(pin)}, { timeoutMs: ${scenario === 'hang' ? 200 : 1500} }); }
            catch (error) { console.error(error.message); process.exitCode = 1; }`;
          const run = promisify(execFile)(process.execPath, ['--input-type=module', '-e', code], {
            env: { ...process.env, NODE_EXTRA_CA_CERTS: scenario === 'untrusted' ? certs['elsewhere.invalid'].certFile : served.certFile },
            timeout: 5000,
          });
          if (scenario === 'valid') {
            const result = await run;
            assert.equal(result.stdout.trim(), 'GET');
            assert.ok(result.stderr.includes('"expectedSpkiMatches":true'));
          } else {
            await assert.rejects(run, error => {
              assert.equal(error.stdout, '', 'no GET after failed identity verification');
              assert.ok(!error.stderr.includes('private-token'));
              const expected = { untrusted: /DEPTH_ZERO_SELF_SIGNED_CERT/, hostname: /ERR_TLS_CERT_ALTNAME_INVALID/,
                pin: /SPKI identity changed/, hang: /deadline exceeded/ }[scenario];
              assert.match(error.stderr, expected);
              return true;
            });
          }
          assert.equal(sockets.size, 0, 'probe closes sockets before child exits');
        } finally {
          for (const socket of sockets) socket.destroy();
          await new Promise(resolve => server.close(resolve));
        }
      });
    }
  } finally { await rm(directory, { recursive: true, force: true }); }
});

// Execute the actual smoke orchestration with fake external services, so this
// checks call order/TLS settings/assertions rather than only searching source.
for (const outcome of ['recover', 'exhaustion', 'trust-failure', 'http-failure']) {
  test(`restart smoke integration: ${outcome}, before chat side effects`, async () => {
    const source = (await readFile(new URL('./production-browser-restart-smoke.mjs', import.meta.url), 'utf8'))
      .replace(/^import .*;\r?\n/gm, '');
    const f = fixture(outcome === 'exhaustion' ? [changed(), changed()] : outcome === 'recover' ? [changed()] : outcome === 'trust-failure'
      ? [new Error(`page.goto: net::ERR_CERT_AUTHORITY_INVALID at ${url}`)] : []);
    const calls = [];
    let sent;
    const page = { ...f.page,
      goto: async (_url, options) => {
        const result = await f.page.goto(url, options);
        return outcome === 'http-failure' ? { status: () => 503 } : result;
      },
      on() {}, async addInitScript() {}, async waitForFunction() {},
      locator: () => ({ async fill(message) { sent = message; calls.push('fill'); }, async click() { calls.push('click'); } }),
      async evaluate() { return []; },
    };
    const context = {
      async newPage() { return page; }, async close() { calls.push('context-close'); },
      request: { async post() {
        calls.push('history');
        return { status: () => 200, json: async () => ({ total: sent ? 3 : 1,
          messages: [{ role: 'user', content: sent ?? 'previous' }] }) };
      } },
    };
    const sandbox = {
      assert, readFileSync: () => JSON.stringify({ characterId: 'character', sessionId: 'session', message: 'previous', total: 1 }),
      process: { env: { AIRP_SMOKE_ORIGIN: 'https://localhost:9443', AIRP_SMOKE_ADMIN_USER: 'private-user',
        AIRP_SMOKE_ADMIN_PASSWORD: 'private-password', AIRP_SMOKE_BROWSER_RESULT_FILE: 'unused', AIRP_CHROME_SPKI: spki } },
      console: { log() {} },
      chromium: { async launch(options) {
        assert.deepEqual(Array.from(options.args), [`--ignore-certificate-errors-spki-list=${spki}`]);
        return { version: () => 'test', async newContext(options) {
          assert.equal(options.ignoreHTTPSErrors, false);
          return context;
        }, async close() { calls.push('browser-close'); } };
      } },
      navigateInitialGet: async (page, target, pin) => {
        assert.equal(target, 'https://localhost:9443/screens/02-chat-space.html?character=character&session=session');
        const response = await navigateInitialGet(page, target, pin, f.options);
        calls.push('navigation-complete');
        return response;
      },
    };
    const realm = vm.createContext(sandbox);
    page.evaluate = async () => vm.runInContext('[]', realm);
    const run = vm.runInContext(`(async () => { ${source}\n })()`, realm);
    if (outcome === 'recover') {
      await run;
      assert.deepEqual(calls, ['navigation-complete', 'history', 'fill', 'click', 'history', 'context-close', 'browser-close']);
      assert.equal(f.calls(), 2);
    } else {
      await assert.rejects(run);
      assert.ok(!calls.includes('history') && !calls.includes('fill') && !calls.includes('click'));
      assert.equal(calls.at(-1), 'browser-close');
    }
  });
}
