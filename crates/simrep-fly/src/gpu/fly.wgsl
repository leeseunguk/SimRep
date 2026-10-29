struct Parameters {
    dims: vec4<u32>,
    cells: vec4<u32>,
    degrees: vec4<u32>,
    flags: vec4<u32>,
    spacing: vec4<f32>,
    axes: array<vec4<f32>, 3>,
    bounds: array<vec4<f32>, 2>,
    detector: vec4<f32>,
    stop_plane: vec4<f32>,
    integration: vec4<f32>,
    tolerance: vec4<f32>,
    counts: vec4<u32>,
    padding: vec4<u32>,
    voltages: array<vec4<f32>, 16>,
    origin: vec4<f32>,
    magnetic: vec4<f32>,
    collision: vec4<f32>,
    gas: vec4<f32>,
    collision_limits: vec4<f32>,
    step_limits: vec4<f32>,
    physics: vec4<u32>,
    physics_padding: vec4<u32>,
}
struct Ion {
    cell: vec4<i32>,
    padding0: vec4<u32>,
    offset: vec4<f32>,
    velocity: vec4<f32>,
    metadata: vec4<f32>,
    outcome: vec4<u32>,
    padding1: vec4<u32>,
    time: vec4<f32>,
    progress: vec4<f32>,
}
struct EndState {
    cell: vec4<i32>,
    outcome: vec4<u32>,
    offset: vec4<f32>,
    velocity: vec4<f32>,
}
struct Sample { position: vec4<f32>, velocity: vec4<f32> }
struct TerminalBuffer { pending: atomic<u32>, padding: array<u32,7>, states: array<EndState> }
struct Coordinate { cell: vec3<i32>, offset: vec3<f32> }
// Displacement is relative to the beginning of this step, in physical mm.
struct Phase { d: vec3<f32>, u: vec3<f32> }
struct Trial { phase: Phase, error: f32 }
struct Event { t: f32, code: u32, index: u32 }
var<private> field_invalid: bool;
const SURFACE_SDF: bool = false;
const SURFACE_SDF_REPLACE: bool = false;
const WORKBENCH: bool = false;
const DYNAMIC: bool = false;
const PHYSICS: bool = false;
const COLLISION: u32 = 0u;
const MAGNETIC: bool = false;
const ELECTRIC: bool = true;
const COILS: u32 = 0u;
const COIL_THREADS: u32 = 64u;
var<workgroup> coil_tile: array<f32,1536>;
const MAGNETIC_PAS: u32 = 0u;
const BORIS: bool = false;
const PHYSICS_NODAL: bool = false;
const IMPORTED_MAPS: bool = false;

@group(0) @binding(0) var<uniform> root: Parameters;
var<private> p: Parameters;
var<private> launch_departure: bool;
@group(0) @binding(1) var<storage, read> basis: array<f32>;
@group(0) @binding(2) var<storage, read_write> powers: array<f32>;
@group(0) @binding(3) var<storage, read> metal: array<u32>;
@group(0) @binding(4) var<storage, read_write> ions: array<Ion>;
@group(0) @binding(5) var<storage, read> instances: array<Parameters>;
@group(0) @binding(6) var<storage, read_write> terminal: TerminalBuffer;
@group(0) @binding(7) var<storage, read_write> samples: array<Sample>;
@group(0) @binding(8) var<storage, read> transport_data: array<f32>;

// SDF_FUNCTIONS
@group(0) @binding(9) var<storage, read> sdf_headers: array<sdf_Volume>;
@group(0) @binding(10) var<storage, read> sdf_samples: array<f32>;

fn voltage(b: u32, t: f32) -> f32 { return p.voltages[b].x; }
fn next_voltage_event(t: f32) -> f32 { return 3.402823e38; }
fn analytic_magnetic(position: vec3<f32>, time: f32) -> vec3<f32> { return vec3<f32>(0.0); }
var<private> stage_voltages: array<f32, 16>;
var<private> field_base: u32;
var<private> field_stride: u32;
var<private> field_bases: u32;
const HERMITE: array<vec4<f32>, 6> = array<vec4<f32>, 6>(
    vec4<f32>(-10.0,15.0,-6.0,0.0), vec4<f32>(-6.0,8.0,-3.0,0.0),
    vec4<f32>(-1.5,1.5,-0.5,0.0), vec4<f32>(10.0,-15.0,6.0,0.0),
    vec4<f32>(-4.0,7.0,-3.0,0.0), vec4<f32>(0.5,-1.0,0.5,0.0));
