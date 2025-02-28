#[derive(Debug)]
struct RingDeque<const CAPACITY: usize, T> {
    ring: [std::mem::MaybeUninit<T>; CAPACITY],
    start: usize,
    stop: usize,
    init: bool,
}

impl<const CAPACITY: usize, T> RingDeque<CAPACITY, T> {
    pub fn new() -> Self {
        if CAPACITY == 0 {
            panic!("Cannot create a RingDeque with a capacity of 0");
        }
        Self { ring: [const { std::mem::MaybeUninit::uninit() }; CAPACITY], start: 0, stop: 0, init: true }
    }

    pub fn push_front(&mut self, item: T) {
        if !self.try_push_front(item) {
            panic!("Cannot add more elements, ring is full")
        }
    }

    pub fn push_back(&mut self, item: T) {
        if !self.try_push_back(item) {
            panic!("Cannot add more elements, ring is full")
        }
    }

    pub fn pop_front(&mut self) -> Option<T> {
        if self.init && self.start == self.stop {
            None
        } else {
            let res = unsafe { self.ring[self.start].assume_init_read() };
            self.start = bounded_increment::<CAPACITY>(self.start);

            if self.start == self.stop {
                self.init = true;
            }

            Some(res)
        }
    }

    pub fn pop_back(&mut self) -> Option<T> {
        if self.init && self.start == self.stop {
            None
        } else {
            self.stop = bounded_decrement::<CAPACITY>(self.stop);
            let res = unsafe { self.ring[self.stop].assume_init_read() };

            if self.start == self.stop {
                self.init = true;
            }

            Some(res)
        }
    }

    pub fn peek_front(&mut self) -> Option<T> {
        if self.init && self.start == self.stop {
            None
        } else {
            let res = unsafe { self.ring[self.start].assume_init_read() };
            Some(res)
        }
    }

    pub fn peek_back(&mut self) -> Option<T> {
        if self.init && self.start == self.stop {
            None
        } else {
            let res = unsafe { self.ring[bounded_decrement::<CAPACITY>(self.stop)].assume_init_read() };
            Some(res)
        }
    }

    pub fn try_push_front(&mut self, item: T) -> bool {
        if !self.init && self.start == self.stop {
            false
        } else {
            self.init = false;
            self.start = bounded_decrement::<CAPACITY>(self.start);
            self.ring[self.start].write(item);
            true
        }
    }

    pub fn try_push_back(&mut self, item: T) -> bool {
        if !self.init && self.start == self.stop {
            false
        } else {
            self.init = false;
            self.ring[self.stop].write(item);
            self.stop = bounded_increment::<CAPACITY>(self.stop);
            true
        }
    }

    pub fn size(&self) -> usize {
        match self.start.cmp(&self.stop) {
            std::cmp::Ordering::Less => self.stop - self.start,
            std::cmp::Ordering::Equal => {
                if self.init {
                    0
                } else {
                    CAPACITY
                }
            }
            std::cmp::Ordering::Greater => CAPACITY - self.start + self.stop,
        }
    }

    pub fn capacity(&self) -> usize {
        return CAPACITY;
    }

    pub fn iter(&self) -> RingIterator<'_, CAPACITY, T> {
        RingIterator { ring: &self.ring, start: self.start, stop: self.stop, init: self.init }
    }
}

struct RingIterator<'a, const CAPACITY: usize, T> {
    ring: &'a [std::mem::MaybeUninit<T>; CAPACITY],
    start: usize,
    stop: usize,
    init: bool,
}

impl<'a, const CAPACITY: usize, T> Iterator for RingIterator<'a, CAPACITY, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.init && self.start == self.stop {
            None
        } else {
            let res = unsafe { self.ring[self.start].assume_init_read() };
            self.start = bounded_increment::<CAPACITY>(self.start);

            if self.start == self.stop {
                self.init = true;
            }

            Some(res)
        }
    }
}

impl<'a, const CAPACITY: usize, T> DoubleEndedIterator for RingIterator<'a, CAPACITY, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.init && self.start == self.stop {
            None
        } else {
            self.stop = bounded_decrement::<CAPACITY>(self.stop);
            let res = unsafe { self.ring[self.stop].assume_init_read() };

            if self.start == self.stop {
                self.init = true;
            }

            Some(res)
        }
    }
}

fn bounded_increment<const CAPACITY: usize>(mut n: usize) -> usize {
    n += 1;
    if n >= CAPACITY {
        0
    } else {
        n
    }
}

