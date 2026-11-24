//! From the drawn picture to the window, on the graphics card: smoothing
//! (option "smooth graphics", the xBR filter of xbr.rs at twice the size)
//! and stretching to the size the window shows (as upscale.rs does).
//!
//! Both give exactly the pixels the processor versions give: the shaders
//! reckon in whole numbers as those do, and the shares of the stretch are
//! worked out by upscale.rs (once per size). The processor versions stay
//! for the pictures made without a window (--screenshot); the test command
//! `gpucheck` compares the two.

/// What the three smoothing passes share.
const XBR_PRELUDE: &str = r#"
struct Out { @builtin(position) pos: vec4<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

// Y, U, V packed as in the grid (Y | U + 128 << 8 | V + 128 << 16)
fn unpack(v: u32) -> vec3<i32> {
    return vec3<i32>(i32(v & 255u), i32((v >> 8u) & 255u) - 128, i32((v >> 16u) & 255u) - 128);
}

fn dd(a: vec3<i32>, b: vec3<i32>) -> i32 {
    let d = abs(a - b);
    return 48 * d.x + 7 * d.y + 6 * d.z;
}
"#;

/// Smoothing, pass 1 (as in `xbr::scale2x_keeping`, step 1): the picture
/// with a border, and its Y, U, V worked out once per pixel.
const XBR_GRID: &str = r#"
@group(0) @binding(0) var pic: texture_2d<f32>;

// one pixel of the grid: the picture with a border of two pixels all round
// (the edge repeated), its colour and its Y, U, V
@fragment fn fs(o: Out) -> @location(0) vec2<u32> {
    let last = vec2<i32>(textureDimensions(pic)) - 1;
    let s = clamp(vec2<i32>(floor(o.pos.xy)) - 2, vec2<i32>(0), last);
    let v = textureLoad(pic, s, 0);
    let k = vec3<u32>(round(clamp(v.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0));
    let r = i32(k.r);
    let g = i32(k.g);
    let b = i32(k.b);
    let yuv = vec3<i32>((299 * r + 587 * g + 114 * b) / 1000, (-169 * r - 331 * g + 500 * b) / 1000, (500 * r - 419 * g - 81 * b) / 1000);
    return vec2<u32>((k.r << 16u) | (k.g << 8u) | k.b, u32(yuv.x) | (u32(yuv.y + 128) << 8u) | (u32(yuv.z + 128) << 16u));
}
"#;

/// Smoothing, pass 2 (step 3 there): per square of 2 x 2 the change along
/// its two diagonals, worked out once instead of for each of its pixels.
const XBR_DIAG: &str = r#"
@group(0) @binding(0) var grid: texture_2d<u32>;

fn at(g: vec2<i32>, last: vec2<i32>) -> vec3<i32> {
    return unpack(textureLoad(grid, clamp(g, vec2<i32>(0), last), 0).y);
}

// per square of 2 x 2 in the grid (by its top left pixel E): the change
// along its "/" (x) and its "\" (y) diagonal, with the parallel neighbours;
// nothing where xbr.rs leaves it at nothing (near the top and bottom)
@fragment fn fs(o: Out) -> @location(0) vec2<i32> {
    let size = vec2<i32>(textureDimensions(grid));
    let last = size - 1;
    let p = vec2<i32>(floor(o.pos.xy));
    let pw = size.x;
    let k = p.y * pw + p.x;
    if k < 2 * pw + 1 || k + 2 * pw + 2 >= pw * size.y {
        return vec2<i32>(0);
    }
    let e = at(p, last);
    let f = at(p + vec2<i32>(1, 0), last);
    let h = at(p + vec2<i32>(0, 1), last);
    let i = at(p + vec2<i32>(1, 1), last);
    let b = at(p + vec2<i32>(0, -1), last);
    let d = at(p + vec2<i32>(-1, 0), last);
    let c = at(p + vec2<i32>(1, -1), last);
    let g = at(p + vec2<i32>(-1, 1), last);
    let h5 = at(p + vec2<i32>(0, 2), last);
    let f4 = at(p + vec2<i32>(2, 0), last);
    let i4 = at(p + vec2<i32>(2, 1), last);
    let i5 = at(p + vec2<i32>(1, 2), last);
    return vec2<i32>(dd(e, c) + dd(e, g) + dd(i, h5) + dd(i, f4) + (dd(h, f) << 2u),
                     dd(h, d) + dd(h, i5) + dd(f, i4) + dd(f, b) + (dd(e, i) << 2u));
}
"#;

/// Smoothing, pass 3 (step 4 there), one source pixel per call: its four
/// new pixels, put out together (as 0x00RRGGBB in one texel). The
/// helpers; the main function follows from `xbr_main`.
const XBR_HEAD: &str = r#"
@group(0) @binding(0) var grid: texture_2d<u32>;
@group(0) @binding(1) var diags: texture_2d<i32>;
@group(0) @binding(2) var sharp: texture_2d<f32>;

fn same(a: vec3<i32>, b: vec3<i32>) -> bool {
    let d = abs(a - b);
    return d.x <= 48 && d.y <= 7 && d.z <= 6;
}

fn blend(dst: u32, src: u32, k: u32) -> u32 {
    let d = (vec3<u32>(dst) >> vec3<u32>(16u, 8u, 0u)) & vec3<u32>(255u);
    let c = (vec3<u32>(src) >> vec3<u32>(16u, 8u, 0u)) & vec3<u32>(255u);
    let m = (d * (256u - k) + c * k) >> vec3<u32>(8u);
    return (m.x << 16u) | (m.y << 8u) | m.z;
}

// one corner (see xbr.rs `corner`): the new values of the three pixels
// n1, n2, n3 it can change
fn corner(ee: i32, ii: i32, ef: i32, eh: i32,
          ce: u32, ci: u32, ch: u32, cf: u32, cg: u32, cc: u32, cd: u32, cb: u32,
          e: vec3<i32>, i: vec3<i32>, h: vec3<i32>, f: vec3<i32>, g: vec3<i32>, c: vec3<i32>, d: vec3<i32>, b: vec3<i32>,
          i4: vec3<i32>, i5: vec3<i32>, n1: u32, n2: u32, n3: u32) -> vec3<u32> {
    if ee > ii || ce == ch || ce == cf {
        return vec3<u32>(n1, n2, n3);
    }
    let px = select(ch, cf, ef <= eh);
    let shape = (!same(f, b) && !same(h, d)) || (same(e, i) && !same(f, i4) && !same(h, i5)) || same(e, g) || same(e, c);
    if ee < ii && shape {
        let ke = dd(f, g);
        let ki = dd(h, c);
        let ex2 = ce != cc && cb != cc;
        let ex3 = ce != cg && cd != cg;
        let shallow = (ke << 1u) <= ki && ex3;
        let steep = ke >= (ki << 1u) && ex2;
        if shallow && steep {
            let m2 = blend(n2, px, 64u);
            return vec3<u32>(m2, m2, blend(n3, px, 224u));
        } else if shallow {
            return vec3<u32>(n1, blend(n2, px, 64u), blend(n3, px, 192u));
        } else if steep {
            return vec3<u32>(blend(n1, px, 64u), n2, blend(n3, px, 192u));
        }
        return vec3<u32>(n1, n2, blend(n3, px, 128u));
    }
    return vec3<u32>(n1, n2, blend(n3, px, 64u));
}

"#;

/// Smoothing, pass 3: its main function. The neighbours of the source
/// pixel are named by their place in its square of 5 x 5, column then row
/// ("22" is the pixel itself, "32" the one to its right, "21" the one
/// above): `t` is the grid's texel, `c` its colour, `y` its Y, U, V.
///
/// Written out pixel by pixel rather than as loops over arrays, so that the
/// shader compiler keeps every value in a register. The four next to the
/// pixel come first: they are enough to find the flat pixels (most of
/// them), which are done then.
fn xbr_main() -> String {
    use std::fmt::Write;
    const NEXT: [(i32, i32); 5] = [(0, 0), (0, -1), (-1, 0), (1, 0), (0, 1)];
    // the four corners, each seen as if it were the bottom right one: its
    // two diagonals, the changes towards its F and its H, its neighbours
    // E I H F G C D B I4 I5 (as `corner` takes them) and its three new
    // pixels n1 n2 n3 (0 top left, 1 top right, 2 bottom left, 3 bottom right)
    const CORNERS: [(&str, &str, &str, &str, [(i32, i32); 10], [usize; 3]); 4] = [
        ("se.x", "se.y", "er", "ed", [(0, 0), (1, 1), (0, 1), (1, 0), (-1, 1), (1, -1), (-1, 0), (0, -1), (2, 1), (1, 2)], [1, 2, 3]),
        ("sb.y", "sb.x", "eu", "er", [(0, 0), (1, -1), (1, 0), (0, -1), (1, 1), (-1, -1), (0, 1), (-1, 0), (1, -2), (2, -1)], [0, 3, 1]),
        ("sa.x", "sa.y", "el", "eu", [(0, 0), (-1, -1), (0, -1), (-1, 0), (1, -1), (-1, 1), (1, 0), (0, 1), (-2, -1), (-1, -2)], [2, 1, 0]),
        ("sd.y", "sd.x", "ed", "el", [(0, 0), (-1, 1), (-1, 0), (0, 1), (-1, -1), (1, 1), (0, -1), (1, 0), (-1, 2), (-2, 1)], [3, 0, 2]),
    ];
    const XYZW: [&str; 4] = ["x", "y", "z", "w"];
    let name = |(dx, dy): (i32, i32)| format!("{}{}", dx + 2, dy + 2);
    let load = |s: &mut String, (dx, dy): (i32, i32)| {
        let n = name((dx, dy));
        let _ = writeln!(s, "    let t{n} = textureLoad(grid, g + vec2<i32>({dx}, {dy}), 0);\n    let c{n} = t{n}.x;");
    };
    let mut s = String::from(
        "// one source pixel: its four new pixels (top left, top right, bottom left,\n\
         // bottom right)\n\
         @fragment fn fs(o: Out) -> @location(0) vec4<u32> {\n    \
         let p = vec2<i32>(floor(o.pos.xy));\n    \
         let g = p + 2;\n",
    );
    for q in NEXT {
        load(&mut s, q);
    }
    s += "    let ce = c22;\n    \
          // flat: the pixel differs from neither neighbour on any side\n    \
          let flat = (ce == c23 || ce == c32) && (ce == c32 || ce == c21) && (ce == c21 || ce == c12) && (ce == c12 || ce == c23);\n    \
          if flat || textureLoad(sharp, p, 0).r > 0.5 {\n        \
          return vec4<u32>(ce);\n    \
          }\n    \
          // the others only where the pixel is not flat\n";
    for dy in -2..=2 {
        for dx in -2..=2 {
            if !NEXT.contains(&(dx, dy)) {
                load(&mut s, (dx, dy));
            }
            let n = name((dx, dy));
            let _ = writeln!(s, "    let y{n} = unpack(t{n}.y);");
        }
    }
    s += "    let er = dd(y22, y32);\n    \
          let el = dd(y12, y22);\n    \
          let eu = dd(y21, y22);\n    \
          let ed = dd(y22, y23);\n    \
          // the diagonals of the squares with their top left at E, B, A and D\n    \
          let se = textureLoad(diags, g, 0).xy;\n    \
          let sb = textureLoad(diags, g + vec2<i32>(0, -1), 0).xy;\n    \
          let sa = textureLoad(diags, g + vec2<i32>(-1, -1), 0).xy;\n    \
          let sd = textureLoad(diags, g + vec2<i32>(-1, 0), 0).xy;\n    \
          var out = vec4<u32>(ce);\n";
    for (k, (ee, ii, ef, eh, near, new)) in CORNERS.iter().enumerate() {
        let list = |v: Vec<String>| v.join(", ");
        let cols = list(near[..8].iter().map(|&q| format!("c{}", name(q))).collect());
        let yuvs = list(near.iter().map(|&q| format!("y{}", name(q))).collect());
        let olds = list(new.iter().map(|&m| format!("out.{}", XYZW[m])).collect());
        let _ = writeln!(s, "    let r{k} = corner({ee}, {ii}, {ef}, {eh}, {cols}, {yuvs}, {olds});");
        let sets: Vec<String> = new.iter().enumerate().map(|(j, &m)| format!("out.{} = r{k}.{};", XYZW[m], XYZW[j])).collect();
        let _ = writeln!(s, "    {}", sets.join(" "));
    }
    s += "    return out;\n}\n";
    s
}

/// The stretch to the window: every window pixel mixes the picture's
/// pixels it covers by their shares (from `upscale::taps`), first along
/// the rows and then down the columns, rounded as upscale.rs rounds.
const STRETCH: &str = r#"
struct Out { @builtin(position) pos: vec4<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

@group(0) @binding(0) var pic: PIC;
// per window column (row): first pixel, how many, and up to six shares
@group(0) @binding(1) var tx: texture_2d<u32>;
@group(0) @binding(2) var ty: texture_2d<u32>;
// where the picture's top left corner is in the window; zw: how many
// window pixels the world is shifted by (fine scrolling, see `Scaler::shift`)
@group(0) @binding(3) var<uniform> at: vec4<i32>;

FETCH

fn share(a: vec4<u32>, b: vec4<u32>, k: u32) -> u32 {
    switch k {
        case 0u: { return a.z; }
        case 1u: { return a.w; }
        case 2u: { return b.x; }
        case 3u: { return b.y; }
        case 4u: { return b.z; }
        default: { return b.w; }
    }
}

@fragment fn fs(o: Out) -> @location(0) vec4<f32> {
    let n = vec2<i32>(i32(textureDimensions(tx).x), i32(textureDimensions(ty).x));
    var w = clamp(vec2<i32>(floor(o.pos.xy)) - at.xy, vec2<i32>(0), n - 1);
    var from_world = false;
    if (at.z != 0 || at.w != 0) {
        // fine scrolling: where the picture shows the world (nothing drawn
        // over it), the world a few window pixels further on
        let sx = i32(textureLoad(tx, vec2<i32>(w.x, 0), 0).x);
        let sy = i32(textureLoad(ty, vec2<i32>(w.y, 0), 0).x);
        if (all(fetch(sx, sy) == fetch_world(sx, sy))) {
            w = clamp(w + at.zw, vec2<i32>(0), n - 1);
            from_world = true;
        }
    }
    let x0 = textureLoad(tx, vec2<i32>(w.x, 0), 0);
    let x1 = textureLoad(tx, vec2<i32>(w.x, 1), 0);
    let y0 = textureLoad(ty, vec2<i32>(w.y, 0), 0);
    let y1 = textureLoad(ty, vec2<i32>(w.y, 1), 0);
    // (in floating point, which is faster on graphics cards than whole
    // numbers; every value stays a whole number below 2^24, so exact)
    var acc = vec3<f32>(0.0);
    for (var k = 0u; k < y0.y; k++) {
        let row = i32(y0.x + k);
        var line = vec3<f32>(0.0);
        for (var m = 0u; m < x0.y; m++) {
            var c = vec3<u32>(0u);
            if (from_world) { c = fetch_world(i32(x0.x + m), row); } else { c = fetch(i32(x0.x + m), row); }
            line += vec3<f32>(c) * f32(share(x0, x1, m));
        }
        acc += floor((line + 128.0) / 256.0) * f32(share(y0, y1, k));
    }
    return vec4<f32>(floor((acc + 128.0) / 256.0) / 255.0, 1.0);
}
"#;

/// The picture as drawn, for the stretch.
const FETCH_PLAIN: &str = r#"
// the world alone, before the windows were drawn over it (fine scrolling)
@group(0) @binding(4) var world: texture_2d<f32>;

fn fetch(x: i32, y: i32) -> vec3<u32> {
    let v = textureLoad(pic, vec2<i32>(x, y), 0);
    return vec3<u32>(round(clamp(v.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0));
}

fn fetch_world(x: i32, y: i32) -> vec3<u32> {
    let v = textureLoad(world, vec2<i32>(x, y), 0);
    return vec3<u32>(round(clamp(v.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0));
}
"#;

/// The smoothed picture (four pixels per texel, see XBR_HEAD), for the stretch.
const FETCH_SMOOTH: &str = r#"
// the world alone, smoothed as well (fine scrolling)
@group(0) @binding(4) var world: texture_2d<u32>;

fn one_of_four(v: vec4<u32>, x: i32, y: i32) -> vec3<u32> {
    var c = v.x;
    switch (y & 1) * 2 + (x & 1) {
        case 1: { c = v.y; }
        case 2: { c = v.z; }
        case 3: { c = v.w; }
        default: {}
    }
    return vec3<u32>((c >> 16u) & 255u, (c >> 8u) & 255u, c & 255u);
}

fn fetch(x: i32, y: i32) -> vec3<u32> {
    return one_of_four(textureLoad(pic, vec2<i32>(x, y) / 2, 0), x, y);
}

fn fetch_world(x: i32, y: i32) -> vec3<u32> {
    return one_of_four(textureLoad(world, vec2<i32>(x, y) / 2, 0), x, y);
}
"#;

/// The look of a tube monitor (option "graphics filter", CRT), after the
/// CRT shader by Timothy Lottes (public domain): each picture row is a
/// beam of light, bright in its middle and dark towards the next row (the
/// scanlines); across, the pixels run softly into each other; and the
/// window's pixel columns take turns in red, green and blue, as the stripes
/// of the tube's screen. Worked out in linear light, as a screen glows.
const CRT: &str = r#"
struct Out { @builtin(position) pos: vec4<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

@group(0) @binding(0) var pic: texture_2d<f32>;
// where the picture lies in the window: x, y, width, height
@group(0) @binding(1) var<uniform> at: vec4<i32>;

// how sharp the beam is across (the pixels of a row) and down (the rows):
// the more negative, the sharper; the stripes' brightness, dark and
// light; the brightness lost to the dark lines and stripes, given back
const HARD_PIX: f32 = -4.0;
const HARD_SCAN: f32 = -8.0;
const MASK_DARK: f32 = 0.75;
const MASK_LIGHT: f32 = 1.25;
const BOOST: f32 = 1.35;

fn fetch(p: vec2<i32>) -> vec3<f32> {
    let last = vec2<i32>(textureDimensions(pic)) - 1;
    let c = textureLoad(pic, clamp(p, vec2<i32>(0), last), 0).rgb;
    return pow(c, vec3<f32>(2.2));
}

fn gaus(d: f32, hard: f32) -> f32 {
    return exp2(hard * d * d);
}

// one row of the picture at the point `fx` across (in pixels from the
// left edge of pixel `base.x`), its pixels mixed by their distance
fn row(base: vec2<i32>, fx: f32, dy: i32) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var i = -2; i <= 2; i++) {
        let k = gaus(fx - 0.5 - f32(i), HARD_PIX);
        sum += fetch(base + vec2<i32>(i, dy)) * k;
        weight += k;
    }
    return sum / weight;
}

@fragment fn fs(o: Out) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(pic));
    // the window pixel's middle, in picture pixels
    let p = (o.pos.xy - vec2<f32>(at.xy)) / vec2<f32>(at.zw) * size;
    let base = vec2<i32>(floor(p));
    let f = p - floor(p);
    // the beams of the row itself and of the ones above and below
    var c = vec3<f32>(0.0);
    for (var dy = -1; dy <= 1; dy++) {
        c += row(base, f.x, dy) * gaus(f.y - 0.5 - f32(dy), HARD_SCAN);
    }
    // the stripes of red, green and blue
    var mask = vec3<f32>(MASK_DARK);
    let col = i32(floor(o.pos.x)) % 3;
    if col == 0 { mask.r = MASK_LIGHT; } else if col == 1 { mask.g = MASK_LIGHT; } else { mask.b = MASK_LIGHT; }
    c = clamp(c * mask * BOOST, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(pow(c, vec3<f32>(1.0 / 2.2)), 1.0);
}
"#;

/// How the picture is shown (option "graphics filter").
#[derive(Clone, Copy)]
pub enum Look<'a> {
    /// Sharp, only stretched.
    Plain,
    /// Smoothed (xBR), with the pixels marked in it left sharp (it may be empty).
    Xbr(&'a [bool]),
    /// As on a tube monitor.
    Crt,
}

/// More shares than this for one window pixel (a picture shrunk to less
/// than a sixth) are not foreseen.
const MAX_TAPS: usize = 6;

/// A texture with its size.
struct Tex {
    texture: wgpu::Texture,
    size: (u32, u32),
}

impl Tex {
    fn new(device: &wgpu::Device, size: (u32, u32), format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> Tex {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pagan"),
            size: wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        Tex { texture, size }
    }

    fn view(&self) -> wgpu::TextureView {
        self.texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn write(&self, queue: &wgpu::Queue, bytes: &[u8], bytes_per_pixel: u32) {
        let (w, h) = self.size;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &bytes[..(w * h * bytes_per_pixel) as usize],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * bytes_per_pixel), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }
}

fn as_bytes(v: &[u32]) -> &[u8] {
    // SAFETY: u32 has no padding and a stricter alignment than u8
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4) }
}

/// The shares for `src` pixels onto `dst` as a texture `dst` wide and two
/// high (see STRETCH); None if one pixel would need too many.
fn taps_texture(device: &wgpu::Device, queue: &wgpu::Queue, src: u32, dst: u32) -> Option<Tex> {
    let taps = crate::upscale::taps(src as usize, dst as usize);
    let mut data = vec![0u32; dst as usize * 8];
    for (o, t) in taps.iter().enumerate() {
        if t.is_empty() || t.len() > MAX_TAPS || t.windows(2).any(|p| p[1].0 != p[0].0 + 1) {
            return None;
        }
        data[o * 4] = t[0].0 as u32;
        data[o * 4 + 1] = t.len() as u32;
        for (k, &(_, w)) in t.iter().enumerate() {
            // shares 0 and 1 in the first row, 2 to 5 in the second
            let at = if k < 2 { o * 4 + 2 + k } else { (dst as usize + o) * 4 + k - 2 };
            data[at] = w;
        }
    }
    let tex = Tex::new(device, (dst, 2), wgpu::TextureFormat::Rgba32Uint, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
    tex.write(queue, as_bytes(&data), 16);
    Some(tex)
}

fn pipeline(device: &wgpu::Device, source: &str, layout: &wgpu::BindGroupLayout, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("pagan"), source: wgpu::ShaderSource::Wgsl(source.into()) });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("pagan"), bind_group_layouts: &[layout], push_constant_ranges: &[] });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("pagan"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
        count: None,
    }
}

