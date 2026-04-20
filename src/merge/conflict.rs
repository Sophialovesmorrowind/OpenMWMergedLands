use crate::land::terrain_map::Vec3;
use crate::merge::round_to::RoundTo;
use num_traits::ToPrimitive;

/// The [`ConflictType`] classifies the severity of a conflict.
/// This is determined by [`ConflictParams`] passed to the
/// [`ConflictResolver::average`] method.
pub enum ConflictType<T> {
    /// A minor [`ConflictType`].
    Minor(T),
    /// A major [`ConflictType`].
    Major(T),
}

/// A [Conflict] is an [Option] wrapper around [`ConflictType`].
pub type Conflict<T> = Option<ConflictType<T>>;

/// Types implementing [`ConflictResolver`] support the method [`ConflictResolver::average`].
pub trait ConflictResolver: Sized {
    /// Attempt to merge `self` with `rhs` per [`ConflictParams`] and return the [Conflict].
    /// [None] is returned when `self == rhs`.
    fn average(self, rhs: Self, params: &ConflictParams) -> Conflict<Self>;
}

/// Controls the classification of a [Conflict] into [`ConflictType::Minor`] or [`ConflictType::Major`].
pub struct ConflictParams {
    pct: f32,
    min: f32,
    max: f32,
}

impl Default for ConflictParams {
    /// The default [`ConflictParams`] are chosen to minimize
    /// the likelihood that a [`ConflictType::Minor`] is noticeable.
    fn default() -> Self {
        Self {
            pct: 0.3,
            min: 10.0,
            max: 64.0,
        }
    }
}

/// Returns [`ConflictType`] for `lhs` and `rhs` per [`ConflictParams`].
fn classify_conflict<U>(lhs: f32, rhs: f32, params: &ConflictParams) -> ConflictType<U>
where
    f32: RoundTo<U>,
{
    let lhs_weight = lhs.abs() / (lhs.abs() + rhs.abs());
    let rhs_weight = 1. - lhs_weight;
    let lhs_weight_2 = lhs_weight.powf(1.5);
    let rhs_weight_2 = rhs_weight.powf(1.5);
    let lhs_weight = lhs_weight_2 / (lhs_weight_2 + rhs_weight_2);
    let rhs_weight = 1. - lhs_weight;
    let average = lhs_weight * lhs + rhs_weight * rhs;
    let minimum = lhs.min(rhs);
    let proportional_threshold = (params.pct * minimum).max(params.min);
    let difference = f32::abs(minimum - average);
    if difference >= proportional_threshold.min(params.max) {
        ConflictType::Major(average.round_to())
    } else {
        ConflictType::Minor(average.round_to())
    }
}

impl<T: Eq + Into<f64>> ConflictResolver for T
where
    f32: RoundTo<T>,
{
    fn average(self, rhs: Self, params: &ConflictParams) -> Conflict<Self> {
        if self == rhs {
            None
        } else {
            let lhs = self
                .into()
                .to_f32()
                .expect("lhs value cannot be represented as f32");
            let rhs = rhs
                .into()
                .to_f32()
                .expect("rhs value cannot be represented as f32");
            Some(classify_conflict(lhs, rhs, params))
        }
    }
}

impl<T> ConflictResolver for Vec3<T>
where
    T: Eq + PartialEq + ConflictResolver + Copy,
{
    fn average(self, rhs: Self, params: &ConflictParams) -> Conflict<Self> {
        if self == rhs {
            None
        } else {
            let mut num_major_conflicts = 0;

            let x = match self.x.average(rhs.x, params) {
                None => self.x,
                Some(ConflictType::Minor(x)) => x,
                Some(ConflictType::Major(x)) => {
                    num_major_conflicts += 1;
                    x
                }
            };

            let y = match self.y.average(rhs.y, params) {
                None => self.y,
                Some(ConflictType::Minor(y)) => y,
                Some(ConflictType::Major(y)) => {
                    num_major_conflicts += 1;
                    y
                }
            };

            let z = match self.z.average(rhs.z, params) {
                None => self.z,
                Some(ConflictType::Minor(z)) => z,
                Some(ConflictType::Major(z)) => {
                    num_major_conflicts += 1;
                    z
                }
            };

            if num_major_conflicts > 0 {
                Some(ConflictType::Major(Self { x, y, z }))
            } else {
                Some(ConflictType::Minor(Self { x, y, z }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ConflictResolver, ConflictType};
    use crate::land::terrain_map::Vec3;

    #[test]
    fn equal_scalars_have_no_conflict() {
        let params = super::ConflictParams::default();
        assert!(10i32.average(10, &params).is_none());
    }

    #[test]
    fn close_scalars_are_minor_conflicts() {
        let params = super::ConflictParams::default();
        match 100i32.average(102, &params) {
            Some(ConflictType::Minor(v)) => assert!((100..=102).contains(&v)),
            _ => panic!("expected minor conflict"),
        }
    }

    #[test]
    fn far_scalars_are_major_conflicts() {
        let params = super::ConflictParams::default();
        match 0i32.average(100, &params) {
            Some(ConflictType::Major(v)) => assert!((0..=100).contains(&v)),
            _ => panic!("expected major conflict"),
        }
    }

    #[test]
    fn vec3_with_major_component_is_major() {
        let params = super::ConflictParams::default();
        let lhs = Vec3::new(0i32, 10i32, 10i32);
        let rhs = Vec3::new(100i32, 12i32, 12i32);

        match lhs.average(rhs, &params) {
            Some(ConflictType::Major(_)) => {}
            _ => panic!("expected major vec3 conflict"),
        }
    }

    #[test]
    fn vec3_with_only_minor_components_is_minor() {
        let params = super::ConflictParams::default();
        let lhs = Vec3::new(100i32, 100i32, 100i32);
        let rhs = Vec3::new(102i32, 101i32, 103i32);

        match lhs.average(rhs, &params) {
            Some(ConflictType::Minor(_)) => {}
            _ => panic!("expected minor vec3 conflict"),
        }
    }
}
