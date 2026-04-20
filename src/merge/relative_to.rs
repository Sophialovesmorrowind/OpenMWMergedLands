use crate::land::terrain_map::Vec3;
use const_default::ConstDefault;
use std::fmt::Debug;

/// Types implementing [`RelativeTo`] can be subtracted with [`RelativeTo::subtract`] to compute
/// some delta of type [`RelativeTo::Delta`]. The delta can be passed to [`RelativeTo::add`] to
/// recompute the original value.
pub trait RelativeTo: Copy + Default + ConstDefault + Eq + Debug + Sized + 'static {
    /// A [`RelativeTo::Delta`] is a signed version of the type implementing [`RelativeTo`].
    type Delta: Copy + Default + ConstDefault + Eq + Debug + Sized + 'static;

    /// Subtract `rhs` from `lhs` and return the [`RelativeTo::Delta`].
    fn subtract(lhs: Self, rhs: Self) -> Self::Delta;

    /// Add the [`RelativeTo::Delta`] `rhs` to `lhs`.
    fn add(lhs: Self, rhs: Self::Delta) -> Self;
}

impl RelativeTo for i32 {
    type Delta = i32;

    fn subtract(lhs: Self, rhs: Self) -> Self::Delta {
        lhs - rhs
    }

    fn add(lhs: Self, rhs: Self::Delta) -> Self {
        lhs + rhs
    }
}

impl RelativeTo for u8 {
    type Delta = i32;

    fn subtract(lhs: Self, rhs: Self) -> Self::Delta {
        Self::Delta::from(lhs) - Self::Delta::from(rhs)
    }

    fn add(lhs: Self, rhs: Self::Delta) -> Self {
        Self::try_from(Self::Delta::from(lhs) + rhs).expect("u8 addition overflow")
    }
}

impl RelativeTo for i8 {
    type Delta = i32;

    fn subtract(lhs: Self, rhs: Self) -> Self::Delta {
        Self::Delta::from(lhs) - Self::Delta::from(rhs)
    }

    fn add(lhs: Self, rhs: Self::Delta) -> Self {
        Self::try_from(Self::Delta::from(lhs) + rhs).expect("i8 addition overflow")
    }
}

impl RelativeTo for u16 {
    type Delta = i32;

    fn subtract(lhs: Self, rhs: Self) -> Self::Delta {
        Self::Delta::from(lhs) - Self::Delta::from(rhs)
    }

    fn add(lhs: Self, rhs: Self::Delta) -> Self {
        Self::try_from(Self::Delta::from(lhs) + rhs).expect("u16 addition overflow")
    }
}

impl<T: RelativeTo> RelativeTo for Vec3<T> {
    type Delta = Vec3<<T as RelativeTo>::Delta>;

    fn subtract(lhs: Self, rhs: Self) -> Self::Delta {
        Self::Delta {
            x: <T as RelativeTo>::subtract(lhs.x, rhs.x),
            y: <T as RelativeTo>::subtract(lhs.y, rhs.y),
            z: <T as RelativeTo>::subtract(lhs.z, rhs.z),
        }
    }

    fn add(lhs: Self, rhs: Self::Delta) -> Self {
        Self {
            x: <T as RelativeTo>::add(lhs.x, rhs.x),
            y: <T as RelativeTo>::add(lhs.y, rhs.y),
            z: <T as RelativeTo>::add(lhs.z, rhs.z),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RelativeTo;
    use crate::land::terrain_map::Vec3;

    #[test]
    fn i32_roundtrip_is_identity() {
        let lhs = 42i32;
        let rhs = 7i32;
        let delta = i32::subtract(lhs, rhs);
        assert_eq!(delta, 35);
        assert_eq!(i32::add(rhs, delta), lhs);
    }

    #[test]
    fn u8_roundtrip_is_identity_for_safe_values() {
        let lhs = 240u8;
        let rhs = 100u8;
        let delta = u8::subtract(lhs, rhs);
        assert_eq!(u8::add(rhs, delta), lhs);
    }

    #[test]
    fn i8_roundtrip_is_identity_for_safe_values() {
        let lhs = 100i8;
        let rhs = -10i8;
        let delta = i8::subtract(lhs, rhs);
        assert_eq!(i8::add(rhs, delta), lhs);
    }

    #[test]
    fn u16_roundtrip_is_identity_for_safe_values() {
        let lhs = 50_000u16;
        let rhs = 12_000u16;
        let delta = u16::subtract(lhs, rhs);
        assert_eq!(u16::add(rhs, delta), lhs);
    }

    #[test]
    fn vec3_roundtrip_is_component_wise_identity() {
        let lhs = Vec3::new(9i32, 6i32, 3i32);
        let rhs = Vec3::new(1i32, 2i32, 3i32);
        let delta = Vec3::<i32>::subtract(lhs, rhs);

        assert_eq!(delta, Vec3::new(8, 4, 0));
        assert_eq!(Vec3::<i32>::add(rhs, delta), lhs);
    }
}