fn voltage_pow(x: f32, y: f32) -> f32 {
    if x < 0.0 {
        if floor(y) != y { return bitcast<f32>(0x7fc00000u); }
        return select(1.0, -1.0, abs(y % 2.0) == 1.0) * pow(-x, y);
    }
    return pow(x, y);
}
fn coefficient(i: u32) -> f32 {
    if !DYNAMIC && !PHYSICS {
        // A static workbench has one fixed background basis per instance.
        if WORKBENCH { return basis[field_base + i]; }
        return powers[i];
    }
    var v = 0.0;
    for (var b = 0u; b < field_bases; b++) {
        v += stage_voltages[b] * basis[field_base + b * field_stride + i];
    }
    return v;
}

fn expand_cell(index: u32) {
    let cell = vec3<u32>(index%p.cells.x, index/p.cells.x%p.cells.y,
        index/(p.cells.x*p.cells.y));
    let dims = p.dims.xyz;
    let nodes = dims.x*dims.y*dims.z;
    var c: array<f32,216>;
    for (var j = 0u; j < 216u; j++) {
        let local = vec3<u32>(j%6u,j/6u%6u,j/36u);
        let node = cell+local/vec3<u32>(3u);
        let component = local.x%3u+3u*(local.y%3u+3u*(local.z%3u));
        let i = component*nodes+(node.z*dims.y+node.y)*dims.x+node.x;
        var value = 0.0;
        for (var b = 0u; b < p.degrees.w; b++) {
            value += p.voltages[b].x*basis[b*p.cells.w+i];
        }
        c[j] = value;
    }
    for (var axis = 0u; axis < 3u; axis++) {
        var stride = 1u;
        if axis == 1u { stride = 6u; }
        if axis == 2u { stride = 36u; }
        for (var i = 0u; i < 216u; i++) {
            if (i/stride)%6u != 0u { continue; }
            let input = array<f32,6>(c[i],c[i+stride],c[i+2u*stride],
                c[i+3u*stride],c[i+4u*stride],c[i+5u*stride]);
            c[i] = input[0];
            c[i+stride] = input[1];
            c[i+2u*stride] = 0.5*input[2];
            for (var k = 0u; k < 3u; k++) {
                var value = 0.0;
                for (var j = 0u; j < 6u; j++) { value += input[j]*HERMITE[j][k]; }
                c[i+(k+3u)*stride] = value;
            }
        }
    }
    for (var j = 0u; j < 216u; j++) { powers[index*216u+j] = c[j]; }
}

@compute @workgroup_size(64)
fn combine(@builtin(global_invocation_id) id: vec3<u32>) {
    p = root;
    let i = id.x + id.y * (64u * 65535u);
    if p.padding.y != 0u {
        if i < p.cells.x*p.cells.y*p.cells.z { expand_cell(i); }
        return;
    }
    if i >= p.cells.w { return; }
    var value = 0.0;
    for (var b = 0u; b < p.degrees.w; b++) {
        value += p.voltages[b].x * basis[b * p.cells.w + i];
    }
    powers[i] = value;
}

fn moved(q: Coordinate, d: vec3<f32>) -> Coordinate {
    let offset = q.offset + d / p.spacing.xyz;
    let carry = vec3<i32>(floor(offset));
    return Coordinate(q.cell + carry, offset - vec3<f32>(carry));
}

fn physical(q: Coordinate) -> vec3<f32> {
    return (vec3<f32>(q.cell) + q.offset) * p.spacing.xyz;
}

fn reflected(input: Coordinate) -> Coordinate {
    var q = input;
    if p.flags.z != 0u {
        let point = physical(q);
        let radius = length(point.yz) / p.spacing.y;
        q.cell.y = i32(floor(radius));
        q.offset.y = radius - floor(radius);
        q.cell.z = 0;
        q.offset.z = 0.0;
    }
    for (var a = 0u; a < 3u; a++) {
        if (p.flags.x & (1u << a)) != 0u && q.cell[a] < 0 {
            q.cell[a] = -q.cell[a] - 1;
            q.offset[a] = 1.0 - q.offset[a];
        }
    }
    return q;
}

