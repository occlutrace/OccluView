//! Build-time support for optional private HPS key embedding.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    println!("cargo:rerun-if-env-changed=OCCLUVIEW_HPS_EMBEDDED_KEY");
    println!("cargo:rerun-if-env-changed=OCCLUVIEW_HPS_KEY_SALT");

    let key = env::var("OCCLUVIEW_HPS_EMBEDDED_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| parse_key_config(&value))
        .unwrap_or_default();
    if !key.is_empty() && !(4..=56).contains(&key.len()) {
        println!("cargo:warning=OCCLUVIEW_HPS_EMBEDDED_KEY must decode to 4..56 bytes");
        return ExitCode::FAILURE;
    }

    // Derive the generated layout only from the key and explicit salt so the
    // same release inputs produce the same embedded bytes.
    let salt = env::var("OCCLUVIEW_HPS_KEY_SALT")
        .unwrap_or_else(|_| env::var("CARGO_PKG_VERSION").unwrap_or_default());
    let embedded = ObfuscatedKey::from_key(&key, salt.as_bytes());

    let Some(out_dir) = env::var_os("OUT_DIR") else {
        println!("cargo:warning=OUT_DIR is not set by Cargo");
        return ExitCode::FAILURE;
    };
    let out_dir = PathBuf::from(out_dir);
    let generated = embedded.to_rust_module();
    if let Err(error) = fs::write(out_dir.join("embedded_hps_key.rs"), generated) {
        println!("cargo:warning=could not write generated embedded HPS key module: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn parse_key_config(value: &str) -> Vec<u8> {
    if value.contains(',') {
        let parsed: Option<Vec<u8>> = value
            .split(',')
            .map(str::trim)
            .map(|token| token.parse::<u8>().ok())
            .collect();
        if let Some(bytes) = parsed.filter(|bytes| !bytes.is_empty()) {
            return bytes;
        }
    }
    value.as_bytes().to_vec()
}

struct ObfuscatedKey {
    order: Vec<u8>,
    payload: Vec<u8>,
    mask_a: Vec<u8>,
    mask_b: Vec<u8>,
    decoy_a: Vec<u8>,
    decoy_b: Vec<u8>,
    salt: [u8; 16],
}

impl ObfuscatedKey {
    fn from_key(key: &[u8], salt: &[u8]) -> Self {
        let mut prng = SplitMix64::new(seed_for_key(key, salt));
        let Ok(key_len) = u8::try_from(key.len()) else {
            unreachable!("validated embedded HPS key length fits in u8");
        };
        let mut order = (0..key_len).collect::<Vec<_>>();
        shuffle(&mut order, &mut prng);

        let mut payload = vec![0; key.len()];
        let mut mask_a = vec![0; key.len()];
        let mut mask_b = vec![0; key.len()];
        let mut salt = [0_u8; 16];
        for byte in &mut salt {
            *byte = prng.next_u8();
        }

        for (slot, original_idx) in order.iter().copied().enumerate() {
            let first_mask = prng.next_u8();
            let second_mask = prng.next_u8();
            let slot_low = slot.to_le_bytes()[0];
            let salt_mask = salt[slot % salt.len()].rotate_left(u32::from(slot_low & 7));
            let slot_mask = slot_low.wrapping_mul(31).rotate_right(1);
            mask_a[slot] = first_mask;
            mask_b[slot] = second_mask;
            payload[slot] =
                key[usize::from(original_idx)] ^ first_mask ^ second_mask ^ salt_mask ^ slot_mask;
        }

        let mut decoy_a = vec![0_u8; 64];
        let mut decoy_b = vec![0_u8; 64];
        for (slot, (left, right)) in decoy_a.iter_mut().zip(decoy_b.iter_mut()).enumerate() {
            let slot_low = slot.to_le_bytes()[0];
            let marker = key_len
                .wrapping_add(slot_low.wrapping_mul(13))
                .rotate_left(u32::from(slot_low & 7));
            *left = prng.next_u8() ^ marker ^ 0xa7;
            *right = prng.next_u8() ^ marker.rotate_right(3) ^ 0x5c;
        }
        decoy_a[0] ^= 0xa5;
        decoy_b[0] ^= 0x5a;

        Self {
            order,
            payload,
            mask_a,
            mask_b,
            decoy_a,
            decoy_b,
            salt,
        }
    }

    fn to_rust_module(&self) -> String {
        format!(
            "pub(super) const EMBEDDED_HPS_KEY_ORDER: &[u8] = &[{}];\n\
             pub(super) const EMBEDDED_HPS_KEY_PAYLOAD: &[u8] = &[{}];\n\
             pub(super) const EMBEDDED_HPS_KEY_MASK_A: &[u8] = &[{}];\n\
             pub(super) const EMBEDDED_HPS_KEY_MASK_B: &[u8] = &[{}];\n\
             pub(super) const EMBEDDED_HPS_KEY_DECOY_A: &[u8] = &[{}];\n\
             pub(super) const EMBEDDED_HPS_KEY_DECOY_B: &[u8] = &[{}];\n\
             pub(super) const EMBEDDED_HPS_KEY_SALT: [u8; 16] = [{}];\n",
            hex_bytes(&self.order),
            hex_bytes(&self.payload),
            hex_bytes(&self.mask_a),
            hex_bytes(&self.mask_b),
            hex_bytes(&self.decoy_a),
            hex_bytes(&self.decoy_b),
            hex_bytes(&self.salt),
        )
    }
}

fn shuffle(values: &mut [u8], prng: &mut SplitMix64) {
    for i in (1..values.len()).rev() {
        let j = prng.next_usize(i + 1);
        values.swap(i, j);
    }
}

/// FNV-1a over the key and the build salt. Deterministic by construction:
/// the same two inputs always produce the same layout.
fn seed_for_key(key: &[u8], salt: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in key.iter().chain(b":".iter()).chain(salt.iter()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn next_u8(&mut self) -> u8 {
        self.next_u64().to_le_bytes()[0]
    }

    fn next_usize(&mut self, upper_exclusive: usize) -> usize {
        if upper_exclusive <= 1 {
            return 0;
        }
        let Ok(upper) = u64::try_from(upper_exclusive) else {
            return 0;
        };
        let value = self.next_u64() % upper;
        usize::try_from(value).unwrap_or_default()
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("0x{byte:02x}"))
        .collect::<Vec<_>>()
        .join(", ")
}