/// What is kept on the graphics card for pictures of one size.
struct Frame {
    /// The picture as drawn, and the pixels to leave sharp when smoothing.
    picture: Tex,
    /// The world alone, for fine scrolling (see `Scaler::shift`).
    world: Tex,
    sharp: Tex,
    /// The mask of pixels left sharp is empty on the graphics card (it
    /// mostly is: it is only written when something is marked).
    sharp_clear: bool,
    /// Made the first time a picture of this size is smoothed: for the
    /// picture, and for the world alone (fine scrolling).
    smoothing: Option<(Smoothing, Smoothing)>,
}

impl Frame {
    fn new(device: &wgpu::Device, size: (u32, u32)) -> Frame {
        // 0x00RRGGBB in memory (little endian) is the bytes B, G, R, 0
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        Frame {
            picture: Tex::new(device, size, wgpu::TextureFormat::Bgra8Unorm, usage),
            world: Tex::new(device, size, wgpu::TextureFormat::Bgra8Unorm, usage),
            sharp: Tex::new(device, size, wgpu::TextureFormat::R8Unorm, usage),
            sharp_clear: false,
            smoothing: None,
        }
    }

    /// Hands over the pixels to leave sharp (`mask` may be empty: none).
    fn set_sharp(&mut self, queue: &wgpu::Queue, mask: &[bool]) {
        let (w, h) = self.sharp.size;
        let marked = mask.len() == (w * h) as usize && mask.contains(&true);
        if marked || !self.sharp_clear {
            let bytes: Vec<u8> = if marked { mask.iter().map(|&s| if s { 255 } else { 0 }).collect() } else { vec![0; (w * h) as usize] };
            self.sharp.write(queue, &bytes, 1);
            self.sharp_clear = !marked;
        }
    }
}

