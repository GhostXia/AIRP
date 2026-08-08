// Engine SSE 合同解析（protocol/sse-events.json）：
// message 事件的 data 为 JSON 对象，dataDiscriminator = "type"，
// 正文块为 {type: "body_chunk", text: "..."}；think_chunk 是心理独白
//（UI 折叠框，不算回复正文）；done / action_options / error 无正文语义。
//
// #522 根因修复：runner 的 sseCall 此前按 OpenAI 风格的
// {role: "assistant", content} 解析，与 engine 实际合同（type/text）完全
// 错位，导致所有依赖 chat completion 内容的探索任务必然拿到空 content。
// 本模块锁定合同语义供测试验证；runner 内 page.evaluate 无法 import
// 模块，其内联实现与此处保持一致（修改任一处须同步另一处）。

/**
 * 从 SSE 文本中提取回复正文（所有 body_chunk 帧的 text 拼接）。
 * 非 JSON 帧（如 "data: [DONE]" 终止标记）与未知 type 帧忽略。
 * @param {string} sseText
 * @returns {string}
 */
export function parseSseContent(sseText) {
  let out = '';
  for (const line of String(sseText).split('\n')) {
    if (!line.startsWith('data:')) continue;
    const data = line.slice(5).trim();
    if (!data) continue;
    try {
      const parsed = JSON.parse(data);
      if (parsed && parsed.type === 'body_chunk' && typeof parsed.text === 'string') {
        out += parsed.text;
      }
    } catch {
      // 非 JSON 帧忽略（合同扩展规则：未知 type 应透传忽略而非失败）。
    }
  }
  return out;
}
