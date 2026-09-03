//! Where a running program's values live, and how they stop living.
//!
//! Three spaces, because three kinds of thing allocate and none of them is the others:
//! arrays, pieces of text, and exact numbers. A handle is an index into one of them,
//! which is what makes it a handle rather than a pointer — nothing generated ever holds
//! an address, so nothing generated has to be told when one moves. Nothing moves here
//! either, which is what makes this the second of the three stages in
//! `notes/the-collector-earns-its-place.md` and not the third.
//!
//! # The header
//!
//! A slot is an `i64` whatever is in it. Nothing about the bits says whether a slot of
//! an array is a number to leave alone or a handle to follow, so every array carries
//! what it holds — [`Elements`] and how many allocations lie under it. That is the
//! whole object header, and it is the thing stage one was said to need and never had.
//!
//! # What is a root
//!
//! Everything a running program could still reach: the slots of every frame on the call
//! stack whose type is a reference, and everything the module was written with. The
//! second is permanent — a constant array or a written piece of text is in the artefact
//! and outlives every collection.

use quench_num::{Decimal, Exact};
use quench_qir::{self as qir, Elements};

/// Which space a handle is an index into, packed into the top byte of a root.
///
/// The interpreter knows a slot's space from QIR, which says what every value in a
/// function is. Compiled code has no such thing to hand at run time — a root there is
/// an `i64` in an array — so the space rides along in bits nothing else uses. A handle
/// is an index into a list, and no list is going to have `2^56` things in it.
pub const SPACE_SHIFT: u32 = 56;

/// Turn a handle and its space into one number, for compiled code to store.
pub fn rooted(ty: qir::Ty, handle: i64) -> i64 {
    let space = match ty {
        qir::Ty::Handle => 1,
        qir::Ty::Text => 2,
        qir::Ty::Exact => 3,
        qir::Ty::Decimal => 4,
        _ => 0,
    };
    (space << SPACE_SHIFT) | (handle & ((1 << SPACE_SHIFT) - 1))
}

/// And back again, for the collector to read.
pub fn unrooted(packed: i64) -> Option<(qir::Ty, i64)> {
    let handle = packed & ((1 << SPACE_SHIFT) - 1);
    Some(match packed >> SPACE_SHIFT {
        1 => (qir::Ty::Handle, handle),
        2 => (qir::Ty::Text, handle),
        3 => (qir::Ty::Exact, handle),
        4 => (qir::Ty::Decimal, handle),
        _ => return None,
    })
}

/// One array on the heap, with the header that says what its slots are.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Object {
    pub holds: Elements,
    /// How many allocations lie under this one. Nought when its slots hold values.
    pub depth: i64,
    pub values: Vec<i64>,
}

impl Object {
    /// Whether this object's slots are handles to follow rather than values.
    fn points_somewhere(&self) -> bool {
        self.depth > 0 || self.holds.is_a_reference()
    }
}

/// One space of one kind of thing, with the slots nothing is using any more.
struct Space<T> {
    held: Vec<Option<T>>,
    free: Vec<usize>,
    /// How many at the front were laid out before the program ran. Never collected:
    /// they are what the program was written with rather than what it made.
    written: usize,
}

impl<T> Default for Space<T> {
    fn default() -> Self {
        Space { held: Vec::new(), free: Vec::new(), written: 0 }
    }
}

impl<T> Space<T> {
    fn new(start: Vec<T>) -> Self {
        let written = start.len();
        Space { held: start.into_iter().map(Some).collect(), free: Vec::new(), written }
    }

    fn put(&mut self, value: T) -> i64 {
        match self.free.pop() {
            Some(at) => {
                self.held[at] = Some(value);
                at as i64
            }
            None => {
                self.held.push(Some(value));
                self.held.len() as i64 - 1
            }
        }
    }

    fn get(&self, at: i64) -> &T {
        self.held[at as usize].as_ref().expect("a handle to something that was freed")
    }

    fn get_mut(&mut self, at: i64) -> &mut T {
        self.held[at as usize].as_mut().expect("a handle to something that was freed")
    }

    /// Let go of everything unmarked, and remember where it was.
    fn sweep(&mut self, marked: &[bool]) -> usize {
        let mut freed = 0;
        for at in self.written..self.held.len() {
            if !marked[at] && self.held[at].is_some() {
                self.held[at] = None;
                self.free.push(at);
                freed += 1;
            }
        }
        freed
    }

    /// How many are alive, for a test to look at.
    fn live(&self) -> usize {
        self.held.iter().filter(|slot| slot.is_some()).count()
    }
}

/// Everything a running program has made, and everything it was written with.
#[derive(Default)]
pub struct Heap {
    arrays: Space<Object>,
    texts: Space<String>,
    exacts: Space<Exact>,
    decimals: Space<Decimal>,
    /// How many things have been made since the last collection.
    since: usize,
    /// How many to allow before collecting again. Grows with the live set, so a program
    /// that genuinely holds a lot does not collect on every allocation.
    allow: usize,
    /// How many collections have happened, for a test to look at.
    pub collections: usize,
}

/// How many allocations to let by before the first collection.
const FIRST: usize = 256;

impl Heap {
    /// A heap laid out with what the module was written with, before anything runs.
    pub fn new(module: &qir::Module) -> Heap {
        Heap::laid_out(&module.tables, &module.text)
    }