/// The smoothing's grid and diagonals, its result (four pixels per texel,
/// see XBR_HEAD), and the inputs of its three passes.
struct Smoothing {
    grid: Tex,
    diags: Tex,
    out: Tex,
    groups: [wgpu::BindGroup; 3],
}

impl Smoothing {
    /// For the pictures of `frame` (`source`: its picture or its world);
    /// `layouts`: those of the three passes.
    fn new(device: &wgpu::Device, frame: &Frame, source: &Tex, layouts: [&wgpu::BindGroupLayout; 3]) -> Smoothing {
        let (w, h) = frame.picture.size;
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT;
        let grid = Tex::new(device, (w + 4, h + 4), wgpu::TextureFormat::Rg32Uint, usage);
        let diags = Tex::new(device, (w + 4, h + 4), wgpu::TextureFormat::Rg32Sint, usage);
        let out = Tex::new(device, (w, h), wgpu::TextureFormat::Rgba32Uint, usage);
        let group = |layout: &wgpu::BindGroupLayout, inputs: &[&Tex]| {
            let views: Vec<_> = inputs.iter().map(|t| t.view()).collect();
            let entries: Vec<_> = views.iter().enumerate().map(|(b, v)| wgpu::BindGroupEntry { binding: b as u32, resource: wgpu::BindingResource::TextureView(v) }).collect();
            device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("pagan xbr"), layout, entries: &entries })
        };
        let groups = [group(layouts[0], &[source]), group(layouts[1], &[&grid]), group(layouts[2], &[&grid, &diags, &frame.sharp])];
        Smoothing { grid, diags, out, groups }
    }
}

