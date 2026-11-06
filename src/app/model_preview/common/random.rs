//! The games' linear congruential generator, and the ways each turns a state
//! into a number. The maps are kept apart: they differ in which bits they take
//! and whether they divide or multiply, and so in the last bit of what comes
//! out.

/// One step of the generator every game uses.
pub(in crate::app::model_preview) fn lcg_step(state: u32) -> u32 {
    state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)
}

/// The state's high 16 bits.
pub(in crate::app::model_preview) fn high_bits(state: u32) -> u32 {
    state >> 16
}

/// The high 16 bits over 65536: `[0, 1)`.
pub(in crate::app::model_preview) fn high16_over_65536(state: u32) -> f32 {
    high_bits(state) as f32 / 65_536.0
}

/// The high 16 bits over 65535: `[0, 1]`.
pub(in crate::app::model_preview) fn high16_over_65535(state: u32) -> f32 {
    high_bits(state) as f32 / 65_535.0
}

/// The high 16 bits times the `f32` nearest 1/65535: `[0, 1]`, but rounded
/// as a product rather than a quotient.
pub(in crate::app::model_preview) fn high16_times_inverse_65535(state: u32) -> f32 {
    high_bits(state) as f32 * 1.525_902_2e-5
}

/// The high 24 bits over 2^24: `[0, 1)`.
pub(in crate::app::model_preview) fn high24_over_2_24(state: u32) -> f32 {
    (state >> 8) as f32 / 16_777_216.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generator_steps_as_the_games_do() {
        assert_eq!(lcg_step(0), 1_013_904_223);
        assert_eq!(lcg_step(1), 1_015_568_748);
        assert_eq!(lcg_step(u32::MAX), 1_012_239_698);
    }

    #[test]
    fn each_map_keeps_its_bits_at_the_ends() {
        assert_eq!(high16_over_65536(0xFFFF_FFFF).to_bits(), (65_535.0f32 / 65_536.0).to_bits());
        assert_eq!(high16_over_65535(0xFFFF_FFFF), 1.0);
        assert_eq!(high24_over_2_24(0xFFFF_FFFF).to_bits(), (16_777_215.0f32 / 16_777_216.0).to_bits());
        assert_eq!(high24_over_2_24(0), 0.0);
        for map in [high16_over_65536, high16_over_65535, high16_times_inverse_65535] {
            assert_eq!(map(0), 0.0);
            assert_eq!(map(0xFFFF), 0.0, "the low 16 bits never reach the 16-bit maps");
        }
    }

    /// Maps that look alike give different bits, which is why they are kept.
    #[test]
    fn maps_that_look_alike_disagree() {
        let differs = (0..=u16::MAX).map(|high| u32::from(high) << 16).find(|&state| {
            high16_over_65535(state).to_bits() != high16_times_inverse_65535(state).to_bits()
        });
        assert!(differs.is_some(), "dividing by 65535 and multiplying by its inverse part somewhere");
        assert_ne!(high16_over_65536(0x8000_0000), high16_over_65535(0x8000_0000));
        assert_ne!(high24_over_2_24(0x0000_0100), high16_over_65536(0x0000_0100));
    }
}
