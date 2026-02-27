#![allow(clippy::cast_sign_loss)]
#![allow(clippy::too_many_lines)]

const fn abs(x: i64) -> i64 {
    if x < 0 { -x } else { x }
}

/// Sign helper for const fn
const fn sign(x: i64) -> i64 {
    if x < 0 {
        -1
    } else if x > 0 {
        1
    } else {
        0
    }
}

const fn get_pt<const N: usize>(
    start: (u8, u8),
    end: (u8, u8),
    intermediates: &[(u8, u8); N],
    i: usize,
) -> (u8, u8) {
    let mut pt = if i == 0 {
        start
    } else if i == N + 1 {
        end
    } else {
        intermediates[i - 1]
    };
    pt.0 =
        pt.0.saturating_add((273 - crate::temp::EC_TEMP_SENSOR_OFFSET) as u8);
    pt
}

/// Computes the slope (secant) between two points in 16.16 fixed-point format
const fn slope(p1: (u8, u8), p2: (u8, u8)) -> i64 {
    if p2.0 == p1.0 {
        return 0; // Prevent divide by zero (should be blocked by validation)
    }
    let dy = (p2.1 as i64) - (p1.1 as i64);
    let dx = (p2.0 as i64) - (p1.0 as i64);
    (dy << 16) / dx
}

