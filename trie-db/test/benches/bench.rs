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

// RocksDB 依赖
use rocksdb::{DB, Options};
use tempfile::TempDir;

criterion_group!(
    benches,
    trie_write_benchmark,
    trie_read_benchmark,
    rocksdb_write_benchmark,
    rocksdb_read_benchmark
);
criterion_main!(benches);

// 固定参数：20000个键值对
const TOTAL_KEYS: usize = 20000;
const KEY_SIZE: usize = 32;
const VALUE_SIZE: usize = 64;
const SEED: u64 = 42;

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

// 生成测试数据：返回 (key, value) 对的列表
fn generate_test_data() -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut rng = SmallRng::seed_from_u64(SEED);
    let mut data = Vec::with_capacity(TOTAL_KEYS);

    println!("生成 {} 个键值对...", TOTAL_KEYS);

    for i in 0..TOTAL_KEYS {
        // 生成随机 key (32 字节)
        let mut key = vec![0u8; KEY_SIZE];
        rng.fill_bytes(&mut key);
        // 在 key 中写入索引以增加变化
        key[0..8].copy_from_slice(&i.to_be_bytes());

        // 生成随机 value (64 字节)
        let mut value = vec![0u8; VALUE_SIZE];
        rng.fill_bytes(&mut value);

        data.push((key, value));
    }

    println!("生成完成，每个 key 大小: {} 字节，value 大小: {} 字节", KEY_SIZE, VALUE_SIZE);
    println!("总数据大小: {} KB", (data.len() * (KEY_SIZE + VALUE_SIZE)) as f64 / 1024.0);

    data
}

// ==================== Trie 基准测试 ====================

fn trie_write_benchmark(c: &mut Criterion) {
    let test_data = generate_test_data();

    c.bench_function("trie_write_20000_keys", |b| {
        b.iter(|| {
            let mut root = Default::default();
            {
                let mut mdb = MemoryDB::<Blake2bHasher, HashKey<_>, _>::default();
                let mut trie = TrieDBMutBuilder::<Layout>::new(&mut mdb, &mut root).build();

                for (key, value) in &test_data {
                    trie.insert(key, value).expect("插入失败");
                }
            }
            black_box(root);
        })
    });
}

fn trie_read_benchmark(c: &mut Criterion) {
    let test_data = generate_test_data();

    // 先创建包含数据的 trie
    let mut root = Default::default();
    let mut mdb = MemoryDB::<Blake2bHasher, HashKey<_>, _>::default();
    {
        let mut trie = TrieDBMutBuilder::<Layout>::new(&mut mdb, &mut root).build();
        for (key, value) in &test_data {
            trie.insert(key, value).expect("插入失败");
        }
    }

    // 构建只读 trie
    let trie_db = trie_db::TrieDBBuilder::<Layout>::new(&mdb, &root).build();

    println!("Trie 构建完成，准备进行读测试...");

    // ---------- 顺序读取测试：使用排序后的 key ----------
    let mut all_keys: Vec<Vec<u8>> = test_data.iter().map(|(k, _)| k.clone()).collect();
    all_keys.sort(); // 按字典序排序
    let sequential_keys: Vec<_> = all_keys.into_iter().take(1000).collect();

    c.bench_function("trie_read_sorted_sequential_1000_keys", |b| {
        b.iter(|| {
            for key in &sequential_keys {
                let result = trie_db.get(key).expect("读取失败");
                black_box(result);
            }
        })
    });

    // ---------- 随机读取测试 ----------
    use rand::seq::SliceRandom;
    let mut rng = SmallRng::seed_from_u64(12345);
    let mut all_keys_random: Vec<_> = test_data.iter().map(|(k, _)| k.clone()).collect();
    all_keys_random.shuffle(&mut rng);
    let random_keys: Vec<_> = all_keys_random.into_iter().take(1000).collect();

    c.bench_function("trie_read_random_1000_keys", |b| {
        b.iter(|| {
            for key in &random_keys {
                let result = trie_db.get(key).expect("读取失败");
                black_box(result);
            }
        })
    });

    // ---------- 热点读取测试 ----------
    let hotspot_key = &test_data[TOTAL_KEYS / 2].0;
    c.bench_function("trie_read_hotspot_1000_times", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                let result = trie_db.get(hotspot_key).expect("读取失败");
                black_box(result);
            }
        })
    });

    // ---------- 不存在键测试 ----------
    let mut nonexistent_keys = Vec::new();
    let mut rng2 = SmallRng::seed_from_u64(67890);
    for _ in 0..1000 {
        let mut key = vec![0u8; KEY_SIZE];
        rng2.fill_bytes(&mut key);
        nonexistent_keys.push(key);
    }

    c.bench_function("trie_read_nonexistent_1000_keys", |b| {
        b.iter(|| {
            for key in &nonexistent_keys {
                let result = trie_db.get(key).expect("读取失败");
                black_box(result);
            }
        })
    });
}

