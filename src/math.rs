//! Scalar helpers and 4x4 matrices.
//!
//! `core` gives us arithmetic but no transcendentals, so those come from
//! `libm`. Everything here is free of world state.

/// Matrices are column-major: element (row, col) is `m[row + col * 4]`.
pub type Mat4 = [f32; 16];

#[inline(always)]
pub fn sin(x: f32) -> f32 {
    libm::sinf(x)
}
#[inline(always)]
pub fn cos(x: f32) -> f32 {
    libm::cosf(x)
}
#[inline(always)]
pub fn tan(x: f32) -> f32 {
    libm::tanf(x)
}
#[inline(always)]
pub fn sqrt(x: f32) -> f32 {
    libm::sqrtf(x)
}
#[inline(always)]
pub fn floor(x: f32) -> f32 {
    libm::floorf(x)
}
#[inline(always)]
pub fn ceil(x: f32) -> f32 {
    libm::ceilf(x)
}
#[inline(always)]
pub fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

#[inline(always)]
pub fn min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}
#[inline(always)]
pub fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}
#[inline(always)]
pub fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    min(max(v, lo), hi)
}

/// Length of a 3-vector.
#[inline]
pub fn length3(x: f32, y: f32, z: f32) -> f32 {
    sqrt(x * x + y * y + z * z)
}
/// Squared length, for comparisons that do not need the square root.
#[inline]
pub fn length_sq3(x: f32, y: f32, z: f32) -> f32 {
    x * x + y * y + z * z
}
/// Squared horizontal length, ignoring height.
#[inline]
pub fn length_sq2(x: f32, z: f32) -> f32 {
    x * x + z * z
}

/// Hermite fade, the same curve `smoothstep` uses once the edges are removed.
#[inline]
pub fn smooth_fade(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Frame-rate independent approach: moves `current` a proportion of the way to
/// `target`, where `rate` is roughly "how many times per second to halve the
/// gap". Clamped so a long frame can never overshoot.
#[inline]
pub fn approach(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    current + (target - current) * min(1.0, dt * rate)
}

/// Perspective projection with a 0..1 depth range (the WebGPU/D3D convention,
/// not OpenGL's -1..1).
pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let focal = 1.0 / tan(fov_y * 0.5);
    let depth_scale = 1.0 / (near - far);
    [
        focal / aspect, 0.0, 0.0, 0.0,
        0.0, focal, 0.0, 0.0,
        0.0, 0.0, far * depth_scale, -1.0,
        0.0, 0.0, far * near * depth_scale, 0.0,
    ]
}

/// Right-handed view matrix looking from `eye` toward `centre`.
pub fn look_at(eye: [f32; 3], centre: [f32; 3], up: [f32; 3]) -> Mat4 {
    let mut back = [eye[0] - centre[0], eye[1] - centre[1], eye[2] - centre[2]];
    normalise_or_identity(&mut back);
    let mut right = [
        up[1] * back[2] - up[2] * back[1],
        up[2] * back[0] - up[0] * back[2],
        up[0] * back[1] - up[1] * back[0],
    ];
    normalise_or_identity(&mut right);
    let true_up = [
        back[1] * right[2] - back[2] * right[1],
        back[2] * right[0] - back[0] * right[2],
        back[0] * right[1] - back[1] * right[0],
    ];
    [
        right[0], true_up[0], back[0], 0.0,
        right[1], true_up[1], back[1], 0.0,
        right[2], true_up[2], back[2], 0.0,
        -(right[0] * eye[0] + right[1] * eye[1] + right[2] * eye[2]),
        -(true_up[0] * eye[0] + true_up[1] * eye[1] + true_up[2] * eye[2]),
        -(back[0] * eye[0] + back[1] * eye[1] + back[2] * eye[2]),
        1.0,
    ]
}

fn normalise_or_identity(v: &mut [f32; 3]) {
    let len = length3(v[0], v[1], v[2]);
    let len = if len == 0.0 { 1.0 } else { len };
    v[0] /= len;
    v[1] /= len;
    v[2] /= len;
}

pub fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0f32;
            for k in 0..4 {
                sum += a[row + k * 4] * b[k + col * 4];
            }
            out[row + col * 4] = sum;
        }
    }
    out
}

/// Full 4x4 inverse by cofactor expansion. Returns all zeros for a singular
/// matrix, which the shader treats as "no camera" rather than producing NaNs.
pub fn invert(m: &Mat4) -> Mat4 {
    let (a00, a01, a02, a03) = (m[0], m[1], m[2], m[3]);
    let (a10, a11, a12, a13) = (m[4], m[5], m[6], m[7]);
    let (a20, a21, a22, a23) = (m[8], m[9], m[10], m[11]);
    let (a30, a31, a32, a33) = (m[12], m[13], m[14], m[15]);

    let s0 = a00 * a11 - a01 * a10;
    let s1 = a00 * a12 - a02 * a10;
    let s2 = a00 * a13 - a03 * a10;
    let s3 = a01 * a12 - a02 * a11;
    let s4 = a01 * a13 - a03 * a11;
    let s5 = a02 * a13 - a03 * a12;
    let c5 = a20 * a31 - a21 * a30;
    let c4 = a20 * a32 - a22 * a30;
    let c3 = a20 * a33 - a23 * a30;
    let c2 = a21 * a32 - a22 * a31;
    let c1 = a21 * a33 - a23 * a31;
    let c0 = a22 * a33 - a23 * a32;

    let determinant = s0 * c0 - s1 * c1 + s2 * c2 + s3 * c3 - s4 * c4 + s5 * c5;
    if determinant == 0.0 {
        return [0.0; 16];
    }
    let inv_det = 1.0 / determinant;
    [
        (a11 * c0 - a12 * c1 + a13 * c2) * inv_det,
        (a02 * c1 - a01 * c0 - a03 * c2) * inv_det,
        (a31 * s5 - a32 * s4 + a33 * s3) * inv_det,
        (a22 * s4 - a21 * s5 - a23 * s3) * inv_det,
        (a12 * c3 - a10 * c0 - a13 * c4) * inv_det,
        (a00 * c0 - a02 * c3 + a03 * c4) * inv_det,
        (a32 * s2 - a30 * s5 - a33 * s1) * inv_det,
        (a20 * s5 - a22 * s2 + a23 * s1) * inv_det,
        (a10 * c1 - a11 * c3 + a13 * c5) * inv_det,
        (a01 * c3 - a00 * c1 - a03 * c5) * inv_det,
        (a30 * s4 - a31 * s2 + a33 * s0) * inv_det,
        (a21 * s2 - a20 * s4 - a23 * s0) * inv_det,
        (a11 * c4 - a10 * c2 - a12 * c5) * inv_det,
        (a00 * c2 - a01 * c4 + a02 * c5) * inv_det,
        (a31 * s1 - a30 * s3 - a32 * s0) * inv_det,
        (a20 * s3 - a21 * s1 + a22 * s0) * inv_det,
    ]
}
