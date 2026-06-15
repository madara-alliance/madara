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

    /// Returns the raw integer value, scaled by `decimals`.
    pub fn value(&self) -> u128 {
        self.value
    }

    /// Returns the number of decimal places the value is scaled to.
    pub fn decimals(&self) -> u32 {
        self.decimals
    }

    /// Returns the fixed-point representation of zero.
    pub const fn zero() -> Self {
        Self { value: 0, decimals: 0 }
    }

    /// Returns the fixed-point representation of one.
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

        let max_u128 = u128::MAX as f64;
        if value.fract() == 0.0 && value <= max_u128 {
            return Self { value: value as u128, decimals: 0 };
        }

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
    use super::FixedPoint;
    use rstest::rstest;

    #[rstest]
    #[case(1.0, 1)]
    #[case(2.0, 2)]
    fn integer_f64s_convert_exactly(#[case] input: f64, #[case] expected_value: u128) {
        let fixed_point = FixedPoint::from(input);
        assert_eq!(fixed_point.value(), expected_value);
        assert_eq!(fixed_point.decimals(), 0);
    }
}
