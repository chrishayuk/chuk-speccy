//! A fixed-capacity, allocation-free vec — the subset-clean replacement for `Vec`.

/// A fixed-capacity, allocation-free vec — the subset-clean replacement for `Vec`
/// (spec 08 §1). Holds up to `N` `T`s inline; `push`/`insert_front` past capacity
/// are dropped and report `false`. `T: Copy + Default`.
#[derive(Copy, Clone)]
pub struct Entities<T: Copy + Default, const N: usize> {
    items: [T; N],
    len: usize,
}

impl<T: Copy + Default, const N: usize> Default for Entities<T, N> {
    fn default() -> Self {
        Entities {
            items: [T::default(); N],
            len: 0,
        }
    }
}

impl<T: Copy + Default, const N: usize> Entities<T, N> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn capacity(&self) -> usize {
        N
    }
    pub fn clear(&mut self) {
        self.len = 0;
    }
    pub fn as_slice(&self) -> &[T] {
        &self.items[..self.len]
    }
    pub fn get(&self, i: usize) -> Option<T> {
        if i < self.len {
            Some(self.items[i])
        } else {
            None
        }
    }
    /// Append to the back. Returns `false` if full.
    pub fn push(&mut self, v: T) -> bool {
        if self.len < N {
            self.items[self.len] = v;
            self.len += 1;
            true
        } else {
            false
        }
    }
    /// Remove and return the back element.
    pub fn pop(&mut self) -> Option<T> {
        if self.len > 0 {
            self.len -= 1;
            Some(self.items[self.len])
        } else {
            None
        }
    }
    /// Insert at the front, shifting the rest right (drops the last if full).
    pub fn insert_front(&mut self, v: T) -> bool {
        if N == 0 {
            return false;
        }
        let top = self.len.min(N - 1);
        let mut i = top;
        while i > 0 {
            self.items[i] = self.items[i - 1];
            i -= 1;
        }
        self.items[0] = v;
        self.len = (self.len + 1).min(N);
        true
    }
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.as_slice().iter()
    }
    pub fn contains(&self, v: &T) -> bool
    where
        T: PartialEq,
    {
        self.as_slice().contains(v)
    }
}

impl<T: Copy + Default, const N: usize> core::ops::Index<usize> for Entities<T, N> {
    type Output = T;
    fn index(&self, i: usize) -> &T {
        &self.items[i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_is_a_fixed_capacity_vec() {
        let mut e: Entities<u16, 4> = Entities::new();
        assert!(e.is_empty() && e.capacity() == 4);
        assert!(e.push(10) && e.push(20) && e.push(30));
        assert_eq!(e.len(), 3);
        assert_eq!(e[0], 10);
        assert!(e.contains(&20) && !e.contains(&99));
        assert_eq!(e.pop(), Some(30));

        // insert_front shifts right; over-capacity drops the last.
        e.insert_front(5); // [5, 10, 20]
        assert_eq!(e[0], 5);
        assert_eq!(e.as_slice(), &[5, 10, 20]);
        assert!(e.push(40)); // [5,10,20,40] full
        assert!(!e.push(50), "push past capacity reports false");
        assert_eq!(e.len(), 4);
    }
}
