//! The prototype meshes every solid instance is drawn from.
//!
//! Conventions, and they are load-bearing:
//!
//! * Every mesh lives inside the -1..1 box on all three axes. The vertex
//!   shader scales by half the instance size, so an instance sized `s` spans
//!   exactly `s` world units on that axis.
//! * Faces are wound counter-clockwise seen from outside. Backwards winding
//!   renders the shape inside out under back-face culling, and the mistake is
//!   invisible until you are inside the model.
//! * Anything with an axis points along **+Y** (cone, cylinder, frustum).
//!   The wedge ridges along **Z**. The frond runs root at **-Z** to tip at
//!   **+Z**, which is also the direction its droop falls.
//!
//! Shapes exist so models do not have to be spelled out in boxes. A palm frond
//! drawn as five stacked cuboids costs five instances and still reads as five
//! stacked cuboids; drawn as one `Frond` it costs one and reads as a leaf. The
//! same argument is why `Crenels` is a whole parapet ring rather than a merlon.

use crate::math::*;
use crate::types::*;

/// Sized for the largest prototype (the parapet ring) with room to spare.
/// These arrays are zero-initialised, so they cost nothing in the binary —
/// they live in `.bss` like the rest of the world.
const MESH_VERTEX_CAPACITY: usize = 3072 * 6;
const MESH_INDEX_CAPACITY: usize = 4096;

pub struct MeshLibrary {
    pub vertices: [[f32; MESH_VERTEX_CAPACITY]; Shape::COUNT],
    pub indices: [[u16; MESH_INDEX_CAPACITY]; Shape::COUNT],
    pub vertex_floats: [usize; Shape::COUNT],
    pub index_counts: [usize; Shape::COUNT],
}

impl MeshLibrary {
    pub const fn new() -> MeshLibrary {
        MeshLibrary {
            vertices: [[0.0; MESH_VERTEX_CAPACITY]; Shape::COUNT],
            indices: [[0; MESH_INDEX_CAPACITY]; Shape::COUNT],
            vertex_floats: [0; Shape::COUNT],
            index_counts: [0; Shape::COUNT],
        }
    }

    fn reset(&mut self, shape: usize) {
        self.vertex_floats[shape] = 0;
        self.index_counts[shape] = 0;
    }

    fn vertex_count(&self, shape: usize) -> usize {
        self.vertex_floats[shape] / 6
    }

    fn add_vertex(&mut self, shape: usize, position: [f32; 3], normal: [f32; 3]) {
        let base = self.vertex_floats[shape];
        if base + 6 > MESH_VERTEX_CAPACITY {
            return;
        }
        self.vertices[shape][base] = position[0];
        self.vertices[shape][base + 1] = position[1];
        self.vertices[shape][base + 2] = position[2];
        self.vertices[shape][base + 3] = normal[0];
        self.vertices[shape][base + 4] = normal[1];
        self.vertices[shape][base + 5] = normal[2];
        self.vertex_floats[shape] = base + 6;
    }

    fn add_triangle(&mut self, shape: usize, a: usize, b: usize, c: usize) {
        let base = self.index_counts[shape];
        if base + 3 > MESH_INDEX_CAPACITY {
            return;
        }
        self.indices[shape][base] = a as u16;
        self.indices[shape][base + 1] = b as u16;
        self.indices[shape][base + 2] = c as u16;
        self.index_counts[shape] = base + 3;
    }

    /// A quad with its own flat normal, wound counter-clockwise from outside.
    /// Flat shading is what makes stone read as stone: shared smooth normals
    /// turn every lump into a billiard ball.
    fn add_flat_quad(&mut self, shape: usize, corners: [[f32; 3]; 4]) {
        let normal = face_normal(corners[0], corners[1], corners[2]);
        let first = self.vertex_count(shape);
        for corner in corners.iter() {
            self.add_vertex(shape, *corner, normal);
        }
        self.add_triangle(shape, first, first + 1, first + 2);
        self.add_triangle(shape, first, first + 2, first + 3);
    }

    fn add_flat_triangle(&mut self, shape: usize, corners: [[f32; 3]; 3]) {
        let normal = face_normal(corners[0], corners[1], corners[2]);
        let first = self.vertex_count(shape);
        for corner in corners.iter() {
            self.add_vertex(shape, *corner, normal);
        }
        self.add_triangle(shape, first, first + 1, first + 2);
    }

