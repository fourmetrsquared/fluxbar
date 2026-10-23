//! Animation primitives: easing functions and interpolation state.

use crate::config::EasingFunction;
use std::time::{Duration, Instant};

/// Evaluate an easing function at `t` in `0..=1`.
#[must_use]
pub fn ease(function: EasingFunction, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match function {
        EasingFunction::Linear => t,
        EasingFunction::EaseInQuad => t * t,
        EasingFunction::EaseOutQuad => 1.0 - (1.0 - t) * (1.0 - t),
        EasingFunction::EaseInOutQuad => {
            if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
            }
        }
        EasingFunction::EaseInCubic => t * t * t,
        EasingFunction::EaseOutCubic => 1.0 - (1.0 - t).powi(3),
        EasingFunction::EaseInOutCubic => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
        EasingFunction::EaseInQuart => t.powi(4),
        EasingFunction::EaseOutQuart => 1.0 - (1.0 - t).powi(4),
        EasingFunction::EaseInOutQuart => {
            if t < 0.5 {
                8.0 * t.powi(4)
            } else {
                1.0 - (-2.0 * t + 2.0).powi(4) / 2.0
            }
        }
        EasingFunction::EaseInQuint => t.powi(5),
        EasingFunction::EaseOutQuint => 1.0 - (1.0 - t).powi(5),
        EasingFunction::EaseInOutQuint => {
            if t < 0.5 {
                16.0 * t.powi(5)
            } else {
                1.0 - (-2.0 * t + 2.0).powi(5) / 2.0
            }
        }
        EasingFunction::EaseInSine => 1.0 - (t * std::f64::consts::FRAC_PI_2).cos(),
        EasingFunction::EaseOutSine => (t * std::f64::consts::FRAC_PI_2).sin(),
        EasingFunction::EaseInOutSine => -(std::f64::consts::PI * t).cos() / 2.0 + 0.5,
        EasingFunction::EaseInCirc => 1.0 - (1.0 - t * t).sqrt(),
        EasingFunction::EaseOutCirc => (1.0 - (t - 1.0).powi(2)).sqrt(),
        EasingFunction::EaseInOutCirc => {
            if t < 0.5 {
                (1.0 - (1.0 - (2.0 * t).powi(2)).sqrt()) / 2.0
            } else {
                ((1.0 - (-2.0 * t + 2.0).powi(2)).sqrt() + 1.0) / 2.0
            }
        }
        EasingFunction::EaseInExpo => {
            if t <= 0.0 {
                0.0
            } else {
                2.0_f64.powf(10.0 * t - 10.0)
            }
        }
        EasingFunction::EaseOutExpo => {
            if t >= 1.0 {
                1.0
            } else {
                1.0 - 2.0_f64.powf(-10.0 * t)
            }
        }
        EasingFunction::EaseInOutExpo => {
            if t <= 0.0 {
                0.0
            } else if t >= 1.0 {
                1.0
            } else if t < 0.5 {
                2.0_f64.powf(20.0 * t - 10.0) / 2.0
            } else {
                (2.0 - 2.0_f64.powf(-20.0 * t + 10.0)) / 2.0
            }
        }
        EasingFunction::EaseInBack => {
            const C1: f64 = 1.701_58;
            const C3: f64 = C1 + 1.0;
            C3 * t * t * t - C1 * t * t
        }
        EasingFunction::EaseOutBack => {
            const C1: f64 = 1.701_58;
            const C3: f64 = C1 + 1.0;
            1.0 + C3 * (t - 1.0).powi(3) + C1 * (t - 1.0).powi(2)
        }
        EasingFunction::EaseInOutBack => {
            const C1: f64 = 1.701_58;
            const C2: f64 = C1 * 1.525;
            if t < 0.5 {
                ((2.0 * t).powi(2) * ((C2 + 1.0) * 2.0 * t - C2)) / 2.0
            } else {
                ((2.0 * t - 2.0).powi(2) * ((C2 + 1.0) * (2.0 * t - 2.0) + C2) + 2.0) / 2.0
            }
        }
        EasingFunction::EaseInElastic => {
            if t <= 0.0 || t >= 1.0 {
                return t;
            }
            const C4: f64 = std::f64::consts::TAU / 3.0;
            -(2.0_f64.powf(10.0 * t - 10.0)) * ((10.0 * t - 10.75) * C4).sin()
        }
        EasingFunction::EaseOutElastic => {
            if t <= 0.0 || t >= 1.0 {
                return t;
            }
            const C4: f64 = std::f64::consts::TAU / 3.0;
            2.0_f64.powf(-10.0 * t) * ((10.0 * t - 0.75) * C4).sin() + 1.0
        }
        EasingFunction::EaseInOutElastic => {
            if t <= 0.0 || t >= 1.0 {
                return t;
            }
            const C5: f64 = std::f64::consts::TAU / 4.5;
            if t < 0.5 {
                -(2.0_f64.powf(20.0 * t - 10.0) * ((20.0 * t - 11.125) * C5).sin()) / 2.0
            } else {
                2.0_f64.powf(-20.0 * t + 10.0) * ((20.0 * t - 11.125) * C5).sin() / 2.0 + 1.0
            }
        }
        EasingFunction::EaseInBounce => 1.0 - ease_out_bounce(1.0 - t),
        EasingFunction::EaseOutBounce => ease_out_bounce(t),
        EasingFunction::EaseInOutBounce => {
            if t < 0.5 {
                (1.0 - ease_out_bounce(1.0 - 2.0 * t)) / 2.0
            } else {
                (1.0 + ease_out_bounce(2.0 * t - 1.0)) / 2.0
            }
        }
    }
}

