use super::EC_TEMP_SENSOR_OFFSET_CELSIUS;

const fn get_pt<const N: usize>(points: &[(u8, u8); N], i: usize) -> (u8, u8) {
    let mut pt = points[i];
    pt.0 = pt.0.saturating_add(EC_TEMP_SENSOR_OFFSET_CELSIUS as u8);
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

/// Generates a perfectly smoothed, overshoot-free spline fan curve lookup table
/// that I totally 100% wrote myself. Trust.
///
/// # Arguments
/// * `points` - An array of (`temp`, `fan_speed`) points defining the curve, in strictly
///   increasing temperature order. The first and last points are the saturation limits.
pub(super) const fn generate_fan_curve_lut<const N: usize, const LUT_SIZE: usize>(
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

/// Runtime version of [`generate_fan_curve_lut`] that accepts a dynamically-sized slice
/// and returns a heap-allocated `Vec<u8>`. Uses the same Fritsch-Carlson monotone cubic
/// Hermite spline algorithm.
///
/// # Panics
/// Panics under the same conditions as the `const fn` version (fewer than 2 points,
/// non-monotone X coordinates, Y values > 100).
pub(super) fn generate_fan_curve_lut_dyn(points: &[(u8, u8)]) -> Vec<u8> {
    debug_assert!(
        points.len() >= 2,
        "At least two points (start and end) are required"
    );

    // Shift all X values by the EC sensor offset up-front.
    let shifted: Vec<(u8, u8)> = points
        .iter()
        .map(|&(x, y)| (x.saturating_add(EC_TEMP_SENSOR_OFFSET_CELSIUS as u8), y))
        .collect();

    // Validate
    debug_assert!(shifted.first().unwrap().1 <= 100, "Start Y must be <= 100");
    debug_assert!(shifted.last().unwrap().1 <= 100, "End Y must be <= 100");
    debug_assert!(
        shifted.windows(2).all(|w| w[1].0 > w[0].0),
        "Curve X coordinates must be strictly increasing"
    );
    debug_assert!(
        shifted.iter().all(|p| p.1 <= 100),
        "Intermediate Y must be <= 100"
    );

    let n = shifted.len();

    // Compute initial tangents (Catmull-Rom style averages at interior points)
    let seg_slopes: Vec<FanCurveFloat> = shifted.windows(2).map(|w| slope(w[0], w[1])).collect();

    let mut tangents: Vec<FanCurveFloat> = (0..n)
        .map(|i| match i {
            0 => seg_slopes[0],
            k if k == n - 1 => seg_slopes[n - 2],
            k => (seg_slopes[k - 1] + seg_slopes[k]) * 0.5,
        })
        .collect();

    // Apply Fritsch-Carlson monotonicity constraints
    for (i, (&s, w)) in seg_slopes.iter().zip(shifted.windows(2)).enumerate() {
        let _ = w; // windows used only to pair; slopes were pre-computed
        let max_t = 3.0 * s.abs();
        if s == 0.0 {
            tangents[i] = 0.0;
            tangents[i + 1] = 0.0;
        } else {
            let clamp_t = |t: FanCurveFloat| {
                if t.signum() != s.signum() {
                    0.0
                } else if t.abs() > max_t {
                    s.signum() * max_t
                } else {
                    t
                }
            };
            tangents[i] = clamp_t(tangents[i]);
            tangents[i + 1] = clamp_t(tangents[i + 1]);
        }
    }

    // Build the LUT by evaluating the spline at every integer X in the range
    let x_start = shifted.first().unwrap().0;
    let x_end = shifted.last().unwrap().0;

    let mut output: Vec<u8> = (x_start..=x_end)
        .map(|x| {
            // Find the segment enclosing `x`
            let seg = shifted
                .windows(2)
                .position(|w| x >= w[0].0 && x <= w[1].0)
                .unwrap_or(n - 2);

            let p0 = shifted[seg];
            let p1 = shifted[seg + 1];

            if x == p0.0 {
                p0.1
            } else if x == p1.0 {
                p1.1
            } else {
                let m0 = tangents[seg];
                let m1 = tangents[seg + 1];
                let dx = FanCurveFloat::from(p1.0 - p0.0);
                let t = (FanCurveFloat::from(x) - FanCurveFloat::from(p0.0)) / dx;
                let t2 = t * t;
                let t3 = t2 * t;
                let h00 = 2.0f64.mul_add(t3, -(3.0 * t2)) + 1.0;
                let h10 = 2.0f64.mul_add(-t2, t3) + t;
                let h01 = (-2.0f64).mul_add(t3, 3.0 * t2);
                let h11 = t3 - t2;
                let y0 = FanCurveFloat::from(p0.1);
                let y1 = FanCurveFloat::from(p1.1);
                let y_f = (h11 * m1).mul_add(dx, (h10 * m0).mul_add(dx, h00 * y0 + h01 * y1));
                y_f.round().clamp(0.0, 100.0) as u8
            }
        })
        .collect();
    // lut will never be modified again after this point
    output.shrink_to_fit();
    output
}
