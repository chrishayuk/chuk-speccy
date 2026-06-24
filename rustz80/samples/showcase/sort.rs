// Insertion sort of a fixed array, in the rustz80 dialect.
// Entry `sort_checksum` -> an order-sensitive checksum of the sorted array (330).
// Shows: arrays, nested loops, `break`, `for` ranges, expression indexing.

fn sort_checksum() -> u16 {
    let mut a = [5u16, 2u16, 8u16, 1u16, 9u16, 3u16, 7u16, 4u16, 6u16, 0u16];
    let n = 10u16;

    // Insertion sort: grow a sorted prefix, sliding each new key left into place.
    let mut i = 1u16;
    while i < n {
        let key = a[i as usize];
        let mut j = i;
        // Shift larger elements right. The dialect has no `&&`, so the loop guard is
        // an explicit `if … break` (Stage 3c control flow).
        while j > 0u16 {
            if a[(j - 1u16) as usize] <= key {
                break;
            }
            a[j as usize] = a[(j - 1u16) as usize];
            j = j - 1u16;
        }
        a[j as usize] = key;
        i = i + 1u16;
    }

    // Order-sensitive checksum: sum of a[k] * (k + 1).
    let mut sum = 0u16;
    for k in 0u16..n {
        sum = sum + a[k as usize] * (k + 1u16);
    }
    sum
}
