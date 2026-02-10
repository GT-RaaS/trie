// Copyright 2017, 2018 Parity Technologies
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use byteorder::{ByteOrder, LittleEndian};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use digest::Digest;
use memory_db::{HashKey, MemoryDB};
use rand::rngs::SmallRng;
use rand::{RngCore, SeedableRng};
use reference_trie::ReferenceNodeCodec;
use std::hash::Hasher;
use trie_db::{Trie, TrieConfiguration, TrieDBMutBuilder, TrieLayout, TrieMut};

criterion_group!(benches, trie_write_benchmark, trie_read_benchmark,);
criterion_main!(benches);

// 固定参数：20000个键值对
const TOTAL_KEYS: usize = 200000;
const PAL_PREFIX_SIZE: usize = 16;
const STORAGE_PREFIX_SIZE: usize = 16;
const ORIGINAL_KEY_SIZE: usize = 32;
const VALUE_SIZE: usize = 64;
const SEED: u32 = 42;

mod blake2b_hasher;
use blake2b_hasher::Blake2bHasher;

/// Trie layout using extension nodes.
#[derive(Default, Clone)]
pub struct Layout;

impl TrieLayout for Layout {
	const USE_EXTENSION: bool = true;
	const ALLOW_EMPTY: bool = false;
	const MAX_INLINE_VALUE: Option<u32> = None;
	type Hash = Blake2bHasher;
	type Codec = ReferenceNodeCodec<Blake2bHasher>;
}

impl TrieConfiguration for Layout {}

pub fn twox_128_into(data: &[u8], dest: &mut [u8; 16]) {
	let r0 = twox_hash::XxHash::with_seed(0).chain_update(data).finish();
	let r1 = twox_hash::XxHash::with_seed(1).chain_update(data).finish();
	LittleEndian::write_u64(&mut dest[0..8], r0);
	LittleEndian::write_u64(&mut dest[8..16], r1);
}

/// Do a XX 128-bit hash and return result.
pub fn twox_128(data: &[u8]) -> [u8; 16] {
	let mut r: [u8; 16] = [0; 16];
	twox_128_into(data, &mut r);
	r
}

// 生成 hash_key: twoxhash(pallet) + twoxhash(storageprefix) + blake2b_hash(key)
fn generate_hash_key(pallet_name: &[u8], storage_prefix: &[u8], original_key: &[u8]) -> Vec<u8> {
	let pallet_hashed = twox_128(pallet_name);
	let storage_hashed = twox_128(storage_prefix);
	let key_hashed = blake2b_hasher::blake2b_hash(original_key);

	let mut hash_key = Vec::with_capacity(16 + 16 + 32);
	hash_key.extend_from_slice(&pallet_hashed);
	hash_key.extend_from_slice(&storage_hashed);
	hash_key.extend_from_slice(&key_hashed);

	hash_key
}

// 生成 append_key: pallet + storageprefix + key
fn generate_append_key(pallet_name: &[u8], storage_prefix: &[u8], original_key: &[u8]) -> Vec<u8> {
	let mut append_key =
		Vec::with_capacity(pallet_name.len() + storage_prefix.len() + original_key.len());
	append_key.extend_from_slice(pallet_name);
	append_key.extend_from_slice(storage_prefix);
	append_key.extend_from_slice(original_key);
	append_key
}

#[derive(Clone)]
pub struct TestData {
	hash_key: Vec<u8>, // twoxhash(pallet) + twoxhash(storageprefix) + blake2b_hash(key) - 64字节
	append_key: Vec<u8>, // pallet + storageprefix + key - 96字节
	value: Vec<u8>,    // 64字节
}

impl TestData {
	fn new(hash_key: Vec<u8>, append_key: Vec<u8>, value: Vec<u8>) -> Self {
		Self { hash_key, append_key, value }
	}
}

