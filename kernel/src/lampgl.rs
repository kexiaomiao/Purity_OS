//! LampGL — PurityOS built-in 3D renderer.
//!
//! A CPU software rasterizer: matrix math, perspective projection,
//! triangle rasterization with a z-buffer. No GPU — everything runs on
//! the CPU and writes directly into the framebuffer.
//!
//! Named "LampGL" because it lights the screen one pixel at a time.

use crate::gui::fb::{self, Fb, Color};
use micromath::F32Ext;

/// 3-component vector.
#[derive(Clone, Copy, Debug)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Vec3 { x, y, z }
    }
    pub fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn norm(self) -> Vec3 {
        let l = (self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if l < 1e-9 {
            return self;
        }
        Vec3::new(self.x / l, self.y / l, self.z / l)
    }
}

/// 4x4 column-major matrix (OpenGL convention).
#[derive(Clone, Copy)]
pub struct Mat4 {
    pub m: [f32; 16],
}

impl Mat4 {
    pub fn identity() -> Self {
        let mut m = [0f32; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[15] = 1.0;
        Mat4 { m }
    }

    pub fn mul(&self, o: &Mat4) -> Mat4 {
        let mut r = [0f32; 16];
        for c in 0..4 {
            for row in 0..4 {
                let mut s = 0.0;
                for k in 0..4 {
                    s += self.m[k * 4 + row] * o.m[c * 4 + k];
                }
                r[c * 4 + row] = s;
            }
        }
        Mat4 { m: r }
    }

    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
        let f = 1.0 / (fov_y / 2.0).tan();
        let mut m = [0f32; 16];
        m[0] = f / aspect;
        m[5] = f;
        m[10] = (far + near) / (near - far);
        m[11] = -1.0;
        m[14] = (2.0 * far * near) / (near - far);
        Mat4 { m }
    }

    pub fn rotate_x(a: f32) -> Mat4 {
        let mut m = Self::identity();
        m.m[5] = a.cos();
        m.m[6] = a.sin();
        m.m[9] = -a.sin();
        m.m[10] = a.cos();
        m
    }
    pub fn rotate_y(a: f32) -> Mat4 {
        let mut m = Self::identity();
        m.m[0] = a.cos();
        m.m[2] = -a.sin();
        m.m[8] = a.sin();
        m.m[10] = a.cos();
        m
    }
    pub fn rotate_z(a: f32) -> Mat4 {
        let mut m = Self::identity();
        m.m[0] = a.cos();
        m.m[1] = a.sin();
        m.m[4] = -a.sin();
        m.m[5] = a.cos();
        m
    }
    pub fn translate(x: f32, y: f32, z: f32) -> Mat4 {
        let mut m = Self::identity();
        m.m[12] = x;
        m.m[13] = y;
        m.m[14] = z;
        m
    }

    /// Transform a homogeneous point.
    pub fn apply(&self, v: Vec3) -> (f32, f32, f32, f32) {
        let x = v.x;
        let y = v.y;
        let z = v.z;
        let wx = self.m[0] * x + self.m[4] * y + self.m[8] * z + self.m[12];
        let wy = self.m[1] * x + self.m[5] * y + self.m[9] * z + self.m[13];
        let wz = self.m[2] * x + self.m[6] * y + self.m[10] * z + self.m[14];
        let w = self.m[3] * x + self.m[7] * y + self.m[11] * z + self.m[15];
        (wx, wy, wz, w)
    }
}

/// A camera positioned at (0,0,3) looking at origin.
pub fn view_matrix() -> Mat4 {
    Mat4::translate(0.0, 0.0, 3.0)
}

/// Renderer context: destination rect inside the framebuffer.
/// The z-buffer is a shared, reused global so we don't allocate ~170 KiB
/// every frame (which would OOM the 256 KiB kernel heap).
pub struct Renderer {
    pub ox: usize,
    pub oy: usize,
    pub w: usize,
    pub h: usize,
}

