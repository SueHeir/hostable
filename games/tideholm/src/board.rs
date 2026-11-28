//! The shape of the archipelago: 19 hexes in a radius-2 hex, their corners
//! (vertices, where huts and lodges go) and sides (edges, where causeways go).
//!
//! The layout never changes between games, only which tile and number token
//! sits on each hex, so the topology is computed once and shared.

use std::sync::OnceLock;

pub const HEX_COUNT: usize = 19;
pub const LETTERS: [char; HEX_COUNT] = [
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S',
];
/// Circumradius of a hex in scene units.
pub const SIZE: f32 = 56.0;
pub const SCENE_W: f32 = 620.0;
pub const SCENE_H: f32 = 580.0;

const VERTEX_DIRS: [&str; 6] = ["N", "NE", "SE", "S", "SW", "NW"];
const SIDE_DIRS: [&str; 6] = ["NE", "E", "SE", "SW", "W", "NW"];
/// Corner offsets of a pointy-top hex in half-units: x in sqrt(3)/2, y in 1/2.
const OFFSETS: [(i32, i32); 6] = [(0, -2), (1, -1), (1, 1), (0, 2), (-1, 1), (-1, -1)];

pub struct Topology {
    /// Axial coordinates `(q, r)` of each hex, rows top to bottom.
    pub hexes: Vec<(i32, i32)>,
    pub hex_verts: Vec<[usize; 6]>,
    pub hex_edges: Vec<[usize; 6]>,
    /// Hexes sharing a side.
    pub hex_neighbors: Vec<Vec<usize>>,
    /// Vertex positions in half-units.
    pub verts: Vec<(i32, i32)>,
    pub vert_hexes: Vec<Vec<usize>>,
    pub vert_edges: Vec<Vec<usize>>,
    pub vert_neighbors: Vec<Vec<usize>>,
    pub edges: Vec<(usize, usize)>,
    pub edge_hexes: Vec<Vec<usize>>,
    pub vert_labels: Vec<String>,
    pub edge_labels: Vec<String>,
}

pub fn topo() -> &'static Topology {
    static TOPO: OnceLock<Topology> = OnceLock::new();
    TOPO.get_or_init(build)
}

fn build() -> Topology {
    let mut hexes = Vec::new();
    for r in -2i32..=2 {
        for q in -2i32..=2 {
            if (q + r).abs() <= 2 {
                hexes.push((q, r));
            }
        }
    }
    let mut verts: Vec<(i32, i32)> = Vec::new();
    let mut vert_hexes: Vec<Vec<usize>> = Vec::new();
    let mut vert_first: Vec<(usize, usize)> = Vec::new();
    let mut hex_verts = vec![[0usize; 6]; hexes.len()];
    for (h, &(q, r)) in hexes.iter().enumerate() {
        for (d, (dx, dy)) in OFFSETS.iter().enumerate() {
            let key = (2 * q + r + dx, 3 * r + dy);
            let idx = match verts.iter().position(|v| *v == key) {
                Some(i) => i,
                None => {
                    verts.push(key);
                    vert_hexes.push(Vec::new());
                    vert_first.push((h, d));
                    verts.len() - 1
                }
            };
            hex_verts[h][d] = idx;
            vert_hexes[idx].push(h);
        }
    }
    let mut edges: Vec<(usize, usize)> = Vec::new();
    let mut edge_hexes: Vec<Vec<usize>> = Vec::new();
    let mut edge_first: Vec<(usize, usize)> = Vec::new();
    let mut hex_edges = vec![[0usize; 6]; hexes.len()];
    for h in 0..hexes.len() {
        for d in 0..6 {
            let a = hex_verts[h][d];
            let b = hex_verts[h][(d + 1) % 6];
            let key = (a.min(b), a.max(b));
            let idx = match edges.iter().position(|e| *e == key) {
                Some(i) => i,
                None => {
                    edges.push(key);
                    edge_hexes.push(Vec::new());
                    edge_first.push((h, d));
                    edges.len() - 1
                }
            };
            hex_edges[h][d] = idx;
            edge_hexes[idx].push(h);
        }
    }
    let mut vert_edges = vec![Vec::new(); verts.len()];
    let mut vert_neighbors = vec![Vec::new(); verts.len()];
    for (e, &(a, b)) in edges.iter().enumerate() {
        vert_edges[a].push(e);
        vert_edges[b].push(e);
        vert_neighbors[a].push(b);
        vert_neighbors[b].push(a);
    }
    let mut hex_neighbors = vec![Vec::new(); hexes.len()];
    for hs in &edge_hexes {
        if let [a, b] = hs[..] {
            hex_neighbors[a].push(b);
            hex_neighbors[b].push(a);
        }
    }
    let letters = |hs: &[usize]| {
        let mut ls: Vec<char> = hs.iter().map(|&h| LETTERS[h]).collect();
        ls.sort_unstable();
        ls.iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join("/")
    };
    let vert_labels = (0..verts.len())
        .map(|v| {
            let hs = &vert_hexes[v];
            if hs.len() == 1 {
                let (h, d) = vert_first[v];
                format!("{} ({} tip)", LETTERS[h], VERTEX_DIRS[d])
            } else {
                letters(hs)
            }
        })
        .collect();
    let edge_labels = (0..edges.len())
        .map(|e| {
            let hs = &edge_hexes[e];
            if hs.len() == 1 {
                let (h, d) = edge_first[e];
                format!("{} ({} shore)", LETTERS[h], SIDE_DIRS[d])
            } else {
                letters(hs).replace('/', "–")
            }
        })
        .collect();
    Topology {
        hexes,
        hex_verts,
        hex_edges,
        hex_neighbors,
        verts,
        vert_hexes,
        vert_edges,
        vert_neighbors,
        edges,
        edge_hexes,
        vert_labels,
        edge_labels,
    }
}

const ROOT3_2: f32 = 0.866_025_4;

/// Scene position of a hex centre.
pub fn hex_pos(h: usize) -> (f32, f32) {
    let (q, r) = topo().hexes[h];
    half_to_scene(2 * q + r, 3 * r)
}

/// Scene position of a vertex.
pub fn vert_pos(v: usize) -> (f32, f32) {
    let (x, y) = topo().verts[v];
    half_to_scene(x, y)
}

fn half_to_scene(x: i32, y: i32) -> (f32, f32) {
    (
        SCENE_W / 2.0 + x as f32 * SIZE * ROOT3_2,
        SCENE_H / 2.0 + y as f32 * SIZE / 2.0,
    )
}