fn grid_status(input: Coordinate) -> Event {
    let q = reflected(input);
    var node = vec3<u32>(0u);
    for (var a = 0u; a < 3u; a++) {
        if p.dims[a] == 1u { continue; }
        if q.cell[a] < 0 || q.cell[a] > i32(p.dims[a] - 1u)
            || (q.cell[a] == i32(p.dims[a] - 1u) && q.offset[a] > 0.0) {
            return Event(0.0, 2u, 0u);
        }
        if p.padding.z != 0u {
            node[a] = u32(min(q.cell[a]+i32(q.offset[a] >= 1.0), i32(p.dims[a])-2));
        } else {
            node[a] = u32(q.cell[a]) + u32(q.offset[a] >= 0.5);
        }
    }
    let index = (node.z * p.dims.y + node.y) * p.dims.x + node.x;
    if p.padding.z != 0u {
        let cell_index = (node.z*p.cells.y+node.y)*p.cells.x+node.x;
        let packed = metal[p.padding.x+cell_index+select(0u,p.counts.x,WORKBENCH)];
        return Event(0.0,select((packed>>12u)&1u,0u,SURFACE_SDF_REPLACE),index);
    }
    return Event(0.0, select(metal[index + select(0u, p.counts.x, WORKBENCH)],0u,SURFACE_SDF_REPLACE), index);
}

fn instance_electric(input: Coordinate, time: f32) -> vec3<f32> {
    let flags = p.flags;
    let spacing = p.spacing.xyz;
    let cells = p.cells;
    let degrees = p.degrees;
    if flags.w != 0u && grid_status(input).code == 2u { return vec3<f32>(0.0); }
    field_base = p.dims.w;
    field_stride = cells.w;
    field_bases = degrees.w;
    if DYNAMIC || PHYSICS {
        for (var b = 0u; b < degrees.w; b++) { stage_voltages[b] = voltage(b, time); }
    }
    let q = reflected(input);
    var cell = clamp(q.cell, vec3<i32>(0), vec3<i32>(cells.xyz) - vec3<i32>(1));
    let t = vec3<f32>(q.cell - cell) + q.offset;
    let size = degrees.x * degrees.y * degrees.z;
    let cell_index = u32((cell.z * i32(cells.y) + cell.y) * i32(cells.x) + cell.x);
    let start = cell_index * size;
    let packed = metal[p.padding.x + cell_index + select(0u, p.counts.x, WORKBENCH)];
    let effective = vec3<u32>(packed & 15u, (packed >> 4u) & 15u, (packed >> 8u) & 15u);
    var value = 0.0;
    var gradient = vec3<f32>(0.0);
    if PHYSICS_NODAL && p.padding.y != 0u {
        gradient = nodal_gradient(vec3<u32>(cell),t);
    } else {
        for (var z = i32(effective.z) - 1; z >= 0; z--) {
            var vy = 0.0;
            var dx = 0.0;
            var dy = 0.0;
            for (var y = i32(effective.y) - 1; y >= 0; y--) {
                var vx = 0.0;
                var d = 0.0;
                for (var x = i32(effective.x) - 1; x >= 0; x--) {
                    d = d * t.x + vx;
                    vx = vx * t.x + coefficient(start + u32((z * i32(degrees.y) + y) * i32(degrees.x) + x));
                }
                dy = dy * t.y + vy;
                vy = vy * t.y + vx;
                dx = dx * t.y + d;
            }
            gradient.z = gradient.z * t.z + value;
            value = value * t.z + vy;
            gradient.x = gradient.x * t.z + dx;
            gradient.y = gradient.y * t.z + dy;
        }
    }
    var reflection = vec3<f32>(1.0);
    var sign = 1.0;
    for (var a = 0u; a < 3u; a++) {
        if (flags.x & (1u << a)) != 0u && input.cell[a] < 0
            && !(flags.z != 0u && a == 1u) {
            reflection[a] = -1.0;
            if (flags.y & (1u << a)) != 0u { sign = -sign; }
        }
    }
    var e = -sign * reflection * gradient / spacing;
    if flags.z != 0u {
        let point = physical(input);
        let r = length(point.yz);
        if r == 0.0 { e.y = 0.0; e.z = 0.0; }
        else { e = vec3<f32>(e.x, e.y * point.y / r, e.y * point.z / r); }
    }
    return e;
}

