//! Passwords without third-party crates: SHA-256 (FIPS 180-4), HMAC-SHA-256
//! (RFC 2104), PBKDF2-HMAC-SHA-256 (RFC 8018) and the one-line record Nova
//! stores instead of a password:
//!
//! ```text
//! nova-pbkdf2-sha256$<iterations>$<salt, 32 hex digits>$<hash, 64 hex digits>
//! ```
//!
//! The salt is random per password (the kernel supplies it), so two equal
//! passwords give different records, and checking a guess costs `iterations`
//! HMACs. Comparison is constant-time. The password itself is never stored.
//!
//! Everything is host-tested against the published test vectors.

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

use alloc::string::String;

// ---- SHA-256 ----------------------------------------------------------------------

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const H0: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];

/// An incremental SHA-256.
#[derive(Clone)]
pub struct Sha256 {
    state: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub const fn new() -> Self {
        Self { state: H0, buf: [0; 64], buf_len: 0, total: 0 }
    }

    fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.buf_len > 0 {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len < 64 {
                return;
            }
            let block = self.buf;
            Self::compress(&mut self.state, &block);
            self.buf_len = 0;
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            Self::compress(&mut self.state, &block);
            data = &data[64..];
        }
        self.buf[..data.len()].copy_from_slice(data);
        self.buf_len = data.len();
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.total.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buf_len != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut out = [0u8; 32];
        for (i, s) in self.state.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&s.to_be_bytes());
        }
        out
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finish()
}

// ---- HMAC and PBKDF2 -----------------------------------------------------------------

/// HMAC-SHA-256 with the key's inner and outer states precomputed (PBKDF2 reuses them).
#[derive(Clone)]
pub struct Hmac {
    inner: Sha256,
    outer: Sha256,
}

impl Hmac {
    pub fn new(key: &[u8]) -> Self {
        let mut k = [0u8; 64];
        if key.len() > 64 {
            k[..32].copy_from_slice(&sha256(key));
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let (mut ipad, mut opad) = ([0x36u8; 64], [0x5cu8; 64]);
        for i in 0..64 {
            ipad[i] ^= k[i];
            opad[i] ^= k[i];
        }
        let mut inner = Sha256::new();
        inner.update(&ipad);
        let mut outer = Sha256::new();
        outer.update(&opad);
        Self { inner, outer }
    }

    pub fn mac(&self, parts: &[&[u8]]) -> [u8; 32] {
        let mut i = self.inner.clone();
        for p in parts {
            i.update(p);
        }
        let mut o = self.outer.clone();
        o.update(&i.finish());
        o.finish()
    }
}

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    Hmac::new(key).mac(&[data])
}

/// PBKDF2-HMAC-SHA-256, one 32-byte block (all Nova needs).
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let prf = Hmac::new(password);
    let mut u = prf.mac(&[salt, &1u32.to_be_bytes()]);
    let mut t = u;
    for _ in 1..iterations.max(1) {
        u = prf.mac(&[&u]);
        for (a, b) in t.iter_mut().zip(u) {
            *a ^= b;
        }
    }
    t
}

/// Equal without leaking *where* they differ through timing.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---- The stored record ---------------------------------------------------------------

pub const SCHEME: &str = "nova-pbkdf2-sha256";
/// Work factor for new passwords: about a tenth of a second on a modern CPU.
pub const DEFAULT_ITERATIONS: u32 = 100_000;
pub const SALT_LEN: usize = 16;
/// Longest password accepted (keeps the lock screen field and the work bounded).
pub const MAX_LEN: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub iterations: u32,
    pub salt: [u8; SALT_LEN],
    pub hash: [u8; 32],
}

fn hex(bytes: &[u8], out: &mut String) {
    const D: &[u8; 16] = b"0123456789abcdef";
    for b in bytes {
        out.push(D[(b >> 4) as usize] as char);
        out.push(D[(b & 15) as usize] as char);
    }
}

fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let s = s.as_bytes();
    if s.len() != 2 * N {
        return None;
    }
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = [0u8; N];
    for i in 0..N {
        out[i] = digit(s[2 * i])? << 4 | digit(s[2 * i + 1])?;
    }
    Some(out)
}

impl Record {
    /// A record for `password` with this salt.
    pub fn new(password: &str, salt: [u8; SALT_LEN], iterations: u32) -> Self {
        Self { iterations, salt, hash: pbkdf2_sha256(password.as_bytes(), &salt, iterations) }
    }

    /// Parses the one-line form (surrounding whitespace allowed).
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.trim().split('$');
        if parts.next()? != SCHEME {
            return None;
        }
        let iterations: u32 = parts.next()?.parse().ok()?;
        let salt = unhex::<SALT_LEN>(parts.next()?)?;
        let hash = unhex::<32>(parts.next()?)?;
        if parts.next().is_some() || iterations == 0 || iterations > 10_000_000 {
            return None;
        }
        Some(Self { iterations, salt, hash })
    }

    pub fn to_line(&self) -> String {
        let mut s = alloc::format!("{SCHEME}${}$", self.iterations);
        hex(&self.salt, &mut s);
        s.push('$');
        hex(&self.hash, &mut s);
        s
    }

    pub fn verify(&self, password: &str) -> bool {
        password.len() <= MAX_LEN && ct_eq(&pbkdf2_sha256(password.as_bytes(), &self.salt, self.iterations), &self.hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::ToString;

    fn h(b: &[u8]) -> String {
        let mut s = String::new();
        hex(b, &mut s);
        s
    }

    #[test]
    fn sha256_vectors() {
        assert_eq!(h(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(h(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            h(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // One million 'a's, fed in uneven pieces (exercises the buffering).
        let mut s = Sha256::new();
        let chunk = [b'a'; 997];
        let mut left = 1_000_000;
        while left > 0 {
            let n = left.min(chunk.len());
            s.update(&chunk[..n]);
            left -= n;
        }
        assert_eq!(h(&s.finish()), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    #[test]
    fn hmac_rfc4231() {
        assert_eq!(h(&hmac_sha256(&[0x0b; 20], b"Hi There")), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
        assert_eq!(h(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        // A key longer than a block is hashed first (test case 6).
        assert_eq!(
            h(&hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn pbkdf2_vectors() {
        assert_eq!(h(&pbkdf2_sha256(b"password", b"salt", 1)), "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b");
        assert_eq!(h(&pbkdf2_sha256(b"password", b"salt", 2)), "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43");
        assert_eq!(h(&pbkdf2_sha256(b"password", b"salt", 4096)), "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a");
    }

    #[test]
    fn record_round_trip_and_verify() {
        let r = Record::new("correct horse", [7; SALT_LEN], 1000);
        let line = r.to_line();
        assert!(line.starts_with("nova-pbkdf2-sha256$1000$07070707"));
        let back = Record::parse(&(line.clone() + "\n")).unwrap();
        assert_eq!(back, r);
        assert!(back.verify("correct horse"));
        assert!(!back.verify("correct hors"));
        assert!(!back.verify(""));
        // A different salt gives a different hash for the same password.
        assert_ne!(Record::new("correct horse", [8; SALT_LEN], 1000).hash, r.hash);
    }

    #[test]
    fn bad_records_are_refused() {
        let good = Record::new("x", [1; SALT_LEN], 10).to_line();
        assert!(Record::parse(&good).is_some());
        for bad in [
            "",
            "plain-text-password",
            &good.replace("nova-pbkdf2-sha256", "md5"),
            &good.replace("$10$", "$0$"),
            &good.replace("$10$", "$ten$"),
            &(good.clone() + "$extra"),
            &good[..good.len() - 2],
            &good.replace("$01", "$zz"),
        ] {
            assert!(Record::parse(bad).is_none(), "{bad}");
        }
        assert!(!ct_eq(b"ab", b"abc"));
        assert_eq!(Record::parse(&good).unwrap().to_line(), good.to_string());
    }
}
