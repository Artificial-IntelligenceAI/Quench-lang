//! The meaning of a program, turned into the IR every engine runs.
//!
//! There are no decisions here. [`quench_check`] resolved every name, settled every
//! type, joined every piece of text and refused everything that did not make sense — so
//! what is left is a transliteration, and that is the point of doing the checking first.
//! Anything in this file that started to look like a judgement would belong further up.

use quench_check::{Arm, Checked, Flow, Func, Local, OpKind, Place, Printed, Stmt, Ty, Value};
use quench_conf::{Division, Logic, NoNumber, Overflow, Settings};
use quench_diag::{Diagnostic, Span};
use quench_qir as qir;

/// What a file became, and everything wrong with it.
pub struct Lowered {
    /// Absent when the file could not be turned into a program at all.
    pub module: Option<qir::Module>,
    pub errors: Vec<Diagnostic>,
}

impl Lowered {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Read a whole file and turn it into something that can run, under the project's
/// default settings.
pub fn lower(source: &str) -> Lowered {
    lower_under(source, Settings::default())
}

/// The same, under settings of your choosing.
///
/// Which matters for exactly one thing so far, and it is the thing the two-piles note
/// warned about: `[defaults] division` decides whether `/` and `mod` round toward zero
/// or toward negative infinity, so the same source is two different programs under the
/// two settings. The choice is made here and written into the IR as an instruction, so
/// nothing below this has to know a setting existed.
pub fn lower_under(source: &str, settings: Settings) -> Lowered {
    let mut checked = quench_check::check(source);

    if !checked.has_start() {
        // Not something the parser could report: a file of declarations is a fine thing
        // to parse and a useless thing to run, and only something trying to run it knows
        // which was wanted.
        checked.errors.push(
            Diagnostic::new("E0301", "this file has no `START`, so there is nothing to run.")
                .primary(Span::at(source.len()), "the file ends here")
                .rule("a program begins at `START`, and a file without one is not a program")
                .tip("a file may hold declarations and no `START`. It just cannot be run.")
                .fix("add `START { … }`"),
        );
        return Lowered { module: None, errors: checked.errors };
    }
    if !checked.errors.is_empty() {
        // Lowering a program that did not check out would build nonsense out of it.
        return Lowered { module: None, errors: checked.errors };
    }

    Lowered { module: Some(build(&checked, settings)), errors: checked.errors }
}

fn build(checked: &Checked, settings: Settings) -> qir::Module {
    let mut module = qir::Module::new();

    // Constant arrays go into the module before any function does, because a handle to
    // one is its place in that list and the code that names it needs the number.
    let mut tables: Vec<Option<u32>> = vec![None; checked.constants.len()];
    for (n, constant) in checked.constants.iter().enumerate() {
        if matches!(constant.ty, Ty::Arr { .. }) {
            tables[n] = Some(lay_out(&mut module, &constant.value));
        }
    }

    // Functions are added in the order they were checked, so a function's place in
    // `checked.funcs` is its id here -- which is what lets a body call something
    // written underneath it, and lets one call itself.
    for (n, func) in checked.funcs.iter().enumerate() {
        let built = lower_func(&mut module, func, checked, &tables, settings);
        let id = module.add(built);
        debug_assert_eq!(id.0 as usize, n, "a function's place is its id");
    }

    if let Some(start) = checked.start {
        module.set_entry(qir::FuncId(start as u32));
    }
    module
}

/// Put a constant array into the module, and say which table it is.
///
/// Inner arrays go in first, so that the handles the outer one holds are already
/// decided by the time it is written — the layout being fixed is what makes a handle
/// something the compiler can know.
fn lay_out(module: &mut qir::Module, value: &Value) -> u32 {
    let Value::Array { elements, .. } = value else {
        unreachable!("refused by the checker: a constant array is written out")
    };
    let mut slots = Vec::with_capacity(elements.len());
    for element in elements {
        slots.push(match element {
            Value::Number { value, .. } => *value,
            Value::Bool(yes) => i64::from(*yes),
            Value::Text(text) => i64::from(module.intern(text)),
            Value::Array { .. } => i64::from(lay_out(module, element)),
            _ => unreachable!("refused by the checker: a constant is worked out here"),
        });
    }
    module.table(slots)
}

fn lower_func(
    module: &mut qir::Module,
    func: &Func,
    checked: &Checked,
    tables: &[Option<u32>],
    settings: Settings,
) -> qir::Function {
    let params: Vec<qir::Ty> =
        func.locals[..func.takes].iter().map(|local| qir_ty(&local.ty)).collect();
    // A function that gives `nothing` back still answers with something, because QIR
    // has one shape of call and this is where that is paid for. The checker refused
    // every use of the answer, so what it is cannot be observed.
    let ret = func.returns.as_ref().map_or(qir::Ty::I64, qir_ty);

    let name = if Some(func) == checked.start.map(|n| &checked.funcs[n]) {
        qir::ENTRY.to_string()
    } else {
        func.name.clone()
    };
    let mut b = qir::Builder::new(name, &params, ret);

    // Where each variable's value ended up. A declaration fills one in; a use reads it;
    // a join replaces the ones that could have come from either side. The parameters
    // are already filled, being the entry block's own.
    let mut held: Vec<Option<qir::Value>> = vec![None; func.locals.len()];
    for n in 0..func.takes {
        held[n] = Some(b.param(n));
    }

    let w = Where { locals: &func.locals, checked, ret, tables, settings };
    let mut loops = Vec::new();
    let answered = lower_body(&mut b, module, &func.body, &mut held, &w, &mut loops);

    // A function that says nothing about how it ended, ended fine. Which is only ever
    // reached by one that gives `nothing` back -- the checker made sure of the rest.
    if !answered {
        let nothing = b.const_i64(0);
        b.ret(nothing);
    }
    b.finish()
}

/// What a QIR value of this type looks like.
fn qir_ty(ty: &Ty) -> qir::Ty {
    match ty {
        // Every whole-number type rides in an `i64`, held normalised for its width.
        Ty::Int { .. } => qir::Ty::I64,
        Ty::Bool => qir::Ty::Bool,
        Ty::Str => qir::Ty::Text,
        Ty::Exact => qir::Ty::Exact,
        Ty::Decimal { .. } => qir::Ty::Decimal,
        Ty::F64 => qir::Ty::F64,
        Ty::F32 => qir::Ty::F32,
        Ty::F16 => qir::Ty::F16,
        Ty::Arr { .. } => qir::Ty::Handle,
    }
}

/// Everything the lowering of one function needs to look up, gathered so that passing
/// it down does not cost a parameter for each thing.
struct Where<'a> {
    /// The locals of the function being lowered — not of any other.
    locals: &'a [Local],
    checked: &'a Checked,
    /// What the function being lowered answers with, for ending a block that nothing
    /// ever reaches.
    ret: qir::Ty,
    /// Which table each constant became, for the ones that are arrays.
    tables: &'a [Option<u32>],
    settings: Settings,
}

