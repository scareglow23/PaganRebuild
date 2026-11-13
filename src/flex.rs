//! FLX archives: a 0x80 byte header (entry count at 0x54) followed by a
//! table of (offset, size) pairs.

use std::fs;
use std::io;
use std::path::Path;

#[derive(Clone)]
pub struct Flex {
    data: Vec<u8>,
    table: Vec<(u32, u32)>,
}

pub fn u16le(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}

pub fn u32le(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

impl Flex {
    pub fn open(path: &Path) -> io::Result<Flex> {
        Flex::from_bytes(fs::read(path)?).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))
    }

    pub fn from_bytes(data: Vec<u8>) -> io::Result<Flex> {
        if data.len() < 0x80 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not a flex file"));
        }
        let count = u32le(&data, 0x54) as usize;
        if 0x80 + count * 8 > data.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated flex table"));
        }
        let table = (0..count).map(|i| (u32le(&data, 0x80 + i * 8), u32le(&data, 0x84 + i * 8))).collect();
        Ok(Flex { data, table })
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Entry `i`, or `None` if it is empty or out of range.
    pub fn get(&self, i: usize) -> Option<&[u8]> {
        let &(off, size) = self.table.get(i)?;
        let (off, size) = (off as usize, size as usize);
        if size == 0 || off + size > self.data.len() {
            return None;
        }
        Some(&self.data[off..off + size])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An archive with these entries (None: an empty one).
    fn archive(entries: &[Option<&[u8]>]) -> Vec<u8> {
        let mut d = vec![0u8; 0x80 + entries.len() * 8];
        d[0x54..0x58].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        for (i, e) in entries.iter().enumerate() {
            if let Some(e) = e {
                let (off, len) = (d.len() as u32, e.len() as u32);
                d[0x80 + i * 8..0x84 + i * 8].copy_from_slice(&off.to_le_bytes());
                d[0x84 + i * 8..0x88 + i * 8].copy_from_slice(&len.to_le_bytes());
                d.extend_from_slice(e);
            }
        }
        d
    }

    #[test]
    fn entries_by_number() {
        let flex = Flex::from_bytes(archive(&[Some(b"one"), None, Some(b"three")])).unwrap();
        assert_eq!(flex.len(), 3);
        assert_eq!(flex.get(0), Some(&b"one"[..]));
        assert_eq!(flex.get(1), None);
        assert_eq!(flex.get(2), Some(&b"three"[..]));
        assert_eq!(flex.get(3), None);
    }

    /// Damaged files are refused or give no entry; nothing is read
    /// beyond the end of the file.
    #[test]
    fn damaged_files() {
        assert!(Flex::from_bytes(vec![0; 0x7F]).is_err());
        // the table says more entries than the file holds
        let mut d = archive(&[Some(b"one")]);
        d[0x54] = 200;
        assert!(Flex::from_bytes(d).is_err());
        // an entry reaching beyond the end
        let mut d = archive(&[Some(b"one")]);
        d[0x84] = 50;
        assert_eq!(Flex::from_bytes(d).unwrap().get(0), None);
        // an offset past the end
        let mut d = archive(&[Some(b"one")]);
        d[0x80..0x84].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        assert_eq!(Flex::from_bytes(d).unwrap().get(0), None);
    }

    #[test]
    fn numbers_low_byte_first() {
        let d = [0x34, 0x12, 0x78, 0x56];
        assert_eq!(u16le(&d, 0), 0x1234);
        assert_eq!(u32le(&d, 0), 0x5678_1234);
    }
}
