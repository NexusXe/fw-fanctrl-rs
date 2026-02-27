#![allow(clippy::cast_sign_loss)]
#![allow(clippy::too_many_lines)]

use crate::temp::EcTemp;

const fn get_pt<const N: usize>(points: &[(u8, u8); N], i: usize) -> (u8, u8) {
    let mut pt = points[i];
    pt.0 =
        pt.0.saturating_add((273 - crate::temp::EC_TEMP_SENSOR_OFFSET) as u8);
    pt
}

pub type FanCurveFloat = f64;

const fn slope(p1: (u8, u8), p2: (u8, u8)) -> FanCurveFloat {
    if p2.0 == p1.0 {
        return 0.0; // Prevent divide by zero (should be blocked by validation)
    }
    let dy = FanCurveFloat::from(p2.1) - FanCurveFloat::from(p1.1);
    let dx = FanCurveFloat::from(p2.0) - FanCurveFloat::from(p1.0);
    dy / dx
}

/// Generates a perfectly smoothed, overshoot-free spline fan curve lookup table.
///
/// # Arguments
/// * `points` - An array of (`temp`, `fan_speed`) points defining the curve, in strictly
///   increasing temperature order. The first and last points are the saturation limits.
pub const fn generate_fan_curve_lut<const N: usize, const LUT_SIZE: usize>(
    points: &[(u8, u8); N],
) -> [u8; LUT_SIZE]
where
    [(); N]:,
{
    assert!(N >= 2, "At least two points (start and end) are required");

    // 1. Validate Inputs (using shifted values)
    let shifted_start = get_pt(points, 0);
    let shifted_end = get_pt(points, N - 1);

    assert!(shifted_start.1 <= 100, "Start Y must be <= 100");
    assert!(shifted_end.1 <= 100, "End Y must be <= 100");

    let mut i = 1;
    let mut last_x = shifted_start.0;
    while i < N {
        let pt = get_pt(points, i);
        assert!(pt.1 <= 100, "Intermediate Y must be <= 100");
        assert!(
            pt.0 > last_x,
            "Curve X coordinates must be strictly increasing"
        );
        last_x = pt.0;
        i += 1;
    }

    // 2. Compute Initial Tangents for Cubic Spline
    let mut tangents: [FanCurveFloat; N] = [0.0; N];

    tangents[0] = slope(get_pt(points, 0), get_pt(points, 1));
    tangents[N - 1] = slope(get_pt(points, N - 2), get_pt(points, N - 1));

    let mut i = 1;
    while i < N - 1 {
        let s1 = slope(get_pt(points, i - 1), get_pt(points, i));
        let s2 = slope(get_pt(points, i), get_pt(points, i + 1));
        tangents[i] = (s1 + s2) * 0.5; // Average contiguous slopes
        i += 1;
    }

    // 3. Apply Fritsch-Carlson Monotonicity Constraints
    let mut i = 0;
    while i < N - 1 {
        let p_i = get_pt(points, i);
        let p_next = get_pt(points, i + 1);
        let s = slope(p_i, p_next);
        let max_t = 3.0 * s.abs();

        if s == 0.0 {
            tangents[i] = 0.0;
            tangents[i + 1] = 0.0;
        } else {
            // Constrain left tangent
            if tangents[i].signum() != s.signum() {
                tangents[i] = 0.0;
            } else if tangents[i].abs() > max_t {
                tangents[i] = s.signum() * max_t;
            }
            // Constrain right tangent
            if tangents[i + 1].signum() != s.signum() {
                tangents[i + 1] = 0.0;
            } else if tangents[i + 1].abs() > max_t {
                tangents[i + 1] = s.signum() * max_t;
            }
        }
        i += 1;
    }

    assert!(
        LUT_SIZE == (points[N - 1].0 - points[0].0 + 1) as usize,
        "LUT_SIZE must match the X range"
    );
    let mut lut = [0u8; LUT_SIZE];
    let mut lut_idx = 0usize;

    let mut x_int = shifted_start.0 as usize;

    while x_int <= shifted_end.0 as usize {
        let x = x_int as u8;

        // Find the segment enclosing `x`
        let mut seg = 0;
        while seg < N - 1 {
            let p_seg = get_pt(points, seg);
            let p_next = get_pt(points, seg + 1);
            if x >= p_seg.0 && x <= p_next.0 {
                break;
            }
            seg += 1;
        }

        let p0 = get_pt(points, seg);
        let p1 = get_pt(points, seg + 1);

        if x == p0.0 {
            lut[lut_idx] = p0.1;
        } else if x == p1.0 {
            lut[lut_idx] = p1.1;
        } else {
            let m0 = tangents[seg];
            let m1 = tangents[seg + 1];
            let dx = FanCurveFloat::from(p1.0 - p0.0);

            // Relative position t in [0, 1]
            let t = (FanCurveFloat::from(x) - FanCurveFloat::from(p0.0)) / dx;
            let t2 = t * t;
            let t3 = t2 * t;

            // Cubic Hermite basis functions
            let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
            let h10 = t3 - 2.0 * t2 + t;
            let h01 = -2.0 * t3 + 3.0 * t2;
            let h11 = t3 - t2;

            let y0 = FanCurveFloat::from(p0.1);
            let y1 = FanCurveFloat::from(p1.1);

            // Evaluate the polynomial (tangents are scaled by dx to match Hermite convention)
            let y_f = h00 * y0 + h01 * y1 + h10 * m0 * dx + h11 * m1 * dx;

            // Round to nearest integer and clamp to 0..=100
            let y = y_f.round().clamp(0.0, 100.0) as u8;
            lut[lut_idx] = y;
        }

        x_int += 1;
        lut_idx += 1;
    }

    lut
}

