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
/// **Quench reserves no words.** `var`, `mut`, `file`, `export`, `print` and `START` are
/// all [`Kind::Word`], and the parser decides what each means from where it stands. That
/// is affordable here for one specific reason: a name is always quoted, so a bare word can
/// never be a name, so no word ever has to be taken away from the programmer to make room
/// for the language. Inherited from Luarust, which worked it out first.
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
    /// `:` — between a type and the written value it reads.
    ///
    /// A written value has no meaning on its own: `*1000*` is a number under `b16` and
    /// four characters under `str`. A declaration supplies the type from its chain, so
    /// nothing has to be said twice. Anywhere else the value states it — `str:*hello*` —
    /// rather than being read as whatever seems likely.
    Colon,
    /// `=`
    Equals,

    /// A bare word: a chain part, a type, or the name of a block. Never a variable's name.
    Word,

    /// `'…'` — a name. Quoted, so it can be anything you can type.
    Name,

    /// `*…*` — a written value.
    ///
    /// One kind for all of them, because there is only one question here. What a written
    /// value *means* is decided by the type it is given to: `*1000*` is a number under
    /// `b16` and text under `str`. Sorting written things into "text" and "numbers" at
    /// this point would be answering, badly and early, a question the type answers
    /// properly and later.
    ///
    /// What is between the marks is **literal** — emoji, punctuation, braces, digits, all
    /// of it, exactly as written. The only thing `\` escapes in there is the closing mark
    /// itself, because that is the one character which could not otherwise be written.
    Written,

    /// `\n` and friends, written **outside** the marks rather than inside them.
    ///
    /// This is what lets a written value be literal. An escape is an item in the list,
    /// sitting next to what it separates rather than hidden inside it, so reading a piece
    /// of Quench text never means working out which of its characters were really
    /// instructions.
    Escape,

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
            Kind::Colon => "`:`",
            Kind::Equals => "`=`",
            Kind::Word => "a word",
            Kind::Name => "a name",
            Kind::Written => "a written value",
            Kind::Escape => "an escape",
            Kind::End => "the end of the file",
        }
    }
}
