//! Whether the space table is Unicode's, checked rather than trusted.
//!
//! `White_Space` is written out in `pieces.rs` rather than taken from the standard
//! library, because what a Quench program answers must not depend on which compiler
//! built it — an artefact travels, and `trim` would otherwise be the one place in the
//! language where the answer moves with the toolchain.
//!
//! Writing it out is only safe if something checks it, which is this. The standard
//! library implements the same property from the same database, so the two must agree
//! on every character there is.

#[test]
fn the_space_table_is_the_one_the_standard_library_has() {
    let mut differ = Vec::new();
    for scalar in 0..=0x10FFFFu32 {
        let Some(c) = char::from_u32(scalar) else { continue };
        if quench_text::pieces::is_space(c) != c.is_whitespace() {
            differ.push(scalar);
        }
    }
    assert!(
        differ.is_empty(),
        "the table and `char::is_whitespace` disagree on {:?}",
        differ.iter().map(|s| format!("U+{s:04X}")).collect::<Vec<_>>()
    );
}

#[test]
fn a_piece_is_taken_by_character_and_the_two_readings_differ() {
    use quench_text::pieces;

    // `é` written as `e` and a combining acute is one cluster and two scalars, which is
    // the difference the setting exists for.
    let text = "ae\u{0301}b";
    assert_eq!(pieces::count(text, true), 3);
    assert_eq!(pieces::count(text, false), 4);
    assert_eq!(pieces::slice(text, 2, 2, true).as_deref(), Some("e\u{0301}"));
    assert_eq!(pieces::slice(text, 2, 2, false).as_deref(), Some("e"));

    // A backwards pair is empty rather than refused, the way a backwards range runs no
    // times; a position off the end is refused, the way an index off the end is.
    assert_eq!(pieces::slice(text, 3, 2, true).as_deref(), Some(""));
    assert_eq!(pieces::slice(text, 1, 9, true), None);
    assert_eq!(pieces::slice(text, 0, 1, true), None);
    assert_eq!(pieces::slice(text, 4, 3, true).as_deref(), Some(""), "one past the end, empty");

    // Finding says which character it begins, so it moves with the setting too.
    assert_eq!(pieces::find(text, "b", true), Some(3));
    assert_eq!(pieces::find(text, "b", false), Some(4));
    assert_eq!(pieces::find(text, "z", true), None);
    assert!(pieces::has(text, "b") && !pieces::has(text, "z"));

    assert_eq!(pieces::split("a,b,,c", ","), ["a", "b", "", "c"]);
    assert_eq!(pieces::split("abc", ","), ["abc"], "no separator is one piece, the whole thing");
    assert_eq!(pieces::trim("  a b\u{3000}"), "a b");
}
