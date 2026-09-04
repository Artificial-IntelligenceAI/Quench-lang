//! Filling in the holes, so that nothing below this has to know there were any.
//!
//! A generic function is a **pattern**, not a function. What gets compiled are copies of
//! it, one per type it was actually used at, and this is where the copies are made. By
//! the time [`Checked`](crate::Checked) leaves the checker there is no `Ty::Hole` left
//! anywhere in it, which is why QIR, the interpreter and the Dev JIT have no idea the
//! feature exists.
//!
//! It has to work this way rather than by boxing. A slot is an `i64` whatever is in it,
//! and the only thing that says whether the collector should follow one is the type at
//! the call site — see `notes/the-collector-earns-its-place.md`. A single copy of
//! `first of` serving both `arr.i64` and `arr.str` would have to carry a tag saying
//! which, on every value, and the collector would have to read it. Monomorphising costs
//! a function per type and nothing at all at runtime.
//!
//! Two passes, because a pattern may call another pattern — or itself — and the copy it
//! wants does not exist yet when the call is first seen:
//!
//! 1. **Discover.** Walk what is real, find every `(pattern, fill)` reached, and make the
//!    copy. Walking a copy discovers more, so this is a worklist rather than a loop.
//! 2. **Rewrite.** Now that every copy exists, point every call at the one it meant.

use crate::{Arm, Flow, Func, Place, Printed, Stmt, Ty, Value};
use quench_diag::Diagnostic;

/// How many copies one program may ask for before this gives up.
///
/// A pattern that calls itself at a *wider* type than it was given — an `any` handed an
/// `arr.any` of itself — asks for an unbounded number of them, and no amount of work
/// finishes it. Rust has the same limit for the same reason. Nothing sensible reaches
/// this; something looping does.
const ENOUGH: usize = 256;

/// Whether a hole appears anywhere in this, however deep.
pub(crate) fn mentioned(ty: &Ty) -> bool {
    match ty {
        Ty::Hole(_) => true,
        Ty::Arr { of, .. } => mentioned(of),
        _ => false,
    }
}

/// What the hole must be, for `found` to be the type `wanted` describes.
///
/// `arr.any (grow)` against `arr.str (grow)` is `str`. Only the shape of the *type* is
/// walked here; whether the sizes agree is the ordinary check, made afterwards against
/// the filled-in type, so a wrong shape gets the error it always got.
pub(crate) fn solve(wanted: &Ty, found: &Ty) -> Option<Ty> {
    match (wanted, found) {
        (Ty::Hole(_), found) => Some(found.clone()),
        (Ty::Arr { of: wanted, .. }, Ty::Arr { of: found, .. }) => solve(wanted, found),
        _ => None,
    }
}

/// `wanted`, with every hole in it replaced by `with`.
pub(crate) fn filled(wanted: &Ty, with: &Ty) -> Ty {
    match wanted {
        Ty::Hole(_) => with.clone(),
        Ty::Arr { of, shape, grows } => Ty::Arr {
            of: Box::new(filled(of, with)),
            shape: shape.clone(),
            grows: *grows,
        },
        settled => settled.clone(),
    }
}

