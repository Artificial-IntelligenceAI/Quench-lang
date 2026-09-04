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

/** Points the specular highlight at the cursor, which is what sells it as glass.
    Nothing resets it on the way out: a light that snapped back to the middle the
    moment you stopped pointing at it would read as a cursor effect, and the whole
    idea is that the panel caught the light and kept it. */
function trackSheen(panel: HTMLElement): void {
  panel.addEventListener("pointermove", (event: PointerEvent) => {
    const box = panel.getBoundingClientRect();
    panel.style.setProperty("--mx", `${event.clientX - box.left}px`);
    panel.style.setProperty("--my", `${event.clientY - box.top}px`);
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

/* --- The language, counted -------------------------------------------------
   Every number below is checked against the source on main at load, so nothing
   here can quietly stop being true while the page keeps saying it. What cannot
   be confirmed is shown as a dash rather than guessed. */

const RAW = "https://raw.githubusercontent.com/Artificial-IntelligenceAI/Quench-lang/main/crates/";

const READS = [
  "quench-lex/src/token.rs",
  "quench-parse/src/lib.rs",
  "quench-parse/src/ast.rs",
  "quench-check/src/lib.rs",
] as const;

/** The words the language answers to, grouped by where a word means something.

    There is no single list inside the compiler to read — the lexer keeps none by
    design, and the parser, the tree and the checker each recognise their own — so
    this is the list, and the page checks it against the source rather than asking
    to be believed. None of these is reserved: every one of them is still available
    as a name, because a name wears marks and a bare word does not. */
const WORDS: Readonly<Record<string, readonly string[]>> = {
  statements: ["var", "set", "add", "print", "call", "give", "if", "else", "loop", "break"],
  topLevel: ["fn", "const", "START"],
  chainLinks: ["mut", "immut", "arr", "grow", "temp", "perm", "range", "while", "nothing"],
  visibility: ["file", "program", "export"],
  types: ["i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64",
          "b16", "b32", "b64", "d32", "d64", "e", "bool", "str"],
  operators: ["x", "mod", "and", "or", "not"],
  beforeAValue: ["share", "copy"],
  literals: ["true", "false"],
  streams: ["stdout", "stderr"],
  provided: ["count", "stitch"],
};

const EVERY_WORD: readonly string[] = Object.values(WORDS).flat();

interface Counted {
  readonly reserved: number | null;
  readonly words: number;
  readonly confirmed: number;
  readonly symbols: number | null;
}

async function source(path: string): Promise<string> {
  const response = await fetch(RAW + path);
  if (!response.ok) {
    throw new Error(`${path} answered ${response.status}`);
  }
  return response.text();
}

async function countTheLanguage(): Promise<Counted> {
  const read = await Promise.all(READS.map(source));
  const all = read.join("\n");
  const token = read[0] ?? "";

  /* The zero is checked, not asserted. Should the lexer ever grow a keyword
     table, this stops saying nought and starts saying nothing at all. */
  const reservesNone = /reserves no words|reserves none/i.test(token);

  /* A symbol is a token kind whose name in an error message is its own spelling
     in backticks — which is exactly what separates `[` from "a written value". */
  const described = [...token.matchAll(/Kind::(\w+)\s*=>\s*"([^"]+)"/g)];
  const symbols = described.filter(([, , text]) => text?.startsWith("`") === true).length;

  /* Quoted or backticked: most words appear as string literals, but the two
     streams are only ever named in the tree's documentation. */
  const confirmed = EVERY_WORD.filter((word) =>
    new RegExp(`["\`]${word}["\`]`).test(all)).length;

  return {
    reserved: reservesNone ? 0 : null,
    words: EVERY_WORD.length,
    confirmed,
    symbols: symbols === 0 ? null : symbols,
  };
}

function show(id: string, value: number | null): void {
  const element = document.getElementById(id);
  if (element !== null) {
    element.textContent = value === null ? "—" : value.toLocaleString();
  }
}

function say(message: string): void {
  const element = document.getElementById("source");
  if (element !== null) {
    element.textContent = message;
  }
}

void countTheLanguage().then(
  (counted) => {
    show("reserved", counted.reserved);
    show("words", counted.words);
    show("symbols", counted.symbols);
    const drift = counted.confirmed === counted.words
      ? "all of them found in it"
      : `${counted.words - counted.confirmed} of them no longer in it`;
    say(`Read from the source on main, just now: ${String(counted.words)} words, ${drift}.`);
  },
  (reason: unknown) => {
    say(`Could not reach the source, so nothing above is claimed: ${String(reason)}`);
  },
);
