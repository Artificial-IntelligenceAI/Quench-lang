//! The meaning of a program, turned into the IR every engine runs.
//!
//! There are no decisions here. [`quench_check`] resolved every name, settled every
//! type, joined every piece of text and refused everything that did not make sense — so
//! what is left is a transliteration, and that is the point of doing the checking first.
//! Anything in this file that started to look like a judgement would belong further up.

use quench_check::{Arm, Checked, Flow, OpKind, Place, Printed, Stmt, Ty, Value};
use quench_conf::{Division, Overflow, Settings};
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

    if !checked.has_start {
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
    let mut b = qir::Builder::new(qir::ENTRY, &[], qir::Ty::I64);

    // Where each variable's value ended up. A declaration fills one in; a use reads it;
    // a join replaces the ones that could have come from either side.
    let mut held: Vec<Option<qir::Value>> = vec![None; checked.locals.len()];

    let mut loops = Vec::new();
    lower_body(&mut b, &mut module, &checked.body, &mut held, checked, &mut loops, settings);

    // A program that says nothing about how it ended, ended fine.
    let nothing = b.const_i64(0);
    b.ret(nothing);

    let id = module.add(b.finish());
    module.set_entry(id);
    module
}

/// What a QIR value of this type looks like.
fn qir_ty(ty: &Ty) -> qir::Ty {
    match ty {
        Ty::I64 => qir::Ty::I64,
        Ty::Bool => qir::Ty::Bool,
        Ty::Str => qir::Ty::Text,
        Ty::Arr { .. } => qir::Ty::Handle,
    }
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
    checked: &Checked,
    loops: &mut Vec<Frame>,
    settings: Settings,
) -> bool {
    for stmt in body {
        match stmt {
            Stmt::Declare { local, value } => {
                let value = emit(b, module, value, held, settings);
                held[local.0 as usize] = Some(value);
            }
            // Changing a variable is naming a new value for it. Inside an arm that is
            // still just a write here -- what makes it correct is that the join below
            // takes whichever value the branch that ran left behind.
            Stmt::Assign { to, value } => {
                let value = emit(b, module, value, held, settings);
                match to {
                    Place::Local(local) => held[local.0 as usize] = Some(value),
                    Place::Element { local, indices, shape } => {
                        let handle = held[local.0 as usize].expect("declared before used");
                        let at = flat_index(b, module, indices, shape, held, settings);
                        b.call_host(qir::Host::ArraySet, &[handle, at, value]);
                    }
                }
            }
            Stmt::If { arms, otherwise, live } => {
                let left = lower_if(
                    b, module, arms, otherwise.as_deref(), *live, held, checked, loops, settings,
                );
                if left {
                    return true;
                }
            }
            Stmt::Loop { flow, body, live } => {
                lower_loop(b, module, flow, body, *live, held, checked, loops, settings);
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
                            let value = emit(b, module, value, held, settings);
                            let host = match ty {
                                Ty::Str => qir::Host::PrintText,
                                Ty::I64 => qir::Host::PrintI64,
                                Ty::Bool => qir::Host::PrintBool,
                                Ty::Arr { .. } => {
                                    unreachable!("refused by the checker: an array is not printed")
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
    checked: &Checked,
    loops: &mut Vec<Frame>,
    settings: Settings,
) {
    let live = live as usize;
    let carried: Vec<usize> = (0..live).filter(|i| held[*i].is_some()).collect();
    let carried_types: Vec<qir::Ty> =
        carried.iter().map(|i| qir_ty(&checked.locals[*i].ty)).collect();

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
            let from = emit(b, module, from, held, settings);
            let to = emit(b, module, to, held, settings);
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
        Flow::While(condition) => emit(b, module, condition, held, settings),
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
    let broke = lower_body(b, module, body, held, checked, loops, settings);
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
    checked: &Checked,
    loops: &mut Vec<Frame>,
    settings: Settings,
) -> bool {
    let carried: Vec<usize> = (0..live as usize).filter(|i| held[*i].is_some()).collect();
    let types: Vec<qir::Ty> =
        carried.iter().map(|i| qir_ty(&checked.locals[*i].ty)).collect();
    let join = b.block(&types);

    // What each path hands the join: everything it is holding by then.
    let handed = |held: &[Option<qir::Value>]| -> Vec<qir::Value> {
        carried.iter().map(|i| held[*i].expect("checked above")).collect()
    };

    let before = held.clone();
    let mut reached = false;
    for arm in arms {
        let condition = emit(b, module, &arm.condition, held, settings);
        let taken = b.block(&[]);
        let next = b.block(&[]);
        b.br_if(condition, (taken, &[]), (next, &[]));

        b.switch_to(taken);
        // Each arm starts from what was true before the `if`, not from what the arm
        // before it did -- only one of them ever runs.
        *held = before.clone();
        if !lower_body(b, module, &arm.body, held, checked, loops, settings) {
            let leaving = handed(held);
            b.jump(join, &leaving);
            reached = true;
        }

        b.switch_to(next);
    }

    // Nothing held. Whatever the `else` says, or nothing at all.
    *held = before.clone();
    let left = match otherwise {
        Some(body) => lower_body(b, module, body, held, checked, loops, settings),
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
        // Every arm left the loop, so nothing arrives here. The block still needs an end
        // on it, and it takes one that uses only its own parameters — which is what makes
        // it well formed while still being unreachable, and so removable.
        let frame = loops.last().expect("every path broke, so there is a loop to break out of");
        let mut leaving: Vec<qir::Value> =
            frame.carried.iter().map(|i| held[*i].expect("carried")).collect();
        if let Some(i) = frame.keep {
            leaving.push(held[i].expect("the counter"));
        }
        b.jump(frame.done, &leaving);
    }
    !reached
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
    settings: Settings,
) -> qir::Value {
    let mut flat = None;
    for (n, index) in indices.iter().enumerate() {
        let this = emit(b, module, index, held, settings);
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
    settings: Settings,
) -> qir::Value {
    match value {
        Value::Text(text) => {
            let at = module.intern(text);
            b.const_text(at)
        }
        Value::Number(n) => b.const_i64(*n),
        Value::Bool(yes) => b.const_bool(*yes),
        // Values do not change, so copying one is naming the same value again rather
        // than doing anything.
        Value::Copy(from) => held[from.0 as usize].expect("declared before used"),
        // The array is made, then filled one element at a time. Both are host calls:
        // asking for memory is a runtime service, and this is the first time Quench
        // asks. Nothing frees it yet, which is the first stage of the collector.
        Value::Array(elements) => {
            let len = b.const_i64(elements.len() as i64);
            let handle = b.call_host(qir::Host::ArrayNew, &[len]);
            for (n, element) in elements.iter().enumerate() {
                let at = b.const_i64(n as i64 + 1); // counted from one
                let value = emit(b, module, element, held, settings);
                b.call_host(qir::Host::ArraySet, &[handle, at, value]);
            }
            handle
        }
        // Row by row: element (i, j) of a (2 3) is at (i - 1) x 3 + j, counting from one.
        // Which is why one `arr` link is one allocation -- the whole address is
        // arithmetic, and no handle is followed on the way.
        Value::At { array, indices, shape } => {
            let handle = emit(b, module, array, held, settings);
            let at = flat_index(b, module, indices, shape, held, settings);
            b.call_host(qir::Host::ArrayGet, &[handle, at])
        }
        Value::Binary { op, lhs, rhs } => {
            let l = emit(b, module, lhs, held, settings);
            let r = emit(b, module, rhs, held, settings);
            let floored = settings.division == Division::Floored;
            match op {
                // Whether a sum that does not fit rounds or stops is the project's
                // decision, written down here as an instruction.
                OpKind::Add if settings.overflow == Overflow::Trap => b.add_trapping(l, r),
                OpKind::Sub if settings.overflow == Overflow::Trap => b.sub_trapping(l, r),
                OpKind::Mul if settings.overflow == Overflow::Trap => b.mul_trapping(l, r),
                OpKind::Add => b.add(l, r),
                OpKind::Sub => b.sub(l, r),
                OpKind::Mul => b.mul(l, r),
                // The project's decision, written down here as an instruction so that no
                // backend has to know a setting was ever involved.
                OpKind::Div => {
                    if floored { b.div_floored(l, r) } else { b.div(l, r) }
                }
                OpKind::Mod => {
                    if floored { b.rem_floored(l, r) } else { b.rem(l, r) }
                }
                OpKind::Lt => b.cmp(qir::CmpOp::Lt, l, r),
                OpKind::Gt => b.cmp(qir::CmpOp::Gt, l, r),
                OpKind::Le => b.cmp(qir::CmpOp::Le, l, r),
                OpKind::Ge => b.cmp(qir::CmpOp::Ge, l, r),
                OpKind::Eq => b.cmp(qir::CmpOp::Eq, l, r),
                OpKind::Ne => b.cmp(qir::CmpOp::Ne, l, r),
                OpKind::Pow | OpKind::And | OpKind::Or => {
                    unreachable!("refused by the checker as not built yet")
                }
            }
        }
    }
}
