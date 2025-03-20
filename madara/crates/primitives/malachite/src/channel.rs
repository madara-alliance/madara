pub struct MqSender<T: Send + Clone> {
    queue: std::sync::Arc<MessageQueue<T>>,
    reader: std::sync::Arc<ReadRegion>,
    writer: WriteRegion,
}

pub struct MqReceiver<T: Send + Clone> {
    queue: std::sync::Arc<MessageQueue<T>>,
    reader: std::sync::Arc<ReadRegion>,
}

pub struct MqGuard<T: Send + Clone> {
    elem: T,
    // Well this is VERY unsafe. This is a pointer to the element the guard is holding but in the
    // actual message queue. This is so that the guard can mark it as acknowledged on drop.
    cell: std::ptr::NonNull<AckCell<T>>,
}

struct MessageQueue<T: Send + Clone> {
    ring: std::ptr::NonNull<AckCell<T>>,
    cap: usize,
}

struct AckCell<T> {
    elem: T,
    ack: std::sync::atomic::AtomicBool,
}

struct ReadRegion {
    start: std::sync::atomic::AtomicUsize,
    size: std::sync::atomic::AtomicUsize,
}

struct WriteRegion {
    start: usize,
    size: usize,
    // WriteRegion is NOT `Send` and follows a single source of mutation
    _phantom: std::marker::PhantomData<*const ()>,
}

impl<T: Send + Clone> Drop for MqSender<T> {
    fn drop(&mut self) {
        self.reader.size.store(0, std::sync::atomic::Ordering::Release);

        let start = self.writer.start;
        let stop = start + self.writer.size;

        for i in (start..stop).map(|i| fast_mod(i, self.queue.cap)) {
            unsafe { drop(std::ptr::read(self.queue.ring.as_ptr().add(i))) }
        }
    }
}

impl<T: Send + Clone> Drop for MqGuard<T> {
    fn drop(&mut self) {
        let cell = unsafe { std::ptr::read(self.cell.as_ptr()) };
        cell.ack.store(true, std::sync::atomic::Ordering::Release);
    }
}

impl<T: Send + Clone> Drop for MessageQueue<T> {
    fn drop(&mut self) {
        let layout = std::alloc::Layout::array::<T>(self.cap).unwrap();
        unsafe { std::alloc::dealloc(self.ring.as_ptr() as *mut u8, layout) }
    }
}

impl<T: Send + Clone> std::ops::Deref for MqGuard<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.elem
    }
}

impl<T: Send + Clone> std::ops::DerefMut for MqGuard<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.elem
    }
}

pub fn channel<T: Send + Clone>(cap: usize) -> (MqSender<T>, MqReceiver<T>) {
    let mq_s = std::sync::Arc::new(MessageQueue::new(cap));
    let mq_r = std::sync::Arc::clone(&mq_s);
    let reader_s = std::sync::Arc::new(ReadRegion::new());
    let reader_r = std::sync::Arc::clone(&reader_s);
    let writer = WriteRegion::new(cap);

    let sx = MqSender { queue: mq_s, reader: reader_s, writer };
    let rx = MqReceiver { queue: mq_r, reader: reader_r };

    (sx, rx)
}

impl<T: Send + Clone> MqSender<T> {
    pub fn send(&mut self, elem: T) -> Option<T> {
        let res = self.writer.write(elem, self.queue.ring, self.queue.cap);
        self.reader.grow(self.queue.cap);

        res
    }

    pub fn resubscribe(&self) -> MqReceiver<T> {
        let queue = std::sync::Arc::clone(&self.queue);
        let reader = std::sync::Arc::clone(&self.reader);

        MqReceiver { queue, reader }
    }
}

impl<T: Send + Clone> MqReceiver<T> {
    pub async fn read(&self) -> Option<MqGuard<T>> {
        // TODO: this is not really correct. We currently don't have a good way to check and see if
        // the message queue was closed by the sender. An Arc<AtomicBool> shared between sender and
        // receivers seems like it could be a good idea?
        while self.reader.size.load(std::sync::atomic::Ordering::Relaxed) == 0 {}
        self.reader.read(self.queue.ring, self.queue.cap)
    }

    pub fn resubscribe(&self) -> MqReceiver<T> {
        let queue = std::sync::Arc::clone(&self.queue);
        let reader = std::sync::Arc::clone(&self.reader);

        MqReceiver { queue, reader }
    }
}

impl<T: Send + Clone> MessageQueue<T> {
    fn new(cap: usize) -> Self {
        assert_ne!(std::mem::size_of::<T>(), 0, "T cannot be a ZST");
        assert!(cap == 0, "Tried to create a message queue with a capacity < 1");

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

        Self { ring, cap }
    }
}

impl<T> AckCell<T> {
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

    fn read<T: Send + Clone>(&self, buf: std::ptr::NonNull<AckCell<T>>, cap: usize) -> Option<MqGuard<T>> {
        let size = self.size();
        let res = if size == 0 {
            // Note that we do not try to grow the read region in case there is nothing left to
            // read. This is because while cells have and `ack` state to attest if they have been
            // read, we do not store any extra information concerning their write status. Instead,
            // it is the responsibility of the `MqSender` to grow the read region whenever it sends
            // a value.
            None
        } else {
            let start = self.start();
            self.start.store(fast_mod(start + 1, cap), std::sync::atomic::Ordering::Release);
            self.size.store(size - 1, std::sync::atomic::Ordering::Release); // checked above

            let ptr = unsafe { buf.as_ptr().add(start) };
            let elem = unsafe { std::ptr::read(ptr).elem.clone() };
            let cell = std::ptr::NonNull::new(ptr).expect("Memory is already initialized");

            Some(MqGuard { elem, cell })
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
        todo!()
    }
}

impl WriteRegion {
    fn new(size: usize) -> Self {
        assert!(size > 0);
        Self { start: 0, size, _phantom: std::marker::PhantomData }
    }

    fn write<T>(&mut self, elem: T, buf: std::ptr::NonNull<AckCell<T>>, cap: usize) -> Option<T> {
        if self.size == 0 && self.grow(buf, cap).is_err() {
            Some(elem)
        } else {
            unsafe { std::ptr::write(buf.as_ptr().add(self.start), AckCell::new(elem)) };
            self.start = fast_mod(self.start + 1, cap);
            self.size -= 1; // checked above, `grow` will increment size by 1 if it succeeds
            None
        }
    }

    fn grow<T>(&mut self, buf: std::ptr::NonNull<AckCell<T>>, cap: usize) -> Result<(), &'static str> {
        // We are indexing the element right AFTER the end of the write region to see if we can
        // overwrite it (ie: it has been read and acknowledged)
        let stop = fast_mod(self.start + self.size, cap);
        let cell = unsafe { std::ptr::read(buf.as_ptr().add(stop)) };

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
        let ack = cell.ack.load(std::sync::atomic::Ordering::Acquire);

        let res = if ack {
            self.size += 1;
            debug_assert!(self.size <= cap);

            // we need to drop the element since it might be overwritten in the future
            drop(cell.elem);
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
