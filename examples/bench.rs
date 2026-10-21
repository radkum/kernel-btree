//! Rough timing vs std: `cargo run --release --example bench`.

use std::collections::BTreeMap as StdMap;
use std::hint::black_box;
use std::time::Instant;

use kernel_btree::BTreeMap;

const N: u64 = 1_000_000;

fn keys() -> impl Iterator<Item = u64> {
    (0..N).map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

fn time(name: &str, f: impl FnOnce()) {
    let start = Instant::now();
    f();
    println!("  {name:<8} {:>8.1} ms", start.elapsed().as_secs_f64() * 1e3);
}

fn main() {
    println!("kernel-btree, {N} random u64 keys");
    let mut map = BTreeMap::new();
    time("insert", || keys().for_each(|k| _ = map.try_insert(k, k)));
    time("get", || keys().for_each(|k| _ = black_box(map.get(&k))));
    time("iter", || _ = black_box(map.iter().map(|(_, v)| v).sum::<u64>()));
    time("remove", || keys().for_each(|k| _ = black_box(map.remove(&k))));

    println!("std::collections::BTreeMap");
    let mut map = StdMap::new();
    time("insert", || keys().for_each(|k| _ = map.insert(k, k)));
    time("get", || keys().for_each(|k| _ = black_box(map.get(&k))));
    time("iter", || _ = black_box(map.values().sum::<u64>()));
    time("remove", || keys().for_each(|k| _ = black_box(map.remove(&k))));
}
