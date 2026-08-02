// Bounded cleanup retry used by the production cancellation smoke.
//
// DELETE can briefly return 409 while a Coordinator generation is still
// generating/committing.  Once the session is idle, retry DELETE on the next
// loop instead of treating the transient 409 as a permanent cleanup failure.

const BUSY_PHASES = new Set(['generating', 'committing', 'recovering']);

export function classifySessionPhase(state) {
  if (!state?.ok) return 'probe-failure';
  const phase = state.data?.phase;
  if (phase === 'idle') return 'idle';
  if (BUSY_PHASES.has(phase)) return phase;
  return 'unknown';
}

export async function deleteSessionWithRetry({
  timeoutMs = 8000,
  deleteAttempt,
  stateAttempt,
  sleep = (ms) => new Promise(resolve => setTimeout(resolve, ms)),
  now = Date.now,
} = {}) {
  if (typeof deleteAttempt !== 'function' || typeof stateAttempt !== 'function') {
    throw new TypeError('deleteAttempt and stateAttempt are required');
  }
  const budget = Number.isFinite(timeoutMs) ? Math.max(0, timeoutMs) : 0;
  const deadline = now() + budget;
  let last = null;
  let lastState = null;
  let lastPhase = null;
  while (now() < deadline) {
    const remaining = deadline - now();
    if (remaining <= 0) break;
    last = await deleteAttempt(remaining);
    if (last?.ok || last?.status !== 409) return last;

    const stateRemaining = deadline - now();
    if (stateRemaining <= 0) break;
    const state = await stateAttempt(stateRemaining);
    lastState = state;
    const phase = state?.data?.phase;
    lastPhase = phase === undefined ? null : phase;
    // Only an explicit idle phase authorizes an immediate retry.  A missing,
    // unknown, or failed probe must remain fail-closed and take a bounded wait.
    if (classifySessionPhase(state) === 'idle') {
      // Idle must not turn the observed 409 into a false cleanup failure. The
      // caller's DELETE helper supplies its own request throttle; retry on the
      // next loop immediately.
      continue;
    }

    // An active generation, an unavailable probe, or an unknown phase gets a
    // bounded pause rather than a tight retry loop.  The absolute deadline is
    // authoritative: never ask a sleep/request to exceed its remainder.
    const delay = Math.min(200, deadline - now());
    if (delay <= 0) break;
    await sleep(delay);
  }
  const result = last || { status: 0, ok: false, data: null };
  return {
    ...result,
    text: result.text || 'delete deadline exceeded',
    deadlineExceeded: true,
    lastDelete: result,
    lastState,
    lastPhase,
  };
}
