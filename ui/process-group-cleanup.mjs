const DEFAULT_TERM_TIMEOUT_MS = 2_000;
const DEFAULT_KILL_TIMEOUT_MS = 2_000;
const DEFAULT_POLL_MS = 50;

function isMissingProcess(error) {
  return error && typeof error === 'object' && error.code === 'ESRCH';
}

function groupExists(pid, signalProcessGroup) {
  try {
    signalProcessGroup(-pid, 0);
    return true;
  } catch (error) {
    if (isMissingProcess(error)) return false;
    throw error;
  }
}

async function waitForGroupExit(pid, timeoutMs, options) {
  const deadline = options.now() + timeoutMs;
  while (options.now() < deadline) {
    if (!groupExists(pid, options.signalProcessGroup)) return true;
    await options.sleep(Math.min(options.pollMs, deadline - options.now()));
  }
  return !groupExists(pid, options.signalProcessGroup);
}

async function waitForTerminationSignal(timeoutMs, options) {
  const deadline = options.now() + timeoutMs;
  while (options.now() < deadline) {
    if (options.hasTerminated()) return true;
    await options.sleep(Math.min(options.pollMs, deadline - options.now()));
  }
  return options.hasTerminated();
}

export async function terminateDetachedProcessGroup(pid, overrides = {}) {
  if (!Number.isSafeInteger(pid) || pid <= 0) {
    throw new Error(`invalid detached process-group id: ${pid}`);
  }
  const options = {
    termTimeoutMs: DEFAULT_TERM_TIMEOUT_MS,
    killTimeoutMs: DEFAULT_KILL_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    now: () => performance.now(),
    sleep: ms => new Promise(resolve => setTimeout(resolve, ms)),
    signalProcessGroup: process.kill,
    hasTerminated: null,
    ...overrides,
  };
  const terminatedBeforeSignal = options.hasTerminated?.() ?? false;

  if (!groupExists(pid, options.signalProcessGroup)) return 'already-exited';
  try {
    options.signalProcessGroup(-pid, 'SIGTERM');
  } catch (error) {
    if (isMissingProcess(error)) return 'already-exited';
    throw error;
  }
  if (options.hasTerminated) {
    await waitForTerminationSignal(options.termTimeoutMs, options);
  } else if (await waitForGroupExit(pid, options.termTimeoutMs, options)) {
    return 'terminated';
  }

  try {
    options.signalProcessGroup(-pid, 'SIGKILL');
  } catch (error) {
    if (isMissingProcess(error)) return 'terminated';
    throw error;
  }
  if (await waitForGroupExit(pid, options.killTimeoutMs, options)) {
    if (terminatedBeforeSignal) return 'already-exited-tree-forced';
    if (options.hasTerminated?.()) return 'terminated-tree-forced';
    return 'forced';
  }
  throw new Error(
    `detached process group ${pid} survived SIGTERM (${options.termTimeoutMs} ms) `
      + `and SIGKILL (${options.killTimeoutMs} ms)`,
  );
}