/// A loop being lowered, so that a `break` somewhere inside it knows where to go and
/// what to take with it.
struct Frame {
    done: qir::BlockId,
    /// Which locals the loop carries across every edge.
    carried: Vec<usize>,
    /// A `perm` counter's local, which `done` wants after the carried values because it
    /// is what the counter will hold afterwards.
    keep: Option<usize>,
}

/// Lower a run of statements. True when the block was left before the end of it — which
/// is to say a `break` ran, and whoever called must not put a terminator after one.
fn lower_body(
    b: &mut qir::Builder,
    module: &mut qir::Module,
    body: &[Stmt],
    held: &mut Vec<Option<qir::Value>>,
    w: &Where<'_>,
    loops: &mut Vec<Frame>,
) -> bool {
    for stmt in body {
        match stmt {
            Stmt::Declare { local, value } => {
                let value = emit(b, module, value, held, w);
                held[local.0 as usize] = Some(value);
            }
            // Changing a variable is naming a new value for it. Inside an arm that is
            // still just a write here -- what makes it correct is that the join below
            // takes whichever value the branch that ran left behind.
            Stmt::Assign { to, value } => {
                let value = emit(b, module, value, held, w);
                match to {
                    Place::Local(local) => held[local.0 as usize] = Some(value),
                    Place::Element { array, indices, shape } => {
                        // Whatever holds it, which for an array of arrays is another
                        // `array-get` away and needs nothing new to say so.
                        let handle = emit(b, module, array, held, w);
                        let at = flat_index(b, module, indices, shape, held, w);
                        b.call_host(qir::Host::ArraySet, &[handle, at, value]);
                    }
                }
            }
            Stmt::If { arms, otherwise, live } => {
                let left = lower_if(
                    b, module, arms, otherwise.as_deref(), *live, held, w, loops,
                );
                if left {
                    return true;
                }
            }
            Stmt::Loop { flow, body, live } => {
                lower_loop(b, module, flow, body, *live, held, w, loops);
            }
            Stmt::Extend { array, value } => {
                let value = emit(b, module, value, held, w);
                let handle = emit(b, module, array, held, w);
                b.call_host(qir::Host::ArrayPush, &[handle, value]);
            }
            Stmt::Give(value) => {
                let answer = match value {
                    Some(value) => emit(b, module, value, held, w),
                    None => b.const_i64(0),
                };
                b.ret(answer);
                return true;
            }
            Stmt::Do { func, args } => {
                let given: Vec<qir::Value> = args
                    .iter()
                    .map(|arg| emit(b, module, arg, held, w))
                    .collect();
                let ret = w.checked.funcs[*func as usize].returns.as_ref().map_or(qir::Ty::I64, qir_ty);
                b.call(qir::FuncId(*func), &given, ret);
            }
            Stmt::Break => {
                let frame = loops.last().expect("refused by the checker: `break` outside a loop");
                let mut leaving: Vec<qir::Value> =
                    frame.carried.iter().map(|i| held[*i].expect("carried")).collect();
                if let Some(i) = frame.keep {
                    leaving.push(held[i].expect("the counter, which the loop set"));
                }
                b.jump(frame.done, &leaving);
                return true;
            }
            Stmt::Print { to, pieces } => {
                for piece in pieces {
                    match piece {
                        Printed::Text(text) => {
                            let at = module.intern(text);
                            let value = b.const_text(at);
                            b.print(qir::Host::PrintText, *to, value);
                        }
                        Printed::Value { value, ty } => {
                            let value = emit(b, module, value, held, w);
                            let host = match ty {
                                Ty::Str => qir::Host::PrintText,
                                Ty::Int { bits: 64, signed: false } => qir::Host::PrintU64,
                                Ty::Int { .. } => qir::Host::PrintI64,
                                Ty::Bool => qir::Host::PrintBool,
                                Ty::Exact => qir::Host::PrintExact,
                                Ty::Decimal { .. } => qir::Host::PrintDecimal,
                                // All three arrive in the same register, so the
                                // runtime is told which it is holding.
                                Ty::F64 | Ty::F32 | Ty::F16 => {
                                    let width = b.const_i64(match ty {
                                        Ty::F16 => 16,
                                        Ty::F32 => 32,
                                        _ => 64,
                                    });
                                    let stream = b.const_i64(*to as i64);
                                    b.call_host(
                                        qir::Host::PrintFloat,
                                        &[stream, value, width],
                                    );
                                    continue;
                                }
                                // An array is the one thing whose printing takes a
                                // third argument, because a slot is an `i64` whatever
                                // is in it and the runtime has to be told which.
                                Ty::Arr { of, .. } => {
                                    let (leaf, depth) = elements(of);
                                    let kind = b.const_i64(leaf as i64);
                                    let deep = b.const_i64(depth);
                                    let stream = b.const_i64(*to as i64);
                                    b.call_host(
                                        qir::Host::PrintArray,
                                        &[stream, value, kind, deep],
                                    );
                                    continue;
                                }
                            };
                            b.print(host, *to, value);
                        }
                    }
                }
            }
        }
    }
    false
}

