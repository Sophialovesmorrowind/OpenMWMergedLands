use crate::land::textures::IndexVTEX;
use num_traits::ToPrimitive;

/// Types implemented [`RoundTo`] may be rounded to `T` via [`RoundTo::round_to`].
pub trait RoundTo<T> {
    /// Round `self` to `T`.
    fn round_to(self) -> T;
}

impl RoundTo<i32> for f32 {
    fn round_to(self) -> i32 {
        self.to_i32().expect("value cannot be represented as i32")
    }
}

impl RoundTo<i8> for f32 {
    fn round_to(self) -> i8 {
        self.clamp(f32::from(i8::MIN), f32::from(i8::MAX))
            .to_i8()
            .expect("bounded f32 should convert to i8")
    }
}

impl RoundTo<u8> for f32 {
    fn round_to(self) -> u8 {
        self.to_u8().expect("value cannot be represented as u8")
    }
}

impl RoundTo<u16> for f32 {
    fn round_to(self) -> u16 {
        self.to_u16().expect("value cannot be represented as u16")
    }
}

impl RoundTo<IndexVTEX> for f32 {
    fn round_to(self) -> IndexVTEX {
        IndexVTEX::new(self.round_to())
    }
}

#[cfg(test)]
mod tests {
    use super::RoundTo;
    use crate::land::textures::IndexVTEX;

    #[test]
    fn round_to_numeric_types_uses_rust_cast_behavior() {
        let rounded_i32: i32 = 3.9f32.round_to();
        let rounded_signed: i8 = (-2.2f32).round_to();
        let rounded_byte: u8 = 255.9f32.round_to();
        let rounded_word: u16 = 512.4f32.round_to();

        assert_eq!(rounded_i32, 3);
        assert_eq!(rounded_signed, -2);
        assert_eq!(rounded_byte, 255);
        assert_eq!(rounded_word, 512);
    }

    #[test]
    fn round_to_i8_saturates_when_value_exceeds_bounds() {
        let too_large: i8 = 200.0f32.round_to();
        let too_small: i8 = (-200.0f32).round_to();

        assert_eq!(too_large, i8::MAX);
        assert_eq!(too_small, i8::MIN);
    }

    #[test]
    fn round_to_index_vtex_wraps_u16_value() {
        let index: IndexVTEX = 42.7f32.round_to();
        assert_eq!(index.as_u16(), 42);
    }
}
