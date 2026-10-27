use crate::{Part, cross, dot, norm, sub};
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{BuildHasher, Hasher};
use std::io;

#[derive(Clone, Default)]
struct FxHasher(u64);
impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut buf = [0u8; 8];
            buf[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(buf));
        }
    }
    fn write_u64(&mut self, value: u64) {
        self.0 = self.0.wrapping_mul(0x517cc1b727220a95) ^ value;
    }
    fn finish(&self) -> u64 {
        self.0
    }
}
#[derive(Clone, Default)]
struct FxBuild;
impl BuildHasher for FxBuild {
    type Hasher = FxHasher;
    fn build_hasher(&self) -> FxHasher {
        FxHasher::default()
    }
}
type FxMap<K, V> = HashMap<K, V, FxBuild>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MeshReport {
    pub duplicate_vertices: usize,
    pub degenerate_triangles: usize,
    pub duplicate_triangles: usize,
    pub boundary_edges: usize,
    /// Boundary components that are simple cycles. These are reported holes,
    /// not automatically filled: the intended surface is unknown.
    pub boundary_loops: usize,
    pub non_manifold_edges: usize,
    pub inconsistent_edges: usize,
    pub inward_components: usize,
    pub orientation_conflicts: usize,
}

impl MeshReport {
    pub fn is_closed_manifold(&self) -> bool {
        self.boundary_edges == 0
            && self.non_manifold_edges == 0
            && self.degenerate_triangles == 0
            && self.duplicate_triangles == 0
            && self.orientation_conflicts == 0
    }
}

#[derive(Clone, Debug)]
pub struct RepairReport {
    pub before: MeshReport,
    pub after: MeshReport,
    pub removed_triangles: usize,
    pub flipped_triangles: usize,
}

type EdgeMap = FxMap<(usize, usize), Vec<(usize, bool)>>;

fn edges(triangles: &[[usize; 3]]) -> EdgeMap {
    let mut edges: EdgeMap = HashMap::with_hasher(FxBuild);
    for (i, [a, b, c]) in triangles.iter().copied().enumerate() {
        for (a, b) in [(a, b), (b, c), (c, a)] {
            edges
                .entry((a.min(b), a.max(b)))
                .or_default()
                .push((i, a < b));
        }
    }
    edges
}

pub(crate) fn weld(part: &mut Part) -> usize {
    let mut unique = HashMap::with_hasher(FxBuild);
    let mut vertices = Vec::new();
    let indices: Vec<_> = part
        .vertices
        .iter()
        .map(|vertex| {
            let key = vertex.map(|v| if v == 0.0 { 0 } else { v.to_bits() });
            *unique.entry(key).or_insert_with(|| {
                vertices.push(*vertex);
                vertices.len() - 1
            })
        })
        .collect();
    let count = part.vertices.len() - vertices.len();
    for triangle in &mut part.triangles {
        *triangle = triangle.map(|i| indices[i]);
    }
    part.vertices = vertices;
    count
}

fn components(triangles: &[[usize; 3]], edges: &EdgeMap) -> Vec<(Vec<(usize, bool)>, bool)> {
    let mut neighbors = vec![Vec::new(); triangles.len()];
    for uses in edges.values().filter(|uses| uses.len() == 2) {
        let (a, da) = uses[0];
        let (b, db) = uses[1];
        neighbors[a].push((b, da == db));
        neighbors[b].push((a, da == db));
    }
    let mut visited = vec![None; triangles.len()];
    let mut components = Vec::new();
    for start in 0..triangles.len() {
        if visited[start].is_some() {
            continue;
        }
        let mut component = Vec::new();
        let mut conflict = false;
        let mut queue = VecDeque::from([start]);
        visited[start] = Some(false);
        while let Some(face) = queue.pop_front() {
            let flip = visited[face].unwrap();
            component.push((face, flip));
            for &(neighbor, invert) in &neighbors[face] {
                let expected = flip ^ invert;
                if let Some(actual) = visited[neighbor] {
                    conflict |= actual != expected;
                } else {
                    visited[neighbor] = Some(expected);
                    queue.push_back(neighbor);
                }
            }
        }
        components.push((component, conflict));
    }
    components
}

fn volume(part: &Part, component: &[(usize, bool)]) -> f64 {
    // Shift the origin before the triple product to reduce cancellation for
    // small meshes located far from world origin.
    let origin = part.vertices[part.triangles[component[0].0][0]];
    component
        .iter()
        .map(|&(i, flip)| {
            let [a, b, c] = part.triangles[i].map(|j| sub(part.vertices[j], origin));
            dot(a, cross(b, c)) * if flip { -1.0 } else { 1.0 }
        })
        .sum::<f64>()
        / 6.0
}