/// A loop — the same join a conditional makes, with an edge back into it.
///
/// The head holds one block parameter per variable the loop carries, exactly as a
/// conditional's join does, and one more for the counter when there is one. What makes
/// it a loop rather than a join is only that the body's last edge goes back to the head
/// instead of onward, and that the head is where the question gets asked again.
///
/// A `perm` counter costs one further parameter, carrying the last value it actually
/// took — because the counter itself is one past the end by the time the loop stops, and
/// nobody means six when they wrote five. Only loops that asked for `perm` pay for it.
fn lower_loop(
    b: &mut qir::Builder,
    module: &mut qir::Module,
    flow: &Flow,
    body: &[Stmt],
    live: u32,
    held: &mut Vec<Option<qir::Value>>,
    w: &Where<'_>,
    loops: &mut Vec<Frame>,
) {
    let live = live as usize;
    let carried: Vec<usize> = (0..live).filter(|i| held[*i].is_some()).collect();
    let carried_types: Vec<qir::Ty> =
        carried.iter().map(|i| qir_ty(&w.locals[*i].ty)).collect();

    let (counting, keeps) = match flow {
        Flow::Range { keeps, .. } => (true, *keeps),
        Flow::While(_) => (false, false),
    };

    let mut inside = carried_types.clone();
    if counting {
        inside.push(qir::Ty::I64);
    }
    if keeps {
        inside.push(qir::Ty::I64);
    }
    let mut outside = carried_types.clone();
    if keeps {
        outside.push(qir::Ty::I64);
    }

    let head = b.block(&inside);
    let pass = b.block(&inside);
    let done = b.block(&outside);

    // Both bounds before the first pass. A loop whose end moved under it would be a loop
    // nobody could read, so the checker made them values and this works them out once.
    let bounds = match flow {
        Flow::Range { from, to, .. } => {
            let from = emit(b, module, from, held, w);
            let to = emit(b, module, to, held, w);
            Some((from, to))
        }
        Flow::While(_) => None,
    };

    let mut entering: Vec<qir::Value> =
        carried.iter().map(|i| held[*i].expect("carried")).collect();
    if let Some((from, _)) = bounds {
        entering.push(from);
        if keeps {
            // Never run is not the same as run once, and this is what says so: the
            // counter would have started here, and never got past the question.
            entering.push(from);
        }
    }
    b.jump(head, &entering);

    // The head: where the question is asked, every pass including the first.
    b.switch_to(head);
    for (n, i) in carried.iter().enumerate() {
        held[*i] = Some(b.block_param(head, n));
    }
    let counter = counting.then(|| b.block_param(head, carried.len()));
    if let Some(value) = counter {
        held[live] = Some(value);
    }
    let last = keeps.then(|| b.block_param(head, carried.len() + 1));

    let more = match flow {
        // Both ends included, which is why this is `<=` and not `<`.
        Flow::Range { .. } => {
            let (_, to) = bounds.expect("a range worked its bounds out above");
            b.cmp(qir::CmpOp::Le, counter.expect("a range counts"), to)
        }
        Flow::While(condition) => emit(b, module, condition, held, w),
    };

    let mut onward: Vec<qir::Value> = carried.iter().map(|i| held[*i].expect("carried")).collect();
    let mut leaving = onward.clone();
    if let Some(value) = counter {
        onward.push(value);
    }
    if let Some(value) = last {
        onward.push(value);
        leaving.push(value);
    }
    b.br_if(more, (pass, &onward), (done, &leaving));

    // One pass.
    b.switch_to(pass);
    for (n, i) in carried.iter().enumerate() {
        held[*i] = Some(b.block_param(pass, n));
    }
    if counting {
        held[live] = Some(b.block_param(pass, carried.len()));
    }

    loops.push(Frame { done, carried: carried.clone(), keep: keeps.then_some(live) });
    let broke = lower_body(b, module, body, held, w, loops);
    loops.pop();

    if !broke {
        let mut back: Vec<qir::Value> =
            carried.iter().map(|i| held[*i].expect("carried")).collect();
        if counting {
            let counter = held[live].expect("the counter, which nothing may change");
            let one = b.const_i64(1);
            let next = b.add(counter, one);
            back.push(next);
            if keeps {
                back.push(counter);
            }
        }
        b.jump(head, &back);
    }

    // Afterwards.
    b.switch_to(done);
    for (n, i) in carried.iter().enumerate() {
        held[*i] = Some(b.block_param(done, n));
    }
    // Whatever the body declared is gone at the closing brace, and holding on to its
    // values would hand a later join something defined where it cannot reach.
    for slot in held.iter_mut().skip(live) {
        *slot = None;
    }
    if keeps {
        held[live] = Some(b.block_param(done, carried.len()));
    }
}