static ZBUF: spin::Mutex<alloc::vec::Vec<f32>> =
    spin::Mutex::new(alloc::vec::Vec::new());

impl Renderer {
    pub fn new(ox: usize, oy: usize, w: usize, h: usize) -> Self {
        let mut z = ZBUF.lock();
        z.resize(w * h, f32::INFINITY);
        Renderer { ox, oy, w, h }
    }

    pub fn clear(&mut self, f: &mut Fb, color: Color) {
        f.fill_rect(self.ox, self.oy, self.w, self.h, color);
        let mut z = ZBUF.lock();
        for z in z.iter_mut() {
            *z = f32::INFINITY;
        }
    }

    /// Project a world-space point into window pixel coordinates.
    fn project(&self, mvp: &Mat4, v: Vec3) -> Option<(usize, usize, f32)> {
        let (x, y, z, w) = mvp.apply(v);
        if w <= 1e-6 {
            return None;
        }
        let nx = x / w;
        let ny = y / w;
        let nz = z / w;
        if nx < -1.0 || nx > 1.0 || ny < -1.0 || ny > 1.0 {
            return None;
        }
        let px = ((nx * 0.5 + 0.5) * self.w as f32) as usize + self.ox;
        let py = ((1.0 - (ny * 0.5 + 0.5)) * self.h as f32) as usize + self.oy;
        Some((px, py, nz))
    }

    /// Draw a line between two projected world points.
    pub fn line(&mut self, f: &mut Fb, mvp: &Mat4, a: Vec3, b: Vec3, color: Color) {
        let (Some(p0), Some(p1)) = (self.project(mvp, a), self.project(mvp, b)) else {
            return;
        };
        let (mut x0, mut y0, _) = p0;
        let (x1, y1, _) = p1;
        let dx = (x1 as isize - x0 as isize).abs();
        let dy = -(y1 as isize - y0 as isize).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            f.pixel(x0, y0, color);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 = (x0 as isize + sx) as usize;
            }
            if e2 <= dx {
                err += dx;
                y0 = (y0 as isize + sy) as usize;
            }
        }
    }

    /// Draw a filled triangle (flat shaded) with z-buffer.
    pub fn triangle(&mut self, f: &mut Fb, mvp: &Mat4, a: Vec3, b: Vec3, c: Vec3, color: Color) {
        let (Some(p0), Some(p1), Some(p2)) = (
            self.project(mvp, a),
            self.project(mvp, b),
            self.project(mvp, c),
        ) else {
            return;
        };
        // Bounding box.
        let minx = p0.0.min(p1.0).min(p2.0).max(self.ox);
        let maxx = p0.0.max(p1.0).max(p2.0).min(self.ox + self.w - 1);
        let miny = p0.1.min(p1.1).min(p2.1).max(self.oy);
        let maxy = p0.1.max(p1.1).max(p2.1).min(self.oy + self.h - 1);
        let area = edge3(p0, p1, p2);
        if area.abs() < 1e-6 {
            return;
        }
        // Take the z-buffer lock ONCE for the whole triangle rather than
        // per-pixel (a cube face covers thousands of pixels).
        let mut zbuf = ZBUF.lock();
        for py in miny..=maxy {
            for px in minx..=maxx {
                let w0 = edge2(p1, p2, (px, py)) / area;
                let w1 = edge2(p2, p0, (px, py)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * p0.2 + w1 * p1.2 + w2 * p2.2;
                let idx = (py - self.oy) * self.w + (px - self.ox);
                if z < zbuf[idx] {
                    zbuf[idx] = z;
                    f.pixel(px, py, color);
                }
            }
        }
    }
}

/// 2D edge function (full 3-vertex, for area).
fn edge3(a: (usize, usize, f32), b: (usize, usize, f32), c: (usize, usize, f32)) -> f32 {
    ((b.0 as f32 - a.0 as f32) * (c.1 as f32 - a.1 as f32))
        - ((b.1 as f32 - a.1 as f32) * (c.0 as f32 - a.0 as f32))
}