    /// A quad drawn from both sides, for sheets with no thickness — leaves,
    /// membranes, banners. Single-sided geometry vanishes when you fly under
    /// it, which on a palm crown is most of the time.
    fn add_double_quad(&mut self, shape: usize, corners: [[f32; 3]; 4]) {
        self.add_flat_quad(shape, corners);
        self.add_flat_quad(shape, [corners[3], corners[2], corners[1], corners[0]]);
    }

    pub fn build(&mut self) {
        for shape in 0..Shape::COUNT {
            self.reset(shape);
        }
        self.build_cuboid();
        self.build_sphere();
        self.build_cone();
        self.build_disc();
        self.build_cylinder();
        self.build_frustum();
        self.build_wedge();
        self.build_frond();
        self.build_boulder();
        self.build_crenels();
    }

    fn build_cuboid(&mut self) {
        const SHAPE: usize = 0;
        const FACES: [([f32; 3], [[f32; 3]; 4]); 6] = [
            ([1.0, 0.0, 0.0], [[1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 1.0, 1.0], [1.0, -1.0, 1.0]]),
            ([-1.0, 0.0, 0.0], [[-1.0, -1.0, 1.0], [-1.0, 1.0, 1.0], [-1.0, 1.0, -1.0], [-1.0, -1.0, -1.0]]),
            ([0.0, 1.0, 0.0], [[-1.0, 1.0, -1.0], [-1.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, 1.0, -1.0]]),
            ([0.0, -1.0, 0.0], [[-1.0, -1.0, 1.0], [-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, -1.0, 1.0]]),
            ([0.0, 0.0, 1.0], [[-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, 1.0, 1.0], [-1.0, 1.0, 1.0]]),
            ([0.0, 0.0, -1.0], [[-1.0, 1.0, -1.0], [1.0, 1.0, -1.0], [1.0, -1.0, -1.0], [-1.0, -1.0, -1.0]]),
        ];
        for (normal, quad) in FACES.iter() {
            let first = self.vertex_count(SHAPE);
            for corner in quad.iter() {
                self.add_vertex(SHAPE, *corner, *normal);
            }
            self.add_triangle(SHAPE, first, first + 1, first + 2);
            self.add_triangle(SHAPE, first, first + 2, first + 3);
        }
    }

    fn build_sphere(&mut self) {
        const SHAPE: usize = 1;
        const SEGMENTS: usize = 14;
        const RINGS: usize = 9;
        for ring in 0..=RINGS {
            let polar = ring as f32 / RINGS as f32 * core::f32::consts::PI;
            for segment in 0..=SEGMENTS {
                let azimuth = segment as f32 / SEGMENTS as f32 * core::f32::consts::TAU;
                let point = [
                    sin(polar) * cos(azimuth),
                    cos(polar),
                    sin(polar) * sin(azimuth),
                ];
                // on a unit sphere the position is also the normal
                self.add_vertex(SHAPE, point, point);
            }
        }
        for ring in 0..RINGS {
            for segment in 0..SEGMENTS {
                let upper = ring * (SEGMENTS + 1) + segment;
                let lower = upper + SEGMENTS + 1;
                self.add_triangle(SHAPE, upper, upper + 1, lower);
                self.add_triangle(SHAPE, upper + 1, lower + 1, lower);
            }
        }
    }