fn ease_out_bounce(t: f64) -> f64 {
    const N1: f64 = 7.562_5;
    const D1: f64 = 2.75;
    if t < 1.0 / D1 {
        N1 * t * t
    } else if t < 2.0 / D1 {
        N1 * (t - 1.5 / D1).powi(2) + 0.75
    } else if t < 2.5 / D1 {
        N1 * (t - 2.25 / D1).powi(2) + 0.937_5
    } else {
        N1 * (t - 2.625 / D1).powi(2) + 0.984_375
    }
}

/// A single animated scalar.
#[derive(Debug, Clone)]
pub struct AnimatedValue {
    from: f64,
    to: f64,
    duration: Duration,
    easing: EasingFunction,
    started: Instant,
}

impl AnimatedValue {
    pub fn new(initial: f64) -> Self {
        Self {
            from: initial,
            to: initial,
            duration: Duration::ZERO,
            easing: EasingFunction::Linear,
            started: Instant::now(),
        }
    }

    /// Start a new transition. If a transition is already running, the current
    /// value becomes the new starting point (no visual snapping).
    pub fn animate_to(&mut self, target: f64, duration: Duration, easing: EasingFunction) {
        let current = self.value();
        self.from = current;
        self.to = target;
        self.duration = duration;
        self.easing = easing;
        self.started = Instant::now();
    }

    /// Start the transition from an explicit starting value.
    pub fn animate_from(
        &mut self,
        start: f64,
        target: f64,
        duration: Duration,
        easing: EasingFunction,
    ) {
        self.from = start;
        self.to = target;
        self.duration = duration;
        self.easing = easing;
        self.started = Instant::now();
    }

    /// Jump immediately to a value.
    pub fn set(&mut self, value: f64) {
        self.from = value;
        self.to = value;
        self.duration = Duration::ZERO;
        self.started = Instant::now();
    }

    /// Current interpolated value.
    pub fn value(&self) -> f64 {
        if self.duration.is_zero() {
            return self.to;
        }
        let elapsed = self.started.elapsed();
        if elapsed >= self.duration {
            return self.to;
        }
        let progress = elapsed.as_secs_f64() / self.duration.as_secs_f64();
        let eased = ease(self.easing, progress);
        self.from + (self.to - self.from) * eased
    }

    /// Target value.
    pub const fn target(&self) -> f64 {
        self.to
    }

    /// Whether the animation has finished.
    pub fn is_finished(&self) -> bool {
        self.duration.is_zero() || self.started.elapsed() >= self.duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_bounds() {
        for function in [
            EasingFunction::Linear,
            EasingFunction::EaseInOutExpo,
            EasingFunction::EaseOutBack,
            EasingFunction::EaseInBounce,
            EasingFunction::EaseOutElastic,
        ] {
            assert!(
                (ease(function, 0.0) - 0.0).abs() < 1e-9,
                "{function:?} at 0"
            );
            assert!(
                (ease(function, 1.0) - 1.0).abs() < 1e-9,
                "{function:?} at 1"
            );
        }
    }

    #[test]
    fn animated_value_reaches_target() {
        let mut value = AnimatedValue::new(0.0);
        value.animate_to(10.0, Duration::from_millis(5), EasingFunction::Linear);
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(value.value(), 10.0);
        assert!(value.is_finished());
    }

    #[test]
    fn animated_value_restarts_from_current() {
        let mut value = AnimatedValue::new(0.0);
        value.animate_to(100.0, Duration::from_millis(100), EasingFunction::Linear);
        std::thread::sleep(Duration::from_millis(10));
        let mid = value.value();
        value.animate_to(0.0, Duration::from_millis(100), EasingFunction::Linear);
        // The new animation starts at (approximately) the current value
        // instead of snapping to 100.
        assert!(value.from >= mid - 1e-6);
        assert!(value.from < 100.0);
    }
}