fn root_world(q: Coordinate) -> vec3<f32> {
    let point = (vec3<f32>(q.cell) + q.offset) * root.spacing.xyz;
    return root.axes[0].xyz*point.x + root.axes[1].xyz*point.y + root.axes[2].xyz*point.z;
}
fn to_instance(point: vec3<f32>) -> Coordinate {
    let delta = point - p.origin.xyz;
    let q = vec3<f32>(dot(p.axes[0].xyz,delta),dot(p.axes[1].xyz,delta),dot(p.axes[2].xyz,delta)) / p.spacing.xyz;
    let cell = vec3<i32>(floor(q));
    return Coordinate(cell,q-vec3<f32>(cell));
}
fn electric(q: Coordinate, time: f32) -> vec3<f32> {
    if !ELECTRIC { return vec3<f32>(0.0); }
    if !WORKBENCH { return instance_electric(q,time); }
    let point = root_world(q);
    var result = vec3<f32>(0.0);
    for (var i=0u; i<select(root.dims.w,root.physics.x,PHYSICS); i++) {
        p = instances[i];
        let local = to_instance(point);
        if grid_status(local).code != 2u {
            let e = instance_electric(local,time);
            let global = p.axes[0].xyz*e.x+p.axes[1].xyz*e.y+p.axes[2].xyz*e.z;
            result = vec3<f32>(dot(root.axes[0].xyz,global),dot(root.axes[1].xyz,global),dot(root.axes[2].xyz,global));
            break;
        }
    }
    p = root;
    return result;
}

fn velocity(u: vec3<f32>) -> vec3<f32> {
    if p.counts.z == 0u { return u; }
    return u * inverseSqrt(1.0 + dot(u, u) / (299792.458 * 299792.458));
}
fn derivative(q: Coordinate, s: Phase, ratio: f32, time: f32) -> Phase {
    let at = moved(q, s.d);
    let v = velocity(s.u);
    let e = electric(at,time);
    if !MAGNETIC { return Phase(v,96.48533215665327*ratio*e); }
    return Phase(v, ratio * (96.48533215665327 * e + 0.009648533215665327 * cross(v, magnetic(at,time))));
}
// PHYSICS_FUNCTIONS
fn scaled(s: Phase, a: f32) -> Phase { return Phase(s.d * a, s.u * a); }
fn plus(a: Phase, b: Phase) -> Phase { return Phase(a.d + b.d, a.u + b.u); }

fn rk45(q: Coordinate, u: vec3<f32>, h: f32, ratio: f32, time: f32) -> Trial {
    var end_time = time+h;
    if end_time == next_voltage_event(time) {
        if end_time == 0.0 { end_time = bitcast<f32>(0x80000001u); }
        else { end_time = bitcast<f32>(bitcast<u32>(end_time) - select(1u, 0xffffffffu, end_time < 0.0)); }
    }
    let y = Phase(vec3<f32>(0.0), u);
    let k1 = derivative(q, y, ratio, time);
    let k2 = derivative(q, plus(y, scaled(k1, h * (1.0/5.0))), ratio, time+h/5.0);
    let k3 = derivative(q, plus(y, scaled(plus(scaled(k1, 3.0/40.0), scaled(k2, 9.0/40.0)), h)), ratio, time+3.0*h/10.0);
    let k4 = derivative(q, plus(y, scaled(plus(plus(scaled(k1, 44.0/45.0), scaled(k2, -56.0/15.0)), scaled(k3, 32.0/9.0)), h)), ratio, time+4.0*h/5.0);
    let k5 = derivative(q, plus(y, scaled(plus(plus(plus(scaled(k1, 19372.0/6561.0), scaled(k2, -25360.0/2187.0)), scaled(k3, 64448.0/6561.0)), scaled(k4, -212.0/729.0)), h)), ratio, time+8.0*h/9.0);
    let k6 = derivative(q, plus(y, scaled(plus(plus(plus(plus(scaled(k1, 9017.0/3168.0), scaled(k2, -355.0/33.0)), scaled(k3, 46732.0/5247.0)), scaled(k4, 49.0/176.0)), scaled(k5, -5103.0/18656.0)), h)), ratio, end_time);
    let increment = plus(plus(plus(plus(scaled(k1, 35.0/384.0), scaled(k3, 500.0/1113.0)), scaled(k4, 125.0/192.0)), scaled(k5, -2187.0/6784.0)), scaled(k6, 11.0/84.0));
    let high = plus(y, scaled(increment, h));
    let k7 = derivative(q, high, ratio, end_time);
    // Embedded difference directly from stages, not high-position minus low-position.
    let diff = scaled(plus(plus(plus(plus(plus(scaled(k1, 35.0/384.0 - 5179.0/57600.0), scaled(k3, 500.0/1113.0 - 7571.0/16695.0)), scaled(k4, 125.0/192.0 - 393.0/640.0)), scaled(k5, -2187.0/6784.0 + 92097.0/339200.0)), scaled(k6, 11.0/84.0 - 187.0/2100.0)), scaled(k7, -1.0/40.0)), h);
    let x = physical(q);
    let pos_scale = vec3<f32>(p.tolerance.y) + p.tolerance.x * max(abs(x), abs(x + high.d));
    let v = velocity(u);
    let vh = velocity(high.u);
    let vel_scale = vec3<f32>(p.tolerance.z) + p.tolerance.x * max(abs(v), abs(vh));
    var dv = abs(diff.u);
    if p.counts.z != 0u {
        let c2 = 299792.458 * 299792.458;
        let gh = sqrt(1.0 + dot(high.u, high.u)/c2);
        let low_u = high.u - diff.u;
        let gl = sqrt(1.0 + dot(low_u, low_u)/c2);
        let dg = (-2.0*dot(high.u, diff.u) + dot(diff.u, diff.u)) / (c2*(gl+gh));
        dv = abs(diff.u/gh + low_u*dg/(gh*gl));
    }
    let error = max(abs(diff.d) / pos_scale, dv / vel_scale);
    return Trial(high, max(error.x, max(error.y, error.z)));
}

