use hash_db::Hasher;
use keccak_hasher::KeccakHasher;
#[cfg(feature = "std")]
use std::hash;

#[cfg(not(feature = "std"))]
use core::hash;
use hash256_std_hasher::Hash256StdHasher;
/// The `Keccak` hash output type.
pub type BlakeHash = [u8; 32];

/// Concrete `Hasher` impl for the Keccak-256 hash
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Blake2bHasher;
impl Hasher for Blake2bHasher {
	type Out = BlakeHash;

	type StdHasher = Hash256StdHasher;

	const LENGTH: usize = 32;

	fn hash(x: &[u8]) -> Self::Out {
		blake2b_hash(x)
	}
}

// Blake2b 哈希函数 - 返回32字节
pub fn blake2b_hash(data: &[u8]) -> [u8; 32] {
	let hash_bytes = blake2b_simd::Params::new().hash_length(32).hash(data);

	let mut result = [0u8; 32];
	result.copy_from_slice(&hash_bytes.as_bytes()[..32]);
	result
}
