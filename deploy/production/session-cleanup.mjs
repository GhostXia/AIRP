// Bounded cleanup retry used by the production cancellation smoke.
//
// DELETE can briefly return 409 while a Coordinator generation is still
// generating/committing.  Once the session is idle, retry DELETE on the next
// loop instead of treating the transient 409 as a permanent cleanup failure.

export async function deleteSessionWithRetry({
  timeoutMs = 8000,
  deleteAttempt,
  stateAttempt,
  sleep = (ms) => new Promise(resolve => setTimeout(resolve, ms)),
} = {}) {
  if (typeof deleteAttempt !== 'function' || typeof stateAttempt !== 'function') {
    throw new TypeError('deleteAttempt and stateAttempt are required');
  }
  const deadline = Date.now() + timeoutMs;
  let last = null;
  while (Date.now() < deadline) {
    const remaining = Math.max(1, deadline - Date.now());
    last = await deleteAttempt(remaining);
    if (last.ok || last.status !== 409) return last;

    const state = await stateAttempt(Math.max(1, deadline - Date.now()));
    const phase = state.data?.phase;
    const busy = state.ok && (phase === 'generating' || phase === 'committing');
    if (state.ok && !busy) {
      // Idle must not turn the observed 409 into a false cleanup failure. The
      // caller's DELETE helper supplies its own request throttle; retry on the
      // next loop immediately.
      continue;
    }
    // An active generation, or an unavailable state probe, gets a bounded
    // pause rather than a tight retry loop.
    await sleep(Math.min(200, Math.max(1, deadline - Date.now())));
  }
  return last || { status: 0, ok: false, data: null, text: 'delete deadline exceeded' };
}