fn cut(t: f32, after: f32, best: f32) -> f32 {
    if t > after && t < best { return t; }
    return best;
}
fn radial_cut(start: vec3<f32>, d: vec3<f32>, r: f32, after: f32, best_in: f32) -> f32 {
    let aa = dot(d.yz, d.yz);
    let bb = dot(start.yz, d.yz);
    let cc = dot(start.yz, start.yz) - r*r;
    let disc = bb*bb - aa*cc;
    if aa == 0.0 || disc < 0.0 { return best_in; }
    let root = sqrt(disc);
    var best = cut((-bb - root) / aa, after, best_in);
    best = cut((-bb + root) / aa, after, best);
    return best;
}
// CPU geometry partitions at cell faces (or optional nearest-node voxel faces),
// mirror planes and cylindrical shells. Find the next partition without a
// variable-length private array or a fixed upper bound on segment length.
fn next_cut(q: Coordinate, d: vec3<f32>, after: f32) -> f32 {
    let start = physical(q);
    var best = 1.0;
    let to = start + d;
    let boundary_offset = select(0.5,0.0,p.padding.z != 0u);
    for (var a = 0u; a < 3u; a++) {
        if p.dims[a] == 1u { continue; }
        let h = p.spacing[a];
        if a == 1u && p.flags.z != 0u {
            let aa = dot(d.yz, d.yz);
            if aa == 0.0 { continue; }
            let closest = clamp(-dot(start.yz, d.yz) / aa, 0.0, 1.0);
            let rmin = length(start.yz + closest*d.yz);
            let rmax = max(length(start.yz), length(to.yz));
            let lo = max(0, i32(ceil(rmin / h - boundary_offset)));
            let hi = min(i32(p.dims[a])-2, i32(floor(rmax/h - boundary_offset)));
            for (var j = lo; j <= hi; j++) {
                best = radial_cut(start, d, (f32(j)+boundary_offset)*h, after, best);
            }
            best = radial_cut(start, d, f32(p.dims[a]-1u)*h, after, best);
        } else if d[a] != 0.0 {
            let sign_count = select(1u, 2u, (p.flags.x & (1u << a)) != 0u);
            for (var s = 0u; s < sign_count; s++) {
                let sign = select(1.0, -1.0, s == 1u);
                let origin = select(q.cell[a], -q.cell[a], s == 1u);
                let offset = sign*q.offset[a];
                let delta = sign*d[a]/h;
                let lo = max(0, origin + i32(ceil(min(offset, offset+delta) - boundary_offset)));
                let hi = min(i32(p.dims[a])-2, origin + i32(floor(max(offset, offset+delta) - boundary_offset)));
                for (var j = lo; j <= hi; j++) {
                    best = cut((f32(j-origin)+boundary_offset-offset)/delta, after, best);
                }
                best = cut((f32(i32(p.dims[a]-1u)-origin)-offset)/delta, after, best);
            }
            best = cut((-f32(q.cell[a])-q.offset[a])/(d[a]/h), after, best);
        }
    }
    return best;
}

fn world(point: vec3<f32>) -> vec3<f32> {
    return p.axes[0].xyz * point.x + p.axes[1].xyz * point.y + p.axes[2].xyz * point.z;
}
fn instance_status(q: Coordinate) -> Event {
    if p.flags.w != 0u {
        let point = world(physical(q));
        if any(point < p.bounds[0].xyz) || any(point > p.bounds[1].xyz) { return Event(0.0, 2u, 0u); }
    }
    let s = grid_status(q);
    if p.flags.w != 0u && s.code == 2u { return Event(0.0, 0u, 0u); }
    return s;
}