// 生成固定20000个TestData
fn generate_20000_testdata() -> Vec<TestData> {
	let mut rng = SmallRng::seed_from_u64(SEED as u64);
	let mut data = Vec::with_capacity(TOTAL_KEYS);

	println!("生成 {} 个TestData...", TOTAL_KEYS);

	// 预先生成一些随机的pallet名称和存储前缀
	let mut pallet_names = Vec::new();
	let mut storage_prefixes = Vec::new();

	for i in 0..20 {
		// 生成20个不同的pallet名称和存储前缀
		let mut pallet_name = vec![0u8; PAL_PREFIX_SIZE];
		rng.fill_bytes(&mut pallet_name);
		// 添加一些可读的标识
		let pallet_str = format!("Pallet{:03}", i);
		let bytes = pallet_str.as_bytes();
		let len = bytes.len().min(PAL_PREFIX_SIZE);
		pallet_name[0..len].copy_from_slice(&bytes[0..len]);

		let mut storage_prefix = vec![0u8; STORAGE_PREFIX_SIZE];
		rng.fill_bytes(&mut storage_prefix);
		let storage_str = format!("Storage{:03}", i);
		let bytes = storage_str.as_bytes();
		let len = bytes.len().min(STORAGE_PREFIX_SIZE);
		storage_prefix[0..len].copy_from_slice(&bytes[0..len]);

		pallet_names.push(pallet_name);
		storage_prefixes.push(storage_prefix);
	}

	for i in 0..TOTAL_KEYS {
		// 随机选择一个pallet_name和storage_prefix
		let pallet_idx = i % pallet_names.len();
		let storage_idx = (i + 5) % storage_prefixes.len(); // 使用不同的索引

		let pallet_name = pallet_names[pallet_idx].clone();
		let storage_prefix = storage_prefixes[storage_idx].clone();

		// 生成原始key（32字节）
		let mut original_key = vec![0u8; ORIGINAL_KEY_SIZE];
		rng.fill_bytes(&mut original_key);
		// 在original_key中存储索引，确保唯一性
		original_key[0..8].copy_from_slice(&i.to_be_bytes());

		// 生成hash_key (twoxhash(pallet) + twoxhash(storageprefix) + blake2b_hash(key))
		let hash_key = generate_hash_key(&pallet_name, &storage_prefix, &original_key);

		// 生成append_key (pallet + storageprefix + key)
		let append_key = generate_append_key(&pallet_name, &storage_prefix, &original_key);

		// 生成value (64字节)
		let mut value = vec![0u8; VALUE_SIZE];
		rng.fill_bytes(&mut value);
		// 在value中存储索引用于验证
		value[0..8].copy_from_slice(&i.to_be_bytes());

		// 验证hash_key长度：twox128(16字节) + twox128(16字节) + blake2b(32字节) = 64字节
		assert_eq!(hash_key.len(), 64, "hash_key长度应为64字节");
		// 验证append_key长度：pallet(16字节) + storage(16字节) + key(32字节) = 64字节
		assert_eq!(append_key.len(), 64, "append_key长度应为64字节");

		// 创建TestData
		let test_data = TestData::new(hash_key, append_key, value);

		data.push(test_data);
	}

	println!("TestData生成完成，共 {} 个", data.len());
	println!("每个TestData大小:");
	println!("  pallet_name: {}字节", PAL_PREFIX_SIZE);
	println!("  storage_prefix: {}字节", STORAGE_PREFIX_SIZE);
	println!("  original_key: {}字节", ORIGINAL_KEY_SIZE);
	println!("  hash_key: {}字节 (16+16+32)", 16 + 16 + 32);
	println!(
		"  append_key: {}字节 (32+32+32)",
		PAL_PREFIX_SIZE + STORAGE_PREFIX_SIZE + ORIGINAL_KEY_SIZE
	);
	println!("  value: {}字节", VALUE_SIZE);
	println!(
		"总数据大小: {} KB",
		(data.len()
			* (PAL_PREFIX_SIZE + STORAGE_PREFIX_SIZE + ORIGINAL_KEY_SIZE + 64 + 96 + VALUE_SIZE))
			as f64 / 1024.0
	);

	data
}

// 从TestData中提取用于trie测试的键值对
fn extract_trie_data(test_data: &[TestData]) -> Vec<(Vec<u8>, Vec<u8>)> {
	// 使用append_key作为trie的key（96字节）
	let v = env::var("USEHASH").unwrap_or_default();
	if (v.is_empty()) {
		test_data.iter().map(|td| (td.append_key.clone(), td.value.clone())).collect()
	} else {
		test_data.iter().map(|td| (td.hash_key.clone(), td.value.clone())).collect()
	}
}

