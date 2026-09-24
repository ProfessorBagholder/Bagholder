//! `Dec`: an exact decimal, the number every amount, quantity and price is.
//!
//! Money is never binary floating point (`docs/architecture.md` §6). `Dec` is
//! made from text or from integers and never from a float, so a float cannot
//! become money anywhere that uses this type. Its arithmetic is checked and
//! exact: an addition that overflows, or a product that would need more than 28
//! decimal places, is an error rather than a rounded value. A total is the one
//! sum that is rounded, and only where its exact value needs more significant
//! digits than a `Dec` holds (`add_to_fit`). Division is the one
//! operation that cannot be exact in general, so it is offered only as
//! `div_rounded`, which names its places and its rounding rule.
//!
//! A float comes out only through `to_f64`, for the statistics (ratios, returns)
//! the design keeps in floating point.

use std::fmt;
use std::str::FromStr;

use rust_decimal::{Decimal, RoundingStrategy};

/// The most decimal places a `Dec` holds.
pub const MAX_PLACES: u32 = 28;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Dec(Decimal);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecError {
    /// Text that is not a plain decimal: `-12.50` is, `1e3`, `+5`, ` 5`, `.5`, `5.` are not.
    Malformed(String),
    /// More digits than 28 significant, or a result that does not fit.
    Overflow,
    /// A sum or product that would need more places than fit to be exact.
    Inexact,
    DivisionByZero,
}

impl fmt::Display for DecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecError::Malformed(s) => write!(f, "not a decimal number: {s:?}"),
            DecError::Overflow => write!(f, "a decimal number too large to hold exactly"),
            DecError::Inexact => write!(f, "a result needing more places than {MAX_PLACES} to be exact"),
            DecError::DivisionByZero => write!(f, "division by zero"),
        }
    }
}

impl std::error::Error for DecError {}

/// How a division's result is brought to its places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    /// To the nearest; a tie to the even digit (banker's rounding).
    HalfEven,
    /// To the nearest; a tie away from zero (the rounding most people learn).
    HalfUp,
    /// Toward zero (truncation).
    TowardZero,
    /// Away from zero.
    AwayFromZero,
    /// Toward negative infinity.
    Floor,
    /// Toward positive infinity.
    Ceiling,
}

impl Rounding {
    fn strategy(self) -> RoundingStrategy {
        match self {
            Rounding::HalfEven => RoundingStrategy::MidpointNearestEven,
            Rounding::HalfUp => RoundingStrategy::MidpointAwayFromZero,
            Rounding::TowardZero => RoundingStrategy::ToZero,
            Rounding::AwayFromZero => RoundingStrategy::AwayFromZero,
            Rounding::Floor => RoundingStrategy::ToNegativeInfinity,
            Rounding::Ceiling => RoundingStrategy::ToPositiveInfinity,
        }
    }
}

impl Dec {
    pub const ZERO: Dec = Dec(Decimal::ZERO);
    pub const ONE: Dec = Dec(Decimal::ONE);

    /// A whole number.
    pub fn from_int(n: i64) -> Dec {
        Dec(Decimal::from(n))
    }

    /// `mantissa × 10^-places`: `Dec::new(12345, 2)` is 123.45.
    pub fn new(mantissa: i64, places: u32) -> Result<Dec, DecError> {
        if places > MAX_PLACES {
            return Err(DecError::Overflow);
        }
        Ok(Dec(Decimal::new(mantissa, places)))
    }

