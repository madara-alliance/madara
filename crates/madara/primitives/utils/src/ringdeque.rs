use itertools::{
    FoldWhile::{Continue, Done},
    Itertools,
};

pub struct RingDeque<const CAPACITY: usize, T> {
    ring: [std::mem::MaybeUninit<T>; CAPACITY],
    start: usize,
    size: usize,
}

impl<const CAPACITY: usize, T: std::fmt::Debug> std::fmt::Debug for RingDeque<CAPACITY, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<const CAPACITY: usize, T: Clone> Clone for RingDeque<CAPACITY, T> {
    fn clone(&self) -> Self {
        let mut ring = Self::new();
        for item in self.iter().cloned() {
            ring.push_back(item);
        }
        ring
    }
}

impl<const CAPACITY: usize, T: PartialEq> PartialEq for RingDeque<CAPACITY, T> {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size
            && self
                .iter()
                .zip(other.iter())
                .fold_while(true, |_, (a, b)| if a == b { Continue(true) } else { Done(false) })
                .into_inner()
    }
}
impl<const CAPACITY: usize, T: Eq> Eq for RingDeque<CAPACITY, T> {}

impl<const CAPACITY: usize, T> Drop for RingDeque<CAPACITY, T> {
    fn drop(&mut self) {
        while self.size != 0 {
            unsafe { self.ring[self.start].assume_init_drop() };
            self.start = wrapping_index::<CAPACITY>(self.start + 1);
            self.size -= 1;
        }
    }
}

impl<const CAPACITY: usize, T> RingDeque<CAPACITY, T> {
    pub fn new() -> Self {
        assert!(CAPACITY > 0, "Cannot create a RingDeque with a capacity of 0");
        Self { ring: [const { std::mem::MaybeUninit::uninit() }; CAPACITY], start: 0, size: 0 }
    }

    pub fn push_front(&mut self, item: T) {
        assert!(self.try_push_front(item).is_none(), "Cannot add more elements, ring is full")
    }

    pub fn push_back(&mut self, item: T) {
        assert!(self.try_push_back(item).is_none(), "Cannot add more elements, ring is full")
    }

    pub fn pop_front(&mut self) -> Option<T> {
        if self.is_empty() {
            None
        } else {
            let res = unsafe { self.ring[self.start].assume_init_read() };
            self.start = wrapping_index::<CAPACITY>(self.start + 1);
            self.size -= 1;
            Some(res)
        }
    }

    pub fn pop_back(&mut self) -> Option<T> {
        if self.is_empty() {
            None
        } else {
            let res = unsafe { self.ring[wrapping_index::<CAPACITY>(self.start + self.size - 1)].assume_init_read() };
            self.size -= 1;
            Some(res)
        }
    }

    pub fn peek_front(&self) -> Option<&T> {
        if self.is_empty() {
            None
        } else {
            let res = unsafe { &*self.ring[self.start].as_ptr() };
            Some(res)
        }
    }

    pub fn peek_back(&self) -> Option<&T> {
        if self.is_empty() {
            None
        } else {
            let res = unsafe { &*self.ring[wrapping_index::<CAPACITY>(self.start + self.size - 1)].as_ptr() };
            Some(res)
        }
    }

    pub fn peek_front_mut(&mut self) -> Option<&mut T> {
        if self.is_empty() {
            None
        } else {
            let res = unsafe { &mut *self.ring[self.start].as_mut_ptr() };
            Some(res)
        }
    }

    pub fn peek_back_mut(&mut self) -> Option<&mut T> {
        if self.is_empty() {
            None
        } else {
            let res = unsafe { &mut *self.ring[wrapping_index::<CAPACITY>(self.start + self.size - 1)].as_mut_ptr() };
            Some(res)
        }
    }

    pub fn try_push_front(&mut self, item: T) -> Option<T> {
        if self.is_full() {
            Some(item)
        } else {
            self.start = wrapping_decrement::<CAPACITY>(self.start);
            self.ring[self.start].write(item);
            self.size += 1;
            None
        }
    }

