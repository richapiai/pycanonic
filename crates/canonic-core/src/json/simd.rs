//! Stage 1: one pass that finds every structural byte, the simdjson way.
//!
//! Each 64-byte block is classified into three bitmasks (quotes, backslashes, `{ } [ ] : ,`) by a
//! backend: NEON on aarch64, AVX2 or SSE2 on x86_64, plain bytes elsewhere. The rest is shared
//! integer code: backslash runs of odd length escape the byte after them, a prefix-XOR over the
//! unescaped quotes gives the in-string mask, and both carry from one block to the next.
//!
//! A backslash escapes the next byte wherever it is, as in simdjson. The parser only lets a
//! backslash escape inside a string, so the two disagree after a backslash outside a string. Valid
//! JSON never has one; when it occurs it is reported in the index so the parser can fall back to
//! scanning bytes from there.

/// Sorted positions of every unescaped `"` (opening and closing), and of `{ } [ ] : , \` outside
/// strings. Positions are `u32`: the parser caps input at 2^31 bytes ([`super::MAX_BYTES`]); past
/// `u32::MAX` bytes they wrap and mean nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Structurals {
    pub idx: Vec<u32>,
}

/// One block classified: bit `i` is byte `i`.
struct Masks {
    quote: u64,
    bs: u64,
    op: u64,
}

/// Picks the fastest backend for this CPU.
#[cfg(target_arch = "aarch64")]
pub fn structurals(buf: &[u8]) -> Structurals {
    // SAFETY: NEON is part of every aarch64 CPU.
    scan(buf, |b| unsafe { neon::classify(b) })
}