fn plane_event(start: vec3<f32>, d: vec3<f32>, plane: vec4<f32>, first: Event) -> Event {
    let distance = dot(plane.xyz, d);
    if distance != 0.0 {
        let t = (plane.w - dot(plane.xyz, start)) / distance;
        if t >= 0.0 && t <= 1.0 && (first.code == 0u || t < first.t) { return Event(t, 3u, 0u); }
    }
    return first;
}

fn instance_intersection(q: Coordinate, d: vec3<f32>) -> Event {
    let start = physical(q);
    var first = instance_status(q);
    var leaving = launch_departure && first.code == 1u;
    if leaving { first.code = 0u; }
    if first.code != 0u { return first; }
    var lo = 0.0;
    loop {
        let hi = next_cut(q, d, lo);
        var s = grid_status(moved(q, (0.5*(lo+hi))*d));
        if p.flags.w != 0u && s.code == 2u { s.code = 0u; }
        if leaving && s.code == 1u { s.code = 0u; }
        else { leaving = false; }
        if s.code != 0u { first = Event(lo, s.code, s.index); break; }
        if hi >= 1.0 {
            s = grid_status(moved(q, d));
            if p.flags.w != 0u && s.code == 2u { s.code = 0u; }
            if leaving && s.code == 1u { s.code = 0u; }
            if s.code != 0u { first = Event(1.0, s.code, s.index); }
            break;
        }
        lo = hi;
    }
    if p.flags.w != 0u {
        let a = world(start);
        let delta = world(d);
        let b = a + delta;
        for (var axis = 0u; axis < 3u; axis++) {
            if b[axis] < p.bounds[0][axis] || b[axis] > p.bounds[1][axis] {
                let edge = select(p.bounds[1][axis], p.bounds[0][axis], b[axis] < p.bounds[0][axis]);
                let t = (edge-a[axis])/delta[axis];
                if t >= 0.0 && t <= 1.0 && (first.code == 0u || t < first.t) { first = Event(t, 2u, 0u); }
            }
        }
    }
    first = plane_event(start, d, p.detector, first);
    first = plane_event(start, d, p.stop_plane, first);
    return first;
}

fn base_status(q: Coordinate) -> Event {
    if !WORKBENCH { return instance_status(q); }
    let point = root_world(q);
    var result = Event(0.0,0u,0u);
    for (var i=0u; i<select(root.dims.w,root.physics.x,PHYSICS); i++) {
        p=instances[i];
        let s=instance_status(to_instance(point));
        if s.code != 0u { result=s; break; }
    }
    if PHYSICS && result.code == 0u {
        var first = Event(0.0,0u,0u);
        for (var i=root.physics.x; i<root.physics.x+root.physics.y; i++) {
            p=instances[i];
            let local = to_instance(point);
            let s = instance_status(local);
            if i == root.physics.x { first = s; }
            if grid_status(local).code != 2u { first=s; break; }
        }
        result = first;
    }
    if PHYSICS && result.code == 0u { result=imported_status(point); }
    p=root;
    return result;
}
fn base_intersection(q: Coordinate, d: vec3<f32>) -> Event {
    if !WORKBENCH { return instance_intersection(q,d); }
    let point = root_world(q);
    let delta = root.axes[0].xyz*d.x+root.axes[1].xyz*d.y+root.axes[2].xyz*d.z;
    var result=Event(0.0,0u,0u);
    for (var i=0u; i<select(root.dims.w,root.physics.x+root.physics.y,PHYSICS); i++) {
        p=instances[i];
        let local_d=vec3<f32>(dot(p.axes[0].xyz,delta),dot(p.axes[1].xyz,delta),dot(p.axes[2].xyz,delta));
        let s=instance_intersection(to_instance(point),local_d);
        if s.code != 0u && (result.code == 0u || s.t < result.t) { result=s; }
    }
    p=root;
    if PHYSICS {
        let s=imported_intersection(point,delta);
        if s.code!=0u && (result.code==0u || s.t<result.t) { result=s; }
    }
    return plane_event(physical(q),d,root.stop_plane,result);
}

