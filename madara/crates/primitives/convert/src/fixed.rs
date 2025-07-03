pub fn f64_to_u128_fixed(f: f64) -> (u128, u32) {
    assert!(f >= 0.0 && f.is_finite(), "Only finite, non-negative numbers supported");
    let max_u128 = u128::MAX as f64;

    let mut fraction_bits = 0;
    while fraction_bits < 128 {
        let scaled = f * (1u128 << fraction_bits) as f64;
        if scaled > max_u128 {
            break;
        }
        fraction_bits += 1;
    }

    let safe_fraction_bits = fraction_bits - 1;
    let scale = 1u128 << safe_fraction_bits;
    let value = (f * scale as f64).round() as u128;

    (value, safe_fraction_bits)
}
