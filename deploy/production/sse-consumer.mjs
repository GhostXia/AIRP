// Strict AIRP Engine SSE consumer used by production smoke.
//
// A successful generation must terminate with the Engine's typed
// `event: message` / `{ "type": "done" }` frame.  An error frame is terminal
// only when its structured error envelope is valid; an early EOF is never
// treated as success.  Cancellation is an explicit opt-in terminal variant.

export async function consumeGenerationSse(
  response,
  { allowCancellation = false, timeoutMs = 30000 } = {},
) {
  const startedAt = Date.now();
  const empty = {
    chunks: [],
    text: '',
    frames: 0,
    readBatches: 0,
    terminal: null,
    typedError: null,
    errors: [],
    error: null,
    timedOut: false,
    elapsedMs: 0,
  };
  if (!response?.body?.getReader) {
    return { ...empty, terminal: 'unavailable', error: 'SSE response has no readable body', elapsedMs: Date.now() - startedAt };
  }

  const result = { ...empty };
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  const deadline = startedAt + timeoutMs;
  let buffer = '';

  const finish = (terminal, error = null) => {
    if (!result.terminal) result.terminal = terminal;
    if (error && !result.error) result.error = error;
  };

  const dispatch = (frame) => {
    let event = 'message';
    const data = [];
    for (const rawLine of frame.split(/\r?\n/)) {
      if (!rawLine || rawLine.startsWith(':')) continue;
      const separator = rawLine.indexOf(':');
      const field = separator < 0 ? rawLine : rawLine.slice(0, separator);
      let value = separator < 0 ? '' : rawLine.slice(separator + 1);
      if (value.startsWith(' ')) value = value.slice(1);
      if (field === 'event') event = value || 'message';
      if (field === 'data') data.push(value);
    }
    if (!data.length) return;
    const payloadText = data.join('\n');
    let payload;
    try {
      payload = JSON.parse(payloadText);
    } catch (error) {
      result.frames++;
      finish('malformed', `SSE ${event} frame JSON parse failed: ${error.message}`);
      return;
    }
    result.frames++;

    if (event === 'error' || payload?.type === 'error') {
      const detail = payload?.error;
      if (payload?.type !== 'error' || !detail || typeof detail.code !== 'string' || typeof detail.commit_state !== 'string') {
        finish('malformed_error', 'SSE error frame lacks the typed error envelope');
        return;
      }
      result.typedError = detail;
      result.errors.push({ code: detail.code, commit_state: detail.commit_state });
      if (allowCancellation && detail.code === 'cancelled' && detail.commit_state === 'partially_committed') {
        finish('cancelled');
      } else if (allowCancellation) {
        finish('invalid_error', `unexpected cancellation error code/state: ${detail.code}/${detail.commit_state}`);
      } else {
        finish('error', detail.message || detail.code);
      }
      return;
    }

    if (event !== 'message') {
      finish('invalid_event', `unexpected SSE event: ${event}`);
      return;
    }
    if (payload?.type === 'done') {
      finish('done');
      return;
    }
    if (payload?.type === 'body_chunk' || payload?.type === 'think_chunk') {
      if (typeof payload.text !== 'string') {
        finish('malformed', `${payload.type} frame has no text string`);
        return;
      }
      result.chunks.push(payload.text);
      if (payload.type === 'body_chunk') result.text += payload.text;
      return;
    }
    finish('invalid_message', `unexpected SSE message type: ${payload?.type || 'missing'}`);
  };

  try {
    while (!result.terminal && Date.now() < deadline) {
      const remaining = deadline - Date.now();
      let timer;
      const timeout = new Promise(resolve => {
        timer = setTimeout(() => resolve({ timedOut: true }), remaining);
      });
      const next = await Promise.race([reader.read(), timeout]);
      clearTimeout(timer);
      if (next.timedOut) {
        result.timedOut = true;
        finish('timeout', 'SSE consumption deadline exceeded');
        break;
      }
      if (next.done) break;
      result.readBatches++;
      buffer += decoder.decode(next.value, { stream: true });
      let index;
      while (!result.terminal && (index = buffer.search(/\r?\n\r?\n/)) >= 0) {
        const separator = buffer.match(/\r?\n\r?\n/);
        const frame = buffer.slice(0, index);
        buffer = buffer.slice(index + separator[0].length);
        dispatch(frame);
      }
    }
  } catch (error) {
    finish('transport_error', error?.message || String(error));
  } finally {
    // Once a typed terminal frame is observed, no further bytes are needed;
    // canceling the reader also makes the mock/provider connection finite.
    if (result.terminal) {
      try { await reader.cancel(); } catch {}
    }
  }

  if (!result.terminal && buffer.trim()) dispatch(buffer);
  if (!result.terminal) finish('eof', 'SSE ended before a typed terminal frame');
  result.elapsedMs = Date.now() - startedAt;
  return result;
}