// ==================== RocksDB 基准测试 ====================

fn rocksdb_write_benchmark(c: &mut Criterion) {
    let test_data = generate_test_data();

    c.bench_function("rocksdb_write_20000_keys", |b| {
        b.iter_batched(
            || {
                // 每次迭代创建新的临时目录
                TempDir::new_in("/dev/shm").expect("创建临时目录失败")
            },
            |tmp_dir| {
                // 打开 RocksDB
                let mut opts = Options::default();
                opts.create_if_missing(true);
                let db = DB::open(&opts, tmp_dir.path()).expect("打开 RocksDB 失败");

                // 批量写入
                for (key, value) in &test_data {
                    db.put(key, value).expect("写入失败");
                }

                black_box(());
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

fn rocksdb_read_benchmark(c: &mut Criterion) {
    let test_data = generate_test_data();

    // 预先创建并填充 RocksDB
    let tmp_dir = TempDir::new().expect("创建临时目录失败");
    {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, tmp_dir.path()).expect("打开 RocksDB 失败");
        for (key, value) in &test_data {
            db.put(key, value).expect("写入失败");
        }
        db.flush().ok(); // 确保数据持久化
    }

    // 打开只读数据库
    let opts = Options::default();
    let db = DB::open(&opts, tmp_dir.path()).expect("重新打开 RocksDB 失败");

    println!("RocksDB 构建完成，准备进行读测试...");

    // ---------- 顺序读取测试：使用排序后的 key ----------
    let mut all_keys: Vec<Vec<u8>> = test_data.iter().map(|(k, _)| k.clone()).collect();
    all_keys.sort();
    let sequential_keys: Vec<_> = all_keys.into_iter().take(1000).collect();

    c.bench_function("rocksdb_read_sorted_sequential_1000_keys", |b| {
        b.iter(|| {
            for key in &sequential_keys {
                let result = db.get(key).expect("读取失败");
                black_box(result);
            }
        })
    });

    // ---------- 随机读取测试 ----------
    use rand::seq::SliceRandom;
    let mut rng = SmallRng::seed_from_u64(12345);
    let mut all_keys_random: Vec<_> = test_data.iter().map(|(k, _)| k.clone()).collect();
    all_keys_random.shuffle(&mut rng);
    let random_keys: Vec<_> = all_keys_random.into_iter().take(1000).collect();

    c.bench_function("rocksdb_read_random_1000_keys", |b| {
        b.iter(|| {
            for key in &random_keys {
                let result = db.get(key).expect("读取失败");
                black_box(result);
            }
        })
    });

    // ---------- 热点读取测试 ----------
    let hotspot_key = &test_data[TOTAL_KEYS / 2].0;
    c.bench_function("rocksdb_read_hotspot_1000_times", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                let result = db.get(hotspot_key).expect("读取失败");
                black_box(result);
            }
        })
    });

    // ---------- 不存在键测试 ----------
    let mut nonexistent_keys = Vec::new();
    let mut rng2 = SmallRng::seed_from_u64(67890);
    for _ in 0..1000 {
        let mut key = vec![0u8; KEY_SIZE];
        rng2.fill_bytes(&mut key);
        nonexistent_keys.push(key);
    }

    c.bench_function("rocksdb_read_nonexistent_1000_keys", |b| {
        b.iter(|| {
            for key in &nonexistent_keys {
                let result = db.get(key).expect("读取失败");
                black_box(result);
            }
        })
    });

    // 临时目录会在函数结束时自动清理
}