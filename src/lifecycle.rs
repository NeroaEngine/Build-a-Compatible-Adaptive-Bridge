use crate::types::ActivityState;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisibilitySample {
    pub distance_meters: f32,

    /// Approximate projected area of node on the current display.
    pub projected_area_pixels: f32,

    pub focused: bool,
    pub occluded: bool,
    pub user_interacting: bool,
}

#[derive(Clone, Debug)]
pub struct LifecyclePolicy {
    pub dormant_distance_meters: f32,
    pub throttle_distance_meters: f32,
    pub frozen_projected_area_pixels: f32,
    pub throttled_fps: u16,
}

impl Default for LifecyclePolicy {
    fn default() -> Self {
        Self {
            dormant_distance_meters: 35.0,
            throttle_distance_meters: 8.0,
            frozen_projected_area_pixels: 1_500.0,
            throttled_fps: 12,
        }
    }
}

impl LifecyclePolicy {
    pub fn desired_state(&self, sample: VisibilitySample) -> ActivityState {
        if sample.focused || sample.user_interacting {
            return ActivityState::Active;
        }

        if sample.occluded && sample.distance_meters >= self.throttle_distance_meters {
            return ActivityState::Dormant;
        }

        if sample.distance_meters >= self.dormant_distance_meters {
            return ActivityState::Dormant;
        }

        if sample.projected_area_pixels <= self.frozen_projected_area_pixels {
            return ActivityState::Frozen;
        }

        if sample.distance_meters >= self.throttle_distance_meters {
            return ActivityState::Throttled {
                max_fps: self.throttled_fps,
            };
        }

        ActivityState::Active
    }
}
