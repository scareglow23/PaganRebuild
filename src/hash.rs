//! Hash tables that keep the same order from one run to the next.
//!
//! The tables of the standard library are seeded at random on every
//! start, so going through one (all fighters, all walking actors) went in
//! a different order each time, and the game with it: the same test
//! commands could end differently. These hash the same way every time.
//!
//! The hash function is written out here (SipHash 1-3 with a zero key,
//! what the standard library's `DefaultHasher` has been so far): that one
//! may change with any version of Rust, and the order of the game with it.

use std::hash::{BuildHasherDefault, Hasher};

pub type Fixed = BuildHasherDefault<Sip13>;
pub type HashMap<K, V> = std::collections::HashMap<K, V, Fixed>;
pub type HashSet<K> = std::collections::HashSet<K, Fixed>;

/// SipHash 1-3 (one round per word, three at the end), key 0.
#[derive(Clone)]
pub struct Sip13 {
    v: [u64; 4],
    /// Bytes not yet making a full word (the low `ntail` bytes), and how
    /// many bytes came in all.
    tail: u64,
    ntail: u32,
    len: u64,
}

impl Default for Sip13 {
    fn default() -> Sip13 {
        Sip13 { v: [0x736f_6d65_7073_6575, 0x646f_7261_6e64_6f6d, 0x6c79_6765_6e65_7261, 0x7465_6462_7974_6573], tail: 0, ntail: 0, len: 0 }
    }
}

impl Sip13 {
    fn round(v: &mut [u64; 4]) {
        v[0] = v[0].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(13) ^ v[0];
        v[0] = v[0].rotate_left(32);
        v[2] = v[2].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(16) ^ v[2];
        v[0] = v[0].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(21) ^ v[0];
        v[2] = v[2].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(17) ^ v[2];
        v[2] = v[2].rotate_left(32);
    }

    fn word(&mut self, m: u64) {
        self.v[3] ^= m;
        Self::round(&mut self.v);
        self.v[0] ^= m;
    }
}

impl Hasher for Sip13 {
    fn write(&mut self, bytes: &[u8]) {
        self.len = self.len.wrapping_add(bytes.len() as u64);
        for &b in bytes {
            self.tail |= (b as u64) << (8 * self.ntail);
            self.ntail += 1;
            if self.ntail == 8 {
                let m = self.tail;
                self.word(m);
                (self.tail, self.ntail) = (0, 0);
            }
        }
    }

    fn finish(&self) -> u64 {
        let mut v = self.v;
        let last = (self.len & 0xFF) << 56 | self.tail;
        v[3] ^= last;
        Self::round(&mut v);
        v[0] ^= last;
        v[2] ^= 0xFF;
        for _ in 0..3 {
            Self::round(&mut v);
        }
        v[0] ^ v[1] ^ v[2] ^ v[3]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{BuildHasher, Hash};

    fn hash<T: Hash + ?Sized>(t: &T) -> u64 {
        Fixed::default().hash_one(t)
    }

    /// The tables must keep the order they had with the standard library's
    /// hasher of the time (savegames and tests were made with it): these
    /// are its values.
    #[test]
    fn same_as_before() {
        assert_eq!(hash(&1u16), 0x91ba_6b64_3800_21f9);
        assert_eq!(hash(&47usize), 0x438d_e4fa_b697_e4a2);
        assert_eq!(hash(&(3i32, -4i32)), 0x8c9d_a37d_712c_f397);
        assert_eq!(hash("Avatar"), 0xbf3c_4a1a_88ff_70c3);
    }
}
