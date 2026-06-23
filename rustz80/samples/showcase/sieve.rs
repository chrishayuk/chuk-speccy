// Sieve of Eratosthenes, in the rustz80 dialect.
// Entry `count_primes` -> the number of primes below 100 (25).
// Shows: `[u8; N]` byte-array flag table, nested `while`, `for` ranges.

fn count_primes() -> u16 {
    // 0 = still prime, 1 = composite.
    let mut sieve = [0u8; 100];
    sieve[0] = 1u8;
    sieve[1] = 1u8;

    let mut i = 2u16;
    while i * i < 100u16 {
        if sieve[i as usize] == 0u8 {
            // Strike out multiples of i, starting at i*i.
            let mut m = i * i;
            while m < 100u16 {
                sieve[m as usize] = 1u8;
                m = m + i;
            }
        }
        i = i + 1u16;
    }

    let mut count = 0u16;
    for k in 2u16..100u16 {
        if sieve[k as usize] == 0u8 {
            count = count + 1u16;
        }
    }
    count
}