fn trie_write_benchmark(c: &mut Criterion) {
	let test_data = generate_20000_testdata();
	let trie_data = extract_trie_data(&test_data);

	// 顺序写入测试
	c.bench_function("trie_write_20000_testdata", |b| {
		b.iter(|| {
			let mut root = Default::default();
			{
				let mut mdb = MemoryDB::<Blake2bHasher, HashKey<_>, _>::default();
				let mut trie = TrieDBMutBuilder::<Layout>::new(&mut mdb, &mut root).build();

				for (key, value) in &trie_data {
					trie.insert(&key, &value).expect("插入失败");
				}
			}
			black_box(root);
		})
	});

	// 更新写入测试
	c.bench_function("trie_update_20000_testdata", |b| {
		b.iter_batched(
			|| {
				// 先创建包含数据的trie
				let mut root = Default::default();
				let mut mdb = MemoryDB::<Blake2bHasher, HashKey<_>, _>::default();
				{
					let mut trie = TrieDBMutBuilder::<Layout>::new(&mut mdb, &mut root).build();

					for (key, value) in &trie_data {
						trie.insert(&key, &value).expect("插入失败");
					}
				}

				(mdb, root)
			},
			|(mut mdb, mut root)| {
				// 更新1000个键的值
				{
					let mut trie = TrieDBMutBuilder::<Layout>::new(&mut mdb, &mut root).build();

					for i in 0..1000 {
						if let Some((key, _)) = trie_data.get(i) {
							let new_value = vec![(i % 256) as u8; VALUE_SIZE];
							trie.insert(&key, &new_value).expect("更新失败");
						}
					}
				}
				black_box(root);
			},
			criterion::BatchSize::SmallInput,
		)
	});
}

fn trie_read_benchmark(c: &mut Criterion) {
	let test_data = generate_20000_testdata();
	let trie_data = extract_trie_data(&test_data);

	// 先创建好包含数据的trie
	let mut root = Default::default();
	let mut mdb = MemoryDB::<Blake2bHasher, HashKey<_>, _>::default();
	{
		let mut trie = TrieDBMutBuilder::<Layout>::new(&mut mdb, &mut root).build();

		for (key, value) in &trie_data {
			trie.insert(&key, &value).expect("插入失败");
		}
	}

	// 构建只读trie
	let trie_db = trie_db::TrieDBBuilder::<Layout>::new(&mdb, &root).build();

	println!("Trie构建完成，准备进行读测试...");

	// 测试1: 顺序读取前1000个键
	let sequential_keys: Vec<_> = trie_data.iter().take(1000).map(|(k, _)| k.clone()).collect();

	c.bench_function("trie_read_sequential_1000_keys", |b| {
		b.iter(|| {
			for key in &sequential_keys {
				let result = trie_db.get(&key).expect("读取失败");
				black_box(result);
			}
		})
	});

	// 测试2: 随机读取1000个键
	use rand::seq::SliceRandom;

	let mut rng = SmallRng::seed_from_u64(12345);
	let mut random_keys: Vec<_> = trie_data.iter().map(|(k, _)| k.clone()).collect();
	random_keys.shuffle(&mut rng);
	let random_keys_subset: Vec<_> = random_keys.iter().take(1000).cloned().collect();

	c.bench_function("trie_read_random_1000_keys", |b| {
		b.iter(|| {
			for key in &random_keys_subset {
				let result = trie_db.get(&key).expect("读取失败");
				black_box(result);
			}
		})
	});

	// 测试3: 热点读取（多次读取同一个键）
	let hotspot_key = &trie_data[TOTAL_KEYS / 2].0;

	c.bench_function("trie_read_hotspot_1000_times", |b| {
		b.iter(|| {
			for _ in 0..1000 {
				let result = trie_db.get(&hotspot_key).expect("读取失败");
				black_box(result);
			}
		})
	});

	// 测试4: 读取不存在的键
	let mut nonexistent_keys = Vec::new();
	let mut rng2 = SmallRng::seed_from_u64(67890);

	for _ in 0..1000 {
		// 生成不存在的append_key
		let mut pallet_name = vec![0u8; PAL_PREFIX_SIZE];
		rng2.fill_bytes(&mut pallet_name);
		let mut storage_prefix = vec![0u8; STORAGE_PREFIX_SIZE];
		rng2.fill_bytes(&mut storage_prefix);
		let mut original_key = vec![0u8; ORIGINAL_KEY_SIZE];
		rng2.fill_bytes(&mut original_key);
		// 确保original_key的前8字节是最大的u64值，这样它不会存在于数据中
		original_key[0..8].copy_from_slice(&u64::MAX.to_be_bytes());

		let append_key = generate_append_key(&pallet_name, &storage_prefix, &original_key);
		nonexistent_keys.push(append_key);
	}

	c.bench_function("trie_read_nonexistent_1000_keys", |b| {
		b.iter(|| {
			for key in &nonexistent_keys {
				let result = trie_db.get(&key).expect("读取失败");
				black_box(result);
			}
		})
	});
}