/// Every value in these statements, outermost first.
fn through(stmts: &mut [Stmt], on: &mut impl FnMut(&mut Value)) {
    for stmt in stmts {
        match stmt {
            Stmt::Declare { value, .. } | Stmt::Give(Some(value)) => within(value, on),
            Stmt::Give(None) | Stmt::Break => {}
            Stmt::If { arms, otherwise, .. } => {
                for Arm { condition, body } in arms {
                    within(condition, on);
                    through(body, on);
                }
                if let Some(body) = otherwise {
                    through(body, on);
                }
            }
            Stmt::Loop { flow, body, .. } => {
                match flow {
                    Flow::Range { from, to, .. } => {
                        within(from, on);
                        within(to, on);
                    }
                    Flow::While(condition) => within(condition, on),
                }
                through(body, on);
            }
            Stmt::Do { func, args, fill } => {
                // A call written for what it does is a call, so it is handed to the
                // walker as one -- otherwise a `Do` naming a pattern would never be
                // rewritten, and the program would call the thing that is not a
                // function.
                let mut standing = Value::Call {
                    func: *func,
                    args: std::mem::take(args),
                    fill: fill.take(),
                };
                within(&mut standing, on);
                if let Value::Call { func: went, args: brought, fill: wanted } = standing {
                    *func = went;
                    *args = brought;
                    *fill = wanted;
                }
            }
            Stmt::Assign { to, value } => {
                if let Place::Element { array, indices, .. } = to {
                    within(array, on);
                    for index in indices {
                        within(index, on);
                    }
                }
                within(value, on);
            }
            Stmt::Extend { array, value } => {
                within(array, on);
                within(value, on);
            }
            Stmt::Print { pieces, .. } => {
                for piece in pieces {
                    if let Printed::Value { value, .. } = piece {
                        within(value, on);
                    }
                }
            }
        }
    }
}

/// This value and every value inside it, outermost first.
fn within(value: &mut Value, on: &mut impl FnMut(&mut Value)) {
    on(value);
    match value {
        Value::Binary { lhs, rhs, .. } => {
            within(lhs, on);
            within(rhs, on);
        }
        Value::Array { elements, .. } => {
            for element in elements {
                within(element, on);
            }
        }
        Value::At { array, indices, .. } => {
            within(array, on);
            for index in indices {
                within(index, on);
            }
        }
        Value::Join(pieces) | Value::Maths { of: pieces, .. } | Value::Slowly { of: pieces, .. } => {
            for piece in pieces {
                within(piece, on);
            }
        }
        Value::Said { of, .. }
        | Value::Not(of)
        | Value::Copied(of)
        | Value::CountText(of)
        | Value::Count(of)
        | Value::CanRead { text: of, .. }
        | Value::Read { text: of, .. } => within(of, on),
        Value::Call { args, .. } => {
            for arg in args {
                within(arg, on);
            }
        }
        Value::Text(_)
        | Value::Number { .. }
        | Value::Exact(_)
        | Value::Decimal { .. }
        | Value::Float { .. }
        | Value::Bool(_)
        | Value::Copy(_)
        | Value::Const(_) => {}
    }
}

/// One copy of a pattern, with `with` written in everywhere the hole was.
fn copied(pattern: &Func, with: &Ty, called: String) -> Func {
    let mut made = pattern.clone();
    made.hole = None;
    made.name = called;
    made.returns = made.returns.as_ref().map(|ty| filled(ty, with));
    for local in &mut made.locals {
        local.ty = filled(&local.ty, with);
    }
    through(&mut made.body, &mut |value| match value {
        Value::Array { of, .. } => **of = filled(of, with),
        Value::Said { ty, .. } | Value::CanRead { ty, .. } | Value::Read { ty, .. } => {
            *ty = filled(ty, with);
        }
        // A pattern calling a pattern: what it asked for may itself have been the hole,
        // and now it is a type.
        Value::Call { fill: Some(fill), .. } => *fill = filled(fill, with),
        _ => {}
    });
    // What a `print` shows is decided by the type written beside it, and that is the
    // one type in a body that does not hang off a value.
    shown_as(&mut made.body, with);
    made
}

/// The type beside every `print`ed value, filled in, nested statements included.
fn shown_as(stmts: &mut [Stmt], with: &Ty) {
    for stmt in stmts {
        match stmt {
            Stmt::Print { pieces, .. } => {
                for piece in pieces {
                    if let Printed::Value { ty, .. } = piece {
                        *ty = filled(ty, with);
                    }
                }
            }
            Stmt::If { arms, otherwise, .. } => {
                for arm in arms {
                    shown_as(&mut arm.body, with);
                }
                if let Some(body) = otherwise {
                    shown_as(body, with);
                }
            }
            Stmt::Loop { body, .. } => shown_as(body, with),
            _ => {}
        }
    }
}

/// Every `(pattern, fill)` a body asks for.
fn asked(body: &mut [Stmt], patterns: &[bool]) -> Vec<(usize, Ty)> {
    let mut found = Vec::new();
    through(body, &mut |value| {
        if let Value::Call { func, fill: Some(fill), .. } = value {
            if patterns.get(*func as usize).copied().unwrap_or(false) {
                found.push((*func as usize, fill.clone()));
            }
        }
    });
    found
}

/// Make every copy the program asks for, and point every call at the one it meant.
///
/// What comes back has no pattern in it and no `Ty::Hole` anywhere, so a function's place
/// in the list is its id in the module again — which is the thing lowering has always
/// assumed and the one thing generics would otherwise have broken.
pub(crate) fn fill_in(
    funcs: Vec<Func>,
    start: Option<usize>,
    errors: &mut Vec<Diagnostic>,
) -> (Vec<Func>, Option<usize>) {
    let patterns: Vec<bool> = funcs.iter().map(|f| f.hole.is_some()).collect();
    if !patterns.iter().any(|is| *is) {
        return (funcs, start);
    }

    // What is real keeps its order; the copies go on the end. So a function's new place
    // is settled before any call is rewritten.
    let mut out: Vec<Func> = Vec::new();
    let mut moved: Vec<Option<usize>> = vec![None; funcs.len()];
    for (which, func) in funcs.iter().enumerate() {
        if !patterns[which] {
            moved[which] = Some(out.len());
            out.push(func.clone());
        }
    }
    let start = start.and_then(|at| moved[at]);

    // Discover. Each copy is walked in its turn, because a copy may ask for another.
    let mut made: Vec<(usize, Ty, usize)> = Vec::new();
    let mut queue: Vec<(usize, Ty)> = Vec::new();
    for func in &mut out {
        queue.extend(asked(&mut func.body, &patterns));
    }
    while let Some((pattern, fill)) = queue.pop() {
        if made.iter().any(|(p, f, _)| *p == pattern && f == &fill) {
            continue;
        }
        if made.len() >= ENOUGH {
            errors.push(
                Diagnostic::new(
                    "E0504",
                    format!("`'{}'` is asked for at more types than this can write out.", funcs[pattern].name),
                )
                .primary(funcs[pattern].at, "here")
                .rule("a function with a hole is copied once for every type it is used at, and the list has to end")
                .tip("a function that calls itself at a *wider* type each time asks for an endless number of copies -- an `any` handed an array of itself, most often.")
                .fix("give the recursive call the type it was given, rather than one built out of it"),
            );
            return (out, start);
        }
        // A name of its own, because a name is how compiled code declares a function and
        // two copies of one pattern are two functions. Kept readable rather than
        // numbered -- `largest (b64)` says which copy it is in a stack trace or an IR
        // dump -- and made unique the dull way, because a writer may name a function
        // anything at all and marks let them.
        let mut called = format!("{} ({})", funcs[pattern].name, fill.name());
        while out.iter().any(|f| f.name == called) {
            called.push('\'');
        }
        let mut copy = copied(&funcs[pattern], &fill, called);
        queue.extend(asked(&mut copy.body, &patterns));
        made.push((pattern, fill, out.len()));
        out.push(copy);
    }

    // Rewrite. Every copy exists now, so every call has something to point at.
    for func in &mut out {
        through(&mut func.body, &mut |value| {
            if let Value::Call { func, fill, .. } = value {
                let which = *func as usize;
                if let Some(wanted) = fill.take() {
                    if let Some((_, _, at)) =
                        made.iter().find(|(p, f, _)| *p == which && f == &wanted)
                    {
                        *func = *at as u32;
                        return;
                    }
                }
                if let Some(at) = moved.get(which).copied().flatten() {
                    *func = at as u32;
                }
            }
        });
    }

    (out, start)
}
