//! Durable message identifiers.
//!
//! New messages use the workspace's existing UUIDv4 implementation. Legacy
//! messages without persisted IDs receive deterministic, scope-bound IDs.

/// Crockford base32 alphabet used by deterministic legacy identifiers.
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Generates an opaque, JSON-Pointer-safe durable message ID.
pub fn new_id() -> String {
    format!("m{}", uuid::Uuid::new_v4().simple())
}

/// Derives a stable ID for a legacy message that had no persisted identifier.
pub fn derive_legacy_id(scope_salt: &str, index: usize) -> String {
    let mut h1: u64 = 0xcbf29ce484222325;
    let mut h2: u64 = 0x84222325cbf29ce4;
    for b in scope_salt.bytes().chain(index.to_le_bytes()) {
        h1 ^= b as u64;
        h1 = h1.wrapping_mul(0x100000001b3);
        h2 = h2.wrapping_add(b as u64).wrapping_mul(0x100000001b3);
    }

    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&h1.to_be_bytes());
    bytes[8..].copy_from_slice(&h2.to_be_bytes());
    let mut out = String::with_capacity(27);
    out.push_str("m0");
    let mut accumulator: u32 = 0;
    let mut bits = 0_u8;
    for byte in bytes {
        accumulator = (accumulator << 8) | byte as u32;
        bits += 8;
        while bits >= 5 && out.len() < 27 {
            bits -= 5;
            out.push(CROCKFORD[((accumulator >> bits) & 0x1f) as usize] as char);
        }
    }
    while out.len() < 27 {
        out.push('0');
    }
    out
}

/// Accepts new UUID-backed IDs and deterministic legacy IDs.
pub fn is_valid_id(value: &str) -> bool {
    if !value.starts_with('m') {
        return false;
    }
    (value.len() == 33 && value.bytes().skip(1).all(|b| b.is_ascii_hexdigit()))
        || (value.len() == 27 && value.bytes().skip(1).all(|b| CROCKFORD.contains(&b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ids_are_unique_and_pointer_safe() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert!(is_valid_id(&a));
        assert!(!a.contains('/') && !a.contains('~'));
    }

    #[test]
    fn legacy_ids_are_stable_and_scope_bound() {
        let a = derive_legacy_id("alice/session-a", 3);
        assert_eq!(a, derive_legacy_id("alice/session-a", 3));
        assert_ne!(a, derive_legacy_id("alice/session-a", 4));
        assert_ne!(a, derive_legacy_id("alice/session-b", 3));
        assert!(is_valid_id(&a));
    }

    #[test]
    fn malformed_ids_are_rejected() {
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("m-short"));
        assert!(!is_valid_id("m/contains~unsafe"));
    }
}
