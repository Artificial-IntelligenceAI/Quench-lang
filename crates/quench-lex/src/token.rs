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
    /// `(` — grouping, and the only way to say what mathematics never settled.
    ///
    /// Quench keeps the precedence mathematics established — exponent, then `x` and `/`,
    /// then `+` and `-`, with comparison looser than all of them — and refuses to invent
    /// any beyond that. Everything programming added has no agreed order, so it is
    /// written with these instead. See `notes/precedence-stops-where-maths-stopped.md`.
    OpenGroup,
    /// `)`
    CloseGroup,

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
    /// `=` — the one in a declaration, between the names and their values.
    Equals,
    /// `==` — equality. The word `eq-to` says the same thing.
    ///
    /// Luarust writes this `=`, and gets away with it because arithmetic lives inside a
    /// `math { … }` block where `=` cannot be a declaration. Quench has no such block —
    /// an expression sits in the value list directly — so `=` would mean assignment
    /// outside the brackets and equality inside them, decided by where it sits.
    ///
    /// Which is the one thing the marks exist to prevent. `'name'` is a name wherever it
    /// is met; `=` would have been the only symbol in the language that was not itself.
    EqualTo,

    /// `+`
    Plus,
    /// `-`, when it stands alone. Inside a word it is part of the word, so
    /// `no-visibility-stated` is one thing rather than three and two subtractions.
    Minus,
    /// `x`, the multiplication sign.
    ///
    /// `*` cannot be multiplication here — it is the mark a written value wears — so the
    /// word `x` does the job as well, and arrives as a [`Kind::Word`] for the parser to
    /// recognise. Only the real sign needs a token of its own.
    ///
    /// The exponent is the word `xx`, multiplying twice, and it is a word for the same
    /// reason. `**` was the obvious spelling and is impossible: `*a* ** *b*` lexes that
    /// `**` as an *empty written value*, because the first `*` opens one and the second
    /// `/` or `/`.
    Slash,
    /// `^` — an exponent, as mathematics writes one. The word `xx` says the same thing.
    ///
    /// Most languages spend `^` on bitwise exclusive-or. Quench cannot follow them and
    /// keep its own rule: it takes the precedence mathematics settled, and mathematics
    /// writes an exponent with this. So if an exclusive-or ever arrives it will be
    /// spelled some other way, and that is the price.
    Power,

    /// `<`
    Less,
    /// `>`
    Greater,
    /// `<==` — less than, or equal to. Three characters: the `==` is the whole of
    /// "equal to", the same `==` written on its own, rather than a shortened one.
    LessEqual,
    /// `>==`
    GreaterEqual,
    /// `!==`
    NotEqual,

    /// A bare word: a chain part, a type, or the name of a block. Never a variable's name.
    Word,
    /// A bare number, with no marks on it.
    ///
    /// Only a *shape* is written this way — `arr.i64 (5 2)` — because a shape is part of
    /// a type rather than a value, and types wear no marks. The `64` in `i64` is a number
    /// inside a type and nobody writes `i*64*`; a dimension is the same kind of thing.
    /// Marks exist to tell a name from a written value, and a shape is neither.
    Number,

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
    /// Every kind of token there is.
    ///
    /// Written down so that something can ask the *other* question: not "is every kind
    /// here still real" — which catches a removal and nothing else — but "is every kind
    /// here ever produced". `Kind::Times` was declared, given a spelling in `describe`,
    /// and matched by the parser, and the lexer never made one; it was two designs old
    /// and nothing noticed for as long as it existed. See `tests/every_kind.rs`.
    pub const ALL: &[Kind] = &[
        Kind::OpenList,
        Kind::CloseList,
        Kind::OpenBlock,
        Kind::CloseBlock,
        Kind::OpenGroup,
        Kind::CloseGroup,
        Kind::Semicolon,
        Kind::Comma,
        Kind::Dot,
        Kind::Colon,
        Kind::Equals,
        Kind::EqualTo,
        Kind::Plus,
        Kind::Minus,
        Kind::Slash,
        Kind::Power,
        Kind::Less,
        Kind::Greater,
        Kind::LessEqual,
        Kind::GreaterEqual,
        Kind::NotEqual,
        Kind::Word,
        Kind::Number,
        Kind::Name,
        Kind::Written,
        Kind::Escape,
        Kind::End,
    ];

    /// Exhaustive on purpose. It exists to stop compiling when a kind is added to the
    /// enum and not to [`Kind::ALL`], which is the only way that list can go stale in
    /// the direction that matters.
    pub fn listed(self) -> bool {
        match self {
            Kind::OpenList
            | Kind::CloseList
            | Kind::OpenBlock
            | Kind::CloseBlock
            | Kind::OpenGroup
            | Kind::CloseGroup
            | Kind::Semicolon
            | Kind::Comma
            | Kind::Dot
            | Kind::Colon
            | Kind::Equals
            | Kind::EqualTo
            | Kind::Plus
            | Kind::Minus
            | Kind::Slash
            | Kind::Power
            | Kind::Less
            | Kind::Greater
            | Kind::LessEqual
            | Kind::GreaterEqual
            | Kind::NotEqual
            | Kind::Word
            | Kind::Number
            | Kind::Name
            | Kind::Written
            | Kind::Escape
            | Kind::End => {}
        }
        Kind::ALL.contains(&self)
    }

    /// What to call this in an error message.
    pub fn describe(self) -> &'static str {
        match self {
            Kind::OpenList => "`[`",
            Kind::CloseList => "`]`",
            Kind::OpenBlock => "`{`",
            Kind::CloseBlock => "`}`",
            Kind::OpenGroup => "`(`",
            Kind::CloseGroup => "`)`",
            Kind::Semicolon => "`;`",
            Kind::Comma => "`,`",
            Kind::Dot => "`.`",
            Kind::Colon => "`:`",
            Kind::Equals => "`=`",
            Kind::EqualTo => "`==`",
            Kind::Plus => "`+`",
            Kind::Minus => "`-`",
            Kind::Slash => "`/`",
            Kind::Power => "`^`",
            Kind::Less => "`<`",
            Kind::Greater => "`>`",
            Kind::LessEqual => "`<==`",
            Kind::GreaterEqual => "`>==`",
            Kind::NotEqual => "`!==`",
            Kind::Word => "a word",
            Kind::Number => "a bare number",
            Kind::Name => "a name",
            Kind::Written => "a written value",
            Kind::Escape => "an escape",
            Kind::End => "the end of the file",
        }
    }
}
