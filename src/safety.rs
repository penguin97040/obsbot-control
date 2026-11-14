//! Bounds and numeric checks shared by the interface, the workers and the tests.

/// The largest preview frame accepted, in bytes (64 MiB).
pub const PREVIEW_LIMIT: u64 = 64 * 1024 * 1024;

/// Clamps a hold-to-move speed to 5..=90 degrees per second (30 if not finite).
pub fn move_speed(speed: f32) -> f32 {
    if speed.is_finite() { speed.clamp(5.0, 90.0) } else { 30.0 }
}

/// A signed gimbal velocity with its size clamped by move_speed; 0 stays 0.
pub fn velocity(value: f32) -> f32 {
    if !value.is_finite() || value == 0.0 { 0.0 } else { move_speed(value.abs()).copysign(value) }
}

/// degrees × per_degree, clamped to min..=max and rounded. Returns min for bad input.
pub fn angle_value(degrees: f64, per_degree: f64, min: i32, max: i32) -> i32 {
    if !degrees.is_finite() || !per_degree.is_finite() || min > max {
        return min;
    }
    // Clamp in floating point before rounding, so the cast can never overflow.
    let value = (degrees * per_degree).clamp(min as f64, max as f64);
    value.round() as i32
}

/// One arrow-pad step: at least `step`, or 1/40 of the range, clamped to the range.
pub fn step_value(value: i32, min: i32, max: i32, step: i32, direction: i32) -> i32 {
    // Widen first so extreme ranges and steps cannot overflow.
    let delta = (step as i64).max((max as i64 - min as i64) / 40);
    let target = value as i64 + direction as i64 * delta;
    target.max(min as i64).min(max as i64) as i32
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoveSign {
    pub pan: i32,
    pub tilt: i32,
}

/// Hold-to-move directions for the arrow pad (dx: +1 right, dy: +1 up). On the
/// Tiny 2 Lite the vendor speed command (smooth) moves up/down the opposite way to
/// the UVC pan/tilt steps, and the steps move left/right the opposite way, so each
/// method flips one axis by default. The user's invert switches apply on top.
pub fn move_signs(smooth: bool, dx: i32, dy: i32, invert_horizontal: bool, invert_vertical: bool) -> MoveSign {
    let x = dx.clamp(-1, 1);
    let y = dy.clamp(-1, 1);
    let flip_x = invert_horizontal != !smooth;
    let flip_y = invert_vertical != smooth;
    MoveSign { pan: if flip_x { -x } else { x }, tilt: if flip_y { -y } else { y } }
}

/// A frame size we are willing to handle.
pub fn geometry(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && width <= 16384 && height <= 16384
        && (width as u64) * (height as u64) * 4 <= PREVIEW_LIMIT
}

/// Whether a locked frame buffer really holds `height` rows of `width` BGRA pixels.
/// `scan_offset` is the top row within the allocation. Both ends are checked before
/// any pointer arithmetic, including bottom-up images and the most negative pitch.
pub fn frame_bounds(width: u32, height: u32, pitch: i32, scan_offset: u64, allocation: u64) -> bool {
    if !geometry(width, height) || allocation > PREVIEW_LIMIT || scan_offset > allocation {
        return false;
    }
    let row = width as u64 * 4;
    let stride = pitch.unsigned_abs() as u64;
    if stride < row {
        return false;
    }
    let span = stride * (height as u64 - 1);
    if pitch < 0 {
        span <= scan_offset && row <= allocation - scan_offset
    } else {
        span <= allocation - scan_offset && row <= allocation - scan_offset - span
    }
}

/// Whether the gimbal may still be moving. Failed stops keep it latched so the next
/// pass can retry.
#[derive(Clone, Copy, Debug, Default)]
pub struct GimbalMotion {
    pub moving: bool,
}

impl GimbalMotion {
    /// Sends the stop command twice when moving; true once stopped.
    pub fn stop(&mut self, mut send: impl FnMut() -> bool) -> bool {
        if !self.moving {
            return true;
        }
        let first = send();
        let second = send();
        if first || second {
            self.moving = false;
        }
        !self.moving
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_and_velocities() {
        assert_eq!(move_speed(-100.0), 5.0);
        assert_eq!(move_speed(1000.0), 90.0);
        assert_eq!(move_speed(f32::NAN), 30.0);
        assert_eq!(velocity(f32::INFINITY), 0.0);
        assert_eq!(velocity(1.0), 5.0);
        assert_eq!(velocity(-1.0), -5.0);
        assert_eq!(velocity(-1000.0), -90.0);
        assert_eq!(velocity(1000.0), 90.0);
        assert_eq!(velocity(0.0), 0.0);
    }

    #[test]
    fn angle_values() {
        let (min, max) = (i32::MIN, i32::MAX);
        assert_eq!(angle_value(max as f64 + 100.0, 1.0, min, max), max);
        assert_eq!(angle_value(min as f64 - 100.0, 1.0, min, max), min);
        assert_eq!(angle_value(1e300, 3600.0, min, max), max);
        assert_eq!(angle_value(-1e300, 3600.0, min, max), min);
        assert_eq!(angle_value(12.5, 3600.0, min, max), 45000);
        assert_eq!(angle_value(f64::NAN, 1.0, -5, 5), -5);
        assert_eq!(angle_value(1.0, f64::INFINITY, -5, 5), -5);
        assert_eq!(angle_value(1.0, 1.0, 5, -5), 5);
    }

    #[test]
    fn step_values() {
        let (min, max) = (i32::MIN, i32::MAX);
        assert_eq!(step_value(max, min, max, max, 1), max);
        assert_eq!(step_value(min, min, max, max, -1), min);
        assert_eq!(step_value(0, min, max, 1, 1), 107374182);
        assert_eq!(step_value(0, -100, 100, 1, -1), -5);
        assert_eq!(step_value(0, -100, 100, 20, 1), 20);
    }

    #[test]
    fn move_sign_directions() {
        for dx in [-1, 0, 1] {
            for dy in [-1, 0, 1] {
                for h in [false, true] {
                    for v in [false, true] {
                        // Smooth flips vertical, steps flip horizontal; each switch flips once more.
                        let smooth = move_signs(true, dx, dy, h, v);
                        let steps = move_signs(false, dx, dy, h, v);
                        assert_eq!(smooth.pan, if h { -dx } else { dx });
                        assert_eq!(smooth.tilt, if v { dy } else { -dy });
                        assert_eq!(steps.pan, if h { dx } else { -dx });
                        assert_eq!(steps.tilt, if v { -dy } else { dy });
                    }
                }
            }
        }
        assert_eq!(move_signs(true, 5, -9, false, false), MoveSign { pan: 1, tilt: 1 });
    }

    #[test]
    fn geometry_limits() {
        assert!(geometry(4096, 4096));
        assert!(!geometry(4096, 4097));
        assert!(!geometry(0, 1));
        assert!(!geometry(16385, 1));
    }

    #[test]
    fn frame_bounds_cases() {
        assert!(frame_bounds(2, 3, 12, 0, 32));
        assert!(frame_bounds(2, 3, -12, 24, 32));
        assert!(!frame_bounds(2, 3, 12, 0, 31));
        assert!(!frame_bounds(2, 3, -12, 24, 31));
        assert!(!frame_bounds(2, 3, -12, 23, 32));
        assert!(!frame_bounds(2, 3, 7, 0, 32));
        assert!(!frame_bounds(2, 3, i32::MIN, 0, 32));
        assert!(!frame_bounds(2, 3, i32::MIN, 1 << 32, 1 << 33));
        assert!(!frame_bounds(1, 1, 4, 9, 8));
        assert!(!frame_bounds(1, 1, 4, 0, PREVIEW_LIMIT + 1));
    }

    #[test]
    fn frame_bounds_matches_row_enumeration() {
        for pitch in [-16i32, -8, 0, 4, 8, 16] {
            for offset in 0u64..40 {
                for length in 0u64..40 {
                    let mut fits = pitch <= -8 || pitch >= 8;
                    for y in 0..3i64 {
                        let row = offset as i64 + pitch as i64 * y;
                        fits = fits && row >= 0 && row + 8 <= length as i64;
                    }
                    assert_eq!(frame_bounds(2, 3, pitch, offset, length), fits, "pitch {pitch} offset {offset} length {length}");
                }
            }
        }
    }

    #[test]
    fn gimbal_stop_retries_until_one_send_works() {
        let mut gimbal = GimbalMotion { moving: true };
        let mut calls = 0;
        assert!(!gimbal.stop(|| { calls += 1; false }));
        assert!(gimbal.moving);
        assert_eq!(calls, 2);
        assert!(gimbal.stop(|| { calls += 1; true }));
        assert!(!gimbal.moving);
        assert_eq!(calls, 4);

        gimbal.moving = true;
        let mut attempts = 0;
        assert!(gimbal.stop(|| { attempts += 1; attempts == 2 }));
        assert!(!gimbal.moving);
        assert_eq!(attempts, 2);

        // Not moving: nothing is sent.
        assert!(gimbal.stop(|| panic!("should not send")));
    }
}
