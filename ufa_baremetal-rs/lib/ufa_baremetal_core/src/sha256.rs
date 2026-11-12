//! Hand-rolled SHA-256 (FIPS 180-4), used only for the `--json` output's
//! per-input `sha256` fingerprint field — a content-identity checksum for
//! change detection, not a security boundary. The Python original uses
//! `hashlib.sha256`; this port implements the algorithm directly from
//! the standard rather than depending on the `sha2` crate, continuing
//! the "stdlib only" discipline (see the core crate's `Cargo.toml`).
//!
//! `H0`/`K` were generated programmatically (first 32 bits of the
//! fractional parts of sqrt/cbrt of the first 8/64 primes, per the spec)
//! rather than transcribed from memory, and the implementation is
//! checked against known FIPS test vectors plus padding-boundary edge
//! cases (55-57 and 63-65 byte inputs, which land exactly on/around the
//! block-padding threshold) in the tests below.

const H0: [u32; 8] =
    [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Returns the lowercase hex digest, matching `hashlib.sha256(data).hexdigest()`.
pub fn sha256_hex(data: &[u8]) -> String {
    sha256(data).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = H0;

    // Padding: append 0x80, then zero bytes, then the 64-bit big-endian
    // bit-length, so the total is a multiple of 64 bytes.
    let bit_len: u64 = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().enumerate().take(16) {
            *word = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn abc() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn quick_fox() {
        assert_eq!(
            sha256_hex(b"The quick brown fox jumps over the lazy dog"),
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
        );
    }

    // Padding-boundary stress: FIPS padding appends 0x80 + zeros + an
    // 8-byte length, so the "does it need one more 64-byte block"
    // decision flips right around the 55-57 and 63-65 byte marks. Each
    // expected digest below was generated fresh from Python's `hashlib`
    // in the same session this port was written, specifically to
    // exercise this boundary.
    #[test]
    fn padding_boundary_55_to_57_bytes() {
        assert_eq!(sha256_hex(&[b'x'; 55]), "d5e285683cd4efc02d021a5c62014694958901005d6f71e89e0989fac77e4072");
        assert_eq!(sha256_hex(&[b'x'; 56]), "04c26261370ee7541549d16dee320c723e3fd14671e66a099afe0a377c16888e");
        assert_eq!(sha256_hex(&[b'x'; 57]), "ae14a2563ccf969d99aca69ce6bb74981f734bbf9f655f73b8f06db68cab5217");
    }

    #[test]
    fn padding_boundary_63_to_65_bytes() {
        assert_eq!(sha256_hex(&[b'x'; 63]), "75220b47218278e656f2013bb8f0c455a25eaf01e86c64924e9d48d89776d6f2");
        assert_eq!(sha256_hex(&[b'x'; 64]), "7ce100971f64e7001e8fe5a51973ecdfe1ced42befe7ee8d5fd6219506b5393c");
        assert_eq!(sha256_hex(&[b'x'; 65]), "9537c5fdf120482f7d58d25e9ed583f52c02b4e304ea814db1633ad565aed7e9");
    }
}
