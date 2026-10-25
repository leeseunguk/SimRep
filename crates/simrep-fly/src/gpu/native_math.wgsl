// Native Vulkan supports double arithmetic and sqrt, but the SPIR-V GLSL
// transcendental instructions take f32. Keep RF phase and amplitude in f64.
fn native_cos(input: f64) -> f64 {
    let pi = 3.14159265358979323846;
    var x = input - floor(input / (2.0*pi) + 0.5) * (2.0*pi);
    var sign = 1.0;
    if x > pi/2.0 { x = pi-x; sign = -1.0; }
    if x < -pi/2.0 { x = -pi-x; sign = -1.0; }
    let z = x*x;
    var term = 1.0;
    var sum = 1.0;
    for (var k = 1; k <= 10; k++) {
        term *= -z / f64((2*k-1)*(2*k));
        sum += term;
    }
    return sign*sum;
}
fn native_sin(x: f64) -> f64 {
    return native_cos(x - 1.57079632679489661923);
}
fn native_exp(x: f64) -> f64 {
    let n = floor(x / 0.69314718055994530942 + 0.5);
    let r = x - n*0.69314718055994530942;
    var term = 1.0;
    var sum = 1.0;
    for (var k = 1; k <= 16; k++) {
        term *= r/f64(k);
        sum += term;
    }
    return ldexp(sum, i32(n));
}
fn native_log(input: f64) -> f64 {
    if input == 0.0 { return f64(bitcast<NARROW>(0xff800000u)); }
    if input < 0.0 { return f64(bitcast<NARROW>(0x7fc00000u)); }
    if !(input <= 1.7976931348623157e308) { return input; }
    var x = input;
    var n = 0.0;
    while x > 1.4142135623730951 { x *= 0.5; n += 1.0; }
    while x < 0.7071067811865476 { x *= 2.0; n -= 1.0; }
    let z = (x-1.0)/(x+1.0);
    let z2 = z*z;
    var term = z;
    var sum = z;
    for (var k = 1; k <= 14; k++) {
        term *= z2;
        sum += term/f64(2*k+1);
    }
    return 2.0*sum + n*0.69314718055994530942;
}
fn native_pow(x: f64, y: f64) -> f64 {
    if x == 0.0 {
        if y < 0.0 { return f64(bitcast<NARROW>(0x7f800000u)); }
        return select(0.0, 1.0, y == 0.0);
    }
    if x < 0.0 {
        if floor(y) != y { return f64(bitcast<NARROW>(0x7fc00000u)); }
        let value = native_exp(y*native_log(-x));
        return select(value,-value,abs(y%2.0) == 1.0);
    }
    return native_exp(y*native_log(x));
}
fn native_atan2(y: f64, x: f64) -> f64 {
    let pi = 3.14159265358979323846;
    if x == 0.0 {
        if y == 0.0 { return 0.0; }
        return select(-0.5*pi, 0.5*pi, y > 0.0);
    }
    var z = abs(y/x);
    let invert = z > 1.0;
    if invert { z = 1.0/z; }
    let shift = z > 0.4142135623730950488;
    if shift { z = (z-1.0)/(z+1.0); }
    var term = z;
    var a = z;
    for (var n = 1u; n <= 24u; n++) {
        term *= -z*z;
        a += term/f64(2u*n+1u);
    }
    if shift { a += pi*0.25; }
    if invert { a = pi*0.5-a; }
    if x < 0.0 { a = pi-a; }
    return select(a, -a, y < 0.0);
}