    /// Plain decimal notation only: an optional `-`, digits, and optionally a
    /// point followed by digits. Everything else is refused, so a value is read
    /// exactly as written or not at all.
    pub fn parse(s: &str) -> Result<Dec, DecError> {
        let malformed = || DecError::Malformed(s.to_string());
        let body = s.strip_prefix('-').unwrap_or(s);
        let (whole, frac) = match body.split_once('.') {
            Some((w, f)) => (w, Some(f)),
            None => (body, None),
        };
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        // a leading zero only before the point (`0.5`), never `007`
        let leading_zero = whole.len() > 1 && whole.starts_with('0');
        if !digits(whole) || frac.is_some_and(|f| !digits(f)) || leading_zero {
            return Err(malformed());
        }
        if frac.map_or(0, str::len) > MAX_PLACES as usize {
            return Err(DecError::Overflow);
        }
        // `from_str_exact` refuses rather than rounds a value it cannot hold
        Decimal::from_str_exact(s).map(Dec).map_err(|_| DecError::Overflow)
    }

    /// The canonical text: no trailing zeros after the point, no point on a whole
    /// number, never `-0`. What is stored and what is sent.
    pub fn to_text(self) -> String {
        let n = self.0.normalize();
        if n.is_zero() {
            return "0".to_string();
        }
        n.to_string()
    }

    /// The decimal places the value needs, trailing zeros not counted.
    pub fn places(self) -> u32 {
        self.0.normalize().scale()
    }

    /// The exact sum, or an error: `rust_decimal` drops places from a sum that no
    /// longer fits in its 96 bits, and that is refused here. An exact sum of the
    /// terms at their own places keeps the larger of their places.
    pub fn checked_add(self, other: Dec) -> Result<Dec, DecError> {
        let (a, b) = (self.0.normalize(), other.0.normalize());
        let out = a.checked_add(b).ok_or(DecError::Overflow)?;
        if out.scale() < a.scale().max(b.scale()) {
            return Err(DecError::Inexact);
        }
        Ok(Dec(out))
    }

    /// A term added to a total: the exact sum where it fits, as `checked_add`;
    /// where the exact sum needs more significant digits than a `Dec` holds (a
    /// large total and a small amount of many places), that sum rounded once,
    /// half to even, to the most places that hold it. Only a sum too large to
    /// hold at all is an error.
    ///
    /// The rounding is of the exact sum: at `k` places each term splits into its
    /// part held at `k` places and the rest below; the parts and the rests add
    /// exactly, and the rest left over below `k` places decides the rounding.
    pub fn add_to_fit(self, other: Dec) -> Result<Dec, DecError> {
        match self.checked_add(other) {
            Err(DecError::Inexact) => {}
            exact => return exact,
        }
        for k in (0..self.places().max(other.places())).rev() {
            let (a, b) = (self.round(k, Rounding::TowardZero), other.round(k, Rounding::TowardZero));
            let Ok(high) = a.checked_add(b) else { continue };
            let low = self.checked_sub(a)?.checked_add(other.checked_sub(b)?)?;
            let low_held = low.round(k, Rounding::TowardZero);
            let Ok(near) = high.checked_add(low_held) else { continue };
            // the exact sum is `near + rest`, `rest` below one unit at `k` places
            let rest = low.checked_sub(low_held)?;
            let unit = Dec(Decimal::new(1, k));
            let twice = rest.abs().checked_add(rest.abs())?;
            let away = twice > unit || (twice == unit && !near.even_at(k));
            let out = match (away, rest.is_negative()) {
                (false, _) => Ok(near),
                (true, false) => near.checked_add(unit),
                (true, true) => near.checked_sub(unit),
            };
            if let Ok(sum) = out {
                return Ok(sum);
            }
        }
        Err(DecError::Overflow)
    }

    /// Whether the digit in the last of `places` places is even.
    fn even_at(self, places: u32) -> bool {
        let mut at = self.0;
        at.rescale(places);
        at.mantissa() % 2 == 0
    }

    /// The exact difference, or an error, as `checked_add`.
    pub fn checked_sub(self, other: Dec) -> Result<Dec, DecError> {
        self.checked_add(other.neg())
    }

