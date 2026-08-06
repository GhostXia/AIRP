// Strict AIRP Engine SSE consumer used by production smoke.
//
// Frame shapes follow the machine-readable contract protocol/sse-events.json
// (additive-only): every locked message data type is accepted, unknown message
// data types and unknown fields are tolerated, and the typed error envelope
// must carry code/message/retryable/commit_state.
//
// A successful generation must terminate with the Engine's typed
// `event: message` / `{ "type": "done" }` frame.  An error frame is terminal
// only when its structured error envelope is valid; an early EOF is never
// treated as success.  Cancellation is an explicit opt-in terminal variant.

export async function consumeGenerationSse(
  response,
  {
    allowCancellation = false,
    timeoutMs = 30000,
    setTimer = setTimeout,
    clearTimer = clearTimeout,
  } = {},
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
    unknownTypes: [],
    optionFrames: 0,
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

    const eventIsError = event === 'error';
    const payloadIsError = payload?.type === 'error';
    if (eventIsError !== payloadIsError) {
      finish('schema_mismatch', 'SSE error event/type fields disagree');
      return;
    }
    if (eventIsError && payloadIsError) {
      const detail = payload?.error;
      const envelopeValid =
        payload?.type === 'error' &&
        typeof payload?.text === 'string' &&
        detail &&
        typeof detail.code === 'string' &&
        typeof detail.message === 'string' &&
        typeof detail.retryable === 'boolean' &&
        typeof detail.commit_state === 'string';
      if (!envelopeValid) {
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
    if (payload?.type === 'action_options') {
      // serde adjacent-tagged shape: {"type":"action_options","text":{"options":[...]}}
      const options = payload?.text?.options;
      if (!Array.isArray(options) || !options.every(option => typeof option === 'string')) {
        finish('malformed', 'action_options frame lacks a string options array');
        return;
      }
      result.optionFrames++;
      return;
    }
    // additive-only contract: unknown message data types are tolerated.
    result.unknownTypes.push(String(payload?.type ?? 'missing'));
  };

  try {
    while (!result.terminal && Date.now() < deadline) {
      const remaining = deadline - Date.now();
      let timer;
      const timeout = new Promise(resolve => {
        timer = setTimer(() => resolve({ timedOut: true }), remaining);
      });
      let next;
      try {
        next = await Promise.race([reader.read(), timeout]);
      } finally {
        if (timer !== undefined) clearTimer(timer);
      }
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
    // AIRP's typed terminal frame is authoritative: once `done`, or the
    // allowed cancellation error, is observed, trailing transport bytes are
    // intentionally ignored. Canceling the reader also makes the
    // mock/provider connection finite instead of waiting for EOF.
    if (result.terminal) {
      try { await reader.cancel(); } catch {}
    }
  }

  if (!result.terminal && buffer.trim()) dispatch(buffer);
  if (!result.terminal) finish('eof', 'SSE ended before a typed terminal frame');
  result.elapsedMs = Date.now() - startedAt;
  return result;
}
