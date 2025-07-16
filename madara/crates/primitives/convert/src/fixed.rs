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
