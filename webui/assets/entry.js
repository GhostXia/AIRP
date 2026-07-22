(function () {
  'use strict';
  const target = new URL('screens/01-role-list.html', location.href);
  target.search = location.search;
  location.replace(target.href);
})();
