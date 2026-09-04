type Theme = "glass" | "solarized-dark" | "solarized-light";

const THEME_KEY = "quench.theme";
const COUNT_KEY = "quench.visits";

const THEMES: readonly Theme[] = ["glass", "solarized-dark", "solarized-light"];

function isTheme(value: string | null): value is Theme {
  return value !== null && (THEMES as readonly string[]).includes(value);
}

/** Reads a key without caring why it failed. A private window has no storage
    and is not an error worth showing anybody. */
function remembered(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function remember(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Nothing to do. The page still works, it just forgets.
  }
}

function applyTheme(theme: Theme): void {
  document.documentElement.dataset["theme"] = theme;
  for (const button of buttons) {
    button.setAttribute("aria-pressed", String(button.dataset["setTheme"] === theme));
  }
  remember(THEME_KEY, theme);
}

/** The whole site. It counts refreshes, because that is what was asked for. */
function countThisVisit(): void {
  const visits = Number(remembered(COUNT_KEY) ?? 0) + 1;
  remember(COUNT_KEY, String(visits));
  const output = document.getElementById("count");
  if (output !== null) {
    output.textContent = visits.toLocaleString();
  }
}

/** Points the specular highlight at the cursor, which is what sells it as glass. */
function trackSheen(panel: HTMLElement): void {
  panel.addEventListener("pointermove", (event: PointerEvent) => {
    const box = panel.getBoundingClientRect();
    panel.style.setProperty("--mx", `${event.clientX - box.left}px`);
    panel.style.setProperty("--my", `${event.clientY - box.top}px`);
  });
  panel.addEventListener("pointerleave", () => {
    panel.style.removeProperty("--mx");
    panel.style.removeProperty("--my");
  });
}

const buttons = document.querySelectorAll<HTMLButtonElement>("[data-set-theme]");

for (const button of buttons) {
  button.addEventListener("click", () => {
    const next = button.dataset["setTheme"] ?? null;
    if (isTheme(next)) {
      applyTheme(next);
    }
  });
}

for (const panel of document.querySelectorAll<HTMLElement>(".glass")) {
  trackSheen(panel);
}

const stored = remembered(THEME_KEY);
applyTheme(isTheme(stored) ? stored : "glass");
countThisVisit();
