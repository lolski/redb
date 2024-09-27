use std::env::current_dir;
use std::mem::size_of;
use std::path::Path;
use std::sync::Arc;
use std::{fs, process, thread};
use heed::EnvFlags;
use tempfile::{NamedTempFile, TempDir};

mod common;
use common::*;

use std::time::{Duration, Instant};
use fastrand::Rng;
use rocksdb::TransactionDB;

const DATA_DIR: &str = "/tmp"; //"/mnt/balanced-pd/tmp";
const RNG_SEED: u64 = 3;

const PRELOAD_KEY_COUNT: usize = 1_000_000;
const PRELOAD_KEY_PER_TX_COUNT: usize = 1_000;
const BENCHMARK_OP_COUNT: usize = 1_000_00;
const BENCHMARK_OP_PER_TX_COUNT: usize = 100;
const BENCHMARK_ITER_PER_OP_COUNT: usize = 1000;
const KEY_SIZE: usize = 48;
// const PARALLELISM: usize = 1;

fn main() {
    let tmpdir = TempDir::new_in(DATA_DIR).unwrap();
    dbg!("Using benchmark dir: {}", &tmpdir);

    // instantiate rocksdb
    let mut rocksdb_opts = rocksdb::Options::default();
    rocksdb_opts.create_if_missing(true);
    let rocksdb_tx_opts = Default::default();
    let rocksdb_db: TransactionDB = rocksdb::TransactionDB::open(&rocksdb_opts, &rocksdb_tx_opts, tmpdir.path()).unwrap();
    let rocksdb_driver = RocksdbBenchDatabase::new(&rocksdb_db);
    let mut rocksdb_rng = create_rng();
    preload(&mut rocksdb_rng, &rocksdb_driver);
    benchmark(&mut rocksdb_rng, &rocksdb_driver);
    print_size(&tmpdir, &rocksdb_driver);

    // instantiate lmdb
    let lmdb_env = unsafe {
        let mut options = heed::EnvOpenOptions::new();
        options.map_size(4096 * 1024 * 1024);
        unsafe { options.flags(EnvFlags::NO_TLS | EnvFlags::NO_SYNC | EnvFlags::NO_READ_AHEAD); }
        options.open(tmpdir.path()).unwrap()
    };
    let lmdb_driver = HeedBenchDatabase::new(&lmdb_env);
    let mut lmdb_rng = create_rng();
    preload(&mut lmdb_rng, &lmdb_driver);
    benchmark(&mut lmdb_rng, &lmdb_driver);
    print_size(&tmpdir, &lmdb_driver);

    fs::remove_dir_all(&tmpdir).unwrap();
}

fn preload<T: BenchDatabase + Send + Sync>(mut rng: &mut Rng, driver: &T) {
    let start = Instant::now();
    for i in 0..(PRELOAD_KEY_COUNT / PRELOAD_KEY_PER_TX_COUNT) {
        let mut tx = driver.write_transaction();
        {
            let mut inserter = tx.get_inserter();
            for i in 0..PRELOAD_KEY_PER_TX_COUNT {
                let key = gen_key(&mut rng);
                let value = Vec::new();
                inserter.insert(&key, &value).unwrap()
            }
        }
        tx.commit().unwrap();
    }
    let end = Instant::now();
    let duration = end - start;
    println!(
        "{}: Preload done: loaded {} keys in {}ms",
        T::db_type_name(),
        PRELOAD_KEY_COUNT,
        duration.as_millis()
    );
}

fn benchmark<T: BenchDatabase + Send + Sync>(mut rng: &mut Rng, driver: &T) {
    let mut total_scanned_key = 0;
    let start = Instant::now();
    for i in 0..(BENCHMARK_OP_COUNT / BENCHMARK_OP_PER_TX_COUNT) {
        let tx = driver.read_transaction();
        {
            let reader = tx.get_reader();
            for i in 0..BENCHMARK_OP_PER_TX_COUNT {
                let key = gen_key(&mut rng); // TODO: should be a prefix of some value, not the value itself
                let mut scanned_key = 0;
                let mut iter = reader.range_from(&key);
                for i in 0..BENCHMARK_ITER_PER_OP_COUNT {
                    scanned_key += 1;
                    match iter.next() {
                        Some((_, value)) => {
                        }
                        None => {
                            break;
                        }
                    }
                }
                total_scanned_key += scanned_key;
            }
        }
        drop(tx);
    }
    let end = Instant::now();
    let duration = end - start;
    println!(
        "{}: Scan done: scanned {} total entries from {} scan ops, in {}ms",
        T::db_type_name(),
        total_scanned_key,
        BENCHMARK_OP_COUNT,
        duration.as_millis()
    );
}

fn print_size<T: BenchDatabase + Send + Sync>(tmpdir: &TempDir, _: &T) {
    let size = database_size(tmpdir.path());
    println!("{}: Database size: {} bytes", T::db_type_name(), size);
}

fn create_rng() -> fastrand::Rng {
    fastrand::Rng::with_seed(RNG_SEED)
}

fn gen_key(rng: &mut fastrand::Rng) -> [u8; KEY_SIZE] {
    let mut key = [0u8; KEY_SIZE];
    fill_slice(&mut key, rng);
    key
}

fn fill_slice(slice: &mut [u8], rng: &mut fastrand::Rng) {
    let mut i = 0;
    while i + size_of::<u128>() < slice.len() {
        let tmp = rng.u128(..);
        slice[i..(i + size_of::<u128>())].copy_from_slice(&tmp.to_le_bytes());
        i += size_of::<u128>()
    }
    if i + size_of::<u64>() < slice.len() {
        let tmp = rng.u64(..);
        slice[i..(i + size_of::<u64>())].copy_from_slice(&tmp.to_le_bytes());
        i += size_of::<u64>()
    }
    if i + size_of::<u32>() < slice.len() {
        let tmp = rng.u32(..);
        slice[i..(i + size_of::<u32>())].copy_from_slice(&tmp.to_le_bytes());
        i += size_of::<u32>()
    }
    if i + size_of::<u16>() < slice.len() {
        let tmp = rng.u16(..);
        slice[i..(i + size_of::<u16>())].copy_from_slice(&tmp.to_le_bytes());
        i += size_of::<u16>()
    }
    if i + size_of::<u8>() < slice.len() {
        slice[i] = rng.u8(..);
    }
}

fn database_size(path: &Path) -> u64 {
    let mut size = 0u64;
    for result in walkdir::WalkDir::new(path) {
        let entry = result.unwrap();
        size += entry.metadata().unwrap().len();
    }
    size
}