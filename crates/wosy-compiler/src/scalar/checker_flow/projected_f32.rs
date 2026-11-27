//! Exact finite f32 transfer for an observed typed path. No floating-point
//! fact is manufactured for an unobserved initializer or an external result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) struct ExactF32(pub(in super::super) u32);

impl ExactF32 {
    pub(in super::super) fn checked_bits(bits: u32) -> Option<Self> {
        f32::from_bits(bits).is_finite().then_some(Self(bits))
    }

    pub(in super::super) fn signed_i32(value: i32) -> Self {
        Self((value as f32).to_bits())
    }

    pub(in super::super) fn add(self, right: Self) -> Option<Self> {
        let computed = f32::from_bits(self.0) + f32::from_bits(right.0);
        Self::checked_bits(computed.to_bits())
    }
}

#[cfg(test)]
#[test]
fn exact_signed_conversion_and_add_preserve_ieee_rounding_and_zero() {
    let positive_zero = ExactF32::checked_bits(0).unwrap();
    let negative_zero = ExactF32::checked_bits(1 << 31).unwrap();
    let subnormal = ExactF32::checked_bits(1).unwrap();
    assert_eq!(negative_zero.add(negative_zero), Some(negative_zero));
    assert_eq!(positive_zero.add(negative_zero), Some(positive_zero));
    assert_eq!(subnormal.add(positive_zero), Some(subnormal));
    assert_eq!(ExactF32::signed_i32(16_777_217).0, 16_777_216f32.to_bits());
    assert_eq!(ExactF32::signed_i32(-16_777_217).0, (-16_777_216f32).to_bits());
    let maximum = ExactF32::checked_bits(f32::MAX.to_bits()).unwrap();
    assert_eq!(maximum.add(maximum), None);
    assert_eq!(ExactF32::checked_bits(f32::NAN.to_bits()), None);
    assert_eq!(ExactF32::checked_bits(f32::INFINITY.to_bits()), None);
}
