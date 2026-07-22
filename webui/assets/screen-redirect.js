(function () {
  'use strict';
  const target = document.body.dataset.target;
  if (!target) return;
  const destination = new URL(target, location.href);
  for (const [key, value] of new URLSearchParams(location.search)) destination.searchParams.set(key, value);
  location.replace(destination.href);
})();
