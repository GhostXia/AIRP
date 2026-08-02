// Small, dependency-free helpers for production smoke HTTP deadlines.
//
// A fetch promise only covers response headers. Reading an error body from a
// response can otherwise wait forever or grow without a bound when a peer
// keeps the connection open. These helpers keep every body read inside the
// caller's absolute deadline and byte budget.

export const DEFAULT_MAX_BYTES = 64 * 1024;
// Independent resource-cleanup grace. The body result is already fail-closed
// when this grace starts; this small bound may run after the body deadline
// solely to give the underlying stream a chance to settle.
export const CANCEL_CLEANUP_GRACE_MS = 50;

export function remainingMs(deadline, now = Date.now) {
  if (!Number.isFinite(deadline)) return Infinity;
  return Math.max(0, deadline - now());
}
export function clampTimeout(deadline, requestedMs, now = Date.now) {
  const remaining = remainingMs(deadline, now);
  if (remaining <= 0) return 0;
  if (!Number.isFinite(requestedMs)) return remaining;
  return Math.min(Math.max(0, requestedMs), remaining);
}

/**
 * HTTP status is not sufficient evidence of a usable response. A 2xx header
 * followed by a timed-out, oversized, unsupported, or failed body is an
 * incomplete response and must remain unsuccessful to callers.
 */
export function responseSucceeded(response, bodyResult) {
  return Boolean(response?.ok && bodyResult?.complete === true);
}

async function cancelReaderBounded(
  reader,
  {
    setTimer = setTimeout,
    clearTimer = clearTimeout,
    timeoutMs = CANCEL_CLEANUP_GRACE_MS,
  } = {},
) {
  let pending;
  try {
    pending = reader?.cancel?.();
  } catch {
    return false;
  }
  if (!pending || typeof pending.then !== 'function') return true;

  const cancelBudget = Math.max(0, timeoutMs);
  const settled = Promise.resolve(pending)
    .then(() => ({ completed: true }))
    .catch(() => ({ completed: false }));
  if (cancelBudget <= 0) return false;

  let timer;
  let outcome = null;
  try {
    outcome = await Promise.race([
      settled,
      new Promise((resolve) => {
        timer = setTimer(() => resolve({ completed: false }), cancelBudget);
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimer(timer);
  }
  return outcome?.completed === true;
}

function resultBase() {
  return {
    text: '',
    bytes: 0,
    complete: false,
    timedOut: false,
    tooLarge: false,
    transportError: false,
    unsupported: false,
    error: null,
    cleanupIncomplete: false,
    cleanupError: null,
    lockReleased: false,
    lockReleaseError: null,
  };
}

/**
 * Read a Response body without exceeding an absolute deadline or byte cap.
 *
 * `complete` is true only after a normal EOF and a fully decoded body. Every
 * other path is fail-closed. `now`, `setTimer`, and `clearTimer` are
 * injectable so tests can exercise timeout paths without wall-clock sleeps.
 * `onIncomplete` (with `onTimeout` retained as a compatibility alias) lets a
 * caller abort the owning fetch controller before the reader is cancelled.
 */
export async function readResponseBodyBounded(
  response,
  {
    deadline = Date.now() + 10000,
    now = Date.now,
    maxBytes = DEFAULT_MAX_BYTES,
    setTimer = setTimeout,
    clearTimer = clearTimeout,
    onIncomplete,
    onTimeout,
  } = {},
) {
  const result = resultBase();
  const byteLimit = Number.isFinite(maxBytes) && maxBytes >= 0
    ? Math.floor(maxBytes)
    : DEFAULT_MAX_BYTES;
  const abortIncomplete = typeof onIncomplete === 'function'
    ? onIncomplete
    : (typeof onTimeout === 'function' ? onTimeout : () => {});

  if (!response) {
    result.unsupported = true;
    result.error = 'response unavailable';
    result.lockReleased = true;
    try { abortIncomplete(result); } catch {}
    return result;
  }

  let reader;
  try {
    reader = response.body?.getReader?.();
  } catch (error) {
    result.unsupported = true;
    result.error = `response body reader unavailable: ${error?.message || error}`;
    result.lockReleased = true;
    try { abortIncomplete(result); } catch {}
    return result;
  }
  if (!reader) {
    result.unsupported = true;
    result.error = 'response body reader unavailable';
    result.lockReleased = true;
    try { abortIncomplete(result); } catch {}
    return result;
  }

  const decoder = new TextDecoder();
  const finishIncomplete = async (fields) => {
    Object.assign(result, fields);
    try { abortIncomplete(result); } catch {}
    // Body completion is already fail-closed. The bounded grace is a separate
    // resource-cleanup allowance and is intentionally not claimed as part of
    // the response deadline.
    const cancelled = await cancelReaderBounded(reader, {
      setTimer,
      clearTimer,
      timeoutMs: CANCEL_CLEANUP_GRACE_MS,
    });
    if (!cancelled) {
      result.cleanupIncomplete = true;
      result.cleanupError = `reader cancellation exceeded ${CANCEL_CLEANUP_GRACE_MS}ms cleanup grace`;
    }
    return result;
  };

  try {
    while (true) {
      const remaining = remainingMs(deadline, now);
      if (remaining <= 0) {
        return await finishIncomplete({
          timedOut: true,
          error: 'response body deadline exceeded',
        });
      }

      let timer;
      let timerFired = false;
      const timeoutPromise = new Promise((resolve) => {
        timer = setTimer(() => {
          timerFired = true;
          resolve({ deadlineExceeded: true });
        }, remaining);
      });
      let next;
      try {
        next = await Promise.race([reader.read(), timeoutPromise]);
      } catch (error) {
        return await finishIncomplete({
          transportError: true,
          error: `response body read failed: ${error?.message || error}`,
        });
      } finally {
        if (timer !== undefined) clearTimer(timer);
      }

      if (timerFired || next?.deadlineExceeded || remainingMs(deadline, now) <= 0) {
        return await finishIncomplete({
          timedOut: true,
          error: 'response body deadline exceeded',
        });
      }
      if (next?.done) {
        try {
          result.text += decoder.decode();
        } catch (error) {
          return await finishIncomplete({
            transportError: true,
            error: `response body decode failed: ${error?.message || error}`,
          });
        }
        result.complete = true;
        return result;
      }

      const chunk = next?.value;
      const chunkBytes = Number.isFinite(chunk?.byteLength) ? chunk.byteLength : 0;
      result.bytes += chunkBytes;
      if (result.bytes > byteLimit) {
        return await finishIncomplete({
          tooLarge: true,
          error: `response body exceeds ${byteLimit} byte limit`,
        });
      }
      try {
        if (chunkBytes > 0) result.text += decoder.decode(chunk, { stream: true });
      } catch (error) {
        return await finishIncomplete({
          transportError: true,
          error: `response body decode failed: ${error?.message || error}`,
        });
      }
    }
  } finally {
    // A reader owns the lock even after cancel/rejection. Release it on every
    // path so callers can observe `response.body.locked === false`.
    try {
      reader.releaseLock?.();
      result.lockReleased = response.body?.locked === false || typeof reader.releaseLock !== 'function';
      if (!result.lockReleased) {
        result.lockReleaseError = 'reader lock remained held after releaseLock';
      }
    } catch (error) {
      result.lockReleased = false;
      result.lockReleaseError = error?.message || String(error);
    }
  }
}