    /// The same, from the pieces rather than the module — which is what compiled code
    /// has to hand, the module being long gone by the time it runs.
    pub fn laid_out(tables: &[Vec<i64>], text: &[String]) -> Heap {
        let tables = tables
            .iter()
            .map(|values| Object { holds: Elements::I64, depth: 0, values: values.clone() })
            .collect();
        Heap {
            arrays: Space::new(tables),
            texts: Space::new(text.to_vec()),
            exacts: Space::new(Vec::new()),
            decimals: Space::new(Vec::new()),
            since: 0,
            allow: FIRST,
            collections: 0,
        }
    }

    /// Everything a set of packed roots reaches. What compiled code hands over.
    pub fn collect_packed(&mut self, packed: &[i64]) {
        let roots: Vec<(qir::Ty, i64)> = packed.iter().filter_map(|p| unrooted(*p)).collect();
        self.collect(&roots);
    }

    pub fn make(&mut self, holds: Elements, depth: i64, values: Vec<i64>) -> i64 {
        self.since += 1;
        self.arrays.put(Object { holds, depth, values })
    }

    pub fn text(&mut self, value: String) -> i64 {
        self.since += 1;
        self.texts.put(value)
    }

    pub fn exact(&mut self, value: Exact) -> i64 {
        self.since += 1;
        self.exacts.put(value)
    }

    pub fn decimal(&mut self, value: Decimal) -> i64 {
        self.since += 1;
        self.decimals.put(value)
    }

    pub fn decimally(&self, at: i64) -> &Decimal {
        self.decimals.get(at)
    }

    pub fn at(&self, handle: i64) -> &Object {
        self.arrays.get(handle)
    }

    pub fn at_mut(&mut self, handle: i64) -> &mut Object {
        self.arrays.get_mut(handle)
    }

    pub fn said(&self, at: i64) -> &str {
        self.texts.get(at)
    }

    pub fn exactly(&self, at: i64) -> &Exact {
        self.exacts.get(at)
    }

    /// Whether enough has been made to be worth looking.
    pub fn worth_collecting(&self) -> bool {
        self.since >= self.allow
    }

    /// How many things are alive in each space, for a test to look at.
    pub fn live(&self) -> (usize, usize, usize) {
        (self.arrays.live(), self.texts.live(), self.exacts.live() + self.decimals.live())
    }

    /// Mark from these roots, then let go of everything else.
    ///
    /// Nothing moves, so no handle anywhere has to be corrected afterwards — which is
    /// the whole reason this stage arrives before statepoints do.
    pub fn collect(&mut self, roots: &[(qir::Ty, i64)]) {
        let mut arrays = vec![false; self.arrays.held.len()];
        let mut texts = vec![false; self.texts.held.len()];
        let mut exacts = vec![false; self.exacts.held.len()];
        let mut decimals = vec![false; self.decimals.held.len()];

        // What the program was written with is always reachable: it is in the artefact.
        for slot in arrays.iter_mut().take(self.arrays.written) {
            *slot = true;
        }
        for slot in texts.iter_mut().take(self.texts.written) {
            *slot = true;
        }

        let mut grey: Vec<i64> = Vec::new();
        for (ty, value) in roots {
            match ty {
                qir::Ty::Handle => {
                    if let Some(seen) = arrays.get_mut(*value as usize) {
                        if !*seen {
                            *seen = true;
                            grey.push(*value);
                        }
                    }
                }
                qir::Ty::Text => {
                    if let Some(seen) = texts.get_mut(*value as usize) {
                        *seen = true;
                    }
                }
                qir::Ty::Exact => {
                    if let Some(seen) = exacts.get_mut(*value as usize) {
                        *seen = true;
                    }
                }
                qir::Ty::Decimal => {
                    if let Some(seen) = decimals.get_mut(*value as usize) {
                        *seen = true;
                    }
                }
                _ => {}
            }
        }

        // An array of arrays is the only thing in Quench with edges to follow, and an
        // array of text or of `e` is the only thing that reaches the other two spaces.
        // Everything else is a leaf, which is why this loop is as short as it is.
        while let Some(handle) = grey.pop() {
            let Some(object) = self.arrays.held[handle as usize].as_ref() else { continue };
            if !object.points_somewhere() {
                continue;
            }
            let (holds, depth) = (object.holds, object.depth);
            let slots = object.values.clone();
            for slot in slots {
                if depth > 0 {
                    if let Some(seen) = arrays.get_mut(slot as usize) {
                        if !*seen {
                            *seen = true;
                            grey.push(slot);
                        }
                    }
                    continue;
                }
                match holds {
                    Elements::Text => {
                        if let Some(seen) = texts.get_mut(slot as usize) {
                            *seen = true;
                        }
                    }
                    Elements::Exact => {
                        if let Some(seen) = exacts.get_mut(slot as usize) {
                            *seen = true;
                        }
                    }
                    Elements::Decimal => {
                        if let Some(seen) = decimals.get_mut(slot as usize) {
                            *seen = true;
                        }
                    }
                    _ => {}
                }
            }
        }

        self.arrays.sweep(&arrays);
        self.texts.sweep(&texts);
        self.exacts.sweep(&exacts);
        self.decimals.sweep(&decimals);

        // Twice what survived, so a program that genuinely holds a lot stops collecting
        // on every allocation and one that holds nothing keeps its heap small.
        let (a, t, e) = self.live();
        self.allow = FIRST.max((a + t + e) * 2);
        self.since = 0;
        self.collections += 1;
    }
}
