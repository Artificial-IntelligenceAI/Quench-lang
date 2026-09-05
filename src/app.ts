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
   The numbers come from `data/language.json`, written by tools/count-language.py
   from the compiler's own source. The page used to do this itself, which meant
   every visitor fetched 226 KB of Rust so that three integers could be printed —
   about six times the weight of the entire site, on every load.

   It is no longer worked out while you watch, so it says which commit it was
   working from instead. A number that has gone stale is then visible rather than
   quietly wrong, which was the only thing doing it live ever bought. */

interface Category {
  readonly id: string;
  readonly label: string;
  readonly count: number;
  readonly words: readonly string[];
  readonly missing: readonly string[];
  /** The file this group was read out of, or null when it is a list kept by hand.
      A derived group cannot go stale; a listed one can only notice a word leaving,
      never one arriving. */
  readonly readFrom: string | null;
}

interface Counted {
  readonly readFrom: { readonly commit: string; readonly date: string };
  readonly reserved: number | null;
  readonly symbols: number | null;
  readonly words: number;
  readonly inModules: number;
  readonly confirmed: number;
  readonly missing: readonly string[];
  readonly categories: readonly Category[];
  readonly tokens: readonly Group[];
}

/** A group of things the page can list: a category of words, or a kind of token. */
interface Group {
  readonly id: string;
  readonly label: string;
  readonly count: number;
  readonly words: readonly string[];
  readonly missing?: readonly string[];
}

/** Fills a picker from whatever groups it is handed and shows the one chosen.
    Nothing about the groups is written down here — they come out of the generated
    file, so a group added to the compiler appears with no change to this. */
function wirePicker(pickerId: string, countId: string, listId: string,
                    groups: readonly Group[], unit: string): void {
  const picker = document.getElementById(pickerId);
  const total = document.getElementById(countId);
  const list = document.getElementById(listId);
  if (!(picker instanceof HTMLSelectElement) || total === null || list === null) {
    return;
  }

  picker.replaceChildren();
  for (const group of groups) {
    const option = document.createElement("option");
    option.value = group.id;
    option.textContent = group.label;
    picker.appendChild(option);
  }

  const draw = (): void => {
    const chosen = groups.find((group) => group.id === picker.value) ?? groups[0];
    if (chosen === undefined) {
      return;
    }
    total.textContent = `${String(chosen.count)} ${unit}${chosen.count === 1 ? "" : "s"}`;
    list.replaceChildren();
    for (const word of chosen.words) {
      const item = document.createElement("li");
      /* A spelling is set in the mono face; a kind described in words is not one. */
      if (word.includes(" ")) {
        item.textContent = word;
        item.classList.add("said");
      } else {
        const code = document.createElement("code");
        code.textContent = word;
        item.appendChild(code);
      }
      if (chosen.missing?.includes(word) === true) {
        item.classList.add("gone");
        item.title = "No longer found in the source";
      }
      list.appendChild(item);
    }
  };

  picker.addEventListener("change", draw);
  draw();
}

