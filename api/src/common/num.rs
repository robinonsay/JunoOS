//! A bound for numeric value types.

use core::ops::{Add, Mul};

/// A plain number: copyable, ordered, and closed under `+` and `*`, with a
/// zero.
///
/// Implemented for `u8`, `u16`, `u32`, `u64`, `i32`, `i64`, `f32` and
/// `f64`, so generic code can sum, scale and compare values of any of them.
pub trait Number: Copy + PartialOrd + Add<Output = Self> + Mul<Output = Self> {
    /// The additive identity: `0` or `0.0`.
    const ZERO: Self;
}
macro_rules! impl_number { ($($t:ty),*) => { $(impl Number for $t { const ZERO: Self = 0 as $t; })* } }
impl_number!(u8, u16, u32, u64, i32, i64, f32, f64);
