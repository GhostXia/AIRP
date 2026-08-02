// Small, dependency-free helpers for production smoke HTTP deadlines.
//
// A fetch promise only covers response headers.  Reading an error body from a
// response can otherwise wait forever when the peer keeps the connection open.
// These helpers keep that read inside the caller's absolute deadline and cancel
// the reader on timeout.

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

function cancelReader(reader) {
  try {
    // Do not make the deadline depend on a peer that also refuses to close.
    // The stream cancellation promise is intentionally detached; Node's
    // reader owns any eventual cleanup and the caller gets its diagnostic now.
    const pending = reader?.cancel?.();
    pending?.catch?.(() => {});
  } catch {}
}

/**
 * Read a Response body without exceeding an absolute deadline.
 *
 * Returns `{ text, timedOut }`; malformed/partial text is retained for
 * diagnostics, while `timedOut` tells callers that it must not be treated as
 * a complete body.  `now`, `setTimer`, and `clearTimer` are injectable so the
 * unit tests can exercise timeout paths without wall-clock sleeps.
 */
export async function readResponseBodyBounded(
  response,
  {
    deadline = Date.now() + 10000,
    now = Date.now,
    setTimer = setTimeout,
    clearTimer = clearTimeout,
    onTimeout = () => {},
  } = {},
) {
  const empty = { text: '', timedOut: false };
  if (!response) return empty;

  let reader;
  try {
    reader = response.body?.getReader?.();
  } catch {
    reader = null;
  }

  // Real fetch Responses expose a reader.  Keep a small fallback for test
  // doubles and older adapters, still racing the text promise against the
  // same aggregate deadline (there is no reader to cancel in this branch).
  if (!reader) {
    if (typeof response.text !== 'function') return empty;
    const remaining = remainingMs(deadline, now);
    if (remaining <= 0) {
      try { onTimeout(); } catch {}
      return { text: '', timedOut: true };
    }
    let timer;
    let timeout = false;
    const timeoutPromise = new Promise((resolve) => {
      timer = setTimer(() => {
        timeout = true;
        resolve('');
      }, remaining);
    });
    try {
      const text = await Promise.race([response.text(), timeoutPromise]);
      timeout = timeout || remainingMs(deadline, now) <= 0;
      if (timeout) {
        try { onTimeout(); } catch {}
      }
      return { text: timeout ? String(text || '') : String(text || ''), timedOut: timeout };
    } finally {
      if (timer !== undefined) clearTimer(timer);
    }
  }

  const decoder = new TextDecoder();
  let text = '';
  while (true) {
    const remaining = remainingMs(deadline, now);
    if (remaining <= 0) {
      try { onTimeout(); } catch {}
      cancelReader(reader);
      return { text, timedOut: true };
    }

    let timer;
    let timedOut = false;
    const timeoutPromise = new Promise((resolve) => {
      timer = setTimer(() => {
        timedOut = true;
        resolve({ timedOut: true });
      }, remaining);
    });
    let next;
    try {
      next = await Promise.race([reader.read(), timeoutPromise]);
    } finally {
      if (timer !== undefined) clearTimer(timer);
    }
    if (timedOut || next?.timedOut || remainingMs(deadline, now) <= 0) {
      try { onTimeout(); } catch {}
      cancelReader(reader);
      return { text, timedOut: true };
    }
    if (next?.done) {
      text += decoder.decode();
      return { text, timedOut: false };
    }
    if (next?.value) text += decoder.decode(next.value, { stream: true });
  }
}
