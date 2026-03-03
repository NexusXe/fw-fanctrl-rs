#![allow(clippy::cast_sign_loss)]
#![allow(clippy::too_many_lines)]

use crate::{
    info,
    temp::{CelsiusTemp, EC_TEMP_SENSOR_OFFSET_CELSIUS, UnvalidatedEcTemp, ValidEcTemp},
};

use std::borrow::Cow;

pub(crate) mod curve_parsing;

mod curve_lut_gen;
use curve_lut_gen::generate_fan_curve_lut;

/// A fan curve profile, either built-in or user-defined.
#[derive(Clone)]
pub(crate) struct FanProfile {
    /// Human-readable curve name
    pub name: Cow<'static, str>,
    /// Start temperature (in EC temp units). Values below this will be clamped to the start value.
    pub start: u8,
    /// End temperature (in EC temp units). Values above this will be clamped to the end value.
    pub end: u8,
    /// Fan speed lookup table. The index is `temp - start`.
    pub lut: Cow<'static, [u8]>,
    /// XXH3 signature of defined points
    pub signature: u64,
}

impl FanProfile {
    pub fn get_fan_speed(&self, temp: ValidEcTemp) -> u8 {
        let start: u8 = self.start;
        let end: u8 = self.end;
        let index = (temp.0.clamp(start, end) - start) as usize;
        self.lut[index]
    }
}

impl std::fmt::Display for FanProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Fan Curve: {}", self.name)?;
        let start_celsius: CelsiusTemp = UnvalidatedEcTemp(self.start)
            .to_celsius()
            .expect("Invalid start temperature");
        let end_celsius: CelsiusTemp = UnvalidatedEcTemp(self.end)
            .to_celsius()
            .expect("Invalid end temperature");

        writeln!(
            f,
            "Defined between {:+}°C and {:+}°C",
            start_celsius.0, end_celsius.0
        )?;
        writeln!(f, "Signature: 0x{:x}", self.signature)?;
        Ok(())
    }
}

macro_rules! define_profile {
    ($prof_ident:ident, $name_str:literal, $points:expr) => {
        pub const $prof_ident: FanProfile = FanProfile {
            name: Cow::Borrowed($name_str),
            start: $points[0].0 + EC_TEMP_SENSOR_OFFSET_CELSIUS as u8,
            end: $points[$points.len() - 1].0 + EC_TEMP_SENSOR_OFFSET_CELSIUS as u8,
            lut: Cow::Borrowed(&{
                const PTS: &[(u8, u8)] = &$points;
                const N: usize = PTS.len();
                const LUT_SIZE: usize = (PTS[N - 1].0 - PTS[0].0 + 1) as usize;
                const LUT: [u8; LUT_SIZE] = generate_fan_curve_lut(&{
                    // Re-express as a fixed-size array for the const fn
                    const fn to_array<const M: usize>(s: &[(u8, u8)]) -> [(u8, u8); M] {
                        let mut arr = [(0u8, 0u8); M];
                        let mut i = 0;
                        while i < M {
                            arr[i] = s[i];
                            i += 1;
                        }
                        arr
                    }
                    to_array::<N>(PTS)
                });
                LUT
            }),
            signature: {
                const PTS: &[(u8, u8)] = &$points;
                xxhash_rust::const_xxh3::xxh3_64(flatten_points(PTS))
            },
        };
    };
}

define_profile!(
    DEFAULT_PROFILE,
    "default",
    [(25, 10), (30, 15), (45, 30), (60, 50), (75, 80), (85, 100)]
);

define_profile!(
    QUIET_PROFILE,
    "quiet",
    [
        (30, 10),
        (35, 15),
        (50, 25),
        (65, 40),
        (80, 60),
        (85, 80),
        (90, 90),
        (92, 100),
    ]
);

define_profile!(
    PERFORMANCE_PROFILE,
    "performance",
    [(25, 15), (35, 30), (50, 50), (65, 75), (80, 100)]
);

define_profile!(
    TURBO_PROFILE,
    "turbo",
    [(25, 25), (30, 35), (45, 50), (60, 75), (70, 100)]
);

define_profile!(
    DEAF_PROFILE,
    "deaf",
    [(25, 35), (30, 40), (45, 50), (60, 75), (65, 100)]
);

define_profile!(MAX_PROFILE, "max", [(0, 99), (1, 100)]);

pub const BUILTIN_PROFILES: &[FanProfile] = &[
    DEFAULT_PROFILE,
    QUIET_PROFILE,
    PERFORMANCE_PROFILE,
    TURBO_PROFILE,
    DEAF_PROFILE,
    MAX_PROFILE,
];

pub(crate) const fn flatten_points(points: &[(u8, u8)]) -> &[u8] {
    let ptr = points.as_ptr().cast::<u8>();
    unsafe { std::slice::from_raw_parts(ptr, points.len() * 2) }
}

pub(crate) fn get_profile_by_name<'a>(
    name: &str,
    profiles: &'a [FanProfile],
) -> Option<&'a FanProfile> {
    profiles.iter().find(|p| p.name == name).map_or_else(
        || {
            info!("Profile \"{name}\" not found.");
            None
        },
        Some,
    )
}

#[cfg(test)]
mod tests;