/** Everything in a set of groups, as a group of its own. */
function allOf(id: string, label: string, groups: readonly Group[]): Group {
  /* A word can stand in two groups — `module` names a block and also says how far
     a name reaches — so it appears once here and once in each group it belongs to. */
  const words = [...new Set(groups.flatMap((group) => [...group.words]))];
  return {
    id, label, count: words.length, words,
    missing: groups.flatMap((group) => [...(group.missing ?? [])]),
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
   have no numbers on them, so they do not go asking for the file. */
if (document.getElementById("reserved") !== null) {
  void fetch("data/language.json")
    .then((response) => {
      if (!response.ok) {
        throw new Error(`answered ${String(response.status)}`);
      }
      return response.json() as Promise<Counted>;
    })
    .then(
      (counted) => {
        show("reserved", counted.reserved);
        show("words", counted.words);
        show("symbols", counted.symbols);
        /* Ask which groups are derived, not how many words — a word standing in two
           groups makes the sum larger than the number of words there are. */
        const listed = counted.categories.filter((category) => category.readFrom === null);
        const derived = listed.length === 0
          ? counted.words
          : counted.categories
              .filter((category) => category.readFrom !== null)
              .reduce((sum, category) => sum + category.count, 0);
        const drift = counted.missing.length === 0
          ? ""
          : ` ${String(counted.missing.length)} are no longer in it.`;
        const how = derived === counted.words
          ? "every one read out of the constants the compiler itself uses"
          : `${String(derived)} of them read out of the compiler rather than listed`;
        say(`Counted from the compiler's own source at ${counted.readFrom.commit}, `
          + `${counted.readFrom.date}: ${String(counted.words)} words in front of a module `
          + `and ${String(counted.inModules)} behind one, ${how}.${drift}`);
        const words = [allOf("everything", "Everything", counted.categories), ...counted.categories];
        wirePicker("category", "category-count", "category-words", words, "word");
        const tokens = [...counted.tokens, allOf("everyKind", "Every kind", counted.tokens)];
        wirePicker("token", "token-count", "token-words", tokens, "kind");
      },
      (reason: unknown) => {
        say(`Could not read the counts, so nothing above is claimed: ${String(reason)}`);
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

wire("copying", () => {
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
});

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

/** Runs one page's worth of wiring. Every section below is independent, but they
    share a file and a top level, so without this a throw in an early one silently
    takes every later one with it — and the language picker, being last, would be
    the first thing to disappear. */
function wire(what: string, work: () => void): void {
  try {
    work();
  } catch (reason: unknown) {
    console.error(`${what} did not start:`, reason);
  }
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

wire("descriptions", () => {
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
});

/* --- Comparing two languages -------------------------------------------- */

/** One thing two languages can disagree about. */
interface Axis {
  readonly id: string;
  readonly label: string;
}

interface Language {
  readonly id: string;
  readonly name: string;
  /** The version these answers were checked against — not whatever is current.
      A newer release could change any of them, and showing its number beside an
      answer nobody re-ran would be a claim about a version never tested. */
  readonly version: string;
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

/* Quench's answers are the ones the README already gives. Go's were produced by
   running them on go1.26.5 rather than recalled — including the row where
   `0.1 + 0.2 == 0.3` is true and false in the same language, depending on whether
   the values are constants. */
const LANGUAGES: readonly Language[] = [
  {
    id: "quench",
    name: "Quench",
    version: "0.0.0",
    on: {
      reserved: "None. Every word the language uses is still yours to name something.",
      names: "Between marks, everywhere — `'a name'`. A bare word is never a name.",
      tenths: "Exactly 0.3 under `e`, its unbounded exact rationals. The binary floats are there when asked for by name.",
      precedence: "Only what mathematics settled. Everything programming invented takes brackets — `mod`, `and` against `or`, bitwise.",
      output: "Said every time: `print.stdout[…]` or `print.stderr[…]`. There is no default.",
      conversion: "None on its own, and both directions are written. `call stitch[…]` makes text of a number; `call as.i64[…]` reads one back, and stops the program unless `call is.i64[…]` was asked first.",
      running: "Compiles once to one artefact; the machine decides how to run it. Two of four ways exist today.",
    },
  },
  {
    id: "go",
    name: "Go",
    version: "1.26.5",
    on: {
      reserved: "Twenty-five, unchanged since Go 1.0. `len` and `nil` are not among them and can be shadowed; `if` and `range` are not yours.",
      names: "A bare identifier, and its first letter decides who can see it — capitalised is exported, lower case is not.",
      tenths: "Both. `0.1 + 0.2 == 0.3` is true for untyped constants, which are evaluated exactly, and false once they are float64 variables — 0.30000000000000004.",
      precedence: "Five binary levels, and `&` binds tighter than `==` — C's trap fixed rather than inherited.",
      output: "`fmt.Println` goes to standard output. The built-in `println` goes to standard error, and nothing about writing it says so.",
      conversion: "None between types: `int64(n)` is written out. Untyped constants adapt to their context on their own.",
      running: "Compiled ahead of time to one native binary with its runtime inside it. Cross-compiling is two environment variables at build time.",
    },
  },
  {
    id: "zig",
    name: "Zig",
    version: "0.16.0",
    on: {
      reserved: "Forty-six — but `@\"…\"` makes any string an identifier, so `@\"if\"` is a name, and so is `@\"a name with spaces\"`. The nearest thing to Quench's marks in a language that also has bare words.",
      names: "A bare identifier, or `@\"anything at all\"` where the bare form will not do — keywords and spaces included.",
      tenths: "False, and unlike Go it stays false at compile time: `comptime_float` is binary too. At runtime, `0.30000000000000004`.",
      precedence: "`&` binds tighter than `==`, so C's trap is fixed. Mixing `and` with `or` needs no parentheses.",
      output: "`std.debug.print` goes to standard error, and nothing in the name says so. Standard output is a separate and wordier thing.",
      conversion: "Widening is implicit; narrowing is refused. An `i64` put into an `i32` is `expected type 'i32', found 'i64'` until `@intCast` is written.",
      running: "Compiled ahead of time to a native binary, with no garbage collector and no runtime to speak of. `comptime` runs code during the compile. Cross-compiling is a first-class feature.",
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
    option.textContent = `${language.name} ${language.version}`;
    option.selected = language.id === chosen;
    picker.appendChild(option);
  }
}

/** Answers are written with `backticks` round anything that is code. Split on
    them and build real elements, rather than handing a string to innerHTML. */
function writeAnswer(cell: HTMLElement, answer: string): void {
  const pieces = answer.split("`");
  pieces.forEach((piece, index) => {
    if (piece === "") {
      return;
    }
    if (index % 2 === 1) {
      const code = document.createElement("code");
      code.textContent = piece;
      cell.appendChild(code);
    } else {
      cell.appendChild(document.createTextNode(piece));
    }
  });
}

/** A column heading is the language and the version its answers were taken from. */
function writeHead(head: HTMLElement | null, language: Language): void {
  if (head === null) {
    return;
  }
  head.replaceChildren(document.createTextNode(language.name));
  const version = document.createElement("span");
  version.className = "ver";
  version.textContent = language.version;
  head.appendChild(version);
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

  writeHead(leftHead, first);
  writeHead(rightHead, second);

  body.replaceChildren();
  for (const axis of AXES) {
    const row = document.createElement("tr");

    const what = document.createElement("th");
    what.scope = "row";
    what.textContent = axis.label;
    row.appendChild(what);

    for (const language of [first, second]) {
      const cell = document.createElement("td");
      writeAnswer(cell, language.on[axis.id] ?? "—");
      row.appendChild(cell);
    }

    /* Worth seeing at a glance which rows are the disagreements. */
    if (first.id !== second.id && first.on[axis.id] === second.on[axis.id]) {
      row.classList.add("agreed");
    }
    body.appendChild(row);
  }
}

wire("the language picker", () => {
const leftPick = document.getElementById("left");
const rightPick = document.getElementById("right");
if (leftPick instanceof HTMLSelectElement && rightPick instanceof HTMLSelectElement) {
  /* Populated fresh every load. Anything a browser restored into these from a
     previous visit is discarded rather than argued with. */
  leftPick.replaceChildren();
  rightPick.replaceChildren();
  fillPicker(leftPick, "quench");
  fillPicker(rightPick, "go");
  const again = (): void => {
    compare(leftPick, rightPick);
  };
  leftPick.addEventListener("change", again);
  rightPick.addEventListener("change", again);
  again();
}
});
