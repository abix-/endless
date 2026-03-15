//! Benchmark: std HashMap (SipHash) vs hashbrown (foldhash) vs ahash
//! on EntityMap-like workloads (usize keys, tuple keys, high-frequency lookups).

use std::hint::black_box;
use std::time::{Duration, Instant};

// ── Hasher configs ──────────────────────────────────────────────────
type StdMap<K, V> = std::collections::HashMap<K, V>;
type BrownMap<K, V> = hashbrown::HashMap<K, V>; // foldhash default
type AhashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

const NPC_COUNT: usize = 50_000;
const BUILDING_COUNT: usize = 5_000;
const LOOKUP_ITERS: u32 = 1_000_000;
const TUPLE_LOOKUP_ITERS: u32 = 500_000;

fn bench<F: Fn() -> u64>(label: &str, f: F) -> (Duration, u64) {
    // Warmup
    for _ in 0..3 {
        black_box(f());
    }
    let start = Instant::now();
    let mut total = 0u64;
    let rounds = 10;
    for _ in 0..rounds {
        total += f();
    }
    let elapsed = start.elapsed() / rounds;
    println!("  {label:40} {elapsed:>10.2?}");
    (elapsed, total)
}

fn main() {
    println!("=== HashMap Hasher Benchmark ===");
    println!("NPCs: {NPC_COUNT}, Buildings: {BUILDING_COUNT}");
    println!();

    // ── 1. usize key insert (simulates entity_map.entities / npcs) ──
    println!("--- INSERT usize key ({NPC_COUNT} entries) ---");
    bench("std (SipHash)", || {
        let mut m = StdMap::with_capacity(NPC_COUNT);
        for i in 0..NPC_COUNT {
            m.insert(i, i as u64);
        }
        black_box(m.len() as u64)
    });
    bench("hashbrown (foldhash)", || {
        let mut m = BrownMap::with_capacity(NPC_COUNT);
        for i in 0..NPC_COUNT {
            m.insert(i, i as u64);
        }
        black_box(m.len() as u64)
    });
    bench("ahash", || {
        let mut m = AhashMap::with_capacity_and_hasher(NPC_COUNT, ahash::RandomState::new());
        for i in 0..NPC_COUNT {
            m.insert(i, i as u64);
        }
        black_box(m.len() as u64)
    });

    println!();

    // ── 2. usize key lookup (hot path: slot→entity resolution) ──
    println!("--- LOOKUP usize key ({LOOKUP_ITERS} lookups, {NPC_COUNT} entries) ---");

    let mut std_map = StdMap::with_capacity(NPC_COUNT);
    let mut brown_map = BrownMap::with_capacity(NPC_COUNT);
    let mut ahash_map = AhashMap::with_capacity_and_hasher(NPC_COUNT, ahash::RandomState::new());
    for i in 0..NPC_COUNT {
        std_map.insert(i, i as u64);
        brown_map.insert(i, i as u64);
        ahash_map.insert(i, i as u64);
    }

    bench("std (SipHash)", || {
        let mut sum = 0u64;
        for i in 0..LOOKUP_ITERS {
            let key = black_box(i as usize % NPC_COUNT);
            sum += std_map.get(&key).copied().unwrap_or(0);
        }
        sum
    });
    bench("hashbrown (foldhash)", || {
        let mut sum = 0u64;
        for i in 0..LOOKUP_ITERS {
            let key = black_box(i as usize % NPC_COUNT);
            sum += brown_map.get(&key).copied().unwrap_or(0);
        }
        sum
    });
    bench("ahash", || {
        let mut sum = 0u64;
        for i in 0..LOOKUP_ITERS {
            let key = black_box(i as usize % NPC_COUNT);
            sum += ahash_map.get(&key).copied().unwrap_or(0);
        }
        sum
    });

    println!();

    // ── 3. Tuple key lookup (simulates by_kind_town, spatial_kind_town) ──
    println!("--- LOOKUP (i32, i32) tuple key ({TUPLE_LOOKUP_ITERS} lookups) ---");

    let grid_size = 200; // 200x200 grid
    let mut std_tuple: StdMap<(i32, i32), Vec<usize>> = StdMap::new();
    let mut brown_tuple: BrownMap<(i32, i32), Vec<usize>> = BrownMap::new();
    let mut ahash_tuple: AhashMap<(i32, i32), Vec<usize>> =
        AhashMap::with_hasher(ahash::RandomState::new());
    for x in 0..grid_size as i32 {
        for y in 0..grid_size as i32 {
            let slots = vec![x as usize * grid_size + y as usize];
            std_tuple.insert((x, y), slots.clone());
            brown_tuple.insert((x, y), slots.clone());
            ahash_tuple.insert((x, y), slots);
        }
    }

    bench("std (SipHash)", || {
        let mut sum = 0u64;
        for i in 0..TUPLE_LOOKUP_ITERS {
            let key = (
                (i % grid_size as u32) as i32,
                (i / grid_size as u32 % grid_size as u32) as i32,
            );
            sum += std_tuple.get(&key).map(|v| v.len() as u64).unwrap_or(0);
        }
        sum
    });
    bench("hashbrown (foldhash)", || {
        let mut sum = 0u64;
        for i in 0..TUPLE_LOOKUP_ITERS {
            let key = (
                (i % grid_size as u32) as i32,
                (i / grid_size as u32 % grid_size as u32) as i32,
            );
            sum += brown_tuple.get(&key).map(|v| v.len() as u64).unwrap_or(0);
        }
        sum
    });
    bench("ahash", || {
        let mut sum = 0u64;
        for i in 0..TUPLE_LOOKUP_ITERS {
            let key = (
                (i % grid_size as u32) as i32,
                (i / grid_size as u32 % grid_size as u32) as i32,
            );
            sum += ahash_tuple.get(&key).map(|v| v.len() as u64).unwrap_or(0);
        }
        sum
    });

    println!();

    // ── 4. Mixed workload: insert + lookup + remove (simulates spawn/despawn churn) ──
    println!("--- CHURN: insert+lookup+remove cycle ({NPC_COUNT} entities) ---");
    bench("std (SipHash)", || {
        let mut m = StdMap::with_capacity(1024);
        let mut sum = 0u64;
        for i in 0..NPC_COUNT {
            m.insert(i, i as u64);
            if i >= 100 {
                sum += m.get(&(i - 50)).copied().unwrap_or(0);
                m.remove(&(i - 100));
            }
        }
        sum
    });
    bench("hashbrown (foldhash)", || {
        let mut m = BrownMap::with_capacity(1024);
        let mut sum = 0u64;
        for i in 0..NPC_COUNT {
            m.insert(i, i as u64);
            if i >= 100 {
                sum += m.get(&(i - 50)).copied().unwrap_or(0);
                m.remove(&(i - 100));
            }
        }
        sum
    });
    bench("ahash", || {
        let mut m = AhashMap::with_capacity_and_hasher(1024, ahash::RandomState::new());
        let mut sum = 0u64;
        for i in 0..NPC_COUNT {
            m.insert(i, i as u64);
            if i >= 100 {
                sum += m.get(&(i - 50)).copied().unwrap_or(0);
                m.remove(&(i - 100));
            }
        }
        sum
    });

    println!();

    // ── 5. Iteration (simulates iter_npcs / iter_buildings) ──
    println!("--- ITERATION over {NPC_COUNT}-entry map (values sum) ---");
    bench("std (SipHash)", || std_map.values().copied().sum::<u64>());
    bench("hashbrown (foldhash)", || {
        brown_map.values().copied().sum::<u64>()
    });
    bench("ahash", || ahash_map.values().copied().sum::<u64>());

    println!();
    println!("=== Done ===");
}
