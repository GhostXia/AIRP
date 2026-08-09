/* AIRP confirmation dialog
 *
 * A small, shared confirmation surface for destructive or otherwise
 * consequential actions.  The API deliberately returns a Promise so callers
 * keep their existing early-return cancellation semantics while using a
 * keyboard- and screen-reader-friendly dialog instead of the native browser
 * confirmation prompt.
 */
(function (root) {
  'use strict';

  const queue = [];
  let active = null;
  let dialog = null;
  let title = null;
  let description = null;
  let confirmButton = null;
  let cancelButton = null;

  function fallbackKeydown(event) {
    if (!active || !active.fallback) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      finish(false);
      return;
    }
    if (event.key !== 'Tab') return;
    // The fallback is a regular element rather than a modal dialog, so keep
    // keyboard focus inside its two actions while it is active.
    event.preventDefault();
    const target = document.activeElement === confirmButton ? cancelButton : confirmButton;
    target.focus();
  }

  function buildDialog() {
    if (dialog) return dialog;
    dialog = document.createElement('dialog');
    dialog.id = 'airp-confirm-dialog';
    dialog.className = 'airp-confirm-dialog';
    dialog.setAttribute('role', 'alertdialog');
    dialog.setAttribute('aria-modal', 'true');
    title = document.createElement('h2');
    title.className = 'm-title';
    title.id = 'airp-confirm-title';
    description = document.createElement('p');
    description.className = 'm-desc';
    description.id = 'airp-confirm-description';
    dialog.setAttribute('aria-labelledby', title.id);
    dialog.setAttribute('aria-describedby', description.id);

    const actions = document.createElement('div');
    actions.className = 'm-actions';
    cancelButton = document.createElement('button');
    cancelButton.type = 'button';
    cancelButton.className = 'btn btn-secondary';
    cancelButton.addEventListener('click', () => finish(false));
    confirmButton = document.createElement('button');
    confirmButton.type = 'button';
    confirmButton.className = 'btn btn-danger-solid';
    confirmButton.addEventListener('click', () => finish(true));
    actions.append(cancelButton, confirmButton);
    const panel = document.createElement('div');
    panel.className = 'modal';
    panel.append(title, description, actions);
    dialog.append(panel);
    dialog.addEventListener('click', event => {
      if (active && active.fallback && event.target === dialog) finish(false);
    });
    dialog.addEventListener('cancel', event => {
      // Treat Escape exactly like the explicit cancel action, while keeping
      // the dialog open long enough for the promise to settle consistently.
      event.preventDefault();
      finish(false);
    });
    document.addEventListener('keydown', fallbackKeydown);
    document.body.appendChild(dialog);
    return dialog;
  }

  function finish(value) {
    if (!active || active.settled) return;
    active.settled = true;
    const current = active;
    active = null;
    if (dialog && dialog.open) {
      if (typeof dialog.close === 'function') dialog.close();
      else {
        dialog.removeAttribute('open');
        dialog.hidden = true;
      }
    }
    if (current.focus && typeof current.focus.focus === 'function') current.focus.focus();
    current.resolve(value);
    pump();
  }

  function pump() {
    if (active || !queue.length) return;
    active = queue.shift();
    const options = active.options || {};
    const modal = buildDialog();
    active.fallback = typeof modal.showModal !== 'function';
    title.textContent = options.title || '请确认操作';
    description.textContent = String(active.message == null ? '' : active.message);
    confirmButton.textContent = options.confirmLabel || '继续';
    cancelButton.textContent = options.cancelLabel || '取消';
    confirmButton.className = 'btn ' + (options.danger === false ? 'btn-primary' : 'btn-danger-solid');
    active.focus = document.activeElement;
    if (!active.fallback) {
      modal.showModal();
    } else {
      // A minimal fallback for embedded WebViews without HTMLDialogElement.
      modal.classList.add('is-fallback');
      modal.hidden = false;
      modal.setAttribute('open', '');
    }
    (options.danger === false ? confirmButton : cancelButton).focus();
  }

  function confirm(message, options) {
    return new Promise(resolve => {
      queue.push({ message, options: options || {}, resolve, settled: false });
      pump();
    });
  }

  root.AIRPConfirm = { confirm };
})(typeof globalThis !== 'undefined' ? globalThis : window);