/// Generates a perfectly smoothed, overshoot-free spline fan curve lookup table at compile time.
///
/// # Arguments
/// * `start` - The (x, y) start point of the curve. Inputs below `x` clamp to `0`.
/// * `end` - The (x, y) end point of the curve. Inputs above `x` clamp to `100`.
/// * `intermediates` - An array of arbitrary interior points.
pub const fn generate_fan_curve_lut<const N: usize, const LUT_SIZE: usize>(
    start: (u8, u8),
    end: (u8, u8),
    intermediates: &[(u8, u8); N],
) -> [u8; LUT_SIZE]
where
    [(); N + 2]:,
{
    // 1. Validate Inputs (using shifted values)
    let shifted_start = get_pt(start, end, intermediates, 0);
    let shifted_end = get_pt(start, end, intermediates, N + 1);

    assert!(shifted_start.1 <= 100, "Start Y must be <= 100");
    assert!(shifted_end.1 <= 100, "End Y must be <= 100");

    let mut i = 1;
    let mut last_x = shifted_start.0;
    while i <= N {
        let pt = get_pt(start, end, intermediates, i);
        assert!(pt.1 <= 100, "Intermediate Y must be <= 100");
        assert!(
            pt.0 > last_x,
            "Curve X coordinates must be strictly increasing"
        );
        last_x = pt.0;
        i += 1;
    }
    assert!(
        shifted_end.0 > last_x,
        "End X must be strictly greater than last intermediate X"
    );

    // 2. Compute Initial Tangents for Cubic Spline
    // We now use generic_const_exprs to size tangents perfectly to N + 2
    let mut tangents = [0i64; N + 2];

    tangents[0] = slope(
        get_pt(start, end, intermediates, 0),
        get_pt(start, end, intermediates, 1),
    );
    tangents[N + 1] = slope(
        get_pt(start, end, intermediates, N),
        get_pt(start, end, intermediates, N + 1),
    );

    let mut i = 1;
    while i <= N {
        let s1 = slope(
            get_pt(start, end, intermediates, i - 1),
            get_pt(start, end, intermediates, i),
        );
        let s2 = slope(
            get_pt(start, end, intermediates, i),
            get_pt(start, end, intermediates, i + 1),
        );
        tangents[i] = i64::midpoint(s1, s2); // Average contiguous slopes
        i += 1;
    }

    // 3. Apply Fritsch-Carlson Monotonicity Constraints
    // This perfectly prevents the curve from overshooting or undershooting between points.
    let mut i = 0;
    while i <= N {
        let p_i = get_pt(start, end, intermediates, i);
        let p_next = get_pt(start, end, intermediates, i + 1);
        let s = slope(p_i, p_next);
        let s_sign = sign(s);
        let max_t = 3 * abs(s);

        if s == 0 {
            tangents[i] = 0;
            tangents[i + 1] = 0;
        } else {
            // Constrain left tangent
            if sign(tangents[i]) != s_sign {
                tangents[i] = 0;
            } else if abs(tangents[i]) > max_t {
                tangents[i] = s_sign * max_t;
            }
            // Constrain right tangent
            if sign(tangents[i + 1]) != s_sign {
                tangents[i + 1] = 0;
            } else if abs(tangents[i + 1]) > max_t {
                tangents[i + 1] = s_sign * max_t;
            }
        }
        i += 1;
    }

    assert!(
        LUT_SIZE == (end.0 - start.0 + 1) as usize,
        "LUT_SIZE must match the X range"
    );
    let mut lut = [0u8; LUT_SIZE];
    let mut lut_idx = 0usize;

    // x traverses from `shifted_start.0` to `shifted_end.0`
    let mut x_int = shifted_start.0 as usize;

    while x_int <= shifted_end.0 as usize {
        let x = x_int as u8;

        // Find the segment enclosing `x`
        let mut seg = 0;
        while seg <= N {
            let p_seg = get_pt(start, end, intermediates, seg);
            let p_next = get_pt(start, end, intermediates, seg + 1);
            if x >= p_seg.0 && x <= p_next.0 {
                break;
            }
            seg += 1;
        }

        let p0 = get_pt(start, end, intermediates, seg);
        let p1 = get_pt(start, end, intermediates, seg + 1);

        if x == p0.0 {
            lut[lut_idx] = p0.1;
        } else if x == p1.0 {
            lut[lut_idx] = p1.1;
        } else {
            let m0 = tangents[seg];
            let m1 = tangents[seg + 1];
            let dx = (p1.0 - p0.0) as i64;

            // Relative position t in 16.16 fixed-point
            let t = ((x as i64 - p0.0 as i64) << 16) / dx;
            let t2 = (t * t) >> 16;
            let t3 = (t2 * t) >> 16;

            let one = 1i64 << 16;

            // Cubic Hermite basis functions
            let h00 = 2 * t3 - 3 * t2 + one;
            let h10 = t3 - 2 * t2 + t;
            let h01 = -2 * t3 + 3 * t2;
            let h11 = t3 - t2;

            let y0 = p0.1 as i64;
            let y1 = p1.1 as i64;

            // Adjust tangents against interval delta
            let m0_t = m0 * dx;
            let m1_t = m1 * dx;

            // Evaluate the polynomial
            let mut y_fp = h00 * y0 + h01 * y1;
            y_fp += (h10 * m0_t) >> 16;
            y_fp += (h11 * m1_t) >> 16;

            // Extract and round to nearest integer
            let mut y = (y_fp + (1 << 15)) >> 16;

            // Guaranteed safeguard 0..=100 bound clamp
            if y < 0 {
                y = 0;
            } else if y > 100 {
                y = 100;
            }

            lut[lut_idx] = y as u8;
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
    pub fn get_fan_speed(&self, temp: u8) -> u8 {
        let start: u8 = self.start + 73;
        let end: u8 = self.end + 73;
        let index = (temp.clamp(start, end) - start) as usize;
        self.lut[index]
    }
}

macro_rules! define_profile {
    ($prof_ident:ident, $name_str:literal, $start:expr, $end:expr, $intermediates:expr) => {
        pub const $prof_ident: FanProfile = FanProfile {
            name: $name_str,
            start: $start.0,
            end: $end.0,
            lut: &{
                const S: (u8, u8) = $start;
                const E: (u8, u8) = $end;
                const LUT: [u8; (E.0 - S.0 + 1) as usize] =
                    generate_fan_curve_lut(S, E, &$intermediates);
                LUT
            },
        };
    };
}

define_profile!(
    FW_LAZIEST,
    "fw-laziest",
    (45, 0),
    (85, 100),
    [(65, 25), (70, 35), (75, 50)]
);

define_profile!(
    FW_LAZY,
    "fw-lazy",
    (50, 15),
    (85, 100),
    [(65, 25), (70, 35), (75, 50)]
);

define_profile!(
    FW_MEDIUM,
    "fw-medium",
    (40, 15),
    (85, 100),
    [(60, 30), (70, 40), (75, 80)]
);

define_profile!(FW_DEAF, "fw-deaf", (0, 20), (60, 100), [(40, 30), (50, 50)]);

define_profile!(FW_AEOLUS, "fw-aeolus", (0, 20), (60, 100), [(40, 50)]);

define_profile!(
    DEFAULT_PROFILE,
    "default",
    (25, 10),
    (85, 100),
    [(30, 15), (45, 30), (60, 50), (75, 80)]
);

define_profile!(
    QUIET_PROFILE,
    "quiet",
    (30, 10),
    (92, 100),
    [
        (35, 15),
        (50, 25),
        (65, 40),
        (80, 60),
        (85, 80),
        (90, 90),
    ]
);

define_profile!(
    PERFORMANCE_PROFILE,
    "performance",
    (25, 15),
    (80, 100),
    [(35, 30), (50, 50), (65, 75)]
);

define_profile!(
    TURBO_PROFILE,
    "turbo",
    (25, 25),
    (70, 100),
    [(30, 35), (45, 50), (60, 75)]
);

define_profile!(
    DEAF_PROFILE,
    "deaf",
    (25, 35),
    (65, 100),
    [(30, 40), (45, 50), (60, 75)]
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
