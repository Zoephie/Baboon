//! Three-component vector arithmetic.
//!
//! The ways to make a vector unit length are kept apart, each under a name
//! that says what it does with a short vector: the games disagree there, in
//! the comparison, the threshold, whether they divide or multiply by the
//! reciprocal, and what comes back. The tests pin each one's bits.

use blam_tags::math::Matrix4;

pub(in crate::app::model_preview) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(in crate::app::model_preview) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(in crate::app::model_preview) fn plus(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] + b[axis])
}

pub(in crate::app::model_preview) fn minus(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] - b[axis])
}

pub(in crate::app::model_preview) fn scaled(a: [f32; 3], by: f32) -> [f32; 3] {
    a.map(|axis| axis * by)
}

pub(in crate::app::model_preview) fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

pub(in crate::app::model_preview) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Divided by its length, or as it is when it has none.
pub(in crate::app::model_preview) fn unit_or_unchanged(a: [f32; 3]) -> [f32; 3] {
    let length = length(a);
    if length > 0.0 {
        a.map(|axis| axis / length)
    } else {
        a
    }
}

/// As it is under a length of 1e-6, else divided by its length. Unlike
/// [`unit_or_unchanged`], a NaN length divides.
pub(in crate::app::model_preview) fn unit_unless_under_micro(a: [f32; 3]) -> [f32; 3] {
    let length = length(a);
    if length < 1e-6 {
        a
    } else {
        a.map(|axis| axis / length)
    }
}

/// Times the reciprocal of its length from 1e-4 on, or as it is below.
pub(in crate::app::model_preview) fn unit_by_reciprocal(a: [f32; 3]) -> [f32; 3] {
    let length = length(a);
    if length >= 0.0001 {
        scaled(a, 1.0 / length)
    } else {
        a
    }
}

/// Divided by its length from 1e-4 on, or none below.
pub(in crate::app::model_preview) fn checked_unit(a: [f32; 3]) -> Option<[f32; 3]> {
    let length = length(a);
    (length >= 0.0001).then(|| a.map(|axis| axis / length))
}

/// Divided by its length, or none when it has none.
pub(in crate::app::model_preview) fn nonzero_unit(a: [f32; 3]) -> Option<[f32; 3]> {
    let length = length(a);
    (length > 0.0).then(|| a.map(|axis| axis / length))
}

/// Divided by its length, or by 1e-6 when that is shorter.
pub(in crate::app::model_preview) fn clamped_unit(a: [f32; 3]) -> [f32; 3] {
    let length = length(a).max(1e-6);
    a.map(|axis| axis / length)
}

/// Divided by its length above 1e-4, or straight up below.
pub(in crate::app::model_preview) fn unit_or_up(a: [f32; 3]) -> [f32; 3] {
    let length = length(a);
    if length > 0.0001 {
        a.map(|axis| axis / length)
    } else {
        [0.0, 0.0, 1.0]
    }
}

pub(in crate::app::model_preview) const IDENTITY: Matrix4 = Matrix4 {
    m: [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ],
};

/// `perpendicular3d`: a vector square to `a`, from its two largest parts.
pub(in crate::app::model_preview) fn perpendicular(a: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = a.map(f32::abs);
    if x > y || x > z {
        if y > z {
            [a[1], -a[0], 0.0]
        } else {
            [-a[2], 0.0, a[0]]
        }
    } else {
        [0.0, a[2], -a[1]]
    }
}

pub(in crate::app::model_preview) fn axis_of(matrix: &Matrix4, axis: usize) -> [f32; 3] {
    std::array::from_fn(|row| matrix.m[row][axis])
}

pub(in crate::app::model_preview) fn place_of(matrix: &Matrix4) -> [f32; 3] {
    axis_of(matrix, 3)
}

pub(in crate::app::model_preview) fn matrix_of(axes: [[f32; 3]; 3], position: [f32; 3]) -> Matrix4 {
    let mut matrix = IDENTITY;
    for row in 0..3 {
        for axis in 0..3 {
            matrix.m[row][axis] = axes[axis][row];
        }
        matrix.m[row][3] = position[row];
    }
    matrix
}

