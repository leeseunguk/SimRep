// All vectors are root-instance physical mm, mm/us and gauss unless stated.
var<private> rng_key: vec2<u32>;
var<private> rng_count: vec4<u32>;
var<private> ion_mass: f32;
var<private> sds_transport: vec4<f32>;

fn hermite_weights(t: f32) -> array<vec2<f32>,6> {
    var weights: array<vec2<f32>,6>;
    for (var j=0u; j<6u; j++) {
        let c = HERMITE[j];
        var value = t*t*t*(c.x+t*(c.y+t*c.z));
        var derivative = t*t*(3.0*c.x+t*(4.0*c.y+5.0*t*c.z));
        if j == 0u { value += 1.0; }
        if j == 1u { value += t; derivative += 1.0; }
        if j == 2u { value += 0.5*t*t; derivative += t; }
        weights[j] = vec2<f32>(value,derivative);
    }
    return weights;
}
fn nodal_index(dims: vec3<u32>, cell: vec3<u32>, j: vec3<u32>) -> u32 {
    let orders = select(vec3<u32>(1u),vec3<u32>(3u),dims>vec3<u32>(1u));
    let node = cell+j/vec3<u32>(3u);
    let d = j%vec3<u32>(3u);
    let component = d.x+orders.x*(d.y+orders.y*d.z);
    return component*dims.x*dims.y*dims.z+(node.z*dims.y+node.y)*dims.x+node.x;
}
fn nodal_value(base: u32, dims: vec3<u32>, cell: vec3<u32>, t: vec3<f32>) -> f32 {
    let coordinate = select(vec3<f32>(0.0),t,dims>vec3<u32>(1u));
    let wx = hermite_weights(coordinate.x);
    let wy = hermite_weights(coordinate.y);
    let wz = hermite_weights(coordinate.z);
    let limits = select(vec3<u32>(1u),vec3<u32>(6u),dims>vec3<u32>(1u));
    var value = 0.0;
    for (var z=0u; z<limits.z; z++) {
        var vy = 0.0;
        for (var y=0u; y<limits.y; y++) {
            var vx = 0.0;
            for (var x=0u; x<limits.x; x++) {
                vx += wx[x].x*transport_data[base+nodal_index(dims,cell,vec3<u32>(x,y,z))];
            }
            vy += wy[y].x*vx;
        }
        value += wz[z].x*vy;
    }
    return value;
}
// Contract the shared nodal derivative jets and tensor quintic basis
// used by expand_cell. Large magnetic arrays need not allocate 216 powers/cell.
fn nodal_gradient(cell: vec3<u32>, t: vec3<f32>) -> vec3<f32> {
    let dims = p.dims.xyz;
    let coordinate = select(vec3<f32>(0.0),t,dims>vec3<u32>(1u));
    let wx = hermite_weights(coordinate.x);
    let wy = hermite_weights(coordinate.y);
    let wz = hermite_weights(coordinate.z);
    let limits = select(vec3<u32>(1u),vec3<u32>(6u),dims>vec3<u32>(1u));
    var gradient = vec3<f32>(0.0);
    for (var z=0u; z<limits.z; z++) {
        var vy = 0.0;
        var dx = 0.0;
        var dy = 0.0;
        for (var y=0u; y<limits.y; y++) {
            var vx = 0.0;
            var derivative = 0.0;
            for (var x=0u; x<limits.x; x++) {
                let index = nodal_index(dims,cell,vec3<u32>(x,y,z));
                let value = coefficient(index);
                vx += wx[x].x*value;
                derivative += wx[x].y*value;
            }
            vy += wy[y].x*vx;
            dx += wy[y].x*derivative;
            dy += wy[y].y*vx;
        }
        gradient += vec3<f32>(wz[z].x*dx,wz[z].x*dy,wz[z].y*vy);
    }
    return gradient;
}

