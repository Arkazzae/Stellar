//! Building Huffman codes for the compressor: lengths limited to what the
//! format allows, and the canonical codes they imply.

/// Fills `lengths` with a length-limited prefix code for `freq`. Every symbol
/// with a non-zero count gets a code, and at least two symbols always do, so
/// the code is complete and every decoder accepts it, a lone symbol included.
pub(crate) fn build_lengths(freq: &[u32], max_bits: u32, lengths: &mut [u8]) {
    debug_assert_eq!(freq.len(), lengths.len());
    let mut counts: Vec<u32> = freq.to_vec();
    let mut used = counts.iter().filter(|&&c| c > 0).count();
    // Pad with unused symbols, so one or no symbol still gets a complete code.
    for count in &mut counts {
        if used >= 2 {
            break;
        }
        if *count == 0 {
            *count = 1;
            used += 1;
        }
    }

    let mut order: Vec<(u32, usize)> = counts
        .iter()
        .enumerate()
        .filter(|(_, c)| **c > 0)
        .map(|(s, c)| (*c, s))
        .collect();
    order.sort_unstable();
    lengths.fill(0);
    let n = order.len();

    // Two-queue Huffman: leaves are sorted, internal nodes come out sorted.
    let mut weight_internal = vec![0u64; n - 1];
    let mut parent_leaf = vec![0usize; n];
    let mut parent_internal = vec![0usize; n - 1];
    let (mut leaf, mut internal) = (0usize, 0usize);
    for made in 0..n - 1 {
        // (weight, is a leaf, index) of the two lightest nodes left.
        let mut picked = [(0u64, false, 0usize); 2];
        for slot in &mut picked {
            let leaf_weight = (leaf < n).then(|| u64::from(order[leaf].0));
            let internal_weight = (internal < made).then(|| weight_internal[internal]);
            *slot = match (leaf_weight, internal_weight) {
                (Some(l), Some(i)) if i < l => {
                    internal += 1;
                    (i, false, internal - 1)
                }
                (Some(l), _) => {
                    leaf += 1;
                    (l, true, leaf - 1)
                }
                (None, Some(i)) => {
                    internal += 1;
                    (i, false, internal - 1)
                }
                (None, None) => (0, true, 0),
            };
        }
        weight_internal[made] = picked[0].0 + picked[1].0;
        for (_, is_leaf, index) in picked {
            if is_leaf {
                parent_leaf[index] = made;
            } else {
                parent_internal[index] = made;
            }
        }
    }

    let mut depth_internal = vec![0u32; n - 1];
    for node in (0..n - 2).rev() {
        depth_internal[node] = depth_internal[parent_internal[node]] + 1;
    }
    let max_bits = max_bits as usize;
    let mut per_length = vec![0u32; n.max(max_bits) + 2];
    for &parent in &parent_leaf {
        per_length[(depth_internal[parent] + 1) as usize] += 1;
    }

    // Fold codes that are too long into the limit, then repair the code's
    // total weight: moving a leaf one level down frees exactly the room one
    // more leaf at the limit takes.
    for length in max_bits + 1..per_length.len() {
        per_length[max_bits] += per_length[length];
        per_length[length] = 0;
    }
    let mut total: u64 = (1..=max_bits)
        .map(|l| u64::from(per_length[l]) << (max_bits - l))
        .sum();
    while total > 1 << max_bits {
        per_length[max_bits] -= 1;
        for length in (1..max_bits).rev() {
            if per_length[length] > 0 {
                per_length[length] -= 1;
                per_length[length + 1] += 2;
                break;
            }
        }
        total -= 1;
    }

    // The rarest symbols get the longest codes.
    let mut next = 0;
    for length in (1..=max_bits).rev() {
        for _ in 0..per_length[length] {
            lengths[order[next].1] = length as u8;
            next += 1;
        }
    }
}

/// The canonical codes for `lengths`, each already bit-reversed because the
/// format sends codes most significant bit first inside a stream that is
/// filled least significant bit first.
pub(crate) fn assign_codes(lengths: &[u8], codes: &mut [u16]) {
    let mut count = [0u16; 16];
    for &length in lengths {
        count[usize::from(length)] += 1;
    }
    count[0] = 0;
    let mut next = [0u16; 16];
    let mut code = 0u16;
    for length in 1..16 {
        code = (code + count[length - 1]) << 1;
        next[length] = code;
    }
    for (symbol, &length) in lengths.iter().enumerate() {
        if length == 0 {
            codes[symbol] = 0;
            continue;
        }
        let c = next[usize::from(length)];
        next[usize::from(length)] += 1;
        codes[symbol] = c.reverse_bits() >> (16 - u32::from(length));
    }
}