pub(in crate::app::model_preview) fn rigid_inverse(matrix: &Matrix4) -> Matrix4 {
    let axes = [axis_of(matrix, 0), axis_of(matrix, 1), axis_of(matrix, 2)];
    let place = place_of(matrix);
    let mut inverse = IDENTITY;
    for row in 0..3 {
        inverse.m[row][0] = axes[row][0];
        inverse.m[row][1] = axes[row][1];
        inverse.m[row][2] = axes[row][2];
        inverse.m[row][3] = -dot(axes[row], place);
    }
    inverse
}

/// Tool's postprocess takes bounds all under this as unset.
pub(in crate::app::model_preview) const UNSET: f32 = 0.0001;

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(a: [f32; 3]) -> [u32; 3] {
        a.map(f32::to_bits)
    }

    fn bits_of(a: Option<[f32; 3]>) -> Option<[u32; 3]> {
        a.map(bits)
    }

    const ZERO: [f32; 3] = [0.0; 3];
    const TINY: [f32; 3] = [1e-7, 0.0, 0.0];
    const NAN: [f32; 3] = [f32::NAN, 0.0, 0.0];
    /// Between 1e-6 and 1e-4: some rules divide it, some leave it.
    const SHORT: [f32; 3] = [1e-5, 0.0, 0.0];
    const LONG: [f32; 3] = [3.0, 4.0, 12.0];

    #[test]
    fn each_unit_rule_keeps_its_bits_at_the_edges() {
        let divided = LONG.map(|axis| axis / 13.0);
        assert_eq!(bits(unit_or_unchanged(ZERO)), bits(ZERO));
        assert_eq!(bits(unit_or_unchanged(TINY)), bits([1.0, 0.0, 0.0]));
        assert_eq!(
            bits(unit_or_unchanged(NAN)),
            bits(NAN),
            "a NaN length leaves it as it is"
        );
        assert_eq!(bits(unit_or_unchanged(LONG)), bits(divided));

        assert_eq!(bits(unit_unless_under_micro(TINY)), bits(TINY));
        assert!(
            unit_unless_under_micro(NAN)[1].is_nan(),
            "a NaN length divides, NaN everywhere"
        );
        assert_eq!(bits(unit_unless_under_micro(SHORT)), bits([1.0, 0.0, 0.0]));

        assert_eq!(bits(unit_by_reciprocal(SHORT)), bits(SHORT));
        assert_eq!(
            bits(unit_by_reciprocal(LONG)),
            bits(scaled(LONG, 1.0 / 13.0))
        );

        assert_eq!(checked_unit(SHORT), None);
        assert_eq!(bits_of(checked_unit(LONG)), Some(bits(divided)));
        assert_eq!(checked_unit(NAN), None);

        assert_eq!(nonzero_unit(ZERO), None);
        assert_eq!(bits_of(nonzero_unit(TINY)), Some(bits([1.0, 0.0, 0.0])));

        assert_eq!(bits(clamped_unit(ZERO)), bits(ZERO));
        assert_eq!(bits(clamped_unit(TINY)), bits([1e-7 / 1e-6, 0.0, 0.0]));
        // NaN max 1e-6 is 1e-6: the NaN axis stays NaN and the rest divide.
        let clamped = clamped_unit(NAN);
        assert!(clamped[0].is_nan());
        assert_eq!([clamped[1].to_bits(), clamped[2].to_bits()], [0, 0]);

        assert_eq!(bits(unit_or_up(SHORT)), bits([0.0, 0.0, 1.0]));
        assert_eq!(bits(unit_or_up(LONG)), bits(divided));
    }

    /// Rules that look alike give different bits, which is why they are kept.
    #[test]
    fn rules_that_look_alike_disagree() {
        assert_ne!(
            bits(unit_or_unchanged(SHORT)),
            bits(unit_by_reciprocal(SHORT))
        );
        assert_ne!(
            bits(unit_or_unchanged(TINY)),
            bits(unit_unless_under_micro(TINY))
        );
        assert_ne!(bits(clamped_unit(TINY)), bits(unit_or_unchanged(TINY)));
        assert_ne!(bits(unit_or_up(SHORT)), bits(unit_or_unchanged(SHORT)));
        // Dividing and multiplying by the reciprocal part in the last bit.
        let skew = [0.1, 0.7, 0.3];
        assert_ne!(
            bits(unit_or_unchanged(skew)),
            bits(unit_by_reciprocal(skew))
        );
    }
}