/// `if` — arms asked in order, and the values they leave behind carried to one place.
///
/// This is where the lowering stops being a transliteration. Up to here a variable was
/// one QIR value and `set` overwrote which; a variable changed inside an arm is a
/// *different* value depending on which arm ran, and that is exactly what a block
/// parameter is for. So the join takes one parameter per variable that existed before
/// the `if`, and every path hands it whatever it ended up with.
///
/// Anything declared *inside* an arm is not carried. It is gone at the closing brace,
/// which the checker already enforced by scope and which `live` records.
fn lower_if(
    b: &mut qir::Builder,
    module: &mut qir::Module,
    arms: &[Arm],
    otherwise: Option<&[Stmt]>,
    live: u32,
    held: &mut Vec<Option<qir::Value>>,
    w: &Where<'_>,
    loops: &mut Vec<Frame>,
) -> bool {
    let carried: Vec<usize> = (0..live as usize).filter(|i| held[*i].is_some()).collect();
    let types: Vec<qir::Ty> =
        carried.iter().map(|i| qir_ty(&w.locals[*i].ty)).collect();
    let join = b.block(&types);

    // What each path hands the join: everything it is holding by then.
    let handed = |held: &[Option<qir::Value>]| -> Vec<qir::Value> {
        carried.iter().map(|i| held[*i].expect("checked above")).collect()
    };

    let before = held.clone();
    let mut reached = false;
    for arm in arms {
        let condition = emit(b, module, &arm.condition, held, w);
        let taken = b.block(&[]);
        let next = b.block(&[]);
        b.br_if(condition, (taken, &[]), (next, &[]));

        b.switch_to(taken);
        // Each arm starts from what was true before the `if`, not from what the arm
        // before it did -- only one of them ever runs.
        *held = before.clone();
        if !lower_body(b, module, &arm.body, held, w, loops) {
            let leaving = handed(held);
            b.jump(join, &leaving);
            reached = true;
        }

        b.switch_to(next);
    }

    // Nothing held. Whatever the `else` says, or nothing at all.
    *held = before.clone();
    let left = match otherwise {
        Some(body) => lower_body(b, module, body, held, w, loops),
        None => false,
    };
    if !left {
        let leaving = handed(held);
        b.jump(join, &leaving);
        reached = true;
    }

    b.switch_to(join);
    for (n, i) in carried.iter().enumerate() {
        held[*i] = Some(b.block_param(join, n));
    }
    for slot in held.iter_mut().skip(live as usize) {
        *slot = None;
    }

    if !reached {
        // Every arm gave an answer or left the loop, so nothing arrives here. The block
        // still needs an end on it before anything will delete it, and the end that is
        // always available is the one that needs nothing from anywhere: an answer made
        // on the spot, of the type this function was going to give back anyway.
        let made_up = nothing_of(b, module, w.ret);
        b.ret(made_up);
    }
    !reached
}

/// What a value already checked is, for the few places lowering has to ask.
///
/// Only the ones that can be an array, because that is the only type whose shape the
/// lowering has to know: everything else already says what it is in the IR.
fn ty_of(value: &Value, w: &Where<'_>) -> Option<Ty> {
    Some(match value {
        Value::Number { bits, signed, .. } => Ty::Int { bits: *bits, signed: *signed },
        Value::Copy(local) => w.locals[local.0 as usize].ty.clone(),
        Value::Copied(of) => ty_of(of, w)?,
        Value::Const(which) => w.checked.constants[*which as usize].ty.clone(),
        Value::Call { func, .. } => w.checked.funcs[*func as usize].returns.clone()?,
        // One allocation down from whatever holds it, which is the whole of what an
        // array of arrays adds.
        Value::At { array, .. } => match ty_of(array, w)? {
            Ty::Arr { of, .. } => *of,
            _ => return None,
        },
        _ => return None,
    })
}

/// What an array holds, as a checker type.
fn element_type(array: &Value, w: &Where<'_>) -> Option<Ty> {
    match ty_of(array, w)? {
        Ty::Arr { of, .. } => Some(*of),
        _ => None,
    }
}

/// What an array holds at the bottom, and how many allocations lie under the top one.
fn elements(of: &Ty) -> (qir::Elements, i64) {
    match of {
        Ty::Int { .. } => (qir::Elements::I64, 0),
        Ty::Bool => (qir::Elements::Bool, 0),
        Ty::Str => (qir::Elements::Text, 0),
        Ty::Exact => (qir::Elements::Exact, 0),
        Ty::Decimal { .. } => (qir::Elements::Decimal, 0),
        Ty::F64 | Ty::F32 | Ty::F16 => (qir::Elements::Float, 0),
        Ty::Arr { of, .. } => {
            let (kind, depth) = elements(of);
            (kind, depth + 1)
        }
    }
}