fn root_vector(v: vec3<f32>) -> vec3<f32> {
    if !finite(v) { field_invalid = true; return vec3<f32>(0.0); }
    return root.magnetic.w*vec3<f32>(dot(root.axes[0].xyz,v),dot(root.axes[1].xyz,v),dot(root.axes[2].xyz,v));
}
fn mapped_coils(point: vec3<f32>) -> vec4<f32> {
    let header = root.physics_padding.w;
    if header == 0u { return vec4<f32>(0.0); }
    let base = u32(transport_data[header])+(u32(transport_data[header+1u])<<16u);
    if base == 0u { return vec4<f32>(0.0); }
    let origin = vec3<f32>(transport_data[base+1u],transport_data[base+2u],transport_data[base+3u]);
    let axis = vec3<f32>(transport_data[base+4u],transport_data[base+5u],transport_data[base+6u]);
    let delta = point-origin;
    let x = dot(delta,axis);
    let radial = delta-x*axis;
    let r = length(radial);
    for (var block=0u; block<u32(transport_data[base]); block++) {
        let record_base = base+7u+12u*block;
        let t = vec2<f32>((x-transport_data[record_base])/transport_data[record_base+2u],r/transport_data[record_base+3u]);
        let cells = vec2<u32>(u32(transport_data[record_base+4u]),u32(transport_data[record_base+5u]));
        if any(t<vec2<f32>(0.0)) || any(t>vec2<f32>(cells)) { continue; }
        let cell = min(vec2<u32>(floor(t)),cells-vec2<u32>(1u));
        let fraction = t-vec2<f32>(cell);
        let dims = vec3<u32>(cells+vec2<u32>(1u),1u);
        var values = vec2<f32>(0.0);
        for (var component=0u; component<2u; component++) {
            let offset = u32(transport_data[record_base+8u+2u*component])+(u32(transport_data[record_base+9u+2u*component])<<16u);
            values[component] = nodal_value(offset,dims,vec3<u32>(cell,0u),vec3<f32>(fraction,0.0));
        }
        return vec4<f32>(values.x*axis+values.y*radial,1.0);
    }
    return vec4<f32>(0.0);
}
fn imported_local(record_base: u32, point: vec3<f32>) -> vec3<f32> {
    let origin = vec3<f32>(transport_data[record_base],transport_data[record_base+1u],transport_data[record_base+2u]);
    let d = point-origin;
    var local = vec3<f32>(0.0);
    for (var a=0u;a<3u;a++) {
        let k=record_base+3u+3u*a;
        local[a]=dot(vec3<f32>(transport_data[k],transport_data[k+1u],transport_data[k+2u]),d)/transport_data[record_base+12u];
    }
    return local;
}
fn imported_dims(record_base: u32) -> vec3<u32> {
    return vec3<u32>(u32(transport_data[record_base+13u]),u32(transport_data[record_base+14u]),u32(transport_data[record_base+15u]));
}
fn imported_spacing(record_base: u32) -> vec3<f32> {
    return vec3<f32>(transport_data[record_base+16u],transport_data[record_base+17u],transport_data[record_base+18u]);
}
fn imported_coordinates(record_base: u32, local: vec3<f32>) -> vec3<f32> {
    var q=local;
    if transport_data[record_base+19u]!=0.0 { q=vec3<f32>(q.x,length(q.yz),0.0); }
    let mirrors=u32(transport_data[record_base+21u]);
    for(var a=0u;a<3u;a++) { if (mirrors&(1u<<a))!=0u { q[a]=abs(q[a]); } }
    return q/imported_spacing(record_base);
}
fn imported_contains(record_base: u32, local: vec3<f32>) -> bool {
    let dims=imported_dims(record_base);
    let q=imported_coordinates(record_base,local);
    for(var a=0u;a<3u;a++) {
        if dims[a]>1u && (q[a]<0.0 || q[a]>f32(dims[a]-1u)) { return false; }
    }
    return true;
}
fn imported_status(point: vec3<f32>) -> Event {
    let base=root.physics_padding.w;
    if !IMPORTED_MAPS || base==0u { return Event(0.0,0u,0u); }
    for(var i=0u;i<u32(transport_data[base+2u]);i++) {
        let record_base=base+3u+31u*i;
        if transport_data[record_base+20u]==0.0 && !imported_contains(record_base,imported_local(record_base,point)) {
            return Event(0.0,2u,0u);
        }
    }
    return Event(0.0,0u,0u);
}
fn imported_intersection(point: vec3<f32>, delta: vec3<f32>) -> Event {
    let base=root.physics_padding.w;
    var event=Event(0.0,0u,0u);
    if !IMPORTED_MAPS || base==0u { return event; }
    for(var i=0u;i<u32(transport_data[base+2u]);i++) {
        let record_base=base+3u+31u*i;
        if transport_data[record_base+20u]!=0.0 { continue; }
        let a=imported_local(record_base,point);
        let b=imported_local(record_base,point+delta);
        if !imported_contains(record_base,a) { return Event(0.0,2u,0u); }
        if imported_contains(record_base,b) { continue; }
        // Each supported domain is convex (box or solid cylinder).
        var lo=0.0;
        var hi=1.0;
        for(var j=0u;j<48u;j++) {
            let mid=0.5*(lo+hi);
            if imported_contains(record_base,a+mid*(b-a)) { lo=mid; } else { hi=mid; }
        }
        if event.code==0u || lo<event.t { event=Event(lo,2u,0u); }
    }
    return event;
}
fn imported_field(point: vec3<f32>) -> vec3<f32> {
    let base=root.physics_padding.w;
    var sum=vec3<f32>(0.0);
    if !IMPORTED_MAPS || base==0u { return sum; }
    for(var i=0u;i<u32(transport_data[base+2u]);i++) {
        let record_base=base+3u+31u*i;
        let local=imported_local(record_base,point);
        let outside=transport_data[record_base+20u];
        if outside==1.0 && !imported_contains(record_base,local) { continue; }
        let dims=imported_dims(record_base);
        var q=imported_coordinates(record_base,local);
        if outside==2.0 { q=clamp(q,vec3<f32>(0.0),vec3<f32>(dims-vec3<u32>(1u))); }
        q=select(q,vec3<f32>(0.0),dims==vec3<u32>(1u));
        let cells=max(dims,vec3<u32>(2u))-vec3<u32>(1u);
        let cell=vec3<u32>(clamp(floor(q),vec3<f32>(0.0),vec3<f32>(cells-vec3<u32>(1u))));
        let fraction=q-vec3<f32>(cell);
        var values=vec3<f32>(0.0);
        for(var c=0u;c<3u;c++) {
            let offset=u32(transport_data[record_base+25u+2u*c])+(u32(transport_data[record_base+26u+2u*c])<<16u);
            var sign=1.0;
            let anti=u32(transport_data[record_base+22u+c]);
            for(var a=0u;a<3u;a++) { if (anti&(1u<<a))!=0u && local[a]<0.0 { sign= -sign; } }
            values[c]=sign*nodal_value(offset,dims,cell,fraction);
        }
        if transport_data[record_base+19u]!=0.0 {
            var radial=local.yz;
            let radius=length(radial);
            let max_radius=f32(dims.y-1u)*imported_spacing(record_base).y;
            if outside==2.0 && radius>max_radius { radial*=max_radius/radius; }
            values=vec3<f32>(values.x,values.y*radial.x,values.y*radial.y);
        }
        for(var a=0u;a<3u;a++) {
            let k=record_base+3u+3u*a;
            sum+=values[a]*vec3<f32>(transport_data[k],transport_data[k+1u],transport_data[k+2u]);
        }
    }
    return sum;
}
fn coil_value(index: u32) -> f32 {
    if COILS <= 128u { return coil_tile[index]; }
    return transport_data[index];
}
fn coil_field(i: u32, point: vec3<f32>) -> vec3<f32> {
    let base = 12u*i;
    let first = vec3<f32>(coil_value(base),coil_value(base+1u),coil_value(base+2u));
    let direction = vec3<f32>(coil_value(base+4u),coil_value(base+5u),coil_value(base+6u));
    let current = coil_value(base+8u);
    if current == 0.0 { return vec3<f32>(0.0); }
    let r = point-first;
    if coil_value(base+3u) == 0.0 {
        let wire = direction-first;
        let size = length(wire);
        let axis = wire/size;
        let along = dot(r,axis);
        let radial = r-along*axis;
        let r2 = dot(radial,radial);
        if r2 == 0.0 {
            if along < 0.0 || along > size { return vec3<f32>(0.0); }
            return vec3<f32>(bitcast<f32>(0x7fc00000u));
        }
        let end = along-size;
        return cross(axis,radial)*(current/r2*(along/sqrt(r2+along*along)-end/sqrt(r2+end*end)));
    }
    let radius = coil_value(base+9u);
    let z = dot(r,direction);
    let radial = r-z*direction;
    let rho = length(radial);
    if rho < radius*1e-8 {
        let a = radius*radius+z*z;
        let axial = 6.2831853071795864769*current*radius*radius/(a*sqrt(a));
        let radial_factor = 9.4247779607693797154*current*radius*radius*z/(a*a*sqrt(a));
        return axial*direction+radial_factor*radial;
    }
    let d = (radius+rho)*(radius+rho)+z*z;
    let q = (radius-rho)*(radius-rho)+z*z;
    if q == 0.0 { return vec3<f32>(bitcast<f32>(0x7fc00000u)); }
    let m = 4.0*radius*rho/d;
    var a = 1.0;
    var b = sqrt(1.0-m);
    var sum = 0.5*m;
    var weight = 1.0;
    for (var j=0u; j<32u; j++) {
        let c = 0.5*(a-b);
        sum += weight*c*c;
        let next = 0.5*(a+b);
        b = sqrt(a*b);
        a = next;
        weight *= 2.0;
        if abs(c) < 2e-16*a { break; }
    }
    let k = 1.5707963267948966192/a;
    let e = k*(1.0-sum);
    let factor = 2.0*current/sqrt(d);
    let axial = factor*(k+(radius*radius-rho*rho-z*z)*e/q);
    let radial_factor = factor*z/(rho*rho)*(-k+(radius*radius+rho*rho+z*z)*e/q);
    return axial*direction+radial_factor*radial;
}
fn magnetic(q: Coordinate, time: f32) -> vec3<f32> {
    if !MAGNETIC { return vec3<f32>(0.0); }
    let point = root_world(q);
    var support_enabled = true;
    if root.physics.z != 0xffffffffu {
        let i = root.physics.z;
        p = instances[i];
        support_enabled = grid_status(to_instance(point)).code != 2u;
    }
    var b = imported_field(point);
    if support_enabled {
        let prescribed = analytic_magnetic(point+root.origin.xyz,time);
        if !finite(prescribed) || !finite(root.magnetic.xyz) {
            field_invalid = true; p = root; return vec3<f32>(0.0);
        }
        b += root.magnetic.xyz+prescribed;
        if COILS != 0u {
            let mapped = mapped_coils(point);
            if mapped.w != 0.0 {
                b += mapped.xyz;
            } else {
                for (var i=0u; i<root.physics.w; i++) {
                    let field = coil_field(i,point);
                    if !finite(field) { field_invalid = true; p = root; return vec3<f32>(0.0); }
                    b += field;
                }
            }
        }
    }
    if MAGNETIC_PAS == 0u { p = root; return root_vector(b); }
    for (var i=root.physics.x; i<root.physics.x+MAGNETIC_PAS; i++) {
        p = instances[i];
        let local = to_instance(point);
        if grid_status(local).code == 2u { continue; }
        var enabled = true;
        if p.magnetic.z != 0.0 {
            let axis = u32(p.magnetic.z)-1u;
            let absolute = point+root.origin.xyz;
            var angle = atan2(absolute[(axis+1u)%3u],absolute[(axis+2u)%3u]);
            if angle < 0.0 { angle += 6.2831853071795864769; }
            enabled = angle >= p.bounds[0].w && angle <= p.bounds[1].w;
        }
        if enabled {
            var sign = 1.0;
            if p.magnetic.y != 0.0 {
                let axis = u32(p.magnetic.y)-1u;
                if local.cell[axis] < 0 { sign = -1.0; }
            }
            let field = instance_electric(local,time)*p.magnetic.x*sign;
            b += p.axes[0].xyz*field.x+p.axes[1].xyz*field.y+p.axes[2].xyz*field.z;
        }
        break;
    }
    p = root;
    return root_vector(b);
}
fn automatic_step(q: Coordinate, u: vec3<f32>, ratio: f32, time: f32) -> f32 {
    let speed = length(velocity(u));
    var gamma = 1.0;
    if root.counts.z != 0u { gamma = sqrt(1.0+dot(u,u)/(299792.458*299792.458)); }
    let acceleration = abs(ratio)*96.48533215665327*length(electric(q,time))/gamma;
    let distance = root.step_limits.x*root.spacing.w;
    var h = root.integration.w;
    if acceleration > 0.0 {
        h = min(h,2.0*distance/(speed+sqrt(speed*speed+2.0*acceleration*distance)));
    } else if speed > 0.0 { h = min(h,distance/speed); }
    let omega = abs(ratio)*0.009648533215665327*length(magnetic(q,time))/gamma;
    if omega > 0.0 { h = min(h,root.step_limits.y/omega); }
    return min(h,root.step_limits.z);
}
// Philox4x32-10. Products are split into 16-bit limbs because portable WGSL
// does not require 64-bit integer arithmetic. No floating arithmetic keys RNG.
fn multiply_wide(a: u32, b: u32) -> vec2<u32> {
    let a0 = a&65535u;
    let a1 = a>>16u;
    let b0 = b&65535u;
    let b1 = b>>16u;
    let low = a0*b0;
    let mid = a1*b0+(low>>16u);
    let high = (mid>>16u)+a1*b1;
    let mid2 = (mid&65535u)+a0*b1;
    return vec2<u32>((mid2<<16u)|(low&65535u),high+(mid2>>16u));
}
fn philox(input_count: vec4<u32>, input_key: vec2<u32>) -> vec4<u32> {
    var c = input_count;
    var k = input_key;
    for (var round=0u; round<10u; round++) {
        let a = multiply_wide(0xd2511f53u,c.x);
        let b = multiply_wide(0xcd9e8d57u,c.z);
        c = vec4<u32>(b.y^c.y^k.x,b.x,a.y^c.w^k.y,a.x);
        k += vec2<u32>(0x9e3779b9u,0xbb67ae85u);
    }
    return c;
}
fn random_uniform() -> f32 {
    let c = philox(rng_count,rng_key);
    rng_count.x++;
    if rng_count.x == 0u { rng_count.y++; }
    return (f32(c.x>>9u)+0.5)/8388608.0;
}
fn random_normal() -> f32 {
    var normal = 0.0;
    loop {
        let x = 2.0*random_uniform()-1.0;
        let y = 2.0*random_uniform()-1.0;
        let s = x*x+y*y;
        if s > 0.0 && s < 1.0 { normal = x*sqrt(-2.0*log(s)/s); break; }
    }
    return normal;
}
fn random_sphere(radius: f32) -> vec3<f32> {
    let z = 2.0*random_uniform()-1.0;
    let phi = 6.2831853071795864769*random_uniform();
    let r = radius*sqrt(1.0-z*z);
    return vec3<f32>(r*cos(phi),r*sin(phi),radius*z);
}
fn collision_rate(u: vec3<f32>) -> f32 {
    if root.collision_limits.z == 0.0 { return 0.0; }
    let speed = length(u-root.gas.xyz);
    if root.collision.w > 0.0 { return max(speed,1e-7)/root.collision.w; }
    let star = root.collision_limits.x;
    let s = speed/star;
    let mean = 1.1283791670955125739*star;
    var relative = mean*(1.0+s*s/3.0-s*s*s*s/30.0);
    if s >= 1e-4 {
        let t = 1.0/(1.0+0.32759109962*s);
        let poly = ((((-1.061405429*t+1.453152027)*t-1.421413741)*t+0.2844966736)*t-0.254829592)*t;
        let erf = 1.0+poly*exp(-s*s);
        relative = mean*((s+0.5/s)*0.8862269254527580136*erf+0.5*exp(-s*s));
    }
    return root.collision.z*relative;
}
fn scatter(u: vec3<f32>) -> vec3<f32> {
    let v = u-root.gas.xyz;
    let sigma = root.collision.y;
    let bound = length(v)+sigma*1.732*3.0;
    var gas = vec3<f32>(0.0);
    loop {
        gas = sigma*vec3<f32>(random_normal(),random_normal(),random_normal());
        if random_uniform() < length(gas-v)/bound { break; }
    }
    let outgoing = random_sphere(length(v-gas));
    return (ion_mass*v+root.gas.w*(gas+outgoing))/(ion_mass+root.gas.w)+root.gas.xyz;
}
fn sds_quantile(percentile: f32) -> f32 {
    let n = percentile*1000.0;
    let low = u32(floor(n));
    let weight = n-f32(low);
    let log_mass = sds_transport.w;
    let table = u32(clamp(ceil(log_mass)-1.0,0.0,3.0));
    let start = u32(root.collision_limits.w)+table*1002u+low;
    let av = transport_data[start]+weight*(transport_data[start+1u]-transport_data[start]);
    let bv = transport_data[start+1002u]+weight*(transport_data[start+1003u]-transport_data[start+1002u]);
    return exp(log(av)+(log_mass-f32(table))*(log(bv)-log(av)));
}
fn collision_accepted(initial_u: vec3<f32>, input: Phase, h: f32, hazard: ptr<function,f32>) -> Phase {
    var next = input;
    if COLLISION == 1u {
        *hazard -= collision_rate(initial_u)*h;
        if *hazard <= 1e-12 {
            next.u = scatter(next.u);
            *hazard = -log(random_uniform());
        }
    } else if COLLISION == 2u && root.collision_limits.z != 0.0 {
        let distance = sds_quantile(random_uniform());
        let n = sds_transport.z/sds_transport.y*h;
        next.d += random_sphere(sqrt(n/100000.0)*distance*sds_transport.y);
    }
    return next;
}
struct Integrals { decay: vec2<f32>, f1: vec2<f32>, f2: vec2<f32> }
fn integrals(damping: f32, omega: f32, h: f32) -> Integrals {
    let z = vec2<f32>(-damping*h,omega*h);
    let decay = exp(z.x)*vec2<f32>(cos(z.y),sin(z.y));
    if length(z) < 0.1 {
        var term = vec2<f32>(1.0,0.0);
        var f1 = term;
        var f2 = 0.5*term;
        for (var n=1u; n<20u; n++) {
            term = vec2<f32>(term.x*z.x-term.y*z.y,term.x*z.y+term.y*z.x)/f32(n+1u);
            f1 += term;
            f2 += term/f32(n+2u);
        }
        return Integrals(decay,f1*h,f2*h*h);
    }
    let denominator = damping*damping+omega*omega;
    let a = decay-vec2<f32>(1.0,0.0);
    let f1 = vec2<f32>(-damping*a.x+omega*a.y,-omega*a.x-damping*a.y)/denominator;
    let b = f1-vec2<f32>(h,0.0);
    let f2 = vec2<f32>(-damping*b.x+omega*b.y,-omega*b.x-damping*b.y)/denominator;
    return Integrals(decay,f1,f2);
}
fn damped_apply(v: vec3<f32>, axis: vec3<f32>, perpendicular: vec2<f32>, parallel: f32) -> vec3<f32> {
    let projection = dot(v,axis);
    return perpendicular.x*(v-projection*axis)+perpendicular.y*cross(v,axis)+parallel*projection*axis;
}
fn damped_step(u: vec3<f32>, e: vec3<f32>, b: vec3<f32>, ratio: f32, h: f32) -> Phase {
    let magnitude = length(b);
    var axis = vec3<f32>(1.0,0.0,0.0);
    if magnitude != 0.0 { axis = b/magnitude; }
    let damping = sds_transport.x;
    let a = integrals(damping,ratio*0.009648533215665327*magnitude,h);
    let parallel = integrals(damping,0.0,h);
    let force = ratio*96.48533215665327*e+damping*root.gas.xyz;
    return Phase(
        damped_apply(u,axis,a.f1,parallel.f1.x)+damped_apply(force,axis,a.f2,parallel.f2.x),
        damped_apply(u,axis,a.decay,parallel.decay.x)+damped_apply(force,axis,a.f1,parallel.f1.x));
}
fn sds_advance(q: Coordinate, u: vec3<f32>, ratio: f32, time: f32, h: f32) -> Phase {
    let mid = damped_step(u,electric(q,time),magnetic(q,time),ratio,0.5*h);
    let at = moved(q,mid.d);
    return damped_step(u,electric(at,time+0.5*h),magnetic(at,time+0.5*h),ratio,h);
}
fn deterministic_trial(q: Coordinate, u: vec3<f32>, h: f32, ratio: f32, time: f32) -> Trial {
    if COLLISION == 2u {
        let full = sds_advance(q,u,ratio,time,h);
        let half = sds_advance(q,u,ratio,time,0.5*h);
        var two = sds_advance(moved(q,half.d),half.u,ratio,time+0.5*h,0.5*h);
        two.d += half.d;
        let x = physical(q);
        let pos_scale = vec3<f32>(root.tolerance.y)+root.tolerance.x*max(abs(x),abs(x+two.d));
        let vel_scale = vec3<f32>(root.tolerance.z)+root.tolerance.x*max(abs(u),abs(two.u));
        let error = max(abs(two.d-full.d)/pos_scale,abs(two.u-full.u)/vel_scale)/3.0;
        return Trial(two,max(error.x,max(error.y,error.z)));
    }
    if !ELECTRIC && !MAGNETIC {
        return Trial(Phase(h*velocity(u),u),0.0);
    }
    if BORIS {
        let v = velocity(u);
        let mid = moved(q,0.5*h*v);
        let e = electric(mid,time+0.5*h);
        let minus = u+0.5*h*96.48533215665327*ratio*e;
        var gamma = 1.0;
        if root.counts.z != 0u { gamma = sqrt(1.0+dot(minus,minus)/(299792.458*299792.458)); }
        let t = 0.5*h*0.009648533215665327*ratio*magnetic(mid,time+0.5*h)/gamma;
        let prime = minus+cross(minus,t);
        let plus = minus+cross(prime,2.0*t/(1.0+dot(t,t)));
        let next = plus+0.5*h*96.48533215665327*ratio*e;
        return Trial(Phase(0.5*h*(v+velocity(next)),next),0.0);
    }
    return rk45(q,u,h,ratio,time);
}
