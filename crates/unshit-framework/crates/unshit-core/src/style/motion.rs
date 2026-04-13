/// Spring: damped harmonic oscillator.
pub struct Spring {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
    value: f32,
    velocity: f32,
    target: f32,
}

impl Spring {
    /// Create a new Spring starting at rest (value=0, velocity=0, target=0).
    pub fn new(stiffness: f32, damping: f32, mass: f32) -> Self {
        Self { stiffness, damping, mass, value: 0.0, velocity: 0.0, target: 0.0 }
    }

    /// Set the target value the spring should settle at.
    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// Advance the spring by `dt_secs` seconds using semi-implicit Euler integration.
    /// Returns the current value after integration.
    pub fn tick(&mut self, dt_secs: f32) -> f32 {
        let acceleration = (self.stiffness * (self.target - self.value)
            - self.damping * self.velocity)
            / self.mass;
        self.velocity += acceleration * dt_secs;
        self.value += self.velocity * dt_secs;
        self.value
    }

    /// Returns true when the spring has effectively settled (position and velocity below threshold).
    pub fn is_settled(&self, threshold: f32) -> bool {
        (self.value - self.target).abs() < threshold && self.velocity.abs() < threshold
    }

    /// Current value of the spring.
    pub fn value(&self) -> f32 {
        self.value
    }
}

/// Easing curves for tweens.
#[derive(Clone, Debug, PartialEq)]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// Custom cubic Bezier with control points (x1, y1, x2, y2).
    /// The endpoints P0=(0,0) and P3=(1,1) are fixed.
    CubicBezier(f32, f32, f32, f32),
}

impl Easing {
    /// Sample the easing curve at `t` in `[0, 1]`, returning the eased value.
    pub fn sample(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            // Cubic ease-in: t^3
            Easing::EaseIn => t * t * t,
            // Cubic ease-out: 1 - (1-t)^3
            Easing::EaseOut => {
                let inv = 1.0 - t;
                1.0 - inv * inv * inv
            }
            // Cubic ease-in-out: combines in and out
            Easing::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    let inv = -2.0 * t + 2.0;
                    1.0 - inv * inv * inv / 2.0
                }
            }
            // Iterative approximation for arbitrary cubic Bezier.
            // Control points: P0=(0,0), P1=(x1,y1), P2=(x2,y2), P3=(1,1).
            Easing::CubicBezier(x1, y1, x2, y2) => cubic_bezier_sample(t, *x1, *y1, *x2, *y2),
        }
    }
}

/// Evaluate a CSS-style cubic Bezier at parameter `t` (progress in x-space).
/// Uses Newton's method to invert the x component, then evaluates y.
fn cubic_bezier_sample(t: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    // Bernstein basis evaluation for x and y given curve parameter `u` in [0,1].
    let bezier_x = |u: f32| -> f32 {
        let inv = 1.0 - u;
        3.0 * inv * inv * u * x1 + 3.0 * inv * u * u * x2 + u * u * u
    };
    let bezier_y = |u: f32| -> f32 {
        let inv = 1.0 - u;
        3.0 * inv * inv * u * y1 + 3.0 * inv * u * u * y2 + u * u * u
    };
    let bezier_x_deriv = |u: f32| -> f32 {
        let inv = 1.0 - u;
        3.0 * (inv * inv * x1 + 2.0 * inv * u * (x2 - x1) + u * u * (1.0 - x2))
    };

    // Newton's method to find curve parameter `u` such that bezier_x(u) == t.
    let mut u = t;
    for _ in 0..8 {
        let x_err = bezier_x(u) - t;
        let deriv = bezier_x_deriv(u);
        if deriv.abs() < 1e-6 {
            break;
        }
        u -= x_err / deriv;
        u = u.clamp(0.0, 1.0);
    }
    bezier_y(u)
}

/// Tween: interpolates a value from `from` to `to` over `duration_ms` milliseconds.
pub struct Tween {
    pub from: f32,
    pub to: f32,
    pub duration_ms: u32,
    pub easing: Easing,
    elapsed_ms: f32,
}

impl Tween {
    /// Create a new Tween.
    pub fn new(from: f32, to: f32, duration_ms: u32, easing: Easing) -> Self {
        Self { from, to, duration_ms, easing, elapsed_ms: 0.0 }
    }

    /// Advance the tween by `dt_ms` milliseconds and return the current interpolated value.
    pub fn tick(&mut self, dt_ms: f32) -> f32 {
        self.elapsed_ms = (self.elapsed_ms + dt_ms).min(self.duration_ms as f32);
        self.value()
    }

    /// Returns true when the tween has completed.
    pub fn is_done(&self) -> bool {
        self.elapsed_ms >= self.duration_ms as f32
    }