/// The picture on the graphics card and what is made of it.
pub struct Scaler {
    /// The three passes of the smoothing (see XBR_GRID, XBR_DIAG, XBR_HEAD).
    xbr: [(wgpu::RenderPipeline, wgpu::BindGroupLayout); 3],
    /// The stretch from the picture as drawn and from the smoothed one.
    stretch: [(wgpu::RenderPipeline, wgpu::BindGroupLayout); 2],
    /// Where the picture's corner is in the window (for the stretch).
    corner: wgpu::Buffer,
    /// What is kept for the size of the last picture.
    frame: Option<Frame>,
    /// The last picture was smoothed (the stretch starts from the result).
    smoothed: bool,
    /// The tube monitor's look, in place of the stretch; its inputs, for
    /// the picture size; the last picture is shown with it.
    crt: (wgpu::RenderPipeline, wgpu::BindGroupLayout),
    crt_group: Option<((u32, u32), wgpu::BindGroup)>,
    crt_on: bool,
    /// For `offscreen`: time stamps of the smoothing's passes go to these six queries.
    timing: Option<(wgpu::QuerySet, u32)>,
    /// The shares, for (picture size, window size).
    taps: Option<((u32, u32, u32, u32), Tex, Tex)>,
    /// The stretch's inputs, for (smoothed, picture size, window size).
    stretch_group: Option<((bool, u32, u32, u32, u32), wgpu::BindGroup)>,
    /// Fine scrolling: the camera is this far (in picture pixels, across
    /// and down) beyond where the picture was drawn from; the world (where
    /// no window is drawn over it) is shown shifted by as many window
    /// pixels. Not with the tube look.
    shift: (f32, f32),
}

