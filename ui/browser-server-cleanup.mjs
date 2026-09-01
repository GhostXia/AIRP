import { once } from 'node:events';

export function hasChildExited(child) {
  return child.exitCode !== null || child.signalCode !== null;
}

export async function withTimeout(promise, timeoutMs, message) {
  let timeout;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timeout = setTimeout(() => reject(new Error(message)), timeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timeout);
  }
}

export async function waitForChildExit(child, timeoutMs, label = 'child') {
  if (hasChildExited(child)) return;
  await withTimeout(
    once(child, 'exit'),
    timeoutMs,
    `${label} process did not exit within ${timeoutMs} ms`,
  );
}

export async function closeOwnedBrowser(browser, browserServer, timeoutMs = 5_000) {
  const browserProcess = browserServer.process();
  try {
    await withTimeout((async () => {
      await browser.close();
      await browserServer.close();
    })(), timeoutMs, `Chromium did not close within ${timeoutMs} ms`);
    await waitForChildExit(browserProcess, 2_000, 'Chromium');
    return 'closed';
  } catch (closeError) {
    try {
      await withTimeout(
        browserServer.kill(),
        timeoutMs,
        `Chromium force-kill did not finish within ${timeoutMs} ms`,
      );
      await waitForChildExit(browserProcess, 2_000, 'Chromium');
      return 'forced';
    } catch (killError) {
      throw new AggregateError([closeError, killError], 'Chromium graceful and forced cleanup failed');
    }
  }
}