/// 2D edge function (point variant, for barycentric weights).
fn edge2(a: (usize, usize, f32), b: (usize, usize, f32), c: (usize, usize)) -> f32 {
    ((b.0 as f32 - a.0 as f32) * (c.1 as f32 - a.1 as f32))
        - ((b.1 as f32 - a.1 as f32) * (c.0 as f32 - a.0 as f32))
}

// ---------------------------------------------------------------------------
// Built-in demo: rotating cube.
// ---------------------------------------------------------------------------

/// 8 cube vertices centered at origin.
pub fn cube_vertices() -> [Vec3; 8] {
    [
        Vec3::new(-1.0, -1.0, -1.0),
        Vec3::new(1.0, -1.0, -1.0),
        Vec3::new(1.0, 1.0, -1.0),
        Vec3::new(-1.0, 1.0, -1.0),
        Vec3::new(-1.0, -1.0, 1.0),
        Vec3::new(1.0, -1.0, 1.0),
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new(-1.0, 1.0, 1.0),
    ]
}

/// 12 cube edges (pairs of vertex indices).
pub const CUBE_EDGES: [(usize, usize); 12] = [
    (0, 1), (1, 2), (2, 3), (3, 0),
    (4, 5), (5, 6), (6, 7), (7, 4),
    (0, 4), (1, 5), (2, 6), (3, 7),
];

/// 6 cube faces as [a, b, c, d] vertex indices.
pub const CUBE_FACES: [[usize; 4]; 6] = [
    [0, 1, 2, 3], // back
    [5, 4, 7, 6], // front
    [1, 5, 6, 2], // right
    [4, 0, 3, 7], // left
    [3, 2, 6, 7], // top
    [4, 5, 1, 0], // bottom
];

/// Render a rotating cube into the given rect. `angle` in radians.
/// Internal render resolution is fixed at 240x180 to stay within the
/// 256 KiB kernel heap; the framebuffer is drawn directly at that size
/// centered in the window rect.
pub fn draw_cube(f: &mut Fb, ox: usize, oy: usize, w: usize, h: usize, angle: f32) {
    const RW: usize = 240;
    const RH: usize = 180;
    // Guard against tiny windows (avoid unsigned underflow).
    if w < RW || h < RH {
        f.fill_rect(ox, oy, w, h, 0x0a0e1a);
        f.text(ox + 8, oy + 8, "window too small", fb::GRAY, None);
        return;
    }
    let cx = ox + (w - RW) / 2;
    let cy = oy + (h - RH) / 2;
    let mut r = Renderer::new(cx, cy, RW, RH);
    r.clear(f, 0x0a0e1a);
    let proj = Mat4::perspective(60.0f32.to_radians(), RW as f32 / RH as f32, 0.1, 100.0);
    let view = view_matrix();
    let rot = Mat4::rotate_y(angle).mul(&Mat4::rotate_x(angle * 0.7));
    let mvp = proj.mul(&view).mul(&rot);
    let verts = cube_vertices();
    let light = Vec3::new(0.3, 0.5, -0.8).norm();
    for face in CUBE_FACES.iter() {
        let a = verts[face[0]];
        let b = verts[face[1]];
        let c = verts[face[2]];
        let n = b.sub(a).cross(c.sub(a)).norm();
        let diff = (n.x * light.x + n.y * light.y + n.z * light.z).abs();
        let shade = (diff * 200.0) as u8;
        let color = ((shade as u32) << 16) | ((shade as u32) << 8) | ((shade as u32) + 40);
        r.triangle(f, &mvp, a, b, c, color);
        r.triangle(f, &mvp, c, verts[face[3]], a, color);
    }
    for (i, j) in CUBE_EDGES.iter() {
        r.line(f, &mvp, verts[*i], verts[*j], fb::WHITE);
    }
}
