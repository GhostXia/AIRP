(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPApi = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  class AirpHttpError extends Error {
    constructor(message, status, data) {
      super(message);
      this.name = 'AirpHttpError';
      this.status = status;
      this.data = data;
    }
  }

  class AirpStreamError extends Error {
    constructor(message, detail) {
      super(message);
      this.name = 'AirpStreamError';
      this.code = detail && detail.code;
      this.retryable = Boolean(detail && detail.retryable);
      this.commitState = detail && detail.commit_state;
    }
  }

  function trimBase(value) {
    return String(value || '').replace(/\/+$/, '');
  }

  function normalizeTimeout(value, fallback) {
    if (value === undefined) return fallback;
    const parsed = Number(value);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
  }

  function timedSignal(signal, timeoutMs) {
    if (signal || !timeoutMs) return { signal, cleanup: function () {} };
    const controller = new AbortController();
    const timer = setTimeout(function () {
      controller.abort(new DOMException('Request timed out', 'TimeoutError'));
    }, timeoutMs);
    return { signal: controller.signal, cleanup: function () { clearTimeout(timer); } };
  }

  function errorMessage(data, fallback) {
    if (typeof data === 'string' && data.trim()) return data.trim();
    if (data && typeof data === 'object') {
      for (const key of ['message', 'detail', 'error', 'hint']) {
        if (typeof data[key] === 'string' && data[key].trim()) return data[key].trim();
      }
    }
    return fallback || '请求失败';
  }

  function parseSseBlock(block) {
    let event = 'message';
    const data = [];
    for (const line of block.split(/\r?\n/)) {
      if (!line || line.startsWith(':')) continue;
      const separator = line.indexOf(':');
      const field = separator < 0 ? line : line.slice(0, separator);
      let value = separator < 0 ? '' : line.slice(separator + 1);
      if (value.startsWith(' ')) value = value.slice(1);
      if (field === 'event') event = value || 'message';
      if (field === 'data') data.push(value);
    }
    return { event, data: data.join('\n') };
  }

  async function consumeSse(response, handlers) {
    const onChunk = handlers.onChunk || function () {};
    const onDone = handlers.onDone || function () {};
    const reader = response.body && response.body.getReader ? response.body.getReader() : null;
    if (!reader) throw new AirpStreamError('浏览器未提供可读取的流式响应', { code: 'stream_unavailable' });

    const decoder = new TextDecoder();
    let buffer = '';
    let finished = false;

    function dispatch(block) {
      if (!block.trim()) return;
      const parsed = parseSseBlock(block);
      if (!parsed.data) return;
      if (parsed.data === '[DONE]') {
        finished = true;
        onDone();
        return;
      }
      let payload;
      try {
        payload = JSON.parse(parsed.data);
      } catch {
        payload = { type: 'body_chunk', text: parsed.data };
      }
      if (parsed.event === 'error' || payload.type === 'error') {
        throw new AirpStreamError(errorMessage(payload, '流式生成失败'), payload);
      }
      if (payload.type === 'done') {
        finished = true;
        onDone(payload);
        return;
      }
      onChunk(payload);
    }

    while (!finished) {
      const part = await reader.read();
      if (part.done) break;
      buffer += decoder.decode(part.value, { stream: true });
      const blocks = buffer.split(/\r?\n\r?\n/);
      buffer = blocks.pop() || '';
      for (const block of blocks) {
        dispatch(block);
        if (finished) break;
      }
    }
    buffer += decoder.decode();
    if (!finished && buffer.trim()) dispatch(buffer);
    if (!finished) {
      throw new AirpStreamError('流式响应在完成事件前中断；请刷新历史确认写入状态', {
        code: 'stream_incomplete', retryable: false, commit_state: 'unknown',
      });
    }
    return { completed: true };
  }

  function createClient(options) {
    const opts = options || {};
    const fetchImpl = opts.fetchImpl || globalThis.fetch;
    const base = trimBase(opts.base || (globalThis.location && globalThis.location.origin));
    const bearer = String(opts.bearer || '');
    const onRequest = typeof opts.onRequest === 'function' ? opts.onRequest : function () {};
    const requestTimeoutMs = normalizeTimeout(opts.requestTimeoutMs, 30_000);
    const streamTimeoutMs = normalizeTimeout(opts.streamTimeoutMs, 300_000);
    if (typeof fetchImpl !== 'function') throw new TypeError('fetch implementation is required');

    function headers(extra) {
      const value = Object.assign({ Accept: 'application/json' }, extra || {});
      if (bearer) value.Authorization = 'Bearer ' + bearer;
      return value;
    }

    async function request(method, path, body, requestOptions) {
      const started = Date.now();
      const timed = timedSignal(requestOptions && requestOptions.signal, requestTimeoutMs);
      const init = {
        method,
        headers: headers(body === undefined ? undefined : { 'Content-Type': 'application/json' }),
        signal: timed.signal,
      };
      if (body !== undefined) init.body = JSON.stringify(body);
      try {
        let response;
        try {
          response = await fetchImpl(base + path, init);
        } catch (error) {
          onRequest({ method, path, status: 0, ms: Date.now() - started });
          throw error;
        }
        const text = await response.text();
        let data = null;
        if (text) {
          try { data = JSON.parse(text); } catch { data = text; }
        }
        onRequest({ method, path, status: response.status, ms: Date.now() - started });
        if (!response.ok) throw new AirpHttpError(errorMessage(data, response.statusText), response.status, data);
        return data;
      } finally {
        timed.cleanup();
      }
    }

    async function stream(path, body, handlers) {
      const started = Date.now();
      const callbacks = handlers || {};
      const timed = timedSignal(callbacks.signal, streamTimeoutMs);
      try {
        let response;
        try {
          response = await fetchImpl(base + path, {
            method: 'POST',
            headers: headers({ 'Content-Type': 'application/json', Accept: 'text/event-stream' }),
            body: JSON.stringify(body),
            signal: timed.signal,
          });
        } catch (error) {
          onRequest({ method: 'POST', path, status: 0, ms: Date.now() - started });
          throw error;
        }
        onRequest({ method: 'POST', path, status: response.status, ms: Date.now() - started });
        if (!response.ok) {
          const text = await response.text();
          let data = text;
          try { data = JSON.parse(text); } catch {}
          throw new AirpHttpError(errorMessage(data, response.statusText), response.status, data);
        }
        try {
          return await consumeSse(response, callbacks);
        } catch (error) {
          if (error instanceof AirpStreamError || (error && error.name === 'AbortError' && callbacks.signal && callbacks.signal.aborted)) throw error;
          throw new AirpStreamError('流式连接中断；请刷新历史确认写入状态', {
            code: 'stream_transport', retryable: false, commit_state: 'unknown',
          });
        }
      } finally {
        timed.cleanup();
      }
    }

    return { base, request, stream };
  }

  return { AirpHttpError, AirpStreamError, consumeSse, createClient, errorMessage, parseSseBlock };
});