// SDF coordinates are source geometry coordinates, before instance placement.
// Header spare components carry the source origin and instance scale.
fn sdf_source(q: Coordinate) -> vec3<f32> {
    let h=sdf_headers[0];
    return physical(q)/h.origin.w+vec3<f32>(h.parameters.zw,h.spacing.w);
}
fn status(q: Coordinate) -> Event {
    let result=base_status(q);
    if SURFACE_SDF && result.code==0u {
        let number=sdf_membership(sdf_source(q));
        if number!=0u { return Event(0.0,1u,number|0x80000000u); }
    }
    return result;
}
fn intersection(q: Coordinate, d: vec3<f32>) -> Event {
    let result=base_intersection(q,d);
    if SURFACE_SDF {
        let hit=sdf_first_hit(sdf_source(q),d/sdf_headers[0].origin.w,launch_departure);
        if hit.z==2.0 { return Event(0.0,7u,0u); }
        if hit.z==1.0 && (result.code==0u || hit.x<result.t || (hit.x==result.t && result.code==3u)) {
            return Event(hit.x,1u,u32(hit.y)|0x80000000u);
        }
    }
    return result;
}

// WGSL permits floating-point reassociation; an ordinary two_sum can therefore
// collapse to (a+b, 0) on real drivers. Compute the close positive operands'
// difference through integer mantissas, so the error term cannot be folded.
fn close_difference(a: f32, b: f32) -> f32 {
    let upper = bitcast<u32>(max(a, b));
    let lower = bitcast<u32>(min(a, b));
    let exponent = (lower >> 23u) & 255u;
    let shift = ((upper >> 23u) & 255u) - exponent;
    let mantissa = (((upper & 0x7fffffu) | 0x800000u) << shift)
        - ((lower & 0x7fffffu) | 0x800000u);
    let value = f32(mantissa) * exp2(f32(exponent) - 150.0);
    return select(-value, value, a >= b);
}
fn time_add(time: vec2<f32>, h: f32) -> vec2<f32> {
    if time.x == 0.0 { return vec2<f32>(h, time.y); }
    let large = max(time.x, h);
    let small = min(time.x, h);
    let sum = large + small;
    let residual = small - close_difference(sum, large);
    let low = time.y + residual;
    let hi = sum + low;
    return vec2<f32>(hi, low - close_difference(hi, sum));
}
fn finite(v: vec3<f32>) -> bool { return all((bitcast<vec3<u32>>(v)&vec3<u32>(0x7f800000u)) != vec3<u32>(0x7f800000u)); }

fn save_sample(index: u32, q: Coordinate, u: vec3<f32>, time: vec2<f32>) {
    samples[index] = Sample(vec4<f32>(physical(q), time.x), vec4<f32>(u, time.y));
}

