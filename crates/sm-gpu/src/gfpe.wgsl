// GFPE sur GPU : un groupe de travail par azimut (lot), un dispatch par pas
// en distance. Même schéma que sm_core::gfpe::run (point milieu, pôle
// soustrait, rotation de relief) ; FFT de Stockham radix 2 en mémoire globale.

struct Params {
    n: u32,
    m: u32,
    log2m: u32,
    batches: u32,
    steps: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
    dz: f32,
    norm: f32,
    hr: f32,
    zs: f32,
    tw_c: vec2<f32>,
    r0: f32,
    dr: f32,
};

struct Step {
    step: u32,
};

// Par indice i < m : kz, tw_k, √(ka² − kz²) − ka, tw_z.
struct Spec {
    kz: f32,
    _pad: f32,
    tw_k: vec2<f32>,
    kzt: vec2<f32>,
    tw_z: vec2<f32>,
};

// Par (pas, lot) : rotation, ds, β, P(β).
struct StepP {
    rot: f32,
    ds: f32,
    beta: vec2<f32>,
    p_beta: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<uniform> S: Step;
@group(0) @binding(2) var<storage, read> SPEC: array<Spec>;
@group(0) @binding(3) var<storage, read> SP: array<StepP>;
@group(0) @binding(4) var<storage, read> DK: array<vec2<f32>>;
@group(0) @binding(5) var<storage, read_write> PSI: array<vec2<f32>>;
@group(0) @binding(6) var<storage, read_write> W: array<vec2<f32>>;
@group(0) @binding(7) var<storage, read_write> OUT: array<f32>;
@group(0) @binding(8) var<storage, read> TW: array<vec2<f32>>;

const WG: u32 = 256u;
var<workgroup> red: array<vec2<f32>, 256>;

fn cmul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

fn conj(a: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x, -a.y);
}

fn cdiv(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let d = dot(b, b);
    return vec2<f32>(a.x * b.x + a.y * b.y, a.y * b.x - a.x * b.y) / d;
}

// e^(i·φ)
fn cis(phi: f32) -> vec2<f32> {
    return vec2<f32>(cos(phi), sin(phi));
}

// e^(i·s·c) pour c complexe : e^(−s·c.y)·e^(i·s·c.x).
fn exp_i(s: f32, c: vec2<f32>) -> vec2<f32> {
    return exp(-s * c.y) * cis(s * c.x);
}

// FFT de Stockham radix 2 sur W[src..src+m] (tampon W[tmp..]) ; renvoie
// l'offset du résultat. TW[q] = e^(−2πi q/m).
fn fft(src: u32, tmp: u32, inverse: bool, t: u32) -> u32 {
    var a = src;
    var b = tmp;
    let half = P.m / 2u;
    var ns = 1u;
    for (var p = 0u; p < P.log2m; p = p + 1u) {
        for (var j = t; j < half; j = j + WG) {
            let k = j & (ns - 1u);
            let v0 = W[a + j];
            var w = TW[k * (half / ns)];
            if (inverse) {
                w = conj(w);
            }
            let v1 = cmul(W[a + j + half], w);
            let o = ((j - k) << 1u) + k;
            W[b + o] = v0 + v1;
            W[b + o + ns] = v0 - v1;
        }
        storageBarrier();
        workgroupBarrier();
        let s = a;
        a = b;
        b = s;
        ns = ns << 1u;
    }
    return a;
}

@compute @workgroup_size(256)
fn step_main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
    let b = wg.x;
    let t = lid.x;
    let n = P.n;
    let m = P.m;
    let sp = SP[S.step * P.batches + b];
    let beta = sp.beta;
    let base = b * 4u * m;
    let wa = base;
    let wb = base + m;
    let wc = base + 2u * m;
    let wd = base + 3u * m;
    let pb = b * n;

    // 1. Rotation de relief, Ψ(kz) (entrée FFT) et réduction pour Ψ(β).
    var acc = vec2<f32>(0.0, 0.0);
    for (var j = t; j < m; j = j + WG) {
        if (j < n) {
            let z = (f32(j) + 0.5) * P.dz;
            var v = PSI[pb + j];
            if (sp.rot != 0.0) {
                v = cmul(v, cis(-sp.rot * z));
                PSI[pb + j] = v;
            }
            W[wa + j] = cmul(v, SPEC[j].tw_z);
            // e^(−iβz) = e^(β.y z)·e^(−i β.x z)
            acc = acc + cmul(v, exp(beta.y * z) * cis(-beta.x * z));
        } else {
            W[wa + j] = vec2<f32>(0.0, 0.0);
        }
    }
    red[t] = acc;
    workgroupBarrier();
    for (var s = WG / 2u; s > 0u; s = s >> 1u) {
        if (t < s) {
            red[t] = red[t] + red[t + s];
        }
        workgroupBarrier();
    }
    let h_beta = cmul(red[0], sp.p_beta);
    storageBarrier();
    workgroupBarrier();

    // 2. Transformée directe.
    let fa = fft(wa, wb, false, t);
    let fa_tmp = select(wa, wb, fa == wa);

    // 3. Propagateur et spectre réfléchi à pôle soustrait.
    for (var i = t; i < m; i = i + WG) {
        let c = SPEC[i];
        let prop = exp_i(sp.ds, c.kzt);
        let p = cmul(cmul(cmul(W[fa + i], c.tw_k), P.tw_c), prop);
        let refl = p + cmul(2.0 * beta, cdiv(p - h_beta, vec2<f32>(c.kz, 0.0) - beta));
        W[fa + i] = cmul(p, conj(c.tw_k));
        W[wc + i] = cmul(refl, c.tw_k);
    }
    storageBarrier();
    workgroupBarrier();

    // 4. Direct : transformée inverse ; réfléchi : transformée directe.
    let ia = fft(fa, fa_tmp, true, t);
    let ib = fft(wc, wd, false, t);

    // 5. Recombinaison et facteur de phase δk(z) (réfraction, absorption).
    for (var j = t; j < n; j = j + WG) {
        let tz = SPEC[j].tw_z;
        let direct = cmul(cmul(W[ia + j], conj(tz)), conj(P.tw_c));
        let reflected = cmul(cmul(W[ib + j], tz), P.tw_c);
        PSI[pb + j] = cmul((direct + reflected) * P.norm, exp_i(sp.ds, DK[pb + j]));
    }
    storageBarrier();
    workgroupBarrier();

    // 6. Niveau au récepteur : ΔL = 20 log|ψ| − 10 log r + 20 log R.
    if (t == 0u) {
        let x = max(P.hr / P.dz - 0.5, 0.0);
        let j = min(u32(floor(x)), n - 2u);
        let tt = x - f32(j);
        let v = PSI[pb + j] * (1.0 - tt) + PSI[pb + j + 1u] * tt;
        let r1 = P.r0 + f32(S.step + 1u) * P.dr;
        let rr = length(vec2<f32>(r1, P.hr - P.zs));
        let l10 = 0.30102999566;
        OUT[b * P.steps + S.step] = 20.0 * l10 * log2(max(length(v), 1e-12))
            - 10.0 * l10 * log2(r1) + 20.0 * l10 * log2(rr);
    }
}