    /// The exact product, or an error: never a rounded one. The mantissas are
    /// multiplied in full (up to 192 bits) and the product is brought back to at
    /// most 28 places and 96 bits only by dropping trailing zeros; a product that
    /// needs more is refused.
    pub fn checked_mul(self, other: Dec) -> Result<Dec, DecError> {
        if self.is_zero() || other.is_zero() {
            return Ok(Dec::ZERO);
        }
        let (a, b) = (self.0.normalize(), other.0.normalize());
        let mut n = Wide::product(a.mantissa().unsigned_abs(), b.mantissa().unsigned_abs());
        let mut scale = a.scale() + b.scale();
        while scale > MAX_PLACES || !n.fits_96() {
            if scale == 0 {
                return Err(DecError::Overflow);
            }
            if n.div10_exact().is_none() {
                return Err(if scale > MAX_PLACES { DecError::Inexact } else { DecError::Overflow });
            }
            scale -= 1;
        }
        let m = n.low_u128() as i128;
        let negative = a.is_sign_negative() != b.is_sign_negative();
        let out = Decimal::from_i128_with_scale(if negative { -m } else { m }, scale);
        Ok(Dec(out))
    }

    /// A value worked out for a figure (a quantity at a price, an amount at a
    /// rate): the exact product where it fits, as `checked_mul`; where it needs
    /// more significant digits than a `Dec` holds, that product rounded once,
    /// half to even, to the most places that hold it. Only a product too large
    /// to hold at all is an error.
    pub fn mul_to_fit(self, other: Dec) -> Result<Dec, DecError> {
        match self.checked_mul(other) {
            Err(DecError::Inexact | DecError::Overflow) => {}
            exact => return exact,
        }
        let (a, b) = (self.0.normalize(), other.0.normalize());
        let mut n = Wide::product(a.mantissa().unsigned_abs(), b.mantissa().unsigned_abs());
        let mut scale = a.scale() + b.scale();
        // the most significant digit dropped, and whether any below it was not zero
        let (mut first, mut below) = (0u8, false);
        while scale > MAX_PLACES || !n.fits_96() {
            if scale == 0 {
                return Err(DecError::Overflow);
            }
            below |= first != 0;
            first = n.div10();
            scale -= 1;
        }
        if first > 5 || (first == 5 && (below || n.0[0] % 2 == 1)) {
            n.add_one();
            if !n.fits_96() {
                // 99…9 rounded up to 10…0: one place fewer holds it exactly
                if scale == 0 {
                    return Err(DecError::Overflow);
                }
                n.div10();
                scale -= 1;
            }
        }
        let m = n.low_u128() as i128;
        let negative = a.is_sign_negative() != b.is_sign_negative();
        Ok(Dec(Decimal::from_i128_with_scale(if negative { -m } else { m }, scale)))
    }

    /// `self ÷ divisor` to `places` decimal places, rounded once, by `rule`,
    /// from the exact quotient. The library's own division already rounds to 28
    /// significant digits, and rounding that again can be wrong at a tie, so its
    /// quotient is used only as a first guess: the truncated quotient is checked
    /// and corrected against the exact remainder, and the remainder decides the
    /// rounding.
    pub fn div_rounded(self, divisor: Dec, places: u32, rule: Rounding) -> Result<Dec, DecError> {
        if divisor.is_zero() {
            return Err(DecError::DivisionByZero);
        }
        if places > MAX_PLACES {
            return Err(DecError::Overflow);
        }
        let ulp = Dec(Decimal::new(1, places));
        let negative = self.is_negative() != divisor.is_negative() && !self.is_zero();
        // work in magnitudes: q = |a| / |b| >= 0
        let (a, b) = (self.abs(), divisor.abs());
        let guess = a.0.checked_div(b.0).ok_or(DecError::Overflow)?;
        let mut t = Dec(guess.trunc_with_scale(places));
        // t is the truncated quotient when 0 <= a - t*b < b*ulp
        let step = b.checked_mul(ulp)?;
        let mut r = a.checked_sub(t.checked_mul(b)?)?;
        while r.is_negative() {
            t = t.checked_sub(ulp)?;
            r = r.checked_add(step)?;
        }
        while r >= step {
            t = t.checked_add(ulp)?;
            r = r.checked_sub(step)?;
        }
        // the part dropped, against half a step: below, a tie, or above
        let twice = r.checked_add(r)?;
        let up = match rule {
            Rounding::TowardZero => false,
            Rounding::AwayFromZero => !r.is_zero(),
            Rounding::Floor => negative && !r.is_zero(),
            Rounding::Ceiling => !negative && !r.is_zero(),
            Rounding::HalfUp => twice >= step,
            Rounding::HalfEven => {
                twice > step || (twice == step && {
                    // a tie goes to the even last digit: the digit in the last
                    // place asked for, read with `t` at exactly those places
                    let mut at_places = t.0;
                    at_places.rescale(places);
                    at_places.mantissa() % 2 != 0
                })
            }
        };
        if up {
            t = t.checked_add(ulp)?;
        }
        Ok(if negative { t.neg() } else { t })
    }