    fn build_cone(&mut self) {
        const SHAPE: usize = 2;
        const SEGMENTS: usize = 14;
        /// Tilts the side normals up so the cone lights like a cone rather
        /// than a cylinder.
        const SIDE_NORMAL_LIFT: f32 = 0.45;
        for segment in 0..SEGMENTS {
            let start = segment as f32 / SEGMENTS as f32 * core::f32::consts::TAU;
            let end = (segment + 1) as f32 / SEGMENTS as f32 * core::f32::consts::TAU;
            let mid = (start + end) * 0.5;
            let normal_length = sqrt(
                cos(mid) * cos(mid) + SIDE_NORMAL_LIFT * SIDE_NORMAL_LIFT + sin(mid) * sin(mid),
            );
            let side_normal = [
                cos(mid) / normal_length,
                SIDE_NORMAL_LIFT / normal_length,
                sin(mid) / normal_length,
            ];
            let rim_start = [cos(start), -1.0, sin(start)];
            let rim_end = [cos(end), -1.0, sin(end)];

            let first = self.vertex_count(SHAPE);
            self.add_vertex(SHAPE, rim_start, side_normal);
            self.add_vertex(SHAPE, rim_end, side_normal);
            self.add_vertex(SHAPE, [0.0, 1.0, 0.0], side_normal);
            self.add_triangle(SHAPE, first, first + 2, first + 1);

            let cap = self.vertex_count(SHAPE);
            let down = [0.0, -1.0, 0.0];
            self.add_vertex(SHAPE, [0.0, -1.0, 0.0], down);
            self.add_vertex(SHAPE, rim_end, down);
            self.add_vertex(SHAPE, rim_start, down);
            self.add_triangle(SHAPE, cap, cap + 2, cap + 1);
        }
    }

    /// A flat unit disc lying in the XZ plane, used only for ground shadows.
    /// Its own y extent is zero: the shadow pass lifts it clear of the terrain
    /// with a bias rather than by scaling.
    fn build_disc(&mut self) {
        const SHAPE: usize = 3;
        const SEGMENTS: usize = 16;
        const UP: [f32; 3] = [0.0, 1.0, 0.0];
        let centre = self.vertex_count(SHAPE);
        self.add_vertex(SHAPE, [0.0, 0.0, 0.0], UP);
        for segment in 0..=SEGMENTS {
            let angle = segment as f32 / SEGMENTS as f32 * core::f32::consts::TAU;
            self.add_vertex(SHAPE, [cos(angle), 0.0, sin(angle)], UP);
        }
        for segment in 0..SEGMENTS {
            self.add_triangle(SHAPE, centre, centre + 2 + segment, centre + 1 + segment);
        }
    }

    /// Round shaft along Y. Smooth radial normals on the sides so it reads as
    /// a drum, flat normals on the caps so the rim stays crisp.
    fn build_cylinder(&mut self) {
        const SHAPE: usize = 4;
        const SEGMENTS: usize = 16;
        for segment in 0..=SEGMENTS {
            let angle = segment as f32 / SEGMENTS as f32 * core::f32::consts::TAU;
            let radial = [cos(angle), 0.0, sin(angle)];
            self.add_vertex(SHAPE, [radial[0], -1.0, radial[2]], radial);
            self.add_vertex(SHAPE, [radial[0], 1.0, radial[2]], radial);
        }
        for segment in 0..SEGMENTS {
            let low = segment * 2;
            self.add_triangle(SHAPE, low, low + 1, low + 2);
            self.add_triangle(SHAPE, low + 1, low + 3, low + 2);
        }
        for (height, normal, downward) in [
            (1.0f32, [0.0f32, 1.0, 0.0], false),
            (-1.0, [0.0, -1.0, 0.0], true),
        ] {
            let centre = self.vertex_count(SHAPE);
            self.add_vertex(SHAPE, [0.0, height, 0.0], normal);
            for segment in 0..=SEGMENTS {
                let angle = segment as f32 / SEGMENTS as f32 * core::f32::consts::TAU;
                self.add_vertex(SHAPE, [cos(angle), height, sin(angle)], normal);
            }
            for segment in 0..SEGMENTS {
                if downward {
                    self.add_triangle(SHAPE, centre, centre + 1 + segment, centre + 2 + segment);
                } else {
                    self.add_triangle(SHAPE, centre, centre + 2 + segment, centre + 1 + segment);
                }
            }
        }
    }

    /// A box that tapers to 45% at the top. Tower shafts, thighs, tree trunks,
    /// chimneys — anything a plain cuboid makes look like a packing crate.
    fn build_frustum(&mut self) {
        const SHAPE: usize = 5;
        const TOP: f32 = 0.45;
        let base = [
            [-1.0f32, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, -1.0, 1.0],
            [-1.0, -1.0, 1.0],
        ];
        let top = [
            [-TOP, 1.0, -TOP],
            [TOP, 1.0, -TOP],
            [TOP, 1.0, TOP],
            [-TOP, 1.0, TOP],
        ];
        for corner in 0..4 {
            let next = (corner + 1) % 4;
            self.add_flat_quad(SHAPE, [base[corner], base[next], top[next], top[corner]]);
        }
        self.add_flat_quad(SHAPE, [top[0], top[1], top[2], top[3]]);
        self.add_flat_quad(SHAPE, [base[3], base[2], base[1], base[0]]);
    }