    pub fn try_push_back(&mut self, item: T) -> Option<T> {
        if self.is_full() {
            Some(item)
        } else {
            self.ring[wrapping_index::<CAPACITY>(self.start + self.size)].write(item);
            self.size += 1;
            None
        }
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_full(&self) -> bool {
        self.size == CAPACITY
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub fn capacity(&self) -> usize {
        CAPACITY
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        Iter { ring: &self.ring, start: self.start, size: self.size }
    }

    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut T> {
        IterMut { ring: &mut self.ring, start: self.start, size: self.size }
    }
}

impl<const CAPACITY: usize, T> IntoIterator for RingDeque<CAPACITY, T> {
    type Item = T;
    type IntoIter = IntoIter<CAPACITY, T>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter { me: std::mem::ManuallyDrop::new(self) }
    }
}

pub struct Iter<'a, const CAPACITY: usize, T> {
    ring: &'a [std::mem::MaybeUninit<T>; CAPACITY],
    start: usize,
    size: usize,
}

impl<'a, const CAPACITY: usize, T> Iterator for Iter<'a, CAPACITY, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.size == 0 {
            None
        } else {
            let res = unsafe { &*self.ring[self.start].as_ptr() };
            self.start = wrapping_index::<CAPACITY>(self.start + 1);
            self.size -= 1;
            Some(res)
        }
    }
}

impl<const CAPACITY: usize, T> DoubleEndedIterator for Iter<'_, CAPACITY, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.size == 0 {
            None
        } else {
            self.size -= 1;
            let res = unsafe { &*self.ring[wrapping_index::<CAPACITY>(self.start + self.size)].as_ptr() };
            Some(res)
        }
    }
}

pub struct IterMut<'a, const CAPACITY: usize, T> {
    ring: &'a mut [std::mem::MaybeUninit<T>; CAPACITY],
    start: usize,
    size: usize,
}

impl<'a, const CAPACITY: usize, T> Iterator for IterMut<'a, CAPACITY, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.size == 0 {
            None
        } else {
            let res = unsafe { &mut *self.ring[self.start].as_mut_ptr() };
            self.start = wrapping_index::<CAPACITY>(self.start + 1);
            self.size -= 1;
            Some(res)
        }
    }
}

impl<const CAPACITY: usize, T> DoubleEndedIterator for IterMut<'_, CAPACITY, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.size == 0 {
            None
        } else {
            self.size -= 1;
            let res = unsafe { &mut *self.ring[wrapping_index::<CAPACITY>(self.start + self.size)].as_mut_ptr() };
            Some(res)
        }
    }
}

pub struct IntoIter<const CAPACITY: usize, T> {
    me: std::mem::ManuallyDrop<RingDeque<CAPACITY, T>>,
}

impl<const CAPACITY: usize, T> Drop for IntoIter<CAPACITY, T> {
    fn drop(&mut self) {
        unsafe { std::mem::ManuallyDrop::drop(&mut self.me) }
    }
}

impl<const CAPACITY: usize, T> Iterator for IntoIter<CAPACITY, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.me.size == 0 {
            None
        } else {
            let res = unsafe { self.me.ring[self.me.start].assume_init_read() };
            self.me.start = wrapping_index::<CAPACITY>(self.me.start + 1);
            self.me.size -= 1;
            Some(res)
        }
    }
}

impl<const CAPACITY: usize, T> DoubleEndedIterator for IntoIter<CAPACITY, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.me.size == 0 {
            None
        } else {
            self.me.size -= 1;
            let res =
                unsafe { self.me.ring[wrapping_index::<CAPACITY>(self.me.start + self.me.size)].assume_init_read() };
            Some(res)
        }
    }
}

fn wrapping_decrement<const CAPACITY: usize>(n: usize) -> usize {
    n.checked_sub(1).unwrap_or(CAPACITY - 1)
}