/// Picks the fastest backend for this CPU. `is_x86_feature_detected!` caches its answer.
#[cfg(target_arch = "x86_64")]
pub fn structurals(buf: &[u8]) -> Structurals {
    if is_x86_feature_detected!("avx2") {
        // SAFETY: the CPU supports AVX2, checked just above.
        unsafe { x86::structurals_avx2(buf) }
    } else {
        structurals_sse2(buf)
    }
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub fn structurals(buf: &[u8]) -> Structurals {
    structurals_portable(buf)
}

/// The SSE2 backend, baseline on x86_64. Public so tests can compare it on AVX2 machines.
#[cfg(target_arch = "x86_64")]
pub fn structurals_sse2(buf: &[u8]) -> Structurals {
    // SAFETY: SSE2 is part of every x86_64 CPU.
    scan(buf, |b| unsafe { x86::classify_sse2(b) })
}

/// The reference backend: same result as [`structurals`], one byte at a time.
pub fn structurals_portable(buf: &[u8]) -> Structurals {
    scan(buf, |b| {
        let mut m = Masks {
            quote: 0,
            bs: 0,
            op: 0,
        };
        for (i, &c) in b.iter().enumerate() {
            let bit = 1u64 << i;
            match c {
                b'"' => m.quote |= bit,
                b'\\' => m.bs |= bit,
                b'{' | b'}' | b'[' | b']' | b':' | b',' => m.op |= bit,
                _ => {}
            }
        }
        m
    })
}

#[inline(always)]
fn scan(buf: &[u8], classify: impl Fn(&[u8; 64]) -> Masks) -> Structurals {
    let mut idx = Vec::with_capacity(buf.len() / 4 + 64);
    let (mut odd_bs, mut in_str) = (0u64, 0u64);
    let mut block = |b: &[u8; 64], base: usize, idx: &mut Vec<u32>| {
        let m = classify(b);
        let quote = m.quote & !escaped(m.bs, &mut odd_bs);
        let inside = prefix_xor(quote) ^ in_str;
        in_str = 0u64.wrapping_sub(inside >> 63);
        flatten(idx, base as u32, ((m.op | m.bs) & !inside) | quote);
    };
    let (blocks, rest) = buf.as_chunks::<64>();
    for (n, b) in blocks.iter().enumerate() {
        block(b, n * 64, &mut idx);
    }
    if !rest.is_empty() {
        // Spaces classify as nothing, so the padding adds no positions.
        let mut pad = [b' '; 64];
        pad[..rest.len()].copy_from_slice(rest);
        block(&pad, blocks.len() * 64, &mut idx);
    }
    Structurals { idx }
}

#[inline(always)]
fn flatten(idx: &mut Vec<u32>, base: u32, mut s: u64) {
    idx.reserve(s.count_ones() as usize);
    while s != 0 {
        idx.push(base.wrapping_add(s.trailing_zeros()));
        s &= s - 1;
    }
}

/// Bytes escaped by an odd backslash run before them. `prev_odd` is 1 when the previous block
/// ended in an odd run. From the simdjson paper (Langdale and Lemire 2019, section 3.1.1).
#[inline(always)]
fn escaped(bs: u64, prev_odd: &mut u64) -> u64 {
    const EVEN: u64 = 0x5555_5555_5555_5555;
    const ODD: u64 = !EVEN;
    let starts = bs & !(bs << 1);
    let even_start_mask = EVEN ^ *prev_odd;
    let even_starts = starts & even_start_mask;
    let odd_starts = starts & !even_start_mask;
    let even_carries = bs.wrapping_add(even_starts);
    let (odd_carries, overflow) = bs.overflowing_add(odd_starts);
    let odd_carries = odd_carries | *prev_odd;
    *prev_odd = overflow as u64;
    ((even_carries & !bs) & ODD) | ((odd_carries & !bs) & EVEN)
}

/// Bit `i` becomes the XOR of bits `0..=i`: 1 from an opening quote up to its closing quote.
#[inline(always)]
fn prefix_xor(mut x: u64) -> u64 {
    x ^= x << 1;
    x ^= x << 2;
    x ^= x << 4;
    x ^= x << 8;
    x ^= x << 16;
    x ^= x << 32;
    x
}

// The backends use no closures: whether a closure inherits its function's target features is
// not relied on. Each `classify` is an `unsafe fn` so its callers' `unsafe` is never redundant.

#[cfg(target_arch = "aarch64")]
mod neon {
    use super::Masks;
    use std::arch::aarch64::*;

    /// One bit per byte from four all-ones-or-zero byte vectors (simdjson's NEON movemask).
    #[target_feature(enable = "neon")]
    fn bits(v: [uint8x16_t; 4]) -> u64 {
        const W: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
        // SAFETY: `W` is 16 readable bytes; `vld1q_u8` needs no alignment.
        let w = unsafe { vld1q_u8(W.as_ptr()) };
        let a = vpaddq_u8(vandq_u8(v[0], w), vandq_u8(v[1], w));
        let b = vpaddq_u8(vandq_u8(v[2], w), vandq_u8(v[3], w));
        let s = vpaddq_u8(a, b);
        let s = vpaddq_u8(s, s);
        vgetq_lane_u64::<0>(vreinterpretq_u64_u8(s))
    }

    #[target_feature(enable = "neon")]
    fn eq(v: &[uint8x16_t; 4], c: u8) -> u64 {
        let c = vdupq_n_u8(c);
        bits([
            vceqq_u8(v[0], c),
            vceqq_u8(v[1], c),
            vceqq_u8(v[2], c),
            vceqq_u8(v[3], c),
        ])
    }

    #[target_feature(enable = "neon")]
    fn op(x: uint8x16_t) -> uint8x16_t {
        // `| 0x20` turns `[` into `{` and `]` into `}` and nothing else into either.
        let l = vorrq_u8(x, vdupq_n_u8(0x20));
        let br = vorrq_u8(vceqq_u8(l, vdupq_n_u8(b'{')), vceqq_u8(l, vdupq_n_u8(b'}')));
        let pu = vorrq_u8(vceqq_u8(x, vdupq_n_u8(b':')), vceqq_u8(x, vdupq_n_u8(b',')));
        vorrq_u8(br, pu)
    }

    /// # Safety
    /// The CPU must support NEON, which every aarch64 CPU does.
    #[target_feature(enable = "neon")]
    pub(super) unsafe fn classify(b: &[u8; 64]) -> Masks {
        // SAFETY: the four 16-byte loads at offsets 0, 16, 32, 48 stay inside the 64-byte array;
        // `vld1q_u8` needs no alignment.
        let v = unsafe {
            let p = b.as_ptr();
            [
                vld1q_u8(p),
                vld1q_u8(p.add(16)),
                vld1q_u8(p.add(32)),
                vld1q_u8(p.add(48)),
            ]
        };
        Masks {
            quote: eq(&v, b'"'),
            bs: eq(&v, b'\\'),
            op: bits([op(v[0]), op(v[1]), op(v[2]), op(v[3])]),
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod x86 {
    use super::{Masks, Structurals, scan};
    use std::arch::x86_64::*;

    #[target_feature(enable = "sse2")]
    fn bits_sse2(v: [__m128i; 4]) -> u64 {
        (_mm_movemask_epi8(v[0]) as u16 as u64)
            | ((_mm_movemask_epi8(v[1]) as u16 as u64) << 16)
            | ((_mm_movemask_epi8(v[2]) as u16 as u64) << 32)
            | ((_mm_movemask_epi8(v[3]) as u16 as u64) << 48)
    }

    #[target_feature(enable = "sse2")]
    fn eq_sse2(v: &[__m128i; 4], c: u8) -> u64 {
        let c = _mm_set1_epi8(c as i8);
        bits_sse2([
            _mm_cmpeq_epi8(v[0], c),
            _mm_cmpeq_epi8(v[1], c),
            _mm_cmpeq_epi8(v[2], c),
            _mm_cmpeq_epi8(v[3], c),
        ])
    }

    #[target_feature(enable = "sse2")]
    fn op_sse2(x: __m128i) -> __m128i {
        // `| 0x20` turns `[` into `{` and `]` into `}` and nothing else into either.
        let l = _mm_or_si128(x, _mm_set1_epi8(0x20));
        let br = _mm_or_si128(
            _mm_cmpeq_epi8(l, _mm_set1_epi8(b'{' as i8)),
            _mm_cmpeq_epi8(l, _mm_set1_epi8(b'}' as i8)),
        );
        let pu = _mm_or_si128(
            _mm_cmpeq_epi8(x, _mm_set1_epi8(b':' as i8)),
            _mm_cmpeq_epi8(x, _mm_set1_epi8(b',' as i8)),
        );
        _mm_or_si128(br, pu)
    }

    /// # Safety
    /// The CPU must support SSE2, which every x86_64 CPU does.
    #[target_feature(enable = "sse2")]
    pub(super) unsafe fn classify_sse2(b: &[u8; 64]) -> Masks {
        // SAFETY: the four 16-byte loads at offsets 0, 16, 32, 48 stay inside the 64-byte array;
        // `_mm_loadu_si128` needs no alignment.
        let v = unsafe {
            let p = b.as_ptr().cast::<__m128i>();
            [
                _mm_loadu_si128(p),
                _mm_loadu_si128(p.add(1)),
                _mm_loadu_si128(p.add(2)),
                _mm_loadu_si128(p.add(3)),
            ]
        };
        Masks {
            quote: eq_sse2(&v, b'"'),
            bs: eq_sse2(&v, b'\\'),
            op: bits_sse2([op_sse2(v[0]), op_sse2(v[1]), op_sse2(v[2]), op_sse2(v[3])]),
        }
    }

    #[target_feature(enable = "avx2")]
    fn bits_avx2(lo: __m256i, hi: __m256i) -> u64 {
        (_mm256_movemask_epi8(lo) as u32 as u64) | ((_mm256_movemask_epi8(hi) as u32 as u64) << 32)
    }

    #[target_feature(enable = "avx2")]
    fn eq_avx2(lo: __m256i, hi: __m256i, c: u8) -> u64 {
        let c = _mm256_set1_epi8(c as i8);
        bits_avx2(_mm256_cmpeq_epi8(lo, c), _mm256_cmpeq_epi8(hi, c))
    }

    #[target_feature(enable = "avx2")]
    fn op_avx2(x: __m256i) -> __m256i {
        // `| 0x20` turns `[` into `{` and `]` into `}` and nothing else into either.
        let l = _mm256_or_si256(x, _mm256_set1_epi8(0x20));
        let br = _mm256_or_si256(
            _mm256_cmpeq_epi8(l, _mm256_set1_epi8(b'{' as i8)),
            _mm256_cmpeq_epi8(l, _mm256_set1_epi8(b'}' as i8)),
        );
        let pu = _mm256_or_si256(
            _mm256_cmpeq_epi8(x, _mm256_set1_epi8(b':' as i8)),
            _mm256_cmpeq_epi8(x, _mm256_set1_epi8(b',' as i8)),
        );
        _mm256_or_si256(br, pu)
    }

    /// # Safety
    /// The CPU must support AVX2.
    #[target_feature(enable = "avx2")]
    unsafe fn classify_avx2(b: &[u8; 64]) -> Masks {
        // SAFETY: the two 32-byte loads at offsets 0 and 32 stay inside the 64-byte array;
        // `_mm256_loadu_si256` needs no alignment.
        let (lo, hi) = unsafe {
            let p = b.as_ptr().cast::<__m256i>();
            (_mm256_loadu_si256(p), _mm256_loadu_si256(p.add(1)))
        };
        Masks {
            quote: eq_avx2(lo, hi, b'"'),
            bs: eq_avx2(lo, hi, b'\\'),
            op: bits_avx2(op_avx2(lo), op_avx2(hi)),
        }
    }

    /// # Safety
    /// The CPU must support AVX2.
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn structurals_avx2(buf: &[u8]) -> Structurals {
        // SAFETY: this function only runs on CPUs with AVX2 (its own contract).
        scan(buf, |b| unsafe { classify_avx2(b) })
    }
}
