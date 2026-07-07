//! HD-2D orbit camera: looks down at the map at a diorama-like pitch.

use glam::{Mat4, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct OrbitCamera {
    pub target: Vec3,
    /// Radians around Y. 0 = looking toward -Z (north edge up on screen).
    pub yaw: f32,
    /// Radians above the horizon (positive = looking down).
    pub pitch: f32,
    pub dist: f32,
    pub fov_y: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        OrbitCamera {
            target: Vec3::ZERO,
            yaw: 0.0,
            pitch: 50f32.to_radians(),
            dist: 16.0,
            fov_y: 38f32.to_radians(),
        }
    }
}

impl OrbitCamera {
    pub fn eye(&self) -> Vec3 {
        let offset = Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        ) * self.dist;
        self.target + offset
    }

    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_at_mat4(self.eye(), self.target, Vec3::Y)
    }

    pub fn proj(&self, aspect: f32) -> Mat4 {
        // wgpu clip space: Z in [0, 1], Y-up.
        glam::camera::rh::proj::directx::perspective(self.fov_y, aspect.max(0.01), 0.1, 400.0)
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    /// World-space right direction for upright billboards (XZ plane).
    pub fn billboard_right(&self) -> Vec3 {
        Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin())
    }

    /// Ray through a point given in normalized device coords (-1..1, y up).
    pub fn screen_ray(&self, ndc_x: f32, ndc_y: f32, aspect: f32) -> (Vec3, Vec3) {
        let inv = self.view_proj(aspect).inverse();
        let near = inv.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
        let far = inv.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        (near, (far - near).normalize())
    }
}
