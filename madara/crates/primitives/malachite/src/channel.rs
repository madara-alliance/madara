pub struct MqSender<T: Send + Clone> {
    queue: std::sync::Arc<MessageQueue<T>>,
    wake: std::sync::Arc<tokio::sync::Notify>,
}

pub struct MqReceiver<T: Send + Clone> {
    queue: std::sync::Arc<MessageQueue<T>>,
    wake: std::sync::Arc<tokio::sync::Notify>,
}

pub struct MqGuard<'a, T: Send + Clone> {
    ack: bool,
    queue: std::sync::Arc<MessageQueue<T>>,
    cell_ptr: std::ptr::NonNull<AckCell<T>>,
    _phantom: std::marker::PhantomData<&'a ()>,
}

struct MessageQueue<T: Send + Clone> {
    ring: std::ptr::NonNull<AckCell<T>>,
    writter: WriteRegion,
    reader: ReadRegion,
    cap: usize,
}

struct AckCell<T: Clone> {
    elem: T,
    ack: std::sync::atomic::AtomicBool,
}

struct ReadRegion {
    start: std::sync::atomic::AtomicUsize,
    size: std::sync::atomic::AtomicUsize,
}

struct WriteRegion {
    start: std::sync::atomic::AtomicUsize,
    size: std::sync::atomic::AtomicUsize,
}

impl<'a, T: Send + Clone> Drop for MqGuard<'a, T> {
    fn drop(&mut self) {
        // If the element was not acknowledged, we add it back to the queue to be picked up again.
        if !self.ack {
            self.queue.write(self.retrieve());
        }
    }
}

impl<T: Send + Clone> Drop for MessageQueue<T> {
    fn drop(&mut self) {
        self.reader.size.store(0, std::sync::atomic::Ordering::Release);

        let start = self.reader.start();
        let stop = start + self.reader.size();

        for i in (start..stop).map(|i| fast_mod(i, self.cap)) {
            unsafe { std::ptr::read(self.ring.as_ptr().add(i)) };
        }

        let layout = std::alloc::Layout::array::<T>(self.cap).unwrap();
        unsafe { std::alloc::dealloc(self.ring.as_ptr() as *mut u8, layout) }
    }
}

pub fn channel<T: Send + Clone>(cap: usize) -> (MqSender<T>, MqReceiver<T>) {
    let queue_s = std::sync::Arc::new(MessageQueue::new(cap));
    let queue_r = std::sync::Arc::clone(&queue_s);

    let wake_s = std::sync::Arc::new(tokio::sync::Notify::new());
    let wake_r = std::sync::Arc::clone(&wake_s);

    let sx = MqSender { queue: queue_s, wake: wake_s };
    let rx = MqReceiver { queue: queue_r, wake: wake_r };

    (sx, rx)
}

impl<T: Send + Clone> MqSender<T> {
    pub async fn send(&self, elem: T) -> Option<T> {
        match self.queue.write(elem) {
            Some(elem) => Some(elem), // Failed to write to the queue
            None => {
                self.wake.notify_waiters();
                None
            }
        }
    }
}

impl<T: Send + Clone> MqReceiver<T> {
    pub async fn rcv(&self) -> Option<MqGuard<T>> {
        loop {
            match self.queue.read() {
                Some(cell_ptr) => break Some(MqGuard::new(std::sync::Arc::clone(&self.queue), cell_ptr)),
                None => self.wake.notified().await,
            }
        }
    }
}

impl<'a, T: Send + Clone> MqGuard<'a, T> {
    pub fn new(queue: std::sync::Arc<MessageQueue<T>>, cell_ptr: std::ptr::NonNull<AckCell<T>>) -> Self {
        MqGuard { ack: false, queue, cell_ptr, _phantom: std::marker::PhantomData }
    }

    pub fn acknowledge(mut self) -> T {
        self.ack = true;
        self.retrieve()
    }

