//! What a piece of Quench source turns into.

use quench_diag::Span;

/// One token, and where it came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Token {
    pub kind: Kind,
    pub span: Span,
}

/// The kinds of thing a Quench program is made of.
///
/// **Quench reserves no words.** `var`, `mut`, `file`, `export`, `b16` and `START` are all
/// [`Kind::Word`], and the parser decides what each means from where it stands. That is
/// affordable here in a way it is not in most languages, and for one specific reason: a
/// name is always quoted, so a bare word can never be a name, so no word ever has to be
/// taken away from the programmer to make room for the language. `export` is a visibility
/// in a chain and an ordinary word everywhere else.
///
/// Inherited from Luarust, which worked it out first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// `[` — opens a list of names or of values.
    OpenList,
    /// `]`
    CloseList,
    /// `{` — opens a block. The word in front of it says which kind.
    OpenBlock,
    /// `}`
    CloseBlock,

    /// `;` — ends a statement.
    Semicolon,
    /// `,` — between the items of a list.
    Comma,
    /// `.` — between the parts of a chain.
    Dot,
    /// `=`
    Equals,

    /// A bare word: a chain part, a type, or the name of a block. Never a variable's name.
    Word,
    /// `'…'` — a name. Quoted, so it can be anything you can type.
    Name,
    /// `*…*` — text, to print. The marks do the job `"` does elsewhere.
    ///
    /// What is between them is **literal**: emoji, punctuation, braces, digits, all of
    /// it, exactly as written. The only thing `\\` escapes in here is a `*`, because the
    /// closing mark is the one character that could not otherwise be written. A `\\n`
    /// between the marks is a backslash and an `n`; a newline is [`Kind::Escape`],
    /// written outside.
    Text,
    /// `\\n` and friends, written **outside** the marks rather than inside them.
    ///
    /// This is why text can be literal. An escape is a thing in the list, next to the
    /// text rather than hidden in it, so reading a piece of text never means working out
    /// which of its characters were really instructions.
    Escape,

    /// `|…|` or `` `…` `` — a written value. What it *means* is decided by the type it is
    /// given to, so the lexer does not read it: `|1000|` is a number under `b16` and text
    /// under `str`, and that is not a decision to take this early.
    Literal,

    /// The end of the file. Always the last token, so a parser can look ahead safely.
    End,
}

impl Kind {
    /// What to call this in an error message.
    pub fn describe(self) -> &'static str {
        match self {
            Kind::OpenList => "`[`",
            Kind::CloseList => "`]`",
            Kind::OpenBlock => "`{`",
            Kind::CloseBlock => "`}`",
            Kind::Semicolon => "`;`",
            Kind::Comma => "`,`",
            Kind::Dot => "`.`",
            Kind::Equals => "`=`",
            Kind::Word => "a word",
            Kind::Name => "a name",
            Kind::Literal => "a written value",
            Kind::Text => "text",
            Kind::Escape => "an escape",
            Kind::End => "the end of the file",
        }
    }
}