/// The same, worked out from a value the checker already typed.
fn elements_of(value: &Value, w: &Where<'_>) -> (qir::Elements, i64) {
    match ty_of(value, w) {
        Some(Ty::Arr { of, .. }) => elements(&of),
        _ => unreachable!("refused by the checker: not an array"),
    }
}

/// How many digits the decimal type of this value keeps.
///
/// Walked rather than looked up, because a written value carries its own digit count
/// and an operation between two of them carries whatever they had: the checker has
/// already refused every case where the two sides differ, so either side will do.
fn digits_of(value: &Value, w: &Where<'_>) -> i64 {
    match value {
        Value::Decimal { digits, .. } => i64::from(*digits),
        Value::Binary { lhs, .. } => digits_of(lhs, w),
        other => match ty_of(other, w) {
            Some(Ty::Decimal { digits }) => i64::from(digits),
            _ => unreachable!("refused by the checker: not a decimal"),
        },
    }
}

/// Arithmetic on exact numbers, which is a call rather than an instruction.
///
/// Both engines call the same code, so however large the numbers get they cannot answer
/// differently — which is the one thing an `e` could have cost the oracle and does not.
fn exactly(b: &mut qir::Builder, op: OpKind, l: qir::Value, r: qir::Value) -> qir::Value {
    let host = match op {
        OpKind::Add => qir::Host::ExactAdd,
        OpKind::Sub => qir::Host::ExactSub,
        OpKind::Mul => qir::Host::ExactMul,
        OpKind::Div => qir::Host::ExactDiv,
        // A negative exponent is fine for an `e` and is where it parts company with
        // `i64`: two to the minus one is a half, and a half is a number this holds.
        OpKind::Pow => qir::Host::ExactPow,
        // Six comparisons, one call: an `e` against an `e` is the sign of their
        // difference, and every comparison is that sign against zero.
        _ => {
            let sign = b.call_host(qir::Host::ExactCompare, &[l, r]);
            let zero = b.const_i64(0);
            let how = match op {
                OpKind::Lt => qir::CmpOp::Lt,
                OpKind::Gt => qir::CmpOp::Gt,
                OpKind::Le => qir::CmpOp::Le,
                OpKind::Ge => qir::CmpOp::Ge,
                OpKind::Eq => qir::CmpOp::Eq,
                OpKind::Ne => qir::CmpOp::Ne,
                _ => unreachable!("refused by the checker, or handled above"),
            };
            return b.cmp(how, sign, zero);
        }
    };
    b.call_host(host, &[l, r])
}

/// Arithmetic on decimal numbers, which is a call for the same reason an `e`'s is —
/// plus one of its own: rounding to seven digits or to sixteen is what the type *is*,
/// and no machine Quench targets does it.
///
/// How many digits to keep rides along as an argument rather than in the type, because
/// a `d32` and a `d64` are the same thing in a register and differ only in what each
/// operation rounds to.
fn decimally(
    b: &mut qir::Builder,
    op: OpKind,
    l: qir::Value,
    r: qir::Value,
    digits: i64,
) -> qir::Value {
    let host = match op {
        OpKind::Add => qir::Host::DecimalAdd,
        OpKind::Sub => qir::Host::DecimalSub,
        OpKind::Mul => qir::Host::DecimalMul,
        OpKind::Div => qir::Host::DecimalDiv,
        // Four answers rather than three: a not-a-number compares as none of less,
        // equal or greater, and every comparison has to hear about it separately.
        // `!==` is the one that says yes to it, and it says yes by not being `Eq`.
        _ => {
            let how = b.call_host(qir::Host::DecimalCompare, &[l, r]);
            let against = b.const_i64(match op {
                OpKind::Lt => -1,
                OpKind::Gt => 1,
                OpKind::Eq | OpKind::Ne => 0,
                // Less-or-equal and greater-or-equal are the two that cannot be one
                // comparison against one number, so they are one against two.
                OpKind::Le | OpKind::Ge => 0,
                _ => unreachable!("refused by the checker, or handled above"),
            });
            return match op {
                OpKind::Lt | OpKind::Gt | OpKind::Eq => b.cmp(qir::CmpOp::Eq, how, against),
                OpKind::Ne => b.cmp(qir::CmpOp::Ne, how, against),
                OpKind::Le => {
                    let one = b.const_i64(1);
                    let above = b.cmp(qir::CmpOp::Eq, how, one);
                    let unordered = b.const_i64(2);
                    let strange = b.cmp(qir::CmpOp::Eq, how, unordered);
                    let out = b.bin(qir::BinOp::Or, above, strange);
                    b.not(out)
                }
                OpKind::Ge => {
                    let less = b.const_i64(-1);
                    let below = b.cmp(qir::CmpOp::Eq, how, less);
                    let unordered = b.const_i64(2);
                    let strange = b.cmp(qir::CmpOp::Eq, how, unordered);
                    let out = b.bin(qir::BinOp::Or, below, strange);
                    b.not(out)
                }
                _ => unreachable!("handled above"),
            };
        }
    };
    let digits = b.const_i64(digits);
    b.call_host(host, &[l, r, digits])
}