    // Invariant: calling this twice will result in a double free
    fn retrieve(&self) -> T {
        let cell = unsafe { std::ptr::read(self.cell_ptr.as_ptr()) };
        let ack = std::mem::ManuallyDrop::new(cell.ack);

        // It is worth taking a moment to understand why this is here.
        //
        // > "If an atomic store in thread A is tagged memory_order_release, an atomic load in
        // > thread B from the same variable is tagged memory_order_acquire, and the load in thread
        // > B reads a value written by the store in thread A, then the store in thread A
        // > synchronizes with the load in thread B.
        // >
        // > All memory writes (including non-atomic and relaxed atomic) that happened-before the
        // > atomic store from the point of view of thread A, become visible side-effects in thread
        // > B. That is, once the atomic load is completed, thread B is guaranteed to see everything
        // > thread A wrote to memory. This promise only holds if B actually returns the value that
        // > A stored, or a value from later in the release sequence."
        //
        // https://en.cppreference.com/w/cpp/atomic/memory_order#Release-Acquire_ordering
        //
        // So first of all note that by using a `Relaxed` ordering on the subsequent store, we are
        // guaranteeing that `drop` happens before other threads can see the effects of the store.
        //
        // When a `MessageQueue` is dropped, it will drop the elements in its read region, as these
        // have not yet been received by any thread but itself. It does not have any mechanism to
        // handle the dropping of elements which have already been read, whether acknowledged or
        // not. For this reason we need to handle this here. The invariant of this method asks that
        // we call it only once, and it operates on a reference because this logic also applies when
        // dropping the guard if the element has not been acknowledged.
        //
        // Why is the atomic store important? Because we use it as a mechanism in `WriteRegion` to
        // check if a `AcqCell` has been acknowledged or not. We do not want this cell to be marked
        // as ready for write while its contents have not been freed yet! Notice that this is a
        // delicate dance we are playing: `WriteRegion` will be reading the atomic ack state of the
        // cell, while the elem it stores has already been dropped! Since we are not reading the
        // element this is okay. It is also very unsafe. A better way to represent this would be to
        // use `MaybeUninitialized`, but that is for another day.
        //
        // Finally, notice the lifetime attached to `MqGuard`: this ensure that the guard's lifetime
        // is tied to its receiver, and hence to the underlying `MessageQueue`. This ensures that
        // the message queue and its underlying array are not dropped and freed while or before we
        // are dropping and (potentially) freeing this element.
        let elem = cell.elem.clone();
        drop(cell.elem);
        ack.store(true, std::sync::atomic::Ordering::Release);

        elem
    }
}

impl<T: Send + Clone> MessageQueue<T> {
    fn new(cap: usize) -> Self {
        assert_ne!(std::mem::size_of::<T>(), 0, "T cannot be a ZST");
        assert!(cap > 0, "Tried to create a message queue with a capacity < 1");

        let cap = cap.checked_next_power_of_two().expect("failed to retrieve the next power of 2 to cap");
        let layout = std::alloc::Layout::array::<AckCell<T>>(cap).unwrap();

        // From the `Layout` docs: "All layouts have an associated size and a power-of-two alignment.
        // The size, when rounded up to the nearest multiple of align, does not overflow isize (i.e.
        // the rounded value will always be less than or equal to isize::MAX)."
        //
        // I could not find anything in the source code of this method that checks that so making
        // sure here, is this really necessary?
        assert!(layout.size() <= std::isize::MAX as usize);

        let ptr = unsafe { std::alloc::alloc(layout) };
        let ring = match std::ptr::NonNull::new(ptr as *mut AckCell<T>) {
            Some(p) => p,
            None => std::alloc::handle_alloc_error(layout),
        };

        let writter = WriteRegion::new(cap);
        let reader = ReadRegion::new();

        Self { ring, cap, writter, reader }
    }

    fn read(&self) -> Option<std::ptr::NonNull<AckCell<T>>> {
        self.reader.read(self.ring, self.cap)
    }

    fn write(&self, elem: T) -> Option<T> {
        let res = self.writter.write(elem, self.ring, self.cap);
        if res.is_none() {
            self.reader.grow(self.cap);
        }
        res
    }
}

impl<T: Clone> AckCell<T> {
    fn new(elem: T) -> Self {
        Self { elem, ack: std::sync::atomic::AtomicBool::new(false) }
    }
}

impl ReadRegion {
    fn new() -> Self {
        Self { start: std::sync::atomic::AtomicUsize::new(0), size: std::sync::atomic::AtomicUsize::new(0) }
    }

    fn start(&self) -> usize {
        self.start.load(std::sync::atomic::Ordering::Acquire)
    }

