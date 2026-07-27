const MESSAGE_LIKE = /message|msg|chat|memory|history|conversation|reply|content/i;

export function sanitizeDomSnapshot(snapshot) {
  return snapshot.map(element => {
    const scope = [
      element.id || '',
      ...(Array.isArray(element.classes) ? element.classes : []),
      element.role || '',
    ].join(' ');
    if (element.text && (element.sensitive === true || MESSAGE_LIKE.test(scope))) {
      return { ...element, text: '[REDACTED]' };
    }
    return element;
  });
}