fn wrapping_index<const CAPACITY: usize>(n: usize) -> usize {
    if n >= CAPACITY {
        n - CAPACITY
    } else {
        n
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn ring_new() {
        let ring = RingDeque::<10, ()>::new();
        assert_eq!(ring.len(), 0);
        assert_eq!(ring.capacity(), 10);
    }

    #[test]
    #[should_panic]
    fn ring_new_panic_zero_capacity() {
        let _ = RingDeque::<0, ()>::new();
    }

    #[test]
    fn ring_push_back_simple() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n);
        }

        unsafe {
            assert_eq!(ring.ring[0].assume_init(), 0);
            assert_eq!(ring.ring[1].assume_init(), 1);
            assert_eq!(ring.ring[2].assume_init(), 2);
            assert_eq!(ring.ring[3].assume_init(), 3);
            assert_eq!(ring.ring[4].assume_init(), 4);
            assert_eq!(ring.ring[5].assume_init(), 5);
            assert_eq!(ring.ring[6].assume_init(), 6);
            assert_eq!(ring.ring[7].assume_init(), 7);
            assert_eq!(ring.ring[8].assume_init(), 8);
            assert_eq!(ring.ring[9].assume_init(), 9);
        }

        assert_eq!(ring.len(), 10);
    }

    #[test]
    fn ring_push_front_simple() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in (0..10).rev() {
            ring.push_front(n);
        }

        unsafe {
            assert_eq!(ring.ring[0].assume_init(), 0);
            assert_eq!(ring.ring[1].assume_init(), 1);
            assert_eq!(ring.ring[2].assume_init(), 2);
            assert_eq!(ring.ring[3].assume_init(), 3);
            assert_eq!(ring.ring[4].assume_init(), 4);
            assert_eq!(ring.ring[5].assume_init(), 5);
            assert_eq!(ring.ring[6].assume_init(), 6);
            assert_eq!(ring.ring[7].assume_init(), 7);
            assert_eq!(ring.ring[8].assume_init(), 8);
            assert_eq!(ring.ring[9].assume_init(), 9);
        }

        assert_eq!(ring.len(), 10);
    }

    #[test]
    #[should_panic]
    fn ring_push_back_panic_max_capacity() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..11 {
            ring.push_back(n);
        }
    }

    #[test]
    #[should_panic]
    fn ring_push_front_panic_max_capacity() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..11 {
            ring.push_front(n);
        }
    }

    #[test]
    fn ring_push_back_try() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            assert!(ring.try_push_back(n).is_none());
        }
        assert_eq!(ring.try_push_back(10), Some(10));
    }

    #[test]
    fn ring_push_front_try() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in (0..10).rev() {
            assert!(ring.try_push_front(n).is_none());
        }
        assert_eq!(ring.try_push_front(10), Some(10));
    }

    #[test]
    fn ring_push_back_pop_back() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n);
        }
        for n in (0..10).rev() {
            assert_eq!(ring.pop_back(), Some(n));
        }

        assert_eq!(ring.pop_back(), None);
        assert_eq!(ring.start, 0);
        assert_eq!(ring.size, 0);

        // This should not fail even though ring.start == ring.stop
        for n in 0..10 {
            ring.push_back(n);
        }
    }

    #[test]
    fn ring_push_back_pop_front() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n);
        }
        for n in 0..10 {
            assert_eq!(ring.pop_front(), Some(n));
        }

        assert_eq!(ring.pop_front(), None);
        assert_eq!(ring.start, 0);
        assert_eq!(ring.size, 0);

        // This should not fail even though ring.start == ring.stop
        for n in (0..10).rev() {
            ring.push_back(n);
        }

        // push_back should wrap around now
        unsafe {
            assert_eq!(ring.ring[0].assume_init(), 9);
            assert_eq!(ring.ring[1].assume_init(), 8);
            assert_eq!(ring.ring[2].assume_init(), 7);
            assert_eq!(ring.ring[3].assume_init(), 6);
            assert_eq!(ring.ring[4].assume_init(), 5);
            assert_eq!(ring.ring[5].assume_init(), 4);
            assert_eq!(ring.ring[6].assume_init(), 3);
            assert_eq!(ring.ring[7].assume_init(), 2);
            assert_eq!(ring.ring[8].assume_init(), 1);
            assert_eq!(ring.ring[9].assume_init(), 0);
        }

        assert_eq!(ring.len(), 10)
    }

    #[test]
    fn ring_push_front_pop_front() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in (0..10).rev() {
            ring.push_front(n);
        }
        for n in 0..10 {
            assert_eq!(ring.pop_front(), Some(n));
        }

        assert_eq!(ring.pop_front(), None);
        assert_eq!(ring.start, 0);
        assert_eq!(ring.size, 0);

        // This should not fail even though ring.start == ring.stop
        for n in (0..10).rev() {
            ring.push_front(n);
        }
    }

    #[test]
    fn ring_push_front_pop_back() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in (0..10).rev() {
            ring.push_front(n);
        }
        for n in (0..10).rev() {
            assert_eq!(ring.pop_back(), Some(n));
        }

        assert_eq!(ring.pop_back(), None);
        assert_eq!(ring.start, 0);
        assert_eq!(ring.size, 0);

        // This should not fail even though ring.start == ring.stop
        for n in 0..10 {
            ring.push_front(n);
        }

        // push_front should wrap around now
        unsafe {
            assert_eq!(ring.ring[0].assume_init(), 9);
            assert_eq!(ring.ring[1].assume_init(), 8);
            assert_eq!(ring.ring[2].assume_init(), 7);
            assert_eq!(ring.ring[3].assume_init(), 6);
            assert_eq!(ring.ring[4].assume_init(), 5);
            assert_eq!(ring.ring[5].assume_init(), 4);
            assert_eq!(ring.ring[6].assume_init(), 3);
            assert_eq!(ring.ring[7].assume_init(), 2);
            assert_eq!(ring.ring[8].assume_init(), 1);
            assert_eq!(ring.ring[9].assume_init(), 0);
        }

        assert_eq!(ring.len(), 10)
    }

    #[test]
    fn ring_peek_back_simple() {
        let mut ring = RingDeque::<10, i32>::new();
        assert_eq!(ring.peek_back(), None);
        for n in 0..10 {
            ring.push_back(n);
            assert_eq!(ring.peek_back(), Some(&n));
        }
    }

    #[test]
    fn ring_peek_back_no_double_free() {
        let mut ring = RingDeque::<10, Vec<i32>>::new();
        for n in 0..10 {
            ring.push_back(vec![n]);
            assert_eq!(ring.peek_back(), Some(&vec![n]));
            assert_eq!(ring.peek_back(), Some(&vec![n]));
        }
    }

    #[test]
    fn ring_peek_front_simple() {
        let mut ring = RingDeque::<10, i32>::new();
        assert_eq!(ring.peek_front(), None);
        for n in (0..10).rev() {
            ring.push_front(n);
            assert_eq!(ring.peek_front(), Some(&n));
        }
    }

    #[test]
    fn ring_peek_front_no_free() {
        let mut ring = RingDeque::<10, Vec<i32>>::new();
        for n in (0..10).rev() {
            ring.push_front(vec![n]);
            assert_eq!(ring.peek_front(), Some(&vec![n]));
            assert_eq!(ring.peek_front(), Some(&vec![n]));
        }
    }

    #[test]
    fn ring_peek_back_mut() {
        let mut ring = RingDeque::<10, i32>::new();
        assert_eq!(ring.peek_back_mut(), None);
        for n in 0..10 {
            ring.push_back(n);
            *ring.peek_back_mut().unwrap() += 1;
        }

        unsafe {
            assert_eq!(ring.ring[0].assume_init(), 1);
            assert_eq!(ring.ring[1].assume_init(), 2);
            assert_eq!(ring.ring[2].assume_init(), 3);
            assert_eq!(ring.ring[3].assume_init(), 4);
            assert_eq!(ring.ring[4].assume_init(), 5);
            assert_eq!(ring.ring[5].assume_init(), 6);
            assert_eq!(ring.ring[6].assume_init(), 7);
            assert_eq!(ring.ring[7].assume_init(), 8);
            assert_eq!(ring.ring[8].assume_init(), 9);
            assert_eq!(ring.ring[9].assume_init(), 10);
        }
    }

    #[test]
    fn ring_peek_front_mut() {
        let mut ring = RingDeque::<10, i32>::new();
        assert_eq!(ring.peek_front_mut(), None);
        for n in (0..10).rev() {
            ring.push_front(n);
            *ring.peek_front_mut().unwrap() += 1;
        }

        unsafe {
            assert_eq!(ring.ring[0].assume_init(), 1);
            assert_eq!(ring.ring[1].assume_init(), 2);
            assert_eq!(ring.ring[2].assume_init(), 3);
            assert_eq!(ring.ring[3].assume_init(), 4);
            assert_eq!(ring.ring[4].assume_init(), 5);
            assert_eq!(ring.ring[5].assume_init(), 6);
            assert_eq!(ring.ring[6].assume_init(), 7);
            assert_eq!(ring.ring[7].assume_init(), 8);
            assert_eq!(ring.ring[8].assume_init(), 9);
            assert_eq!(ring.ring[9].assume_init(), 10);
        }
    }

    // start < stop
    #[test]
    fn ring_size_1() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..9 {
            ring.push_back(n);
        }
        assert_eq!(ring.len(), 9);
    }

    // start > stop
    #[test]
    fn ring_size_2() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..5 {
            ring.push_back(n);
            assert_eq!(ring.pop_front(), Some(n));
        }

        for n in 0..9 {
            ring.push_back(n);
        }
        assert_eq!(ring.len(), 9);
    }

    // start == stop, init == true
    #[test]
    fn ring_size_3() {
        let ring = RingDeque::<10, i32>::new();
        assert_eq!(ring.len(), 0);
    }

    // start == stop, init == false
    #[test]
    fn ring_size_4() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n);
        }
        assert_eq!(ring.len(), 10);
    }

    #[test]
    fn ring_iter_forwards() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n)
        }

        let mut iter = ring.iter();
        for n in 0..10 {
            assert_eq!(iter.next(), Some(&n));
        }
        assert_eq!(iter.next(), None);
        drop(iter);

        // iter should not mutate the base ring
        unsafe {
            assert_eq!(ring.ring[0].assume_init(), 0);
            assert_eq!(ring.ring[1].assume_init(), 1);
            assert_eq!(ring.ring[2].assume_init(), 2);
            assert_eq!(ring.ring[3].assume_init(), 3);
            assert_eq!(ring.ring[4].assume_init(), 4);
            assert_eq!(ring.ring[5].assume_init(), 5);
            assert_eq!(ring.ring[6].assume_init(), 6);
            assert_eq!(ring.ring[7].assume_init(), 7);
            assert_eq!(ring.ring[8].assume_init(), 8);
            assert_eq!(ring.ring[9].assume_init(), 9);
        }

        assert_eq!(ring.try_push_front(10), Some(10));
    }

    #[test]
    fn ring_iter_reversed() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n)
        }

        let mut iter = ring.iter().rev();
        for n in (0..10).rev() {
            assert_eq!(iter.next(), Some(&n));
        }
        assert_eq!(iter.next(), None);
        drop(iter);

        // iter should not mutate the base ring
        unsafe {
            assert_eq!(ring.ring[0].assume_init(), 0);
            assert_eq!(ring.ring[1].assume_init(), 1);
            assert_eq!(ring.ring[2].assume_init(), 2);
            assert_eq!(ring.ring[3].assume_init(), 3);
            assert_eq!(ring.ring[4].assume_init(), 4);
            assert_eq!(ring.ring[5].assume_init(), 5);
            assert_eq!(ring.ring[6].assume_init(), 6);
            assert_eq!(ring.ring[7].assume_init(), 7);
            assert_eq!(ring.ring[8].assume_init(), 8);
            assert_eq!(ring.ring[9].assume_init(), 9);
        }

        assert_eq!(ring.try_push_front(10), Some(10));
    }

    #[test]
    fn ring_iter_mut_forwards() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n);
        }

        for n in ring.iter_mut() {
            *n += 1;
        }

        // iter SHOULD mutate the base ring
        unsafe {
            assert_eq!(ring.ring[0].assume_init(), 1);
            assert_eq!(ring.ring[1].assume_init(), 2);
            assert_eq!(ring.ring[2].assume_init(), 3);
            assert_eq!(ring.ring[3].assume_init(), 4);
            assert_eq!(ring.ring[4].assume_init(), 5);
            assert_eq!(ring.ring[5].assume_init(), 6);
            assert_eq!(ring.ring[6].assume_init(), 7);
            assert_eq!(ring.ring[7].assume_init(), 8);
            assert_eq!(ring.ring[8].assume_init(), 9);
            assert_eq!(ring.ring[9].assume_init(), 10);
        }

        assert_eq!(ring.try_push_front(10), Some(10));
    }

    #[test]
    fn ring_iter_mut_reversed() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n);
        }

        let mut iter = ring.iter_mut().rev();
        for n in 0..10 {
            *iter.next().unwrap() += n;
        }
        drop(iter);

        // iter SHOULD mutate the base ring
        unsafe {
            assert_eq!(ring.ring[0].assume_init(), 9);
            assert_eq!(ring.ring[1].assume_init(), 9);
            assert_eq!(ring.ring[2].assume_init(), 9);
            assert_eq!(ring.ring[3].assume_init(), 9);
            assert_eq!(ring.ring[4].assume_init(), 9);
            assert_eq!(ring.ring[5].assume_init(), 9);
            assert_eq!(ring.ring[6].assume_init(), 9);
            assert_eq!(ring.ring[7].assume_init(), 9);
            assert_eq!(ring.ring[8].assume_init(), 9);
            assert_eq!(ring.ring[9].assume_init(), 9);
        }

        assert_eq!(ring.try_push_front(10), Some(10));
    }

    #[test]
    fn ring_into_iter_forwards() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n)
        }

        let mut iter = ring.into_iter();
        for n in 0..10 {
            assert_eq!(iter.next(), Some(n));
        }
    }

    #[test]
    fn ring_into_iter_reversed() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n)
        }

        let mut iter = ring.into_iter().rev();
        for n in (0..10).rev() {
            assert_eq!(iter.next(), Some(n));
        }
    }

    #[test]
    fn ring_debug() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n);
        }

        assert_eq!(&format!("{ring:?}"), "[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]");
    }

    #[test]
    fn ring_clone() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n)
        }

        let ring_clone = ring.clone();

        unsafe {
            assert_eq!(ring.ring[0].assume_init(), 0);
            assert_eq!(ring.ring[1].assume_init(), 1);
            assert_eq!(ring.ring[2].assume_init(), 2);
            assert_eq!(ring.ring[3].assume_init(), 3);
            assert_eq!(ring.ring[4].assume_init(), 4);
            assert_eq!(ring.ring[5].assume_init(), 5);
            assert_eq!(ring.ring[6].assume_init(), 6);
            assert_eq!(ring.ring[7].assume_init(), 7);
            assert_eq!(ring.ring[8].assume_init(), 8);
            assert_eq!(ring.ring[9].assume_init(), 9);
        }

        assert_eq!(ring.len(), 10);

        unsafe {
            assert_eq!(ring_clone.ring[0].assume_init(), 0);
            assert_eq!(ring_clone.ring[1].assume_init(), 1);
            assert_eq!(ring_clone.ring[2].assume_init(), 2);
            assert_eq!(ring_clone.ring[3].assume_init(), 3);
            assert_eq!(ring_clone.ring[4].assume_init(), 4);
            assert_eq!(ring_clone.ring[5].assume_init(), 5);
            assert_eq!(ring_clone.ring[6].assume_init(), 6);
            assert_eq!(ring_clone.ring[7].assume_init(), 7);
            assert_eq!(ring_clone.ring[8].assume_init(), 8);
            assert_eq!(ring_clone.ring[9].assume_init(), 9);
        }

        assert_eq!(ring_clone.len(), 10);
    }

    #[test]
    fn ring_eq() {
        let mut ring1 = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring1.push_back(n)
        }

        let mut ring2 = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring2.push_back(n)
        }

        assert_eq!(ring1, ring2);
    }

    struct DropCounter<T> {
        _val: T,
        counter: std::rc::Rc<std::cell::Cell<usize>>,
    }

    impl<T> Drop for DropCounter<T> {
        fn drop(&mut self) {
            let count = self.counter.get() + 1;
            self.counter.set(count);
        }
    }

    impl<T> DropCounter<T> {
        fn new(cell: &std::rc::Rc<std::cell::Cell<usize>>, val: T) -> Self {
            Self { _val: val, counter: std::rc::Rc::clone(cell) }
        }
    }

    #[test]
    fn ring_drop_simple() {
        let mut ring = RingDeque::<10, DropCounter<Vec<i32>>>::new();
        let rc = std::rc::Rc::default();
        for n in 0..10 {
            ring.push_back(DropCounter::new(&rc, vec![n]));
        }

        drop(ring);
        assert_eq!(rc.get(), 10);
    }

    #[test]
    fn ring_drop_into_iter() {
        let mut ring = RingDeque::<10, DropCounter<Vec<i32>>>::new();
        let rc = std::rc::Rc::default();
        for n in 0..10 {
            ring.push_back(DropCounter::new(&rc, vec![n]));
        }

        drop(ring.into_iter());
        assert_eq!(rc.get(), 10);
    }
}
