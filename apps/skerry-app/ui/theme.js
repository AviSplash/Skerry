"use strict";

// Apply the saved theme ("system", "light" or "dark") before the page paints,
// so Skerry never flashes the wrong colors when it opens. app.js changes it.
(function () {
  let theme = "system";
  try {
    theme = localStorage.getItem("skerry-theme") || "system";
  } catch {
    // Storage can be unavailable; follow the system then.
  }
  document.documentElement.dataset.theme = theme;
})();