    /// `self` rounded to `places`, by `rule`.
    pub fn round(self, places: u32, rule: Rounding) -> Dec {
        Dec(self.0.round_dp_with_strategy(places.min(MAX_PLACES), rule.strategy()))
    }

    pub fn neg(self) -> Dec {
        Dec(-self.0)
    }

    pub fn abs(self) -> Dec {
        Dec(self.0.abs())
    }

    pub fn is_zero(self) -> bool {
        self.0.is_zero()
    }

    pub fn is_positive(self) -> bool {
        self.0.is_sign_positive() && !self.0.is_zero()
    }

    pub fn is_negative(self) -> bool {
        self.0.is_sign_negative() && !self.0.is_zero()
    }

    /// For statistics only (ratios, returns), which the design keeps in floating
    /// point. Nothing comes back the other way.
    pub fn to_f64(self) -> f64 {
        use rust_decimal::prelude::ToPrimitive;
        self.0.to_f64().unwrap_or(f64::NAN)
    }
}

/// An unsigned integer of up to 192 bits, in three 64-bit limbs (least
/// significant first): enough for the product of two 96-bit mantissas.
struct Wide([u64; 3]);

impl Wide {
    fn product(a: u128, b: u128) -> Wide {
        let (a0, a1) = (a as u64 as u128, (a >> 64) as u64 as u128);
        let (b0, b1) = (b as u64 as u128, (b >> 64) as u64 as u128);
        // a*b = a1*b1*2^128 + (a1*b0 + a0*b1)*2^64 + a0*b0, each partial below 2^128
        let lo = a0 * b0;
        let mid1 = a1 * b0;
        let mid2 = a0 * b1;
        let hi = a1 * b1;
        let l0 = lo as u64;
        let carry = (lo >> 64) + (mid1 as u64 as u128) + (mid2 as u64 as u128);
        let l1 = carry as u64;
        let carry = (carry >> 64) + (mid1 >> 64) + (mid2 >> 64) + (hi as u64 as u128);
        let l2 = carry as u64;
        // the product of two 96-bit numbers is below 2^192: nothing is left over
        debug_assert!((carry >> 64) + (hi >> 64) == 0);
        Wide([l0, l1, l2])
    }

    fn fits_96(&self) -> bool {
        self.0[2] == 0 && self.0[1] >> 32 == 0
    }

    fn low_u128(&self) -> u128 {
        (self.0[1] as u128) << 64 | self.0[0] as u128
    }

    /// Divide by ten, answering the digit dropped.
    fn div10(&mut self) -> u8 {
        let mut rem: u128 = 0;
        for i in (0..3).rev() {
            let cur = rem << 64 | self.0[i] as u128;
            self.0[i] = (cur / 10) as u64;
            rem = cur % 10;
        }
        rem as u8
    }