    /// Triangular prism, ridged along Z at the top. Roofs, blades, buttresses,
    /// dorsal fins, ramps.
    fn build_wedge(&mut self) {
        const SHAPE: usize = 6;
        let corner = [
            [-1.0f32, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, -1.0, 1.0],
            [-1.0, -1.0, 1.0],
        ];
        let ridge_back = [0.0f32, 1.0, -1.0];
        let ridge_front = [0.0f32, 1.0, 1.0];
        self.add_flat_quad(SHAPE, [corner[3], corner[0], ridge_back, ridge_front]);
        self.add_flat_quad(SHAPE, [corner[1], corner[2], ridge_front, ridge_back]);
        self.add_flat_triangle(SHAPE, [corner[0], corner[1], ridge_back]);
        self.add_flat_triangle(SHAPE, [corner[2], corner[3], ridge_front]);
        self.add_flat_quad(SHAPE, [corner[3], corner[2], corner[1], corner[0]]);
    }

    /// A palm leaf. Root at -Z, tip at +Z; it narrows, droops under its own
    /// weight and carries a raised central rib, and its edges are cut into
    /// leaflets offset by one between the two sides so the halves never
    /// mirror. Drawn from both faces — a crown is seen from below as often as
    /// from above.
    fn build_frond(&mut self) {
        const SHAPE: usize = 7;
        const STATIONS: usize = 12;
        /// How far the tip falls below the root.
        const DROOP: f32 = 1.35;
        /// Height of the rib above the blade, at the root.
        const RIB: f32 = 0.16;
        /// Depth of the cuts between leaflets, as a fraction of half-width.
        const LEAFLET_CUT: f32 = 0.34;

        let station = |index: usize| -> ([f32; 3], [f32; 3], [f32; 3]) {
            let along = index as f32 / STATIONS as f32;
            let z = -1.0 + 2.0 * along;
            // shallow at the root, falling away toward the tip
            let blade_y = 0.28 - DROOP * along * along;
            // widest a fifth of the way out, tapering to a point
            let taper = sin((0.18 + along * 0.82) * core::f32::consts::PI);
            let left_cut = if index % 2 == 0 { 1.0 } else { 1.0 - LEAFLET_CUT };
            let right_cut = if index % 2 == 1 { 1.0 } else { 1.0 - LEAFLET_CUT };
            let half = taper * (1.0 - along * 0.25);
            (
                [-half * left_cut, blade_y, z],
                [0.0, blade_y + RIB * (1.0 - along), z],
                [half * right_cut, blade_y, z],
            )
        };

        for index in 0..STATIONS {
            let (left_near, spine_near, right_near) = station(index);
            let (left_far, spine_far, right_far) = station(index + 1);
            self.add_double_quad(SHAPE, [left_near, spine_near, spine_far, left_far]);
            self.add_double_quad(SHAPE, [spine_near, right_near, right_far, spine_far]);
        }
    }

