//! The chip-driver trait.

use crate::common::{Read, Write};

/// A driver for a chip: written values of type `T`, read values of type `U`.
///
/// A driver knows one chip's protocol and turns it into typed values: it
/// accepts writes of `T` (a configuration or a command) and produces reads
/// of `U` (a measurement or a message), through [`Write`] and [`Read`] and
/// their one shared error type. A chip can have several drivers, one per
/// type pair, such as an IMU read as SI units or as raw counts.
pub trait Driver<T, U>: Write<T> + Read<U> {
    /// A short human-readable name for this driver, such as the chip's part
    /// number.
    const DRIVER_NAME: &'static str;
}