fn bounded_decrement<const CAPACITY: usize>(n: usize) -> usize {
    n.checked_sub(1).unwrap_or(CAPACITY - 1)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn ring_new() {
        let ring = RingDeque::<10, ()>::new();
        assert_eq!(ring.size(), 0);
        assert_eq!(ring.capacity(), 10);
    }

    #[test]
    #[should_panic]
    fn ring_new_panic_zero_capacity() {
        let _ = RingDeque::<0, ()>::new();
    }

    #[test]
    fn ring_push_back() {
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

        assert_eq!(ring.size(), 10);
    }

    #[test]
    fn ring_push_front() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in (0..10).into_iter().rev() {
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

        assert_eq!(ring.size(), 10);
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
            ring.push_back(n);
        }
        assert!(!ring.try_push_back(10));
    }

    #[test]
    fn ring_push_front_try() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in (0..10).into_iter().rev() {
            ring.push_front(n);
        }
        assert!(!ring.try_push_front(10));
    }

    #[test]
    fn ring_push_back_pop_back() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n);
        }
        for n in (0..10).into_iter().rev() {
            assert_eq!(ring.pop_back(), Some(n));
        }

        assert_eq!(ring.pop_back(), None);
        assert_eq!(ring.start, 0);
        assert_eq!(ring.stop, 0);

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
        assert_eq!(ring.stop, 0);

        // This should not fail even though ring.start == ring.stop
        for n in (0..10).into_iter().rev() {
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

        assert_eq!(ring.size(), 10)
    }

    #[test]
    fn ring_push_front_pop_front() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in (0..10).into_iter().rev() {
            ring.push_front(n);
        }
        for n in 0..10 {
            assert_eq!(ring.pop_front(), Some(n));
        }

        assert_eq!(ring.pop_front(), None);
        assert_eq!(ring.start, 0);
        assert_eq!(ring.stop, 0);

        // This should not fail even though ring.start == ring.stop
        for n in (0..10).into_iter().rev() {
            ring.push_front(n);
        }
    }

    #[test]
    fn ring_push_front_pop_back() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in (0..10).into_iter().rev() {
            ring.push_front(n);
        }
        for n in (0..10).into_iter().rev() {
            assert_eq!(ring.pop_back(), Some(n));
        }

        assert_eq!(ring.pop_back(), None);
        assert_eq!(ring.start, 0);
        assert_eq!(ring.stop, 0);

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

        assert_eq!(ring.size(), 10)
    }

    #[test]
    fn ring_peek_back() {
        let mut ring = RingDeque::<10, i32>::new();
        assert_eq!(ring.peek_back(), None);
        for n in 0..10 {
            ring.push_back(n);
            assert_eq!(ring.peek_back(), Some(n));
        }
    }

    #[test]
    fn ring_peek_front() {
        let mut ring = RingDeque::<10, i32>::new();
        assert_eq!(ring.peek_front(), None);
        for n in (0..10).into_iter().rev() {
            ring.push_front(n);
            assert_eq!(ring.peek_front(), Some(n));
        }
    }

    // start < stop
    #[test]
    fn ring_size_1() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..9 {
            ring.push_back(n);
        }
        assert_eq!(ring.size(), 9);
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
        assert_eq!(ring.size(), 9);
    }

    // start == stop, init == true
    #[test]
    fn ring_size_3() {
        let ring = RingDeque::<10, i32>::new();
        assert_eq!(ring.size(), 0);
    }

    // start == stop, init == false
    #[test]
    fn ring_size_4() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n);
        }
        assert_eq!(ring.size(), 10);
    }

    #[test]
    fn ring_iter_forwards() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n)
        }

        let mut iter = ring.iter();
        for n in 0..10 {
            assert_eq!(iter.next(), Some(n));
        }
        assert_eq!(iter.next(), None);

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

        assert_eq!(ring.try_push_front(10), false);
    }

    #[test]
    fn ring_iter_reversed() {
        let mut ring = RingDeque::<10, i32>::new();
        for n in 0..10 {
            ring.push_back(n)
        }

        let mut iter = ring.iter().rev();
        for n in (0..10).into_iter().rev() {
            assert_eq!(iter.next(), Some(n));
        }
        assert_eq!(iter.next(), None);

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

        assert_eq!(ring.try_push_front(10), false);
    }
}
