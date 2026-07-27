(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPDialogueFlow = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  function mesExample(card) {
    const data = card && card.data ? card.data : card;
    return data && typeof data.mes_example === 'string' ? data.mes_example : '';
  }

  function create(client) {
    async function load(characterId) {
      const card = await client.request(
        'GET',
        '/v1/characters/' + encodeURIComponent(characterId),
      );
      return { card, mesExample: mesExample(card) };
    }

    function generate(characterId, body) {
      return client.request(
        'POST',
        '/v1/characters/' + encodeURIComponent(characterId) + '/dialogue-examples',
        body,
      );
    }

    async function writePreviewAndReload(characterId, preview, append) {
      if (!preview || typeof preview.mes_example !== 'string' || !preview.mes_example) {
        throw new Error('无可写入的预览内容');
      }
      const response = await generate(characterId, {
        dry_run: false,
        append: Boolean(append),
        mes_example_override: preview.mes_example,
      });
      // Write已提交——reload 仅作落盘验证，失败不能让调用方误以为写入失败。
      // 否则 append 模式下用户重试会把同一内容二次追加，导致 mes_example 重复。
      let current = null;
      try {
        current = await load(characterId);
      } catch {
        // reload 失败时 response 仍代表已提交的写入结果。
      }
      return { response, current };
    }

    return { load, generate, writePreviewAndReload };
  }

  return { create, mesExample };
});
