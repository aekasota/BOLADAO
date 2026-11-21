export const VERTEX_SHADER_GLSL = `#version 300 es
precision highp float;
const vec2 POS[3] = vec2[3](
  vec2(-1.0, -1.0),
  vec2( 3.0, -1.0),
  vec2(-1.0,  3.0)
);
void main() {
  gl_Position = vec4(POS[gl_VertexID], 0.0, 1.0);
}
`;

export const FRAGMENT_SHADER_GLSL = `#version 300 es
precision highp float;

uniform vec2  uRes;        // canvas px
uniform float uTime;       // seconds
uniform float uPhase;      // integrated flow phase (linked to network MB/s)
uniform vec2  uMouse;      // normalized p-space
uniform vec2  uMouseVel;   // smoothed cursor velocity in p-space
uniform float uForce;      // 0..1 damped spring
uniform vec4  uRipple;     // xy center in p-space, z age (s), w strength
uniform vec3  uAttract;    // xy target in p-space (URL bar), z strength (drag-over)
uniform vec4  uStage;      // stage rect in GL px (x, y_bottom, w, h)
uniform vec4  uHole;       // badge cutout rect in GL px (x, y_bottom, w, h) — always 100% contrast!
uniform float uOutside;    // contrast outside stage (~0.12)
uniform float uDensity;    // user stripe density multiplier (0.7..1.4)
uniform float uDuty;       // ~0.60 = band A proportion
uniform vec3  uColA;       // e.g. #FA023E
uniform vec3  uColB;       // e.g. #161616
uniform vec3  uBg;         // app background outside stage

out vec4 outColor;

float hash(vec2 p) {
  p = fract(p * vec2(123.34, 456.21));
  p += dot(p, p + 45.32);
  return fract(p.x * p.y);
}

float noise(vec2 p) {
  vec2 i = floor(p);
  vec2 f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  return mix(
    mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x),
    mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x),
    u.y
  );
}

float fbm(vec2 p) {
  float s = 0.0;
  float a = 0.56;
  mat2 m = mat2(1.6, 1.2, -1.2, 1.6);
  for (int i = 0; i < 3; i++) {
    s += a * noise(p);
    p = m * p;
    a *= 0.34;
  }
  return s;
}

mat2 rot(float a) {
  float c = cos(a), s = sin(a);
  return mat2(c, -s, s, c);
}

float boxSignedInside(vec2 px, vec4 rect) {
  vec2 lo = rect.xy;
  vec2 hi = rect.xy + rect.zw;
  return min(min(px.x - lo.x, hi.x - px.x), min(px.y - lo.y, hi.y - px.y));
}

void main() {
  vec2 p = (gl_FragCoord.xy - 0.5 * uRes) / max(uRes.y, 1.0);

  // 1. Cursor lens + swirl + velocity drag with Gaussian falloff
  vec2 d = p - uMouse;
  float fall = exp(-dot(d, d) / 0.038) * uForce;
  p += d * fall * 0.52;
  p = uMouse + rot(fall * 1.85) * (p - uMouse);
  p -= uMouseVel * fall * 0.14;

  // 2. Drag-over attraction well pulling bands toward the URL input
  if (uAttract.z > 0.001) {
    vec2 ad = p - uAttract.xy;
    float afall = exp(-dot(ad, ad) / 0.22) * uAttract.z;
    p -= ad * afall * 0.36;
  }

  // 3. Click / Download-completed radial ripple
  if (uRipple.w > 0.001 && uRipple.z < 3.5) {
    vec2 rd = p - uRipple.xy;
    float rr = length(rd);
    float ring = sin(rr * 38.0 - uRipple.z * 9.5)
               * exp(-rr * 3.4)
               * exp(-uRipple.z * 1.55)
               * uRipple.w;
    p += normalize(rd + 1e-5) * ring * 0.028;
  }

  // 4. Domain-warped topographic field (smooth bold ribbons like the reference banner)
  float t = uTime * 0.028;
  vec2 sp = p * 0.68;
  vec2 q = vec2(
    fbm(sp + t),
    fbm(sp + vec2(5.2, 1.3) - t)
  );
  vec2 r = vec2(
    fbm(sp + 2.1 * q + vec2(1.7, 9.2) + 0.55 * t),
    fbm(sp + 2.1 * q + vec2(8.3, 2.8) - 0.38 * t)
  );
  float f = fbm(sp * 0.82 + 1.95 * r);

  // 5. Spatially varying frequency: visibly denser toward the right, as in the reference banner
  float freq = mix(5.8, 15.5, smoothstep(-0.92, 0.92, p.x + 0.28 * fbm(sp * 0.55))) * uDensity;
  float v = f * freq - uPhase;

  // 6. Sharp contour bands with 60/40 duty cycle and derivative anti-aliasing
  float c  = cos(6.2831853 * v);
  float th = cos(3.1415927 * uDuty);
  float a  = smoothstep(-1.0, 1.0, (c - th) / max(fwidth(c), 1e-4));

  // 7. Anti-moiré: blend toward mean duty when bands compress below ~2 px
  float thin = smoothstep(0.35, 0.72, fwidth(v));
  a = mix(a, uDuty, thin);

  // 8. Uniform flat two-tone Op-Art across the entire background (zero vignette / halo around the search box!)
  vec2 px = gl_FragCoord.xy;
  float insideHole = boxSignedInside(px, uHole);
  float kHole = smoothstep(-2.0, 1.0, insideHole);
  float k = max(clamp(uOutside, 0.0, 1.0), kHole);

  vec3 patternCol = mix(uColB, uColA, a);
  vec3 col = mix(uBg, patternCol, k);
  outColor = vec4(col, 1.0);
}
`;