    fn add_one(&mut self) {
        for limb in self.0.iter_mut() {
            let (v, carry) = limb.overflowing_add(1);
            *limb = v;
            if !carry {
                return;
            }
        }
    }

    /// Divide by ten when that is exact; leave it alone and say so when not.
    fn div10_exact(&mut self) -> Option<()> {
        let mut q = [0u64; 3];
        let mut rem: u128 = 0;
        for i in (0..3).rev() {
            let cur = rem << 64 | self.0[i] as u128;
            q[i] = (cur / 10) as u64;
            rem = cur % 10;
        }
        if rem != 0 {
            return None;
        }
        self.0 = q;
        Some(())
    }
}

impl fmt::Display for Dec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}

impl fmt::Debug for Dec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Dec({})", self.to_text())
    }
}

impl FromStr for Dec {
    type Err = DecError;
    fn from_str(s: &str) -> Result<Dec, DecError> {
        Dec::parse(s)
    }
}

impl serde::Serialize for Dec {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_text())
    }
}

impl<'de> serde::Deserialize<'de> for Dec {
    /// Text only, as written: a JSON number would already have passed through a float.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Dec, D::Error> {
        let s = String::deserialize(d)?;
        Dec::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// ```compile_fail
/// // a float never becomes a `Dec`
/// let _ = bagholder_core::Dec::from(1.5_f64);
/// ```
///
/// ```compile_fail
/// let _: bagholder_core::Dec = 1.5_f64.into();
/// ```
#[allow(dead_code)]
fn no_float_constructor() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Dec {
        Dec::parse(s).unwrap()
    }

    #[test]
    fn text_round_trips_exactly() {
        for (input, canonical) in [
            ("0", "0"),
            ("-0", "0"),
            ("0.000", "0"),
            ("-12.50", "-12.5"),
            ("1050", "1050"),
            ("1050.00", "1050"),
            ("0.321720925242232", "0.321720925242232"),
            ("0.000000000000000001", "0.000000000000000001"),
            ("123456789012345678.1234567891", "123456789012345678.1234567891"),
            ("9999999999999999999999999999", "9999999999999999999999999999"),
            ("0.1234567890123456789012345678", "0.1234567890123456789012345678"),
        ] {
            let v = d(input);
            assert_eq!(v.to_text(), canonical, "{input}");
            assert_eq!(d(canonical), v, "{input}");
        }
    }

    #[test]
    fn anything_but_plain_decimal_notation_is_refused() {
        for bad in ["", "-", "1e3", "1E3", "+5", " 5", "5 ", ".5", "5.", "1,000", "0x10", "NaN", "inf", "1.2.3", "--1", "١٢", "007", "-01.5"] {
            assert!(matches!(Dec::parse(bad), Err(DecError::Malformed(_))), "{bad:?}");
        }
        assert_eq!(Dec::parse("0.12345678901234567890123456789"), Err(DecError::Overflow));
        assert_eq!(Dec::parse("99999999999999999999999999999"), Err(DecError::Overflow));
    }

    #[test]
    fn sums_are_exact() {
        assert_eq!(d("0.1").checked_add(d("0.2")).unwrap(), d("0.3"));
        assert_eq!(d("1050").checked_sub(d("0.01")).unwrap(), d("1049.99"));
        assert_eq!(d("79228162514264337593543950335").checked_add(Dec::ONE), Err(DecError::Overflow));
        // a sum whose places no longer fit is refused, not rounded
        assert_eq!(d("9999999999999999999999999999").checked_add(d("0.1")), Err(DecError::Inexact));
        assert_eq!(d("9999999999999999999999999999").checked_sub(d("0.1")), Err(DecError::Inexact));
    }

