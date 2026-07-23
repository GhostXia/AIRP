// OpenAI 兼容 LLM 客户端。与 engine provider 解耦：runner 自带 client。
// 环境变量：OPENAI_BASE_URL, OPENAI_API_KEY, OPENAI_MODEL（默认 gpt-4o-mini）

const BASE_URL = process.env.OPENAI_BASE_URL || 'https://api.openai.com/v1';
const API_KEY = process.env.OPENAI_API_KEY;
const MODEL = process.env.OPENAI_MODEL || 'gpt-4o-mini';

if (!API_KEY) {
  console.error('[llm-client] OPENAI_API_KEY is required for agent exploration');
  process.exit(2);
}

export async function chatCompletion(messages, { maxTokens = 2048, temperature = 0.2, timeoutMs = 60000 } = {}) {
  // Bounded deadline: 防止 stalled provider 把 task/workflow 拖到 30 分钟超时
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  let res;
  try {
    res = await fetch(BASE_URL + '/chat/completions', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': 'Bearer ' + API_KEY,
      },
      body: JSON.stringify({ model: MODEL, messages, max_tokens: maxTokens, temperature }),
      signal: controller.signal,
    });
  } catch (err) {
    if (err && err.name === 'AbortError') {
      throw new Error('LLM request timed out after ' + timeoutMs + 'ms (provider stalled?)');
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`LLM ${res.status}: ${text}`);
  }
  const json = await res.json();
  return json.choices?.[0]?.message?.content || '';
}

export function getModel() { return MODEL; }