    fn size(&self) -> usize {
        self.size.load(std::sync::atomic::Ordering::Acquire)
    }

    fn read<T: Send + Clone>(
        &self,
        buf: std::ptr::NonNull<AckCell<T>>,
        cap: usize,
    ) -> Option<std::ptr::NonNull<AckCell<T>>> {
        let size = self.size();
        let res = if size == 0 {
            // Note that we do not try to grow the read region in case there is nothing left to
            // read. This is because while cells have and `ack` state to attest if they have been
            // read, we do not store any extra information concerning their write status. Instead,
            // it is the responsibility of the queue to grow the read region whenever it sends a new
            // value.
            None
        } else {
            let start = self.start();
            self.start.store(fast_mod(start + 1, cap), std::sync::atomic::Ordering::Release);
            self.size.store(size - 1, std::sync::atomic::Ordering::Release); // checked above

            Some(unsafe { buf.add(start) })
        };

        // Since we perform atomic loads and stores separately, this fence is required to ensure
        // those operations remain ordered across multiple invocations of this function and from
        // different threads.
        //
        // Example:
        //
        // Thread 1: * loads [0] ---> * stores [1] ---> *fence
        // Thread 2: -- * loads ......................... [1] ---> * stores [2] ---> *fence
        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        res
    }

    fn grow(&self, cap: usize) {
        let size = self.size.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        debug_assert_ne!(size, cap);
    }
}

impl WriteRegion {
    fn new(size: usize) -> Self {
        assert!(size > 0);
        Self { start: std::sync::atomic::AtomicUsize::new(0), size: std::sync::atomic::AtomicUsize::new(size) }
    }

    fn start(&self) -> usize {
        self.start.load(std::sync::atomic::Ordering::Acquire)
    }

    fn size(&self) -> usize {
        self.size.load(std::sync::atomic::Ordering::Acquire)
    }

    fn write<T: Clone>(&self, elem: T, buf: std::ptr::NonNull<AckCell<T>>, cap: usize) -> Option<T> {
        let size = self.size();
        let res = if size == 0 && self.grow(buf, cap).is_err() {
            Some(elem)
        } else {
            let start = self.start();

            // size - 1 is checked above and `grow` will increment size by 1 if it succeeds, so
            // whatever happens size > 0
            self.start.store(fast_mod(start + 1, cap), std::sync::atomic::Ordering::Release);
            self.size.store(size - 1, std::sync::atomic::Ordering::Release);

            unsafe { std::ptr::write(buf.as_ptr().add(start), AckCell::new(elem)) };
            None
        };

        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        res
    }

    fn grow<T: Clone>(&self, buf: std::ptr::NonNull<AckCell<T>>, cap: usize) -> Result<(), &'static str> {
        // We are indexing the element right AFTER the end of the write region to see if we can
        // overwrite it (ie: it has been read and acknowledged)
        let size = self.size();
        let start = self.start();
        let stop = fast_mod(start + size, cap);

        // There are a few invariants which guarantee that this will never index into uninitialized
        // memory:
        //
        // 1. We do not allow to create a empty write region.
        // 2. We only ever call grow if we have no more space left to write.
        // 3. A write region should initially cover the entirety of the array being written to.
        //
        // Inv. [1] and Inv. [3] guarantee that we are not writing into an empty array.
        // Consequentially, Inv. [2] guarantees that if there is no more space left to write, then
        // we must have filled up the array, hence we will wrap around to a value which was already
        // written to previously.
        //
        // Note that the `ack` state of that value/cell might have been updated by a `MqGuard` in
        // the meantime, which is what we are checking for: we cannot grow and mark a value as ready
        // to write to if it has not already been read and acknowledged.
        let cell = unsafe { std::ptr::read(buf.as_ptr().add(stop)) };
        let ack = cell.ack.load(std::sync::atomic::Ordering::Acquire);

        // See the note in `MqGuard` to understand why this comparison is risky!
        let res = if ack {
            debug_assert!(size + 1 <= cap);
            self.size.store(size + 1, std::sync::atomic::Ordering::Release);
            Ok(())
        } else {
            Err("Failed to grow write region, next element has not been acknowledged yet")
        };

        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        res
    }
}

fn fast_mod(n: usize, pow_of_2: usize) -> usize {
    n & (pow_of_2 - 1)
}