pub struct FanProfile {
    pub name: &'static str,
    pub start: u8,
    pub end: u8,
    pub lut: &'static [u8],
}

impl FanProfile {
    pub fn get_fan_speed<T: Into<EcTemp>>(&self, temp: T) -> u8 {
        let temp: EcTemp = temp.into();
        let start: u8 = self.start + 73;
        let end: u8 = self.end + 73;
        let index = (temp.0.clamp(start, end) - start) as usize;
        self.lut[index]
    }
}

macro_rules! define_profile {
    ($prof_ident:ident, $name_str:literal, $points:expr) => {
        pub const $prof_ident: FanProfile = FanProfile {
            name: $name_str,
            start: $points[0].0,
            end: $points[$points.len() - 1].0,
            lut: &{
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
            },
        };
    };
}

define_profile!(
    FW_LAZIEST,
    "fw-laziest",
    [(45, 0), (65, 25), (70, 35), (75, 50), (85, 100)]
);

define_profile!(
    FW_LAZY,
    "fw-lazy",
    [(50, 15), (65, 25), (70, 35), (75, 50), (85, 100)]
);

define_profile!(
    FW_MEDIUM,
    "fw-medium",
    [(40, 15), (60, 30), (70, 40), (75, 80), (85, 100)]
);

define_profile!(FW_DEAF, "fw-deaf", [(0, 20), (40, 30), (50, 50), (60, 100)]);

define_profile!(FW_AEOLUS, "fw-aeolus", [(0, 20), (40, 50), (60, 100)]);

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

pub const PROFILES: &[FanProfile] = &[
    FW_LAZIEST,
    FW_LAZY,
    FW_MEDIUM,
    FW_DEAF,
    FW_AEOLUS,
    DEFAULT_PROFILE,
    QUIET_PROFILE,
    PERFORMANCE_PROFILE,
    TURBO_PROFILE,
    DEAF_PROFILE,
];

pub fn get_profile_by_name(name: &str) -> Option<&'static FanProfile> {
    PROFILES.iter().find(|p| p.name == name)
}
