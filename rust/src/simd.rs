//! SIMD-accelerated batch operations.
//!
//! Each function has AVX2 + scalar variants with runtime dispatch.
//! First SIMD module in Endless -- establishes the pattern for future work.

// ============================================================================
// ARRIVAL CHECK
// ============================================================================

/// Batch result codes for arrival check.
pub const ARRIVAL_NOT: u8 = 0;
pub const ARRIVAL_YES: u8 = 1;
pub const ARRIVAL_HIDDEN: u8 = 2;

/// Runtime-dispatched batch arrival check.
///
/// Computes dist_sq between positions\[i\] and targets\[i\] for each entity,
/// compares against threshold_sq. Returns per-entity result code.
///
/// positions/targets: interleaved \[x0,y0,x1,y1,...\] (GPU buffer layout).
/// count: number of entities (not floats).
pub fn batch_arrival_check(
    positions: &[f32],
    targets: &[f32],
    threshold_sq: f32,
    count: usize,
) -> Vec<u8> {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            // Safety: AVX2 feature detected at runtime
            return unsafe {
                batch_arrival_check_avx2(positions, targets, threshold_sq, count)
            };
        }
    }
    batch_arrival_check_scalar(positions, targets, threshold_sq, count)
}

/// Scalar implementation. Reference for correctness testing.
pub fn batch_arrival_check_scalar(
    positions: &[f32],
    targets: &[f32],
    threshold_sq: f32,
    count: usize,
) -> Vec<u8> {
    let mut result = vec![ARRIVAL_NOT; count];
    for i in 0..count {
        let base = i * 2;
        if base + 1 >= positions.len() || base + 1 >= targets.len() {
            continue;
        }
        if positions[base] < -9000.0 {
            result[i] = ARRIVAL_HIDDEN;
            continue;
        }
        let dx = positions[base] - targets[base];
        let dy = positions[base + 1] - targets[base + 1];
        if dx * dx + dy * dy <= threshold_sq {
            result[i] = ARRIVAL_YES;
        }
    }
    result
}

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// AVX2 implementation. Processes 4 entities (8 floats) per iteration.
///
/// Safety: caller must ensure AVX2 is available (checked by dispatch fn).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn batch_arrival_check_avx2(
    positions: &[f32],
    targets: &[f32],
    threshold_sq: f32,
    count: usize,
) -> Vec<u8> {
    let mut result = vec![ARRIVAL_NOT; count];
    // Safety: all intrinsics below require AVX2 which is guaranteed by #[target_feature]
    let thresh = unsafe { _mm256_set1_ps(threshold_sq) };
    let hidden_val = unsafe { _mm256_set1_ps(-9000.0) };

    // 4 entities per chunk (each entity = 2 floats, 4 entities = 8 floats = 1 AVX2 reg)
    let full_chunks = count / 4;
    let mut scalar_start = full_chunks * 4;

    for chunk in 0..full_chunks {
        let base = chunk * 8;
        if base + 7 >= positions.len() || base + 7 >= targets.len() {
            // Buffer too short for full SIMD load -- fall through to scalar tail
            scalar_start = chunk * 4;
            break;
        }

        // Safety: bounds checked above, AVX2 guaranteed by #[target_feature]
        let pos = unsafe { _mm256_loadu_ps(positions.as_ptr().add(base)) };
        let tgt = unsafe { _mm256_loadu_ps(targets.as_ptr().add(base)) };

        // delta = pos - target
        let delta = unsafe { _mm256_sub_ps(pos, tgt) };
        // delta_sq = delta * delta
        let delta_sq = unsafe { _mm256_mul_ps(delta, delta) };

        // Horizontal add pairs within each 128-bit lane:
        //   lane 0: [dx0^2+dy0^2, dx1^2+dy1^2, dx0^2+dy0^2, dx1^2+dy1^2]
        //   lane 1: [dx2^2+dy2^2, dx3^2+dy3^2, dx2^2+dy2^2, dx3^2+dy3^2]
        // Entity dist_sq values land at bit positions 0,1,4,5 in movemask output.
        let dist_sq = unsafe { _mm256_hadd_ps(delta_sq, delta_sq) };

        // Compare: dist_sq <= threshold
        let arrived_mask = unsafe { _mm256_cmp_ps(dist_sq, thresh, _CMP_LE_OQ) };
        let arrived_bits = unsafe { _mm256_movemask_ps(arrived_mask) } as u32;

        // Check hidden: pos.x < -9000 (x values at even float positions: 0, 2, 4, 6)
        let hidden_mask = unsafe { _mm256_cmp_ps(pos, hidden_val, _CMP_LT_OQ) };
        let hidden_bits = unsafe { _mm256_movemask_ps(hidden_mask) } as u32;

        let entity_base = chunk * 4;

        // hadd output bit mapping: entity 0 -> bit 0, entity 1 -> bit 1,
        //                          entity 2 -> bit 4, entity 3 -> bit 5
        // Hidden x-component bit mapping: entity 0 -> bit 0, entity 1 -> bit 2,
        //                                 entity 2 -> bit 4, entity 3 -> bit 6
        const ARRIVED_BIT: [u32; 4] = [0, 1, 4, 5];
        const HIDDEN_BIT: [u32; 4] = [0, 2, 4, 6];

        for e in 0..4usize {
            if hidden_bits & (1 << HIDDEN_BIT[e]) != 0 {
                result[entity_base + e] = ARRIVAL_HIDDEN;
            } else if arrived_bits & (1 << ARRIVED_BIT[e]) != 0 {
                result[entity_base + e] = ARRIVAL_YES;
            }
        }
    }

    // Scalar tail for remaining entities
    for i in scalar_start..count {
        let base = i * 2;
        if base + 1 >= positions.len() || base + 1 >= targets.len() {
            continue;
        }
        if positions[base] < -9000.0 {
            result[i] = ARRIVAL_HIDDEN;
            continue;
        }
        let dx = positions[base] - targets[base];
        let dy = positions[base + 1] - targets[base + 1];
        if dx * dx + dy * dy <= threshold_sq {
            result[i] = ARRIVAL_YES;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_empty_input() {
        let result = batch_arrival_check_scalar(&[], &[], 400.0, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn scalar_basic_arrival() {
        // Entity at (100, 200), target at (105, 203) -> dist_sq = 25+9 = 34 <= 400
        let positions = vec![100.0, 200.0];
        let targets = vec![105.0, 203.0];
        let result = batch_arrival_check_scalar(&positions, &targets, 400.0, 1);
        assert_eq!(result, vec![ARRIVAL_YES]);
    }

    #[test]
    fn scalar_not_arrived() {
        // Entity at (0, 0), target at (100, 100) -> dist_sq = 20000 > 400
        let positions = vec![0.0, 0.0];
        let targets = vec![100.0, 100.0];
        let result = batch_arrival_check_scalar(&positions, &targets, 400.0, 1);
        assert_eq!(result, vec![ARRIVAL_NOT]);
    }

    #[test]
    fn scalar_hidden_entity() {
        let positions = vec![-9999.0, -9999.0];
        let targets = vec![0.0, 0.0];
        let result = batch_arrival_check_scalar(&positions, &targets, 400.0, 1);
        assert_eq!(result, vec![ARRIVAL_HIDDEN]);
    }

    #[test]
    fn scalar_boundary_threshold() {
        // dist_sq exactly == threshold_sq should be ARRIVAL_YES
        // dist = 20, so target offset = (20, 0) -> dist_sq = 400
        let positions = vec![0.0, 0.0];
        let targets = vec![20.0, 0.0];
        let result = batch_arrival_check_scalar(&positions, &targets, 400.0, 1);
        assert_eq!(result, vec![ARRIVAL_YES]);
    }

    #[test]
    fn scalar_just_outside_threshold() {
        // dist_sq just above threshold -> ARRIVAL_NOT
        let positions = vec![0.0, 0.0];
        let targets = vec![20.1, 0.0];
        let result = batch_arrival_check_scalar(&positions, &targets, 400.0, 1);
        assert_eq!(result, vec![ARRIVAL_NOT]);
    }

    #[test]
    fn scalar_mixed_entities() {
        // 5 entities: arrived, not arrived, hidden, arrived, not arrived
        let positions = vec![
            10.0, 10.0,     // e0: near target
            0.0, 0.0,       // e1: far from target
            -9999.0, -9999.0, // e2: hidden
            50.0, 50.0,     // e3: near target
            0.0, 0.0,       // e4: far from target
        ];
        let targets = vec![
            12.0, 11.0,     // e0: dist_sq = 4+1 = 5
            100.0, 100.0,   // e1: dist_sq = 20000
            0.0, 0.0,       // e2: doesn't matter
            51.0, 50.0,     // e3: dist_sq = 1
            200.0, 200.0,   // e4: dist_sq = 80000
        ];
        let result = batch_arrival_check_scalar(&positions, &targets, 400.0, 5);
        assert_eq!(result, vec![ARRIVAL_YES, ARRIVAL_NOT, ARRIVAL_HIDDEN, ARRIVAL_YES, ARRIVAL_NOT]);
    }

    #[test]
    fn scalar_short_buffer() {
        // count says 3 entities but buffer only has 2
        let positions = vec![10.0, 10.0, 20.0, 20.0];
        let targets = vec![10.0, 10.0, 20.0, 20.0];
        let result = batch_arrival_check_scalar(&positions, &targets, 400.0, 3);
        assert_eq!(result, vec![ARRIVAL_YES, ARRIVAL_YES, ARRIVAL_NOT]);
    }

    #[test]
    fn dispatch_returns_correct_results() {
        let positions = vec![10.0, 10.0, -9999.0, 0.0, 0.0, 0.0];
        let targets = vec![11.0, 10.0, 0.0, 0.0, 100.0, 100.0];
        let result = batch_arrival_check(&positions, &targets, 400.0, 3);
        assert_eq!(result, vec![ARRIVAL_YES, ARRIVAL_HIDDEN, ARRIVAL_NOT]);
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn avx2_matches_scalar() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        // 1000 entities with deterministic pseudo-random data
        let count = 1000;
        let positions: Vec<f32> = (0..count * 2)
            .map(|i| {
                if i % 20 == 0 {
                    -9999.0
                } else {
                    ((i as f32) * 7.3 + 11.0) % 1000.0
                }
            })
            .collect();
        let targets: Vec<f32> = (0..count * 2)
            .map(|i| ((i as f32) * 7.3 + 14.0) % 1000.0)
            .collect();

        let scalar = batch_arrival_check_scalar(&positions, &targets, 400.0, count);
        let avx2 = unsafe {
            batch_arrival_check_avx2(&positions, &targets, 400.0, count)
        };

        assert_eq!(scalar.len(), avx2.len(), "length mismatch");
        for i in 0..count {
            assert_eq!(
                scalar[i], avx2[i],
                "mismatch at entity {i}: scalar={}, avx2={}, pos=({},{}), tgt=({},{})",
                scalar[i], avx2[i],
                positions[i * 2], positions[i * 2 + 1],
                targets[i * 2], targets[i * 2 + 1],
            );
        }
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn avx2_matches_scalar_all_hidden() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        let count = 100;
        let positions: Vec<f32> = vec![-9999.0; count * 2];
        let targets: Vec<f32> = vec![0.0; count * 2];

        let scalar = batch_arrival_check_scalar(&positions, &targets, 400.0, count);
        let avx2 = unsafe {
            batch_arrival_check_avx2(&positions, &targets, 400.0, count)
        };
        assert_eq!(scalar, avx2);
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn avx2_matches_scalar_odd_count() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        // Non-multiple-of-4 count to exercise scalar tail
        let count = 13;
        let positions: Vec<f32> = (0..count * 2)
            .map(|i| (i as f32) * 3.0)
            .collect();
        let targets: Vec<f32> = (0..count * 2)
            .map(|i| (i as f32) * 3.0 + 1.0)
            .collect();

        let scalar = batch_arrival_check_scalar(&positions, &targets, 400.0, count);
        let avx2 = unsafe {
            batch_arrival_check_avx2(&positions, &targets, 400.0, count)
        };
        assert_eq!(scalar, avx2);
    }
}