/// A value of this type, standing for one that is never looked at.
fn nothing_of(b: &mut qir::Builder, module: &mut qir::Module, ty: qir::Ty) -> qir::Value {
    match ty {
        qir::Ty::Bool => b.const_bool(false),
        qir::Ty::Text => {
            let at = module.intern("");
            b.const_text(at)
        }
        qir::Ty::I64 | qir::Ty::Handle => b.const_i64(0),
        // Never looked at, and so never read. Making one would mean calling into the
        // runtime for a number the checker has already promised nobody wants.
        qir::Ty::Exact | qir::Ty::Decimal => b.const_i64(0),
        qir::Ty::F64 => b.const_float(0f64.to_bits(), qir::Ty::F64),
        qir::Ty::F32 => b.const_float(u64::from(0f32.to_bits()), qir::Ty::F32),
        qir::Ty::F16 => b.const_float(u64::from(0f32.to_bits()), qir::Ty::F16),
    }
}

/// Where element (i, j, …) sits in a block laid out row by row.
///
/// Counting from one, so each index is shifted down before it is scaled. This is the
/// whole of what one `arr` link costs to index: no handle is followed on the way.
fn flat_index(
    b: &mut qir::Builder,
    module: &mut qir::Module,
    indices: &[Value],
    shape: &[usize],
    held: &[Option<qir::Value>],
    w: &Where<'_>,
) -> qir::Value {
    let mut flat = None;
    for (n, index) in indices.iter().enumerate() {
        let this = emit(b, module, index, held, w);
        flat = Some(match flat {
            None => this,
            Some(so_far) => {
                let one = b.const_i64(1);
                let zeroed = b.sub(so_far, one);
                let stride = b.const_i64(shape[n] as i64);
                let scaled = b.mul(zeroed, stride);
                b.add(scaled, this)
            }
        });
    }
    flat.expect("the checker refused an index with no numbers in it")
}

