//! Encoding utilities for compact representation.
//!
//! This module provides various encoding schemes for memory-efficient storage:
//! - Variable-length integers (VarInt)
//! - Prefix compression
//! - Delta encoding

/// Encode a u64 as a variable-length integer.
///
/// Uses 1-10 bytes depending on the value:
/// - 0-127: 1 byte
/// - 128-16383: 2 bytes
/// - etc.
pub fn encode_varint(mut value: u64, buf: &mut [u8]) -> usize {
    let mut i = 0;
    while value >= 0x80 {
        buf[i] = (value as u8) | 0x80;
        value >>= 7;
        i += 1;
    }
    buf[i] = value as u8;
    i + 1
}

/// Decode a variable-length integer.
///
/// Returns (value, bytes_consumed).
pub fn decode_varint(buf: &[u8]) -> (u64, usize) {
    let mut value = 0u64;
    let mut shift = 0;
    let mut i = 0;
    
    loop {
        let byte = buf[i];
        value |= ((byte & 0x7F) as u64) << shift;
        i += 1;
        
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    
    (value, i)
}

/// Calculate the number of bytes needed to encode a value as varint.
pub fn varint_size(value: u64) -> usize {
    if value == 0 {
        return 1;
    }
    let bits = 64 - value.leading_zeros() as usize;
    (bits + 6) / 7
}

/// Encode a length-prefixed byte slice.
pub fn encode_bytes(data: &[u8], buf: &mut Vec<u8>) {
    let mut len_buf = [0u8; 10];
    let len_size = encode_varint(data.len() as u64, &mut len_buf);
    buf.extend_from_slice(&len_buf[..len_size]);
    buf.extend_from_slice(data);
}

/// Decode a length-prefixed byte slice.
///
/// Returns (data, bytes_consumed).
pub fn decode_bytes(buf: &[u8]) -> (&[u8], usize) {
    let (len, len_size) = decode_varint(buf);
    let len = len as usize;
    (&buf[len_size..len_size + len], len_size + len)
}

/// Compute the shared prefix length between two byte slices.
pub fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

/// Encode a key using front-coding (prefix compression) relative to a previous key.
///
/// Returns (shared_prefix_len, suffix).
pub fn front_encode<'a>(prev_key: &[u8], key: &'a [u8]) -> (usize, &'a [u8]) {
    let shared = common_prefix_len(prev_key, key);
    (shared, &key[shared..])
}

/// Decode a front-coded key.
pub fn front_decode(prev_key: &[u8], shared_len: usize, suffix: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(shared_len + suffix.len());
    result.extend_from_slice(&prev_key[..shared_len]);
    result.extend_from_slice(suffix);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_varint_roundtrip() {
        let test_values = [0, 1, 127, 128, 16383, 16384, u64::MAX];
        
        for &value in &test_values {
            let mut buf = [0u8; 10];
            let size = encode_varint(value, &mut buf);
            let (decoded, decoded_size) = decode_varint(&buf);
            
            assert_eq!(decoded, value);
            assert_eq!(size, decoded_size);
            assert_eq!(size, varint_size(value));
        }
    }

    #[test]
    fn test_bytes_roundtrip() {
        let data = b"hello world";
        let mut buf = Vec::new();
        encode_bytes(data, &mut buf);
        
        let (decoded, size) = decode_bytes(&buf);
        assert_eq!(decoded, data);
        assert_eq!(size, buf.len());
    }

    #[test]
    fn test_front_coding() {
        let key1 = b"user:alice";
        let key2 = b"user:bob";
        let key3 = b"user:carol";
        
        let (shared, suffix) = front_encode(key1, key2);
        assert_eq!(shared, 5); // "user:"
        assert_eq!(suffix, b"bob");
        
        let decoded = front_decode(key1, shared, suffix);
        assert_eq!(decoded, key2);
        
        let (shared2, suffix2) = front_encode(key2, key3);
        assert_eq!(shared2, 5);
        assert_eq!(suffix2, b"carol");
    }

    #[test]
    fn test_common_prefix_len() {
        assert_eq!(common_prefix_len(b"hello", b"help"), 3);
        assert_eq!(common_prefix_len(b"hello", b"world"), 0);
        assert_eq!(common_prefix_len(b"hello", b"hello"), 5);
        assert_eq!(common_prefix_len(b"", b"hello"), 0);
    }
}