impl Scaler {
    /// For pictures put out in `format`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Scaler {
        let float = wgpu::TextureSampleType::Float { filterable: false };
        let (uint, sint) = (wgpu::TextureSampleType::Uint, wgpu::TextureSampleType::Sint);
        let xbr = |source: &str, inputs: &[wgpu::TextureSampleType], out| {
            let entries: Vec<_> = inputs.iter().enumerate().map(|(i, &t)| texture_entry(i as u32, t)).collect();
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("pagan xbr"), entries: &entries });
            (pipeline(device, &format!("{XBR_PRELUDE}{source}"), &layout, out), layout)
        };
        let stretch_layout = |pic, world| {
            let mut entries = vec![
                texture_entry(0, pic),
                texture_entry(1, wgpu::TextureSampleType::Uint),
                texture_entry(2, wgpu::TextureSampleType::Uint),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ];
            entries.push(texture_entry(4, world));
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("pagan stretch"), entries: &entries })
        };
        let stretch = |pic, ty: &str, fetch: &str| {
            let layout = stretch_layout(pic, pic);
            (pipeline(device, &STRETCH.replace("PIC", ty).replace("FETCH", fetch), &layout, format), layout)
        };
        let crt_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pagan crt"),
            entries: &[
                texture_entry(0, float),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let crt = (pipeline(device, CRT, &crt_layout, format), crt_layout);
        let corner = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pagan corner"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Scaler {
            xbr: [
                xbr(XBR_GRID, &[float], wgpu::TextureFormat::Rg32Uint),
                xbr(XBR_DIAG, &[uint], wgpu::TextureFormat::Rg32Sint),
                xbr(&format!("{XBR_HEAD}{}", xbr_main()), &[uint, sint, float], wgpu::TextureFormat::Rgba32Uint),
            ],
            stretch: [
                stretch(float, "texture_2d<f32>", FETCH_PLAIN),
                stretch(wgpu::TextureSampleType::Uint, "texture_2d<u32>", FETCH_SMOOTH),
            ],
            corner,
            frame: None,
            smoothed: false,
            crt,
            crt_group: None,
            crt_on: false,
            timing: None,
            taps: None,
            stretch_group: None,
            shift: (0.0, 0.0),
        }
    }

    /// Takes the next picture (0x00RRGGBB per pixel, `w` x `h`), to be
    /// shown with `look`.
    /// `world`: for fine scrolling, the world alone (before the windows
    /// were drawn over it) and how far beyond it the camera is (see
    /// `shift`).
    pub fn set_picture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, pixels: &[u32], w: u32, h: u32, look: Look, world: Option<(&[u32], (f32, f32))>) {
        if self.frame.as_ref().map(|f| f.picture.size) != Some((w, h)) {
            self.frame = Some(Frame::new(device, (w, h)));
            self.stretch_group = None;
            self.crt_group = None;
        }
        let Some(frame) = &mut self.frame else { return };
        frame.picture.write(queue, as_bytes(pixels), 4);
        self.shift = (0.0, 0.0);
        if let Some((world, shift)) = world.filter(|(px, _)| px.len() >= (w * h) as usize && !matches!(look, Look::Crt)) {
            frame.world.write(queue, as_bytes(world), 4);
            self.shift = shift;
        }
        self.smoothed = matches!(look, Look::Xbr(_));
        self.crt_on = matches!(look, Look::Crt);
        let Look::Xbr(mask) = look else { return };
        frame.set_sharp(queue, mask);
        if frame.smoothing.is_none() {
            let layouts = [&self.xbr[0].1, &self.xbr[1].1, &self.xbr[2].1];
            let smoothing = (Smoothing::new(device, frame, &frame.picture, layouts), Smoothing::new(device, frame, &frame.world, layouts));
            frame.smoothing = Some(smoothing);
        }
        let Some((picture, world)) = &frame.smoothing else { return };
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pagan xbr") });
        // (the world as well when fine scrolling; time stamps for the picture only)
        let fine = self.shift != (0.0, 0.0);
        let passes = [picture, world].into_iter().enumerate().filter(|&(n, _)| n == 0 || fine);
        for (n, s) in passes {
            for (k, target) in [&s.grid, &s.diags, &s.out].into_iter().enumerate() {
                smoothing_pass(&self.xbr, self.timing.as_ref(), &mut enc, s, k, target, (n == 0).then_some(k));
            }
        }
        queue.submit([enc.finish()]);
    }

    /// What the stretch starts from, and its size in pixels.
    fn input(&self) -> Option<(&Tex, (u32, u32))> {
        let frame = self.frame.as_ref()?;
        match &frame.smoothing {
            Some((s, _)) if self.smoothed => Some((&s.out, (2 * s.out.size.0, 2 * s.out.size.1))),
            _ => Some((&frame.picture, frame.picture.size)),
        }
    }

    /// Puts the stretch of the last picture into `pass` at `rect` (x, y,
    /// width, height in its target's pixels). False if there is no picture
    /// or it cannot be stretched to that size.
    pub fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, pass: &mut wgpu::RenderPass, rect: (u32, u32, u32, u32)) -> bool {
        let Some((_, (sw, sh))) = self.input() else { return false };
        let (x, y, w, h) = rect;
        if self.crt_on {
            if w == 0 || h == 0 {
                return false;
            }
            if self.crt_group.as_ref().map(|g| g.0) != Some((sw, sh)) {
                let Some(frame) = &self.frame else { return false };
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("pagan crt"),
                    layout: &self.crt.1,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&frame.picture.view()) },
                        wgpu::BindGroupEntry { binding: 1, resource: self.corner.as_entire_binding() },
                    ],
                });
                self.crt_group = Some(((sw, sh), group));
            }
            let Some((_, group)) = &self.crt_group else { return false };
            queue.write_buffer(&self.corner, 0, as_bytes(&[x, y, w, h]));
            pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
            pass.set_pipeline(&self.crt.0);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
            return true;
        }
        let key = (sw, sh, w, h);
        if self.taps.as_ref().map(|t| t.0) != Some(key) {
            self.taps = None;
            self.stretch_group = None;
            let (Some(tx), Some(ty)) = (taps_texture(device, queue, sw, w), taps_texture(device, queue, sh, h)) else { return false };
            self.taps = Some((key, tx, ty));
        }
        let gkey = (self.smoothed, sw, sh, w, h);
        if self.stretch_group.as_ref().map(|g| g.0) != Some(gkey) {
            let (Some((input, _)), Some((_, tx, ty)), Some(frame)) = (self.input(), &self.taps, &self.frame) else { return false };
            // (the world as the stretch takes it: smoothed or not)
            let world = match &frame.smoothing {
                Some((_, w)) if self.smoothed => w.out.view(),
                _ => frame.world.view(),
            };
            let (input, tx, ty) = (input.view(), tx.view(), ty.view());
            let entries = [
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&input) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&tx) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&ty) },
                wgpu::BindGroupEntry { binding: 3, resource: self.corner.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&world) },
            ];
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("pagan stretch"), layout: &self.stretch[self.smoothed as usize].1, entries: &entries });
            self.stretch_group = Some((gkey, group));
        }
        let Some((_, group)) = &self.stretch_group else { return false };
        // (fine scrolling: the shift in whole window pixels; it is in pixels
        // of the picture as drawn, the smoothed one has twice as many)
        let (pw, ph) = if self.smoothed { (sw / 2, sh / 2) } else { (sw, sh) };
        let (dx, dy) = ((self.shift.0 * w as f32 / pw as f32).round() as i32, (self.shift.1 * h as f32 / ph as f32).round() as i32);
        queue.write_buffer(&self.corner, 0, as_bytes(&[x, y, dx as u32, dy as u32]));
        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        pass.set_pipeline(&self.stretch[self.smoothed as usize].0);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
        true
    }
}

