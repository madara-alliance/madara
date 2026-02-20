/// Fixed-point representation of a number with a specified number of decimal places.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixedPoint {
    /// The value represented as an integer, scaled by the number of decimal places.
    value: u128,
    /// The number of decimal places the value is scaled to.
    decimals: u32,
}

impl FixedPoint {
    /// Creates a new `FixedPoint` with the given value and decimal places.
    pub fn new(value: u128, decimals: u32) -> Self {
        assert!(decimals <= 38, "Decimals must be less than or equal to 38");
        Self { value, decimals }
    }

    pub fn value(&self) -> u128 {
        self.value
    }

    pub fn decimals(&self) -> u32 {
        self.decimals
    }

    pub const fn zero() -> Self {
        Self { value: 0, decimals: 0 }
    }

    pub const fn one() -> Self {
        Self { value: 1, decimals: 0 }
    }

    /// Returns the value as a floating-point number.
    pub fn to_f64(&self) -> f64 {
        self.value as f64 / 10f64.powi(self.decimals as i32)
    }
}

impl From<f64> for FixedPoint {
    fn from(value: f64) -> Self {
        assert!(value >= 0.0 && value.is_finite(), "Only finite, non-negative numbers supported");
        if value == 0.0 {
            return Self::zero();
        }

        if value == 1.0 {
            return Self::one();
        }

        let max_u128 = u128::MAX as f64;

        let mut scale = 0u32;

        while scale < 38 {
            let factor = 10f64.powi(scale as i32);
            let scaled = value * factor;

            if scaled > max_u128 {
                break;
            }

            scale += 1;
        }

        // Step back to safe scale
        let decimals = scale.saturating_sub(1);

        let factor = 10f64.powi(decimals as i32);
        let scaled = value * factor;

        let mantissa = scaled.round() as u128;

        Self { value: mantissa, decimals }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_whole_number() {
        let fp = FixedPoint::from(1.0);
        assert_eq!(fp.decimals(), 0);
        assert_eq!(fp.value(), 1);
    }

    #[test]
    fn from_fractional() {
        let fp = FixedPoint::from(1.5);
        assert!((fp.to_f64() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn from_zero() {
        let fp = FixedPoint::from(0.0);
        assert_eq!(fp.decimals(), 0);
        assert_eq!(fp.value(), 0);
    }

    /// Regression: with max-scale approach, 1.0 became FixedPoint { value: 999999999999999966662502642608529408, decimals: 36 }
    /// which caused 25000 / strk_per_eth to truncate to 24999 instead of 25000.
    /// Verify the representation is exact so integer division won't lose precision.
    #[test]
    fn one_is_exactly_representable() {
        let fp = FixedPoint::from(1.0);
        // value must equal 10^decimals exactly for the representation to be precise
        assert_eq!(fp.value(), 10u128.pow(fp.decimals()));
    }

    #[test]
    fn roundtrip_to_f64() {
        for val in [1.0, 0.5, 3.14, 1000.0, 0.001] {
            let fp = FixedPoint::from(val);
            assert!((fp.to_f64() - val).abs() < 1e-9, "roundtrip failed for {val}");
        }
    }
}