    #[test]
    fn a_total_is_exact_where_it_fits_and_rounded_once_to_fit_where_not() {
        // exact where it fits
        assert_eq!(d("0.1").add_to_fit(d("0.2")).unwrap(), d("0.3"));
        // a portfolio's total and a dust coin's value: 33 significant digits, held to 23 places
        assert_eq!(d("632070.326688").add_to_fit(d("0.000000264188636702736309916")).unwrap(), d("632070.32668826418863670273631"));
        assert_eq!(d("-30725.453312").add_to_fit(d("-0.000000234442363297263690084")).unwrap(), d("-30725.453312234442363297263690"));
        // a tie goes to the even digit of the whole sum, not of the part below
        assert_eq!(d("10000000000000000000000000000").add_to_fit(d("0.5")).unwrap(), d("10000000000000000000000000000"));
        assert_eq!(d("10000000000000000000000000001").add_to_fit(d("0.5")).unwrap(), d("10000000000000000000000000002"));
        assert_eq!(d("-10000000000000000000000000001").add_to_fit(d("-0.5")).unwrap(), d("-10000000000000000000000000002"));
        assert_eq!(d("10000000000000000000000000001").add_to_fit(d("-0.5")).unwrap(), d("10000000000000000000000000000"));
        // terms of opposite signs, the rest below the unit borrowed across
        assert_eq!(d("1000000000000000000000000000").add_to_fit(d("-0.06")).unwrap(), d("999999999999999999999999999.9"));
        // only a sum too large to hold is an error
        assert_eq!(d("79228162514264337593543950335").add_to_fit(Dec::ONE), Err(DecError::Overflow));
    }

    #[test]
    fn a_value_is_exact_where_it_fits_and_rounded_once_to_fit_where_not() {
        assert_eq!(d("35").mul_to_fit(d("0.3")).unwrap(), d("10.5"));
        // a product needing 29 places: rounded at 28, half to even
        assert_eq!(d("0.123456789012345").mul_to_fit(d("0.12345678901233")).unwrap(), d("0.0152415787532368172687272138"));
        // a quantity of many digits at a price of many places
        assert_eq!(d("74505439.293609").mul_to_fit(d("0.00001234567890123456789")).unwrap(), d("919.8202299143215591323339536"));
        // ties go to the even digit
        assert_eq!(d("0.0000000000000000000000000001").mul_to_fit(d("0.5")).unwrap(), Dec::ZERO);
        assert_eq!(d("0.0000000000000000000000000003").mul_to_fit(d("0.5")).unwrap(), d("0.0000000000000000000000000002"));
        assert_eq!(d("-0.0000000000000000000000000003").mul_to_fit(d("0.5")).unwrap(), d("-0.0000000000000000000000000002"));
        // only a product too large to hold is an error
        assert_eq!(d("79228162514264337593543950335").mul_to_fit(d("2")), Err(DecError::Overflow));
    }

    #[test]
    fn products_are_exact_or_refused() {
        assert_eq!(d("35").checked_mul(d("0.3")).unwrap(), d("10.5"));
        assert_eq!(d("154.699294").checked_mul(d("0.321720925242232")).unwrap(), d("49.770000000000069384208"));
        // an exact product of 29 places
        assert_eq!(d("0.123456789012345").checked_mul(d("0.12345678901233")), Err(DecError::Inexact));
        // 15 + 14 places whose product ends in a zero: exact in 28
        assert_eq!(d("0.123456789012345").checked_mul(d("0.12345678901234")).unwrap(), d("0.0152415787532380518366173373"));
        // 14 + 15 places, but the exact product needs only 28
        assert_eq!(d("0.00000000000005").checked_mul(d("0.000000000000002")).unwrap(), d("0.0000000000000000000000000001"));
        assert_eq!(d("-2.5").checked_mul(d("4")).unwrap(), d("-10"));
        assert_eq!(d("79228162514264337593543950335").checked_mul(d("1")).unwrap(), d("79228162514264337593543950335"));
        // more significant digits than 96 bits hold: refused, never rounded
        assert_eq!(d("123456789012345.123456").checked_mul(d("98765432109876.54321")), Err(DecError::Overflow));
        assert_eq!(d("79228162514264337593543950335").checked_mul(d("2")), Err(DecError::Overflow));
    }

