// Engine SSE 合同解析（protocol/sse-events.json）。
//
// 探索 runner 的消费策略是 fail-closed：
// - event:error 与 typed error data 必须同时出现，并立即抛出结构化错误；
// - EOF 会先解析没有最终空行的残余帧，但只有 {type:"done"} 终态才算成功；
// - 未知 message type 按合同的 additive-only 规则忽略。
//
// 解析状态机放在本模块中。runner 不再维护一份 page.evaluate 内联副本，而是
// 取得浏览器上下文中的原始 SSE 文本后调用这里的同一实现。

/**
 * 结构化 SSE 合同错误。
 *
 * `code` 是 parser/engine 错误代码；typed engine error 的完整 envelope 位于
 * `engineError`，而 `details.content` 保留已经收到的正文（包括 EOF 残余前缀）。
 */
export class SseProtocolError extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.name = 'SseProtocolError';
    this.code = code;
    this.details = details;
    if (details.engineError) this.engineError = details.engineError;
  }
}

function protocolError(code, message, details, content) {
  return new SseProtocolError(code, message, { ...details, content });
}

/**
 * 创建一个可按任意传输块推进的 Engine SSE 正文解析器。
 *
 * @returns {{ push(chunk: string): void, finish(): {content: string} }}
 */
export function createSseContentParser() {
  let buffer = '';
  let content = '';
  let terminal = null;

  const dispatch = frame => {
    let event = 'message';
    const data = [];
    for (const rawLine of frame.split(/\r?\n/)) {
      if (!rawLine || rawLine.startsWith(':')) continue;
      const separator = rawLine.indexOf(':');
      const field = separator < 0 ? rawLine : rawLine.slice(0, separator);
      let value = separator < 0 ? '' : rawLine.slice(separator + 1);
      if (value.startsWith(' ')) value = value.slice(1);
      if (field === 'event') event = value || 'message';
      else if (field === 'data') data.push(value);
    }
    if (!data.length) {
      if (event === 'error') {
        throw protocolError(
          'malformed_error',
          'SSE error event has no data payload',
          { event },
          content,
        );
      }
      return;
    }

    const payloadText = data.join('\n');
    // Some upstream-compatible providers still send OpenAI's [DONE] sentinel.
    // It has no Engine body semantics, so retain the existing compatibility rule.
    if (!payloadText.trim()) {
      if (event === 'error') {
        throw protocolError(
          'malformed_error',
          'SSE error event has an empty data payload',
          { event, payloadText },
          content,
        );
      }
      return;
    }
    if (event !== 'error' && payloadText === '[DONE]') return;
    let payload;
    try {
      payload = JSON.parse(payloadText);
    } catch (error) {
      throw protocolError(
        event === 'error' ? 'malformed_error' : 'malformed',
        `SSE ${event} frame JSON parse failed: ${error?.message || String(error)}`,
        { event, payloadText },
        content,
      );
    }

    const eventIsError = event === 'error';
    const payloadIsError = payload?.type === 'error';
    if (eventIsError !== payloadIsError) {
      throw protocolError(
        'schema_mismatch',
        'SSE error event/type fields disagree',
        { event, payload },
        content,
      );
    }

    if (eventIsError && payloadIsError) {
      const detail = payload?.error;
      const envelopeValid =
        typeof payload?.text === 'string' &&
        detail &&
        typeof detail.code === 'string' &&
        typeof detail.message === 'string' &&
        typeof detail.retryable === 'boolean' &&
        typeof detail.commit_state === 'string';
      if (!envelopeValid) {
        throw protocolError(
          'malformed_error',
          'SSE error frame lacks the typed error envelope',
          { event, payload },
          content,
        );
      }
      throw protocolError(
        detail.code,
        detail.message || payload.text,
        { kind: 'engine_error', event, payload, engineError: detail },
        content,
      );
    }

    if (event !== 'message') {
      throw protocolError('invalid_event', `unexpected SSE event: ${event}`, { event, payload }, content);
    }

    if (payload?.type === 'done') {
      terminal = 'done';
      return;
    }

    if (payload?.type === 'body_chunk' || payload?.type === 'think_chunk') {
      if (typeof payload.text !== 'string') {
        throw protocolError(
          'malformed',
          `${payload.type} frame has no text string`,
          { event, payload },
          content,
        );
      }
      if (payload.type === 'body_chunk') content += payload.text;
    }
    // action_options and unknown message types have no reply-body semantics.
  };

  const parser = {
    push(chunk) {
      buffer += String(chunk);
      let separator;
      while ((separator = buffer.match(/\r?\n\r?\n/))) {
        const frame = buffer.slice(0, separator.index);
        buffer = buffer.slice(separator.index + separator[0].length);
        if (terminal) {
          if (frame.trim()) {
            throw protocolError(
              'post_terminal',
              'SSE frame received after a terminal frame',
              { terminal, frame },
              content,
            );
          }
          continue;
        }
        dispatch(frame);
      }
    },

    finish() {
      // SSE dispatch normally requires an empty line. At EOF, process the
      // residual frame once so a body chunk (or done frame) is not silently lost.
      if (buffer.trim()) {
        const residual = buffer;
        buffer = '';
        if (terminal) {
          throw protocolError(
            'post_terminal',
            'SSE frame received after a terminal frame',
            { terminal, frame: residual },
            content,
          );
        }
        dispatch(residual);
      }
      if (terminal === 'done') return { content };
      throw protocolError(
        'eof',
        'SSE ended before a typed terminal frame',
        { terminal },
        content,
      );
    },
  };

  return parser;
}

/**
 * Parse a complete SSE response and return the generated reply body.
 *
 * @param {string} sseText
 * @returns {string}
 */
export function parseSseContent(sseText) {
  const parser = createSseContentParser();
  parser.push(sseText);
  return parser.finish().content;
}
