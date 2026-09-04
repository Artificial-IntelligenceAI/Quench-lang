type Theme = "glass" | "solarized-dark" | "solarized-light";

const THEME_KEY = "quench.theme";

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

for (const panel of document.querySelectorAll<HTMLElement>(".glass:not(.sheet)")) {
  trackSheen(panel);
}

const stored = remembered(THEME_KEY);
applyTheme(isTheme(stored) ? stored : "glass");

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

/* Only the start page carries the panels. The other pages share this script and
   have no numbers on them, so they do not go asking for the source. */
if (document.getElementById("reserved") !== null) {
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
}

/* --- Copying a number --------------------------------------------------- */

/** The number and what it counts, as a sentence: `54 non-reserved keywords`. */
function readingOf(panel: Element): string | null {
  const shown = panel.querySelector(".stat-count")?.textContent?.trim() ?? "";
  const unit = panel.querySelector(".stat-label")?.textContent?.trim() ?? "";
  if (shown === "" || shown === "—" || unit === "") {
    return null;
  }
  return `${shown} ${unit.charAt(0).toLowerCase()}${unit.slice(1)}`;
}

function flash(button: HTMLButtonElement): void {
  const mark = button.querySelector<SVGElement>(".mark");
  const done = button.querySelector<SVGElement>(".done");
  if (mark === null || done === null) {
    return;
  }
  button.classList.add("copied");
  mark.toggleAttribute("hidden", true);
  done.toggleAttribute("hidden", false);
  button.setAttribute("aria-label", "Copied");
  window.setTimeout(() => {
    button.classList.remove("copied");
    mark.toggleAttribute("hidden", false);
    done.toggleAttribute("hidden", true);
    button.setAttribute("aria-label", "Copy");
  }, 1400);
}

/** The old way, kept because the new one is refused more often than it ought to
    be — an insecure origin, a browser that wants a permission first, or an
    embedded view that simply says no. It must run inside the click itself, so it
    is tried before anything is awaited rather than after. */
function copyTheOldWay(text: string): boolean {
  const field = document.createElement("textarea");
  field.value = text;
  field.setAttribute("readonly", "");
  field.style.position = "fixed";
  field.style.top = "-1000px";
  document.body.appendChild(field);
  field.select();
  let done = false;
  try {
    done = document.execCommand("copy");
  } catch {
    done = false;
  }
  field.remove();
  return done;
}

for (const button of document.querySelectorAll<HTMLButtonElement>(".copy")) {
  button.addEventListener("click", () => {
    const target = copyTargetOf(button);
    if (target === null) {
      /* Nothing was counted, so there is nothing to hand over. */
      return;
    }
    const reading = target.text;

    /* Done inside the gesture, so that it still works when the promise below is
       refused — by then the click is over and the old way is no longer allowed. */
    const already = copyTheOldWay(reading);
    if (already) {
      flash(button);
    }

    void navigator.clipboard?.writeText(reading).then(
      () => flash(button),
      () => {
        if (!already) {
          button.setAttribute("aria-label", "Could not copy");
        }
      },
    );
  });
}

interface CopyTarget {
  readonly text: string;
  readonly description: string;
}

/** What a copy button is standing next to. A panel hands over its one reading;
    a sheet hands over everything written on it. */
function copyTargetOf(button: Element): CopyTarget | null {
  const panel = button.closest(".stat");
  if (panel !== null) {
    const reading = readingOf(panel);
    return reading === null ? null : { text: reading, description: `Copy “${reading}”` };
  }

  const sheet = button.closest<HTMLElement>(".sheet");
  if (sheet !== null) {
    /* `innerText` rather than `textContent`, because it is the laid-out text:
       headings and paragraphs come out on their own lines instead of running
       together. The button contributes nothing, being two drawings. */
    const written = sheet.innerText.replace(/\n{3,}/g, "\n\n").trim();
    return written === ""
      ? null
      : { text: written, description: "Copy everything written here" };
  }

  return null;
}

/* --- Descriptions ------------------------------------------------------- */

const tip = document.getElementById("tip");

/** What to say about a control. The copy buttons describe what they would put on
    the clipboard, which is not known until the numbers have arrived. */
function describe(control: HTMLElement): string | null {
  const written = control.dataset["tip"];
  if (written !== undefined && written !== "") {
    return written;
  }
  if (control.classList.contains("copy")) {
    const target = copyTargetOf(control);
    return target === null ? "Nothing counted yet, so nothing to copy." : target.description;
  }
  return null;
}

function hideTip(): void {
  if (tip !== null) {
    tip.classList.remove("shown");
    tip.hidden = true;
  }
}

function showTip(control: HTMLElement): void {
  if (tip === null) {
    return;
  }
  const words = describe(control);
  if (words === null) {
    return;
  }
  tip.textContent = words;
  tip.hidden = false;

  /* Measured after it is in the layout, so the clamping below knows its width. */
  const box = control.getBoundingClientRect();
  const own = tip.getBoundingClientRect();
  const margin = 8;
  const left = Math.min(
    Math.max(margin, box.left + box.width / 2 - own.width / 2),
    window.innerWidth - own.width - margin,
  );
  /* Above by preference: a copy button sits inside the panel it describes, and a
     tooltip under it would cover the very number it is naming. The bar's buttons
     have nothing above them, so they fall through to below on their own. */
  const above = box.top - own.height - margin;
  const roomAbove = above >= margin;
  tip.style.left = `${left}px`;
  tip.style.top = `${roomAbove ? above : box.bottom + margin}px`;
  tip.classList.add("shown");
}

