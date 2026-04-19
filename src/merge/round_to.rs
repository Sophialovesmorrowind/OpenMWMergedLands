use crate::land::textures::IndexVTEX;

/// Types implemented [RoundTo] may be rounded to `T` via [RoundTo::round_to].
pub trait RoundTo<T> {
    /// Round `self` to `T`.
    fn round_to(self) -> T;
}

impl RoundTo<i32> for f32 {
    fn round_to(self) -> i32 {
        self as i32
    }
}

impl RoundTo<i8> for f32 {
    fn round_to(self) -> i8 {
        self as i8
    }
}

impl RoundTo<u8> for f32 {
    fn round_to(self) -> u8 {
        self as u8
    }
}

impl RoundTo<u16> for f32 {
    fn round_to(self) -> u16 {
        self as u16
    }
}

impl RoundTo<IndexVTEX> for f32 {
    fn round_to(self) -> IndexVTEX {
        IndexVTEX::new(self as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::RoundTo;
    use crate::land::textures::IndexVTEX;

    #[test]
    fn round_to_numeric_types_uses_rust_cast_behavior() {
        let as_i32: i32 = 3.9f32.round_to();
        let as_i8: i8 = (-2.2f32).round_to();
        let as_u8: u8 = 255.9f32.round_to();
        let as_u16: u16 = 512.4f32.round_to();

        assert_eq!(as_i32, 3);
        assert_eq!(as_i8, -2);
        assert_eq!(as_u8, 255);
        assert_eq!(as_u16, 512);
    }

    #[test]
    fn round_to_index_vtex_wraps_u16_value() {
        let index: IndexVTEX = 42.7f32.round_to();
        assert_eq!(index.as_u16(), 42);
    }
}