    /// An irregular lump: a coarse sphere pushed in and out by a fixed hash,
    /// then flat-shaded. Every facet catches the light separately, which is
    /// the difference between a rock and an egg.
    fn build_boulder(&mut self) {
        const SHAPE: usize = 8;
        const SEGMENTS: usize = 10;
        const RINGS: usize = 7;
        /// How far a vertex may move in along its own radius.
        const RELIEF: f32 = 0.26;

        // The wrap column has to land on the same radius as the first, or the
        // mesh splits open along one meridian.
        let radius_at = |ring: usize, segment: usize| -> f32 {
            let wrapped = (segment % SEGMENTS) as u32;
            let mut hash = (ring as u32)
                .wrapping_mul(374_761_393)
                .wrapping_add(wrapped.wrapping_mul(668_265_263));
            hash = (hash ^ (hash >> 13)).wrapping_mul(1_274_126_177);
            hash ^= hash >> 16;
            1.0 - RELIEF * ((hash & 0xffff) as f32 / 65535.0)
        };
        let point = |ring: usize, segment: usize| -> [f32; 3] {
            let polar = ring as f32 / RINGS as f32 * core::f32::consts::PI;
            let azimuth = segment as f32 / SEGMENTS as f32 * core::f32::consts::TAU;
            let radius = radius_at(ring, segment);
            [
                sin(polar) * cos(azimuth) * radius,
                cos(polar) * radius,
                sin(polar) * sin(azimuth) * radius,
            ]
        };

        for ring in 0..RINGS {
            for segment in 0..SEGMENTS {
                let near_top = point(ring, segment);
                let far_top = point(ring, segment + 1);
                let far_bottom = point(ring + 1, segment + 1);
                let near_bottom = point(ring + 1, segment);
                if ring == 0 {
                    self.add_flat_triangle(SHAPE, [near_top, far_bottom, near_bottom]);
                } else if ring == RINGS - 1 {
                    self.add_flat_triangle(SHAPE, [near_top, far_top, near_bottom]);
                } else {
                    self.add_flat_quad(SHAPE, [near_top, far_top, far_bottom, near_bottom]);
                }
            }
        }
    }

    /// A whole crenellated parapet ring: outer face, inner face, walkway lip
    /// and merlons on alternate segments. One instance is an entire
    /// battlement, which is the only reason a keep can afford several.
    fn build_crenels(&mut self) {
        const SHAPE: usize = 9;
        const SEGMENTS: usize = 16;
        const INNER: f32 = 0.74;
        /// Top of the solid parapet; merlons rise from here to +1.
        const SILL: f32 = 0.1;

        let ring = |radius: f32, segment: usize, height: f32| -> [f32; 3] {
            let angle = segment as f32 / SEGMENTS as f32 * core::f32::consts::TAU;
            [cos(angle) * radius, height, sin(angle) * radius]
        };

        for segment in 0..SEGMENTS {
            let next = segment + 1;
            let merlon = segment % 2 == 0;
            let top = if merlon { 1.0 } else { SILL };

            let outer_low_near = ring(1.0, segment, -1.0);
            let outer_low_far = ring(1.0, next, -1.0);
            let outer_high_near = ring(1.0, segment, top);
            let outer_high_far = ring(1.0, next, top);
            let inner_low_near = ring(INNER, segment, -1.0);
            let inner_low_far = ring(INNER, next, -1.0);
            let inner_high_near = ring(INNER, segment, top);
            let inner_high_far = ring(INNER, next, top);

            self.add_flat_quad(
                SHAPE,
                [outer_low_near, outer_low_far, outer_high_far, outer_high_near],
            );
            self.add_flat_quad(
                SHAPE,
                [inner_low_far, inner_low_near, inner_high_near, inner_high_far],
            );
            self.add_flat_quad(
                SHAPE,
                [outer_high_near, outer_high_far, inner_high_far, inner_high_near],
            );

            // the two cheeks of a merlon, facing along the walkway
            if merlon {
                let sill_outer_near = ring(1.0, segment, SILL);
                let sill_inner_near = ring(INNER, segment, SILL);
                let sill_outer_far = ring(1.0, next, SILL);
                let sill_inner_far = ring(INNER, next, SILL);
                self.add_flat_quad(
                    SHAPE,
                    [sill_outer_near, sill_inner_near, inner_high_near, outer_high_near],
                );
                self.add_flat_quad(
                    SHAPE,
                    [sill_inner_far, sill_outer_far, outer_high_far, inner_high_far],
                );
            }
        }
    }
}

/// Outward normal of a face from three of its corners. Degenerate faces fall
/// back to up rather than to NaN.
fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let edge_a = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let edge_b = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [
        edge_a[1] * edge_b[2] - edge_a[2] * edge_b[1],
        edge_a[2] * edge_b[0] - edge_a[0] * edge_b[2],
        edge_a[0] * edge_b[1] - edge_a[1] * edge_b[0],
    ];
    let length = length3(cross[0], cross[1], cross[2]);
    if length < 0.000_001 {
        return [0.0, 1.0, 0.0];
    }
    [cross[0] / length, cross[1] / length, cross[2] / length]
}
