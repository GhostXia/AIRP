import test from 'node:test';
import assert from 'node:assert/strict';
import { sanitizeDomSnapshot } from './dom-privacy.mjs';

test('redacts leaf text marked sensitive by a message-like ancestor', () => {
  const snapshot = [
    {
      tag: 'span',
      id: null,
      classes: [],
      role: null,
      text: 'private user message',
      ariaLabel: 'private message preview',
      sensitive: true,
    },
  ];

  assert.deepEqual(sanitizeDomSnapshot(snapshot), [
    {
      ...snapshot[0],
      text: '[REDACTED]',
      ariaLabel: '[REDACTED]',
    },
  ]);
});

test('retains direct attribute matching as a fallback', () => {
  const snapshot = [
    {
      tag: 'div',
      id: 'conversation-reply',
      classes: [],
      role: null,
      text: 'private assistant reply',
      sensitive: false,
    },
  ];

  assert.equal(sanitizeDomSnapshot(snapshot)[0].text, '[REDACTED]');
});

test('leaves unrelated interface labels intact', () => {
  const snapshot = [
    {
      tag: 'button',
      id: 'save-settings',
      classes: ['btn'],
      role: null,
      text: 'Save',
      sensitive: false,
    },
  ];

  assert.deepEqual(sanitizeDomSnapshot(snapshot), snapshot);
});