fn closed(component: &[(usize, bool)], triangles: &[[usize; 3]], edges: &EdgeMap) -> bool {
    component.iter().all(|&(i, _)| {
        let [a, b, c] = triangles[i];
        [(a, b), (b, c), (c, a)]
            .into_iter()
            .all(|(a, b)| edges[&(a.min(b), a.max(b))].len() == 2)
    })
}

pub fn inspect(part: &Part) -> io::Result<MeshReport> {
    part.validate()?;
    let mut part = part.clone();
    let mut report = MeshReport {
        duplicate_vertices: weld(&mut part),
        ..MeshReport::default()
    };
    let mut faces = HashSet::with_hasher(FxBuild);
    for &triangle in &part.triangles {
        let [a, b, c] = triangle.map(|i| part.vertices[i]);
        if norm(cross(sub(b, a), sub(c, a))) == 0.0 {
            report.degenerate_triangles += 1;
        }
        let mut canonical = triangle;
        canonical.sort_unstable();
        if !faces.insert(canonical) {
            report.duplicate_triangles += 1;
        }
    }
    let edges = edges(&part.triangles);
    let mut boundary: FxMap<usize, Vec<usize>> = HashMap::with_hasher(FxBuild);
    for (&(a, b), uses) in &edges {
        match uses.len() {
            1 => {
                report.boundary_edges += 1;
                boundary.entry(a).or_default().push(b);
                boundary.entry(b).or_default().push(a);
            }
            2 => report.inconsistent_edges += usize::from(uses[0].1 == uses[1].1),
            _ => report.non_manifold_edges += 1,
        }
    }
    let mut visited = HashSet::with_hasher(FxBuild);
    for &start in boundary.keys() {
        if !visited.insert(start) {
            continue;
        }
        let mut queue = vec![start];
        let mut simple = true;
        while let Some(vertex) = queue.pop() {
            simple &= boundary[&vertex].len() == 2;
            for &neighbor in &boundary[&vertex] {
                if visited.insert(neighbor) {
                    queue.push(neighbor);
                }
            }
        }
        report.boundary_loops += usize::from(simple);
    }
    for (component, conflict) in components(&part.triangles, &edges) {
        report.orientation_conflicts += usize::from(conflict);
        if !conflict
            && closed(&component, &part.triangles, &edges)
            && volume(&part, &component) < 0.0
        {
            report.inward_components += 1;
        }
    }
    Ok(report)
}

pub(crate) fn shell_count(triangles: &[[usize; 3]]) -> usize {
    components(triangles, &edges(triangles)).len()
}

/// Weld exact coincident vertices, remove zero-area/duplicate faces, orient
/// orientable edge-connected components consistently, and orient each closed
/// shell outward. Open boundaries and non-manifold junctions remain reported.
/// No tolerance welding, hole filling, or topology guessing is performed.
pub fn repair(part: &mut Part) -> io::Result<RepairReport> {
    let before = inspect(part)?;
    let (removed_triangles, flipped_triangles) = weld_and_orient_counts(part)?;
    Ok(RepairReport {
        before,
        after: inspect(part)?,
        removed_triangles,
        flipped_triangles,
    })
}

/// Same mesh mutations as [`repair`], without the before/after topology reports.
pub fn weld_and_orient(part: &mut Part) -> io::Result<()> {
    weld_and_orient_counts(part)?;
    Ok(())
}

fn weld_and_orient_counts(part: &mut Part) -> io::Result<(usize, usize)> {
    weld(part);
    let count = part.triangles.len();
    let mut faces = HashSet::with_hasher(FxBuild);
    part.triangles.retain(|triangle| {
        let [a, b, c] = triangle.map(|i| part.vertices[i]);
        let mut canonical = *triangle;
        canonical.sort_unstable();
        norm(cross(sub(b, a), sub(c, a))) > 0.0 && faces.insert(canonical)
    });
    let removed_triangles = count - part.triangles.len();
    if part.triangles.is_empty() {
        return Err(crate::invalid("repair removed all triangles"));
    }
    let edges = edges(&part.triangles);
    let mut flipped_triangles = 0;
    for (component, conflict) in components(&part.triangles, &edges) {
        if conflict {
            continue;
        }
        let invert = closed(&component, &part.triangles, &edges) && volume(part, &component) < 0.0;
        for (i, flip) in component {
            if flip ^ invert {
                part.triangles[i].swap(1, 2);
                flipped_triangles += 1;
            }
        }
    }
    Ok((removed_triangles, flipped_triangles))
}
