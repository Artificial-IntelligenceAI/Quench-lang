//! Turning Quench source into tokens, and saying so when it cannot.
//!
//! The lexer decides as little as it can get away with. It does not sort words into
//! keywords, because Quench reserves none — see [`Kind::Word`]. It does not read what is
//! written between bars, because what `|1000|` means is decided by the type it is given
//! to, and deciding it here would be deciding it in the wrong place.
//!
//! What it *does* do carefully is stop well. A lexer that gives up at the first odd
//! character reports one problem in a file that has four, so every error here skips to
//! somewhere it can believe in — usually the end of the line — and carries on. The result
//! is that a run reports what is wrong with a file rather than what is wrong with its
//! first line.

pub mod token;

pub use token::{Kind, Token};

use quench_diag::{Diagnostic, Span};

/// Everything a file turned into, and everything wrong with it.
///
/// Tokens are produced even when there are errors, because a parser that can keep going
/// finds more than one that stops. Whether to trust them is the caller's decision.
#[derive(Clone, Debug)]
pub struct Lexed {
    pub tokens: Vec<Token>,
    pub errors: Vec<Diagnostic>,
}

impl Lexed {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Read a whole file.
pub fn lex(source: &str) -> Lexed {
    Lexer { source, at: 0, tokens: Vec::new(), errors: Vec::new() }.run()
}

struct Lexer<'a> {
    source: &'a str,
    at: usize,
    tokens: Vec<Token>,
    errors: Vec<Diagnostic>,
}

impl<'a> Lexer<'a> {
    fn run(mut self) -> Lexed {
        while let Some(c) = self.peek() {
            match c {
                // Whitespace carries no meaning outside quotes, including newlines: a
                // statement ends at its semicolon, not at the edge of the paper.
                c if c.is_whitespace() => self.at += c.len_utf8(),
                '#' => self.comment(),
                '[' => self.single(Kind::OpenList),
                ']' => self.single(Kind::CloseList),
                '{' => self.single(Kind::OpenBlock),
                '}' => self.single(Kind::CloseBlock),
                '(' => self.single(Kind::OpenGroup),
                ')' => self.single(Kind::CloseGroup),
                '+' => self.single(Kind::Plus),
                '-' => self.single(Kind::Minus),
                '\u{d7}' => self.wide(Kind::Times, '\u{d7}'),
                '/' => self.single(Kind::Slash),
                '^' => self.single(Kind::Power),
                '\u{f7}' => self.wide(Kind::Slash, '\u{f7}'),
                '\u{2260}' => self.wide(Kind::NotEqual, '\u{2260}'),
                // `</=` and `>/=` carry a `/` that is part of the operator rather than a
                // division. Nothing else begins this way, so one look ahead settles it.
                '<' => self.maybe_slash_equals(Kind::LessEqual, Kind::Less),
                '>' => self.maybe_slash_equals(Kind::GreaterEqual, Kind::Greater),
                '!' => self.bang(),
                ';' => self.single(Kind::Semicolon),
                ',' => self.single(Kind::Comma),
                '.' => self.single(Kind::Dot),
                ':' => self.single(Kind::Colon),
                '=' => {
                    if self.source[self.at + 1..].starts_with('=') {
                        let start = self.at;
                        self.at += 2;
                        self.tokens
                            .push(Token { kind: Kind::EqualTo, span: Span::new(start, self.at) });
                    } else {
                        self.single(Kind::Equals);
                    }
                }
                '"' => self.double_quoted(),
                '\\' => self.escape(),
                '\'' => self.delimited('\'', Kind::Name, "a name"),
                '*' => self.delimited('*', Kind::Written, "a written value"),
                c if starts_word(c) => self.word(),
                _ => self.unknown(),
            }
        }

        let end = Span::at(self.source.len());
        self.tokens.push(Token { kind: Kind::End, span: end });
        Lexed { tokens: self.tokens, errors: self.errors }
    }

    fn peek(&self) -> Option<char> {
        self.source[self.at..].chars().next()
    }

    /// A one-character token whose character is more than one byte.
    fn wide(&mut self, kind: Kind, c: char) {
        let start = self.at;
        self.at += c.len_utf8();
        self.tokens.push(Token { kind, span: Span::new(start, self.at) });
    }

    /// `<` and `>` on their own, or `</=` and `>/=` when the rest follows.
    fn maybe_slash_equals(&mut self, both: Kind, alone: Kind) {
        let start = self.at;
        if self.source[self.at + 1..].starts_with("/=") {
            self.at += 3;
            self.tokens.push(Token { kind: both, span: Span::new(start, self.at) });
        } else {
            self.single(alone);
        }
    }

    /// `!=`. A `!` on its own is not anything.
    fn bang(&mut self) {
        let start = self.at;
        if self.source[self.at + 1..].starts_with('=') {
            self.at += 2;
            self.tokens.push(Token { kind: Kind::NotEqual, span: Span::new(start, self.at) });
            return;
        }
        self.at += 1;
        self.errors.push(
            Diagnostic::new("E0005", "`!` on its own is not something Quench reads.")
                .primary(Span::new(start, self.at), "here")
                .rule("the ways to say not-equal are `!=`, `\u{2260}` and `not=`")
                .tip("`!` never means `not` by itself here — the word `not` does that.")
                .fix("`!=` if a comparison was meant"),
        );
    }

    fn single(&mut self, kind: Kind) {
        let start = self.at;
        self.at += 1;
        self.tokens.push(Token { kind, span: Span::new(start, self.at) });
    }

