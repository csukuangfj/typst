//! Function memoization.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::{self, Display, Formatter};
use std::hash::Hasher;

thread_local! {
    /// The thread-local cache.
    static CACHE: RefCell<Cache> = RefCell::default();
}

/// A map from hashes to cache entries.
type Cache = HashMap<u64, CacheEntry>;

/// Access the cache.
fn with<F, R>(f: F) -> R
where
    F: FnOnce(&mut Cache) -> R,
{
    CACHE.with(|cell| f(&mut cell.borrow_mut()))
}

/// An entry in the cache.
struct CacheEntry {
    /// The memoized function's result plus constraints on the input.
    data: Box<dyn Any>,
    /// How many evictions have passed since the entry has been last used.
    age: usize,
}

/// Execute a memoized function call.
///
/// This hashes all inputs to the function and then either returns a cached
/// version from the thread-local cache or executes the function and saves a
/// copy of the results in the cache.
///
/// Note that `f` must be a pure function.
pub fn memoized<I, O>(input: I, f: fn(input: I) -> (O, I::Constraint)) -> O
where
    I: Track,
    O: Clone + 'static,
{
    memoized_ref(input, f, Clone::clone)
}

/// Execute a function and then call another function with a reference to the
/// result.
///
/// This hashes all inputs to the function and then either
/// - calls `g` with a cached version from the thread-local cache,
/// - or executes `f`, calls `g` with the fresh version and saves the result in
///   the cache.
///
/// Note that `f` must be a pure function, while `g` does not need to be pure.
pub fn memoized_ref<I, O, G, R>(
    input: I,
    f: fn(input: I) -> (O, I::Constraint),
    g: G,
) -> R
where
    I: Track,
    O: 'static,
    G: Fn(&O) -> R,
{
    let mut state = fxhash::FxHasher64::default();
    input.key(&mut state);

    let key = state.finish();
    let result = with(|cache| {
        let entry = cache.get_mut(&key)?;
        entry.age = 0;
        entry
            .data
            .downcast_ref::<(O, I::Constraint)>()
            .filter(|(_, constraint)| input.matches(constraint))
            .map(|(output, _)| g(output))
    });

    result.unwrap_or_else(|| {
        let output = f(input);
        let result = g(&output.0);
        let entry = CacheEntry {
            data: Box::new(output) as Box<(O, I::Constraint)> as Box<dyn Any>,
            age: 0,
        };
        with(|cache| cache.insert(key, entry));
        result
    })
}

/// Garbage-collect the thread-local cache.
///
/// This deletes elements which haven't been used in a while and returns details
/// about the eviction.
pub fn evict() -> Eviction {
    with(|cache| {
        const MAX_AGE: usize = 5;

        let before = cache.len();
        cache.retain(|_, entry| {
            entry.age += 1;
            entry.age <= MAX_AGE
        });

        Eviction { before, after: cache.len() }
    })
}

/// Details about a cache eviction.
pub struct Eviction {
    /// The number of items in the cache before the eviction.
    pub before: usize,
    /// The number of items in the cache after the eviction.
    pub after: usize,
}

impl Display for Eviction {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        writeln!(f, "Before: {}", self.before)?;
        writeln!(f, "Evicted: {}", self.before - self.after)?;
        writeln!(f, "After: {}", self.after)
    }
}

/// Tracks input dependencies of a memoized function.
pub trait Track {
    /// The type of constraint generated by this input.
    type Constraint: 'static;

    /// Feed the key portion of the input into a hasher.
    fn key<H: Hasher>(&self, hasher: &mut H);

    /// Whether this instance matches the given constraint.
    fn matches(&self, constraint: &Self::Constraint) -> bool;
}

impl<T: Track> Track for &T {
    type Constraint = T::Constraint;

    fn key<H: Hasher>(&self, hasher: &mut H) {
        Track::key(*self, hasher)
    }

    fn matches(&self, constraint: &Self::Constraint) -> bool {
        Track::matches(*self, constraint)
    }
}

macro_rules! impl_track_empty {
    ($ty:ty) => {
        impl $crate::memo::Track for $ty {
            type Constraint = ();

            fn key<H: std::hash::Hasher>(&self, _: &mut H) {}

            fn matches(&self, _: &Self::Constraint) -> bool {
                true
            }
        }
    };
}

macro_rules! impl_track_hash {
    ($ty:ty) => {
        impl $crate::memo::Track for $ty {
            type Constraint = ();

            fn key<H: std::hash::Hasher>(&self, hasher: &mut H) {
                std::hash::Hash::hash(self, hasher)
            }

            fn matches(&self, _: &Self::Constraint) -> bool {
                true
            }
        }
    };
}

macro_rules! impl_track_tuple {
    ($($idx:tt: $field:ident),*) => {
        #[allow(unused_variables)]
        impl<$($field: Track),*> Track for ($($field,)*) {
            type Constraint = ($($field::Constraint,)*);

            fn key<H: Hasher>(&self, hasher: &mut H) {
                $(self.$idx.key(hasher);)*
            }

            fn matches(&self, constraint: &Self::Constraint) -> bool {
                true $(&& self.$idx.matches(&constraint.$idx))*
            }
        }
    };
}

impl_track_tuple! {}
impl_track_tuple! { 0: A }
impl_track_tuple! { 0: A, 1: B }
impl_track_tuple! { 0: A, 1: B, 2: C }
impl_track_tuple! { 0: A, 1: B, 2: C, 3: D }
