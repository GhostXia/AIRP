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
      const current = await load(characterId);
      return { response, current };
    }

    return { load, generate, writePreviewAndReload };
  }

  return { create, mesExample };
});
