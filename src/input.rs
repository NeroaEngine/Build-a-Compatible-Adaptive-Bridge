use crate::types::{
    BrowserInput, BrowserPoint, ButtonState, Modifiers, MouseButton, ScrollMode, Viewport,
};
use glam::{Mat4, Vec2, Vec3};

const EPSILON: f32 = 1.0e-6;

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

impl Ray {
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        Self {
            origin,
            direction: direction.normalize_or_zero(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SpatialNodeGeometry {
    /// Transform from browser-node local coordinates into world
    /// coordinates.
    pub world_from_local: Mat4,

    /// Physical plane size in world/local units.
    pub size: Vec2,

    /// Browser backing viewport.
    pub viewport: Viewport,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MappedPoint {
    /// 0..1 coordinate in the web surface.
    pub uv: Vec2,

    /// Physical/device framebuffer pixels.
    pub device_px: BrowserPoint,

    /// CSS pixels.
    pub css_px: BrowserPoint,

    /// Intersection distance along the world ray.
    pub ray_distance: f32,
}

pub struct SpatialInputTranslator;

impl SpatialInputTranslator {
    pub fn ray_to_browser(ray: Ray, geometry: &SpatialNodeGeometry) -> Option<MappedPoint> {
        if geometry.size.x <= EPSILON || geometry.size.y <= EPSILON {
            return None;
        }

        let determinant = geometry.world_from_local.determinant();

        if !determinant.is_finite() || determinant.abs() <= EPSILON {
            return None;
        }

        let local_from_world = geometry.world_from_local.inverse();

        let local_origin = local_from_world.transform_point3(ray.origin);

        let local_direction = local_from_world
            .transform_vector3(ray.direction)
            .normalize_or_zero();

        if local_direction.length_squared() <= EPSILON {
            return None;
        }

        // Browser plane is local Z = 0.
        if local_direction.z.abs() <= EPSILON {
            return None;
        }

        let t_local = -local_origin.z / local_direction.z;

        if t_local < 0.0 {
            return None;
        }

        let local_hit = local_origin + local_direction * t_local;

        let half = geometry.size * 0.5;

        let u = (local_hit.x + half.x) / geometry.size.x;

        // Browser coordinates start top-left while our local plane
        // convention treats +Y as up.
        let v = 1.0 - ((local_hit.y + half.y) / geometry.size.y);

        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return None;
        }

        let device_x = u as f64 * geometry.viewport.width as f64;

        let device_y = v as f64 * geometry.viewport.height as f64;

        let scale = geometry.viewport.device_scale_factor as f64;

        let css_x = device_x / scale;
        let css_y = device_y / scale;

        // Compute actual world-space distance rather than using
        // transformed local parameter t.
        let local_world_hit = geometry.world_from_local.transform_point3(local_hit);

        let world_distance = local_world_hit.distance(ray.origin);

        Some(MappedPoint {
            uv: Vec2::new(u, v),
            device_px: BrowserPoint::new(device_x, device_y),
            css_px: BrowserPoint::new(css_x, css_y),
            ray_distance: world_distance,
        })
    }

    pub fn pointer_move(
        ray: Ray,
        geometry: &SpatialNodeGeometry,
        modifiers: Modifiers,
    ) -> Option<BrowserInput> {
        let mapped = Self::ray_to_browser(ray, geometry)?;

        Some(BrowserInput::PointerMove {
            position: mapped.device_px,
            modifiers,
        })
    }

    pub fn pointer_button(
        ray: Ray,
        geometry: &SpatialNodeGeometry,
        button: MouseButton,
        state: ButtonState,
        modifiers: Modifiers,
    ) -> Option<BrowserInput> {
        let mapped = Self::ray_to_browser(ray, geometry)?;

        Some(BrowserInput::PointerButton {
            position: mapped.device_px,
            button,
            state,
            modifiers,
        })
    }

    pub fn scroll(
        ray: Ray,
        geometry: &SpatialNodeGeometry,
        delta_x: f64,
        delta_y: f64,
        mode: ScrollMode,
        modifiers: Modifiers,
    ) -> Option<BrowserInput> {
        let mapped = Self::ray_to_browser(ray, geometry)?;

        Some(BrowserInput::Scroll {
            position: mapped.device_px,
            delta_x,
            delta_y,
            mode,
            modifiers,
        })
    }
}