/// Puts the smoothing's pass `k` (of `xbr`) for `s` into `enc`,
/// rendering into `target`; `timed`: which time stamps to write (see
/// `Scaler::timing`).
fn smoothing_pass(xbr: &[(wgpu::RenderPipeline, wgpu::BindGroupLayout); 3], timing: Option<&(wgpu::QuerySet, u32)>, enc: &mut wgpu::CommandEncoder, s: &Smoothing, k: usize, target: &Tex, timed: Option<usize>) {
    // (time stamps: the start and the end of each pass)
    let timestamp_writes = timing.zip(timed).map(|((q, at), i)| wgpu::RenderPassTimestampWrites {
        query_set: q,
        beginning_of_pass_write_index: Some(at + 2 * i as u32),
        end_of_pass_write_index: Some(at + 2 * i as u32 + 1),
    });
    let view = target.view();
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("pagan xbr"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes,
        occlusion_query_set: None,
    });
    pass.set_pipeline(&xbr[k].0);
    pass.set_bind_group(0, &s.groups[k], &[]);
    pass.draw(0..3, 0..1);
}

/// For the test command `gpucheck`: the picture shown with `look` and
/// stretched to `dst` on the graphics card, without a window.
/// `world`: as for `Scaler::set_picture` (fine scrolling).
pub fn offscreen(pixels: &[u32], w: u32, h: u32, look: Look, world: Option<(&[u32], (f32, f32))>, dst: (u32, u32)) -> Result<Vec<u32>, String> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok_or("no graphics adapter")?;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("pagan"),
            required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
            required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::Performance,
        },
        None,
    ))
    .map_err(|e| format!("no device: {e}"))?;
    let format = wgpu::TextureFormat::Bgra8Unorm;
    let mut scaler = Scaler::new(&device, format);
    scaler.set_picture(&device, &queue, pixels, w, h, look, world);
    let target = Tex::new(&device, dst, format, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
    // how long the graphics card takes (its own time stamps, if it has
    // them): the smoothing and the stretch, 50 times
    const RUNS: u32 = 50;
    // time stamps per run: two for each smoothing pass (if smoothing) and
    // two for the stretch; only ones that get written may be read back (a
    // Vulkan driver waits forever for the others)
    let smooth = matches!(look, Look::Xbr(_));
    let per: u32 = if smooth { 8 } else { 2 };
    let queries = device.features().contains(wgpu::Features::TIMESTAMP_QUERY).then(|| {
        device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("pagan check"), ty: wgpu::QueryType::Timestamp, count: per * RUNS })
    });
    let t = std::time::Instant::now();
    for run in 0..RUNS {
        scaler.timing = queries.clone().map(|q| (q, per * run));
        scaler.set_picture(&device, &queue, pixels, w, h, look, world);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pagan check") });
        {
            let view = target.view();
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pagan check"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: queries.as_ref().map(|q| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(per * run + per - 2), end_of_pass_write_index: Some(per * run + per - 1) }),
                occlusion_query_set: None,
            });
            scaler.draw(&device, &queue, &mut pass, (0, 0, dst.0, dst.1));
        }
        queue.submit([enc.finish()]);
    }
    device.poll(wgpu::Maintain::Wait);
    let wall = t.elapsed().as_secs_f64() * 1000.0 / RUNS as f64;
    scaler.timing = None;
    if let Some(q) = &queries {
        let size = 8 * (per * RUNS) as u64;
        let resolved = device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.resolve_query_set(q, 0..per * RUNS, &resolved, 0);
        enc.copy_buffer_to_buffer(&resolved, 0, &read, 0, size);
        queue.submit([enc.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::Maintain::Wait);
        let data = read.slice(..).get_mapped_range();
        let stamps: Vec<u64> = data.chunks_exact(8).map(|b| u64::from_le_bytes(b.try_into().unwrap())).collect();
        let ns = queue.get_timestamp_period() as f64;
        // (the first runs warm up)
        let ms = |k: usize| stamps.chunks_exact(per as usize).skip(10).map(|c| c[k + 1].saturating_sub(c[k]) as f64 * ns / 1e6).sum::<f64>() / (RUNS - 10) as f64;
        let (a, b, c) = if smooth { (ms(0), ms(2), ms(4)) } else { (0.0, 0.0, 0.0) };
        println!("graphics card: smoothing {:.2} ms ({a:.2} + {b:.2} + {c:.2}), stretch {:.2} ms (all with the processor's part {wall:.2} ms)", a + b + c, ms(per as usize - 2));
    } else {
        println!("graphics card: {wall:.2} ms per picture (no time stamps)");
    }
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pagan check") });
    {
        let view = target.view();
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pagan check"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        if !scaler.draw(&device, &queue, &mut pass, (0, 0, dst.0, dst.1)) {
            return Err("cannot stretch to that size".into());
        }
    }
    // rows are copied out in steps of 256 bytes
    let row = (dst.0 * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pagan check"),
        size: (row * dst.1) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: &target.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(dst.1) } },
        wgpu::Extent3d { width: dst.0, height: dst.1, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((dst.0 * dst.1) as usize);
    for y in 0..dst.1 {
        let line = &data[(y * row) as usize..(y * row + dst.0 * 4) as usize];
        out.extend(line.chunks_exact(4).map(|b| (b[2] as u32) << 16 | (b[1] as u32) << 8 | b[0] as u32));
    }
    Ok(out)
}