    #[test]
    fn division_names_its_rounding() {
        let third = Dec::ONE.div_rounded(d("3"), 4, Rounding::HalfEven).unwrap();
        assert_eq!(third, d("0.3333"));
        assert_eq!(d("2.5").div_rounded(Dec::ONE, 0, Rounding::HalfEven).unwrap(), d("2"));
        assert_eq!(d("2.5").div_rounded(Dec::ONE, 0, Rounding::HalfUp).unwrap(), d("3"));
        assert_eq!(d("-2.5").div_rounded(Dec::ONE, 0, Rounding::HalfUp).unwrap(), d("-3"));
        assert_eq!(d("2.9").div_rounded(Dec::ONE, 0, Rounding::TowardZero).unwrap(), d("2"));
        assert_eq!(d("-2.1").div_rounded(Dec::ONE, 0, Rounding::Floor).unwrap(), d("-3"));
        assert_eq!(d("2.1").div_rounded(Dec::ONE, 0, Rounding::Ceiling).unwrap(), d("3"));
        assert_eq!(d("2.1").div_rounded(Dec::ONE, 0, Rounding::AwayFromZero).unwrap(), d("3"));
        assert_eq!(Dec::ONE.div_rounded(Dec::ZERO, 2, Rounding::HalfEven), Err(DecError::DivisionByZero));
        assert_eq!(d("-7").div_rounded(d("2"), 0, Rounding::HalfEven).unwrap(), d("-4"));
        assert_eq!(d("7").div_rounded(d("-2"), 0, Rounding::TowardZero).unwrap(), d("-3"));
        assert_eq!(d("0.125").div_rounded(Dec::ONE, 2, Rounding::HalfEven).unwrap(), d("0.12"));
        assert_eq!(d("0.135").div_rounded(Dec::ONE, 2, Rounding::HalfEven).unwrap(), d("0.14"));
        assert_eq!(d("1050").div_rounded(d("35"), 2, Rounding::HalfEven).unwrap(), d("30"));
        // a quotient just below a tie, closer to it than 28 significant digits can
        // show: the library's own rounding would make it a tie, and a second
        // rounding would then take it up
        let a = d("0.4999999999999999999999999999");
        let b = d("0.9999999999999999999999999999");
        assert_eq!(a.div_rounded(b, 0, Rounding::HalfUp).unwrap(), Dec::ZERO);
        // a tie whose truncated quotient has fewer places than asked: 0.1 is 0.10,
        // whose last digit is even
        assert_eq!(d("0.105").div_rounded(Dec::ONE, 2, Rounding::HalfEven).unwrap(), d("0.1"));
        assert_eq!(d("0.115").div_rounded(Dec::ONE, 2, Rounding::HalfEven).unwrap(), d("0.12"));
    }

    #[test]
    fn serde_is_text_and_strict() {
        assert_eq!(serde_json::to_string(&d("-0.50")).unwrap(), "\"-0.5\"");
        assert_eq!(serde_json::from_str::<Dec>("\"12.30\"").unwrap(), d("12.3"));
        assert!(serde_json::from_str::<Dec>("\"1e3\"").is_err());
        assert!(serde_json::from_str::<Dec>("1.5").is_err(), "a JSON number has already been a float");
    }

    #[test]
    fn signs() {
        assert!(d("-1").is_negative() && !d("-1").is_positive());
        assert!(!Dec::ZERO.is_negative() && !Dec::ZERO.is_positive());
        assert!(!d("-0").is_negative());
        assert_eq!(d("-3.5").abs(), d("3.5"));
        assert_eq!(d("3.5").neg(), d("-3.5"));
        assert_eq!(d("12.3400").places(), 2);
    }
}