    /// `#` to the end of the line. Comments are not tokens; nothing downstream wants them.
    fn comment(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.at += c.len_utf8();
        }
    }

    /// A word: a chain part, a type, or the name of a block.
    ///
    /// Hyphens are allowed inside one, so `no-visibility-stated` is a single word rather
    /// than three and two subtractions. A trailing hyphen is not part of the word.
    fn word(&mut self) {
        let start = self.at;
        while let Some(c) = self.peek() {
            if in_word(c) {
                self.at += c.len_utf8();
            } else if c == '-' && self.after_hyphen_is_word() {
                self.at += 1;
            } else {
                break;
            }
        }
        self.tokens.push(Token { kind: Kind::Word, span: Span::new(start, self.at) });
    }

    fn after_hyphen_is_word(&self) -> bool {
        self.source[self.at + 1..].chars().next().is_some_and(in_word)
    }

    /// Anything between a pair of marks: `'a name'`, `*a written value*`.
    ///
    /// The span covers the marks as well as what is between them, so an error can
    /// underline the whole thing rather than its middle. A `\` in here escapes the
    /// closing mark and nothing else — that mark is the single character which could not
    /// otherwise be written, and every other character is the one it looks like.
    fn delimited(&mut self, mark: char, kind: Kind, what: &str) {
        let start = self.at;
        self.at += mark.len_utf8();

        while let Some(c) = self.peek() {
            match c {
                // A newline is always a missing closing mark rather than a very tall
                // name, and saying so where it opened is more use than saying it at the
                // end of the file.
                '\n' => break,
                '\\' => {
                    self.at += 1;
                    if let Some(next) = self.peek() {
                        self.at += next.len_utf8();
                    }
                }
                c if c == mark => {
                    self.at += mark.len_utf8();
                    self.tokens.push(Token { kind, span: Span::new(start, self.at) });
                    return;
                }
                _ => self.at += c.len_utf8(),
            }
        }

        self.errors.push(
            Diagnostic::new("E0002", format!("{what} was opened here and never closed."))
                .primary(Span::new(start, self.at), format!("this `{mark}` has no partner"))
                .rule(format!("{what} begins and ends with `{mark}`, on one line"))
                .tip("a line ending closes nothing — it is the mark that does.")
                .tip(format!("to write a `{mark}` inside one, put a `\\` in front of it."))
                .fix(format!("add a closing `{mark}` before the end of the line")),
        );
        // The opening mark was the mistake, so keep what followed it: far more likely to
        // be real source than the inside of a very long name.
        self.at = start + mark.len_utf8();
    }

    /// `\n` and friends, standing on their own between the things they separate.
    fn escape(&mut self) {
        let start = self.at;
        self.at += 1;
        let Some(c) = self.peek() else {
            self.errors.push(
                Diagnostic::new("E0004", "a `\\` at the very end of the file escapes nothing.")
                    .primary(Span::new(start, self.at), "here")
                    .rule("a `\\` is always followed by the thing it escapes")
                    .fix("remove it, or finish it"),
            );
            return;
        };

        if ESCAPES.contains(&c) {
            self.at += c.len_utf8();
            self.tokens.push(Token { kind: Kind::Escape, span: Span::new(start, self.at) });
            return;
        }

        let span = Span::new(start, self.at + c.len_utf8());
        self.at += c.len_utf8();
        self.errors.push(
            Diagnostic::new("E0004", format!("`\\{c}` is not an escape Quench knows."))
                .primary(span, "here")
                .rule("the escapes are `\\n`, `\\t`, `\\r` and `\\\\`")
                .tip("inside a written value a `\\` is only for writing the closing mark. Out here it makes one of the four above.")
                .fix(format!("`\\\\{c}` if a backslash and a `{c}` is what was wanted")),
        );
    }

    /// `"…"`, which Quench does not have.
    ///
    /// Taken as a whole rather than a character at a time, so the habit is reported once
    /// with both replacements, instead of twice with half the story each.
    fn double_quoted(&mut self) {
        let start = self.at;
        self.at += 1;
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.at += c.len_utf8();
            if c == '"' {
                break;
            }
        }
        let span = Span::new(start, self.at);
        let inside = self.source[span.start + 1..span.end.saturating_sub(1).max(span.start + 1)]
            .to_string();
        self.errors.push(
            Diagnostic::new("E0003", "Quench does not write anything between double quotes.")
                .primary(span, "here")
                .rule("a name is quoted with `'`, and a written value goes between `*` marks")
                .tip("the two are kept apart on purpose, so a quoted thing is always a name and never has to be read as a value depending on where it sits.")
                .fix(format!("`'{inside}'` if it is a name, `*{inside}*` if it is a written value")),
        );
    }

    fn unknown(&mut self) {
        let start = self.at;
        let c = self.peek().expect("called with a character waiting");
        self.at += c.len_utf8();
        let span = Span::new(start, self.at);

        let mut diag = Diagnostic::new("E0001", format!("`{c}` is not something Quench reads."))
            .primary(span, "here")
            .rule("every character outside a name, a value or a comment is part of the language");

        // The ones that are almost always a habit from another language rather than a typo.
        diag = match c {
            // A word cannot start with a digit, so this is always a value written bare.
            // When Quench grows arithmetic this stops being an error and starts being a
            // number, and this arm goes with it.
            '0'..='9' => diag
                .tip("a written value goes between bars, so that a quoted thing is always a name and never has to be read as a value depending on where it sits.")
                .fix(format!("`|{c}…|`")),
            '/' => diag
                .tip("comments start with `#`, and run to the end of the line.")
                .fix("`# like this`"),
            _ => diag.fix("remove it"),
        };
        self.errors.push(diag);
    }
}

/// The escapes that stand on their own, outside text.
const ESCAPES: [char; 4] = ['n', 't', 'r', '\\'];

/// What a word may start with. Not a digit, so nothing that looks like a number is one.
fn starts_word(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

/// What a word may continue with.
fn in_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