/// One value, put into the IR.
fn emit(
    b: &mut qir::Builder,
    module: &mut qir::Module,
    value: &Value,
    held: &[Option<qir::Value>],
    w: &Where<'_>,
) -> qir::Value {
    match value {
        Value::Text(text) => {
            let at = module.intern(text);
            b.const_text(at)
        }
        Value::Number { value, .. } => b.const_i64(*value),
        Value::Float { bits, width } => {
            let ty = match width {
                16 => qir::Ty::F16,
                32 => qir::Ty::F32,
                _ => qir::Ty::F64,
            };
            b.const_float(*bits, ty)
        }
        // Read by the runtime, from the text it was written with -- because what it
        // reads to does not fit in anything the IR can carry.
        Value::Exact(written) => {
            let at = module.intern(written);
            let text = b.const_text(at);
            b.call_host(qir::Host::ExactRead, &[text])
        }
        // The same, and rounded to the digits its type keeps as it is read: `*0.1*`
        // under a `d32` is a tenth to seven digits from the moment it exists.
        Value::Decimal { written, digits } => {
            let at = module.intern(written);
            let text = b.const_text(at);
            let digits = b.const_i64(i64::from(*digits));
            b.call_host(qir::Host::DecimalRead, &[text, digits])
        }
        Value::Bool(yes) => b.const_bool(*yes),
        // Values do not change, so copying one is naming the same value again rather
        // than doing anything.
        Value::Copy(from) => held[from.0 as usize].expect("declared before used"),
        // The array is made, then filled one element at a time. Both are host calls:
        // asking for memory is a runtime service, and this is the first time Quench
        // asks. Nothing frees it yet, which is the first stage of the collector.
        Value::Array { of, elements: written } => {
            let len = b.const_i64(written.len() as i64);
            // The header: what the slots hold, so a collector can tell a number to
            // leave alone from a handle to follow. It comes from the type rather than
            // from the elements, because an array written empty has none — and an array
            // written empty is exactly what a growing one starts as.
            let (leaf, depth) = elements(of);
            let kind = b.const_i64(leaf as i64);
            let deep = b.const_i64(depth);
            let handle = b.call_host(qir::Host::ArrayNew, &[len, kind, deep]);
            for (n, element) in written.iter().enumerate() {
                let at = b.const_i64(n as i64 + 1); // counted from one
                let value = emit(b, module, element, held, w);
                b.call_host(qir::Host::ArraySet, &[handle, at, value]);
            }
            handle
        }
        // Row by row: element (i, j) of a (2 3) is at (i - 1) x 3 + j, counting from one.
        // Which is why one `arr` link is one allocation -- the whole address is
        // arithmetic, and no handle is followed on the way.
        Value::At { array, indices, shape } => {
            let handle = emit(b, module, array, held, w);
            let at = flat_index(b, module, indices, shape, held, w);
            // What comes out is whatever the array holds, and the IR is told here
            // because this is the first place anything could use it.
            let of = match element_type(array, w) {
                Some(ty) => qir_ty(&ty),
                None => qir::Ty::I64,
            };
            b.call_host_giving(qir::Host::ArrayGet, &[handle, at], of)
        }
        // A constant has no storage: its value is written in here, wherever it was
        // named. Which is the whole of what the word means.
        Value::Const(which) => {
            let constant = &w.checked.constants[*which as usize];
            // An array's value is its place in the module's tables, which was decided
            // before any of this was written. So a constant array costs nothing at all
            // where it is named -- which is what `const` ought to mean.
            if let Ty::Arr { .. } = constant.ty {
                return b.const_handle(w.tables[*which as usize].expect("laid out above"));
            }
            let value = constant.value.clone();
            emit(b, module, &value, held, w)
        }
        Value::Call { func, args } => {
            let given: Vec<qir::Value> =
                args.iter().map(|arg| emit(b, module, arg, held, w)).collect();
            let ret =
                w.checked.funcs[*func as usize].returns.as_ref().map_or(qir::Ty::I64, qir_ty);
            b.call(qir::FuncId(*func), &given, ret)
        }
        // Asked while it runs, because the shape said `grow` and so nobody knew.
        Value::Count(of) => {
            let handle = emit(b, module, of, held, w);
            b.call_host(qir::Host::ArrayLen, &[handle])
        }
        // One piece at a time, left to right, because that is the order they were
        // written and joining is not something to be clever about.
        Value::Join(pieces) => {
            let mut joined: Option<qir::Value> = None;
            for piece in pieces {
                let next = emit(b, module, piece, held, w);
                joined = Some(match joined {
                    None => next,
                    Some(so_far) => b.call_host(qir::Host::TextJoin, &[so_far, next]),
                });
            }
            joined.expect("the checker refused a join with nothing in it")
        }
        Value::Not(of) => {
            let value = emit(b, module, of, held, w);
            b.not(value)
        }
        // `share` needs nothing here: naming the variable already gave the handle, and
        // sharing is what a handle does. `copy` is what costs something, which is why
        // it is the one that had to be written down.
        Value::Copied(of) => {
            let handle = emit(b, module, of, held, w);
            b.call_host(qir::Host::ArrayCopy, &[handle])
        }
        // Stopping early is control flow, because that is what stopping early *is*: the
        // right side has to sit in a block that only one of the two paths reaches.
        Value::Binary { op: op @ (OpKind::And | OpKind::Or), lhs, rhs }
            if w.settings.logic == Logic::StopsEarly =>
        {
            let left = emit(b, module, lhs, held, w);
            let join = b.block(&[qir::Ty::Bool]);
            let rest = b.block(&[]);

            // `and` asks the right side when the left was true, `or` when it was false,
            // and either way the left side is the answer when it is not asked.
            let settled = b.const_bool(*op == OpKind::Or);
            match op {
                OpKind::And => b.br_if(left, (rest, &[]), (join, &[settled])),
                _ => b.br_if(left, (join, &[settled]), (rest, &[])),
            }

            b.switch_to(rest);
            let right = emit(b, module, rhs, held, w);
            b.jump(join, &[right]);

            b.switch_to(join);
            b.block_param(join, 0)
        }
        Value::Binary { op, lhs, rhs } => {
            let l = emit(b, module, lhs, held, w);
            let r = emit(b, module, rhs, held, w);

            // An `e` is arithmetic the runtime does, because the answer does not fit in
            // a register and the two engines must not each have their own idea of it.
            if b.ty_of(l) == qir::Ty::Exact {
                return exactly(b, *op, l, r);
            }
            if b.ty_of(l) == qir::Ty::Decimal {
                return decimally(b, *op, l, r, digits_of(lhs, w));
            }
            // Two arrays hold the same things or they do not, which is a walk of both
            // and so a call. Not whether they are the same array — `share` is what makes
            // two names for one, and this is the other question.
            if b.ty_of(l) == qir::Ty::Handle {
                let (leaf, depth) = elements_of(lhs, w);
                let kind = b.const_i64(leaf as i64);
                let deep = b.const_i64(depth);
                let same = b.call_host(qir::Host::ArrayEqual, &[l, r, kind, deep]);
                return match op {
                    OpKind::Eq => same,
                    OpKind::Ne => b.not(same),
                    _ => unreachable!("refused by the checker: arrays have no order"),
                };
            }
            // IEEE, and nothing else. What a compiler could do to make two engines
            // differ — fuse a multiply into an add, keep extra precision, flush a
            // denormal to nought — it only does when asked, and nothing here asks.
            if matches!(b.ty_of(l), qir::Ty::F64 | qir::Ty::F32 | qir::Ty::F16) {
                let half = b.ty_of(l) == qir::Ty::F16;
                let stops = w.settings.no_number == NoNumber::Stops;
                // A `b16` is worked out in its carrier and put back afterwards. One
                // `f32` operation rounded once to binary16 *is* binary16's own answer,
                // which is why this is a rounding rather than an approximation.
                let narrow = |b: &mut qir::Builder, v: qir::Value| {
                    if !half {
                        return v;
                    }
                    let ty = b.ty_of(v);
                    b.call_host_giving(qir::Host::ToB16, &[v], ty)
                };
                return match op {
                    OpKind::Add if stops => { let v = b.bin(qir::BinOp::FAddChecked, l, r); narrow(b, v) }
                    OpKind::Sub if stops => { let v = b.bin(qir::BinOp::FSubChecked, l, r); narrow(b, v) }
                    OpKind::Mul if stops => { let v = b.bin(qir::BinOp::FMulChecked, l, r); narrow(b, v) }
                    OpKind::Div if stops => { let v = b.bin(qir::BinOp::FDivChecked, l, r); narrow(b, v) }
                    OpKind::Add => { let v = b.bin(qir::BinOp::FAdd, l, r); narrow(b, v) }
                    OpKind::Sub => { let v = b.bin(qir::BinOp::FSub, l, r); narrow(b, v) }
                    OpKind::Mul => { let v = b.bin(qir::BinOp::FMul, l, r); narrow(b, v) }
                    OpKind::Div => { let v = b.bin(qir::BinOp::FDiv, l, r); narrow(b, v) }
                    OpKind::Lt => b.fcmp(qir::CmpOp::Lt, l, r),
                    OpKind::Gt => b.fcmp(qir::CmpOp::Gt, l, r),
                    OpKind::Le => b.fcmp(qir::CmpOp::Le, l, r),
                    OpKind::Ge => b.fcmp(qir::CmpOp::Ge, l, r),
                    OpKind::Eq => b.fcmp(qir::CmpOp::Eq, l, r),
                    OpKind::Ne => b.fcmp(qir::CmpOp::Ne, l, r),
                    _ => unreachable!("refused by the checker: not built for a `b64`"),
                };
            }
            // Text is compared by what it holds rather than by which piece it is, which
            // is also a call: an index is not the thing it points at.
            if b.ty_of(l) == qir::Ty::Text {
                let order = b.call_host(qir::Host::TextCompare, &[l, r]);
                let zero = b.const_i64(0);
                let how = match op {
                    OpKind::Eq => qir::CmpOp::Eq,
                    OpKind::Ne => qir::CmpOp::Ne,
                    _ => unreachable!("refused by the checker: text has no order yet"),
                };
                return b.cmp(how, order, zero);
            }

            let floored = w.settings.division == Division::Floored;
            // Which whole-number type this is. Every one rides in an `i64`, so what
            // makes a `u8` a `u8` is the narrowing after the operation and the
            // unsigned reading of the ones that care.
            let (bits, signed) = match ty_of(lhs, w) {
                Some(Ty::Int { bits, signed }) => (bits, signed),
                _ => (64, true),
            };
            let stops = w.settings.overflow == Overflow::Trap;
            // A narrower type finds its own overflow when it is put back: an `i64` add
            // of two `u8`s cannot overflow, and 256 is not a `u8`. Only the widest two
            // need the operation itself to notice.
            let widest = bits >= 64;
            let put = |b: &mut qir::Builder, v: qir::Value| {
                if widest { v } else { b.narrow(v, bits, signed, stops) }
            };
            match op {
                // Whether a sum that does not fit rounds or stops is the project's
                // decision, written down here as an instruction.
                OpKind::Add if stops && widest && signed => b.add_trapping(l, r),
                OpKind::Sub if stops && widest && signed => b.sub_trapping(l, r),
                OpKind::Mul if stops && widest && signed => b.mul_trapping(l, r),
                OpKind::Add if stops && widest => b.bin(qir::BinOp::AddTrappingU, l, r),
                OpKind::Sub if stops && widest => b.bin(qir::BinOp::SubTrappingU, l, r),
                OpKind::Mul if stops && widest => b.bin(qir::BinOp::MulTrappingU, l, r),
                OpKind::Add => { let v = b.add(l, r); put(b, v) }
                OpKind::Sub => { let v = b.sub(l, r); put(b, v) }
                OpKind::Mul => { let v = b.mul(l, r); put(b, v) }
                // The project's decision, written down here as an instruction so that no
                // backend has to know a setting was ever involved. An unsigned division
                // has neither of the two edges a signed one has.
                OpKind::Div if !signed => { let v = b.bin(qir::BinOp::DivU, l, r); put(b, v) }
                OpKind::Mod if !signed => { let v = b.bin(qir::BinOp::RemU, l, r); put(b, v) }
                OpKind::Div => {
                    let v = if floored { b.div_floored(l, r) } else { b.div(l, r) };
                    put(b, v)
                }
                OpKind::Mod => {
                    let v = if floored { b.rem_floored(l, r) } else { b.rem(l, r) };
                    put(b, v)
                }
                // By squaring, in the runtime rather than as an instruction: it needs
                // a loop, and two engines each writing their own would be two chances
                // to write it differently.
                OpKind::Pow if stops => {
                    let v = b.call_host(qir::Host::PowI64Trapping, &[l, r]);
                    put(b, v)
                }
                OpKind::Pow => { let v = b.call_host(qir::Host::PowI64, &[l, r]); put(b, v) }
                // A `u64` past `i64::MAX` is a negative number in a slot and is not a
                // negative number. Every narrower unsigned type is normalised into the
                // positive half and orders the same either way, so only this one asks.
                OpKind::Lt if !signed && widest => b.cmp_unsigned(qir::CmpOp::Lt, l, r),
                OpKind::Gt if !signed && widest => b.cmp_unsigned(qir::CmpOp::Gt, l, r),
                OpKind::Le if !signed && widest => b.cmp_unsigned(qir::CmpOp::Le, l, r),
                OpKind::Ge if !signed && widest => b.cmp_unsigned(qir::CmpOp::Ge, l, r),
                OpKind::Lt => b.cmp(qir::CmpOp::Lt, l, r),
                OpKind::Gt => b.cmp(qir::CmpOp::Gt, l, r),
                OpKind::Le => b.cmp(qir::CmpOp::Le, l, r),
                OpKind::Ge => b.cmp(qir::CmpOp::Ge, l, r),
                OpKind::Eq => b.cmp(qir::CmpOp::Eq, l, r),
                OpKind::Ne => b.cmp(qir::CmpOp::Ne, l, r),
                // Only reached under `asks-both`; stopping early was handled above.
                OpKind::And => b.bin(qir::BinOp::And, l, r),
                OpKind::Or => b.bin(qir::BinOp::Or, l, r),
            }
        }
    }
}