    /// Current interpolated value based on elapsed time.
    pub fn value(&self) -> f32 {
        if self.duration_ms == 0 {
            return self.to;
        }
        let eased = self.easing.sample(self.progress());
        self.from + (self.to - self.from) * eased
    }

    /// Normalized progress in `[0, 1]`.
    pub fn progress(&self) -> f32 {
        if self.duration_ms == 0 {
            return 1.0;
        }
        (self.elapsed_ms / self.duration_ms as f32).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1. Spring converges to target
    #[test]
    fn spring_converges_to_target() {
        let mut spring = Spring::new(200.0, 20.0, 1.0);
        spring.set_target(1.0);
        for _ in 0..500 {
            spring.tick(0.016);
        }
        assert!(
            (spring.value() - 1.0).abs() < 0.01,
            "spring should converge close to target, got {}",
            spring.value()
        );
    }

    // 2. Spring is_settled after convergence
    #[test]
    fn spring_is_settled_after_convergence() {
        let mut spring = Spring::new(200.0, 20.0, 1.0);
        spring.set_target(1.0);
        for _ in 0..500 {
            spring.tick(0.016);
        }
        assert!(spring.is_settled(0.01), "spring should be settled");
    }

    // 3. Higher stiffness means faster convergence
    #[test]
    fn spring_higher_stiffness_faster_convergence() {
        let simulate_steps_to_settle = |stiffness: f32| -> u32 {
            let mut spring = Spring::new(stiffness, 20.0, 1.0);
            spring.set_target(1.0);
            for i in 0..2000 {
                spring.tick(0.016);
                if spring.is_settled(0.01) {
                    return i;
                }
            }
            2000
        };

        let steps_low = simulate_steps_to_settle(50.0);
        let steps_high = simulate_steps_to_settle(500.0);
        assert!(
            steps_high < steps_low,
            "higher stiffness ({} steps) should settle faster than lower stiffness ({} steps)",
            steps_high,
            steps_low
        );
    }

    // 4. Tween linear produces linear interpolation
    #[test]
    fn tween_linear_interpolation() {
        let mut tween = Tween::new(0.0, 100.0, 1000, Easing::Linear);
        // At 250ms (25%) value should be ~25
        tween.tick(250.0);
        assert!(
            (tween.value() - 25.0).abs() < 0.001,
            "linear tween at 25% should be 25, got {}",
            tween.value()
        );
        // At 500ms total (50%) value should be ~50
        tween.tick(250.0);
        assert!(
            (tween.value() - 50.0).abs() < 0.001,
            "linear tween at 50% should be 50, got {}",
            tween.value()
        );
    }

    // 5. Tween is_done after duration
    #[test]
    fn tween_is_done_after_duration() {
        let mut tween = Tween::new(0.0, 1.0, 500, Easing::Linear);
        assert!(!tween.is_done(), "tween should not be done at start");
        tween.tick(500.0);
        assert!(tween.is_done(), "tween should be done after full duration");
        assert!(
            (tween.value() - 1.0).abs() < 0.001,
            "tween value should be 1.0 at end, got {}",
            tween.value()
        );
    }

    // 6. Easing EaseIn/EaseOut shape (EaseIn should be slow at start, EaseOut fast at start)
    #[test]
    fn easing_ease_in_out_shape() {
        // EaseIn: value at 0.25 should be less than 0.25 (slow start)
        let ease_in_early = Easing::EaseIn.sample(0.25);
        assert!(
            ease_in_early < 0.25,
            "EaseIn at t=0.25 should be less than 0.25 (slow start), got {}",
            ease_in_early
        );

        // EaseOut: value at 0.25 should be greater than 0.25 (fast start)
        let ease_out_early = Easing::EaseOut.sample(0.25);
        assert!(
            ease_out_early > 0.25,
            "EaseOut at t=0.25 should be greater than 0.25 (fast start), got {}",
            ease_out_early
        );

        // Both should pass through 0 and 1 at the endpoints
        assert!((Easing::EaseIn.sample(0.0)).abs() < 1e-6);
        assert!((Easing::EaseIn.sample(1.0) - 1.0).abs() < 1e-6);
        assert!((Easing::EaseOut.sample(0.0)).abs() < 1e-6);
        assert!((Easing::EaseOut.sample(1.0) - 1.0).abs() < 1e-6);
    }

    // 7. Easing Linear returns t unchanged
    #[test]
    fn easing_linear_returns_t() {
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let result = Easing::Linear.sample(t);
            assert!(
                (result - t).abs() < 1e-6,
                "Linear.sample({}) should return {}, got {}",
                t,
                t,
                result
            );
        }
    }
}
