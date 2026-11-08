// Runs before the stylesheet's first paint: follow the system theme until
// the saved choice loads (store/settings.ts then sets data-theme for good),
// so a light desktop never flashes dark. tokens.css keys its light palette
// on data-theme alone.
document.documentElement.dataset.theme = matchMedia("(prefers-color-scheme: light)").matches
  ? "light"
  : "dark";