@compute @workgroup_size(64)
fn fly(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    if COILS != 0u && COILS <= 128u {
        for (var i=lane; i<12u*COILS; i+=COIL_THREADS) { coil_tile[i] = transport_data[i]; }
        workgroupBarrier();
    }
    p = root;
    if id.x >= p.counts.x { return; }
    var ion = ions[id.x];
    if ion.outcome.z != 0u { return; }
    let first = ion.padding0.w == 0u;
    var q = Coordinate(ion.cell.xyz, ion.offset.xyz);
    var u = ion.velocity.xyz;
    rng_key = root.physics_padding.xy;
    rng_count = vec4<u32>(ion.padding1.zw,ion.padding1.xy);
    ion_mass = ion.offset.w;
    sds_transport = ion.time;
    var hazard = ion.progress.w;
    if first && COLLISION == 1u { hazard = -log(random_uniform()); }
    var time = ion.progress.xy;
    var accepted = ion.outcome.x;
    var rejected = ion.outcome.y;
    let sampling = ion.padding0;
    var recorded = 0u;
    var last_recorded = 0u;
    if sampling.x != 0xffffffffu {
        if first { save_sample(sampling.x*sampling.z, q, u, time); }
        recorded = 1u+accepted/sampling.y;
        last_recorded = accepted-accepted%sampling.y;
    }
    var reason = Event(0.0,0u,0u);
    var departing = ion.metadata.w != 0.0;
    if first {
        reason = status(q);
        departing = departing && reason.code == 1u;
        if departing { reason.code = 0u; }
    }
    launch_departure = departing;
    let point = physical(q);
    if first && reason.code == 0u
        && ((any(p.stop_plane.xyz != vec3<f32>(0.0)) && dot(p.stop_plane.xyz, point) == p.stop_plane.w)
        || (any(p.detector.xyz != vec3<f32>(0.0)) && dot(p.detector.xyz, point) == p.detector.w)) {
        reason = Event(0.0, 3u, 0u);
    }
    var h = ion.progress.z;
    if first { h = clamp(p.integration.y, p.integration.z, p.integration.w); }
    var localization = 0u;
    var low = 0.0;
    var high = 0.0;
    var saved_hit = Event(0.0, 0u, 0u);
    var trials = 0u;
    var paused = false;
    loop {
        let remaining = (p.integration.x - time.x) + (p.tolerance.w - time.y);
        if reason.code != 0u || remaining <= 0.0 { break; }
        if accepted + rejected >= p.counts.y { reason.code = 5u; break; }
        if PHYSICS && trials >= root.physics_padding.z && localization == 0u {
            paused = true;
            break;
        }
        h = min(h, remaining);
        if root.padding.w != 0u { h = min(h, automatic_step(q,u,ion.metadata.x,ion.metadata.y+time.x+time.y)); }
        if field_invalid { reason.code = 7u; break; }
        if h <= 0.0 || all(time_add(time, h) == time) { reason.code = 6u; break; }
        let absolute_time = ion.metadata.y + (ion.metadata.z + time.x + time.y);
        h = min(h, next_voltage_event(absolute_time) - absolute_time);
        if COLLISION == 1u {
            let rate = collision_rate(u);
            if rate > 0.0 { h = min(h,min(hazard,1.0/root.collision_limits.y)/rate); }
        }
        var trial = deterministic_trial(q, u, h, ion.metadata.x, absolute_time);
        trials++;
        if field_invalid || !finite(trial.phase.d) || !finite(trial.phase.u) || !(trial.error <= 3.402823e38) {
            reason.code = 7u; break;
        }
        if localization > 1u {
            let event = intersection(q, trial.phase.d);
            if event.code != 0u { high = h; saved_hit = event; }
            else { low = h; }
            localization--;
            h = select(0.5*(low+high), high, localization == 1u);
            continue;
        }
        if localization == 1u {
            reason = saved_hit;
            localization = 0u;
        } else if trial.error > 1.0 {
            rejected++;
            let reduced = h * clamp(0.9 * pow(trial.error, -0.2), 0.1, 0.5);
            if reduced < p.integration.z { reason.code = 6u; break; }
            h = reduced;
            continue;
        }
        if reason.code == 0u {
            let hit = intersection(q, trial.phase.d);
            if hit.code != 0u {
                low = 0.0;
                high = h;
                saved_hit = hit;
                localization = 27u;
                h *= 0.5;
                continue;
            }
        }
        if COLLISION != 0u && reason.code == 0u {
            let deterministic = trial.phase.d;
            trial.phase = collision_accepted(u, trial.phase, h, &hazard);
            if COLLISION == 2u && root.collision_limits.z != 0.0 {
                let jump = trial.phase.d-deterministic;
                let jump_hit = intersection(moved(q,deterministic),jump);
                if jump_hit.code != 0u {
                    trial.phase.d = deterministic+jump_hit.t*jump;
                    reason = jump_hit;
                }
            }
        }
        time = time_add(time, h);
        q = moved(q, trial.phase.d);
        if departing && status(q).code != 1u { departing = false; }
        launch_departure = departing;
        u = trial.phase.u;
        accepted++;
        if sampling.x != 0xffffffffu && accepted%sampling.y == 0u {
            if recorded < sampling.z {
                save_sample(sampling.x*sampling.z+recorded, q, u, time);
            }
            recorded++;
            last_recorded = accepted;
        }
        var factor = 5.0;
        if trial.error != 0.0 { factor = clamp(0.9*pow(trial.error, -0.2), 0.2, 5.0); }
        h = clamp(h*factor, p.integration.z, p.integration.w);
        if BORIS { h = min(p.integration.y,p.integration.w); }
    }
    if !paused && reason.code == 0u { reason.code = 4u; }
    ion.cell = vec4<i32>(q.cell, 0);
    ion.offset = vec4<f32>(q.offset, ion_mass);
    ion.velocity = vec4<f32>(u, 0.0);
    ion.progress = vec4<f32>(time,h,hazard);
    ion.padding1 = vec4<u32>(ion.padding1.xy,rng_count.xy);
    ion.padding0.w = 1u;
    ion.metadata.w = select(0.0,1.0,departing);
    ion.outcome = vec4<u32>(accepted, rejected, reason.code, reason.index);
    if !paused && sampling.x != 0xffffffffu && last_recorded != accepted {
        if recorded < sampling.z {
            save_sample(sampling.x*sampling.z+recorded, q, u, time);
        }
        recorded++;
    }
    ion.cell.w = i32(recorded);
    ions[id.x] = ion;
    if paused { atomicStore(&terminal.pending,1u); }
    terminal.states[id.x] = EndState(ion.cell, ion.outcome,
        vec4<f32>(q.offset, time.x), vec4<f32>(u, time.y));
}