for (const control of document.querySelectorAll<HTMLElement>("[data-tip], .copy")) {
  control.addEventListener("pointerenter", (event: PointerEvent) => {
    /* A touch has no hover, and a tooltip that appears on tap is just a delay. */
    if (event.pointerType !== "touch") {
      showTip(control);
    }
  });
  control.addEventListener("pointerleave", hideTip);
  control.addEventListener("focus", () => showTip(control));
  control.addEventListener("blur", hideTip);
}

window.addEventListener("keydown", (event: KeyboardEvent) => {
  if (event.key === "Escape") {
    hideTip();
  }
});

window.addEventListener("scroll", hideTip, { passive: true });

/* --- Comparing two languages -------------------------------------------- */

/** One thing two languages can disagree about. */
interface Axis {
  readonly id: string;
  readonly label: string;
}

interface Language {
  readonly id: string;
  readonly name: string;
  /** Keyed by axis id. A missing answer shows as a dash rather than a blank. */
  readonly on: Readonly<Record<string, string>>;
}

const AXES: readonly Axis[] = [
  { id: "reserved", label: "Reserved words" },
  { id: "names", label: "How a name is written" },
  { id: "tenths", label: "0.1 + 0.2" },
  { id: "precedence", label: "Precedence" },
  { id: "output", label: "Where output goes" },
  { id: "conversion", label: "Implicit conversion" },
  { id: "running", label: "How it runs" },
];

/* Quench's answers are the ones the README already gives. Test and Test2 are
   placeholders, and say so, so that changing the pickers is visibly doing
   something before there is a second real language to put here. */
const LANGUAGES: readonly Language[] = [
  {
    id: "quench",
    name: "Quench",
    on: {
      reserved: "None. Every word the language uses is still yours to name something.",
      names: "Between marks, everywhere — 'a name'. A bare word is never a name.",
      tenths: "Exactly 0.3 under e, its unbounded exact rationals. Binary floats are there when asked for by name.",
      precedence: "Only what mathematics settled. Everything programming invented takes brackets.",
      output: "Said every time: print.stdout[…] or print.stderr[…]. There is no default.",
      conversion: "None. call stitch[…] is the one conversion, and it is written down.",
      running: "Compiles once to one artefact; the machine decides how to run it. Two of four ways exist today.",
    },
  },
  {
    id: "test",
    name: "Test",
    on: {
      reserved: "Placeholder. Test is not a language.",
      names: "Placeholder. Test is not a language.",
      tenths: "Placeholder. Test is not a language.",
      precedence: "Placeholder. Test is not a language.",
      output: "Placeholder. Test is not a language.",
      conversion: "Placeholder. Test is not a language.",
      running: "Placeholder. Test is not a language.",
    },
  },
  {
    id: "test2",
    name: "Test2",
    on: {
      reserved: "Placeholder the second.",
      names: "Placeholder the second.",
      tenths: "Placeholder the second.",
      precedence: "Placeholder the second.",
      output: "Placeholder the second.",
      conversion: "Placeholder the second.",
      running: "Placeholder the second.",
    },
  },
];

function languageBy(id: string): Language | undefined {
  return LANGUAGES.find((language) => language.id === id);
}

function fillPicker(picker: HTMLSelectElement, chosen: string): void {
  for (const language of LANGUAGES) {
    const option = document.createElement("option");
    option.value = language.id;
    option.textContent = language.name;
    option.selected = language.id === chosen;
    picker.appendChild(option);
  }
}

function compare(left: HTMLSelectElement, right: HTMLSelectElement): void {
  const first = languageBy(left.value);
  const second = languageBy(right.value);
  const body = document.getElementById("compare-body");
  const leftHead = document.getElementById("left-head");
  const rightHead = document.getElementById("right-head");
  if (first === undefined || second === undefined || body === null) {
    return;
  }

  if (leftHead !== null) leftHead.textContent = first.name;
  if (rightHead !== null) rightHead.textContent = second.name;

  body.replaceChildren();
  for (const axis of AXES) {
    const row = document.createElement("tr");

    const what = document.createElement("th");
    what.scope = "row";
    what.textContent = axis.label;
    row.appendChild(what);

    for (const language of [first, second]) {
      const cell = document.createElement("td");
      cell.textContent = language.on[axis.id] ?? "—";
      row.appendChild(cell);
    }

    /* Worth seeing at a glance which rows are the disagreements. */
    if (first.id !== second.id && first.on[axis.id] === second.on[axis.id]) {
      row.classList.add("agreed");
    }
    body.appendChild(row);
  }
}

const leftPick = document.getElementById("left");
const rightPick = document.getElementById("right");
if (leftPick instanceof HTMLSelectElement && rightPick instanceof HTMLSelectElement) {
  fillPicker(leftPick, "quench");
  fillPicker(rightPick, "test");
  const again = (): void => {
    compare(leftPick, rightPick);
  };
  leftPick.addEventListener("change", again);
  rightPick.addEventListener("change", again);
  again();
}
