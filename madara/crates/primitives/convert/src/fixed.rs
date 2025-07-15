/// Converts a `f64` floating-point number to a fixed-point representation
/// as a tuple of `(u128, u32)`, where the first element is the
/// mantissa and the second element is the exponent in base 10
pub fn f64_to_u128_fixed(f: f64) -> (u128, u32) {
    assert!(f >= 0.0 && f.is_finite(), "Only finite, non-negative numbers supported");
    if f == 0.0 {
        return (0, 0);
    }

    let max_u128 = u128::MAX as f64;

    let mut scale = 0u32;

    while scale < 38 {
        let factor = 10f64.powi(scale as i32);
        let scaled = f * factor;

        if scaled > max_u128 {
            break;
        }

        scale += 1;
    }

    // Step back to safe scale
    let safe_scale = scale.saturating_sub(1);

    let factor = 10f64.powi(safe_scale as i32);
    let scaled = f * factor;

    let mantissa = scaled.round() as u128;

    (mantissa, safe_scale)
}
