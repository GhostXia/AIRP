(function () {
  'use strict';
  let onboarded = false;
  try { onboarded = localStorage.getItem('airp_onboarded') === 'true'; } catch {}
  const target = new URL(onboarded ? 'screens/01-role-list.html' : 'screens/16-onboarding.html', location.href);
  target.search = location.search;
  location.replace(target.href);
})();
