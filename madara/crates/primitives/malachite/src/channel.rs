#[cfg(test)]
use loom::alloc;
#[cfg(test)]
use loom::sync;
#[cfg(test)]
use loom::sync::Notify;

#[cfg(not(test))]
use std::alloc;
#[cfg(not(test))]
use std::sync;
#[cfg(not(test))]
use tokio::sync::Notify;

const STRT_MSK: u64 = 0xffffffff00000000;
const SIZE_MSK: u64 = 0x00000000ffffffff;
const STRT_INC: u64 = 0x0000000100000000;
const SIZE_INC: u64 = 0x0000000000000001;

pub struct MqSender<T: Send + Clone + std::fmt::Debug + tracing::Value> {
    queue: sync::Arc<MessageQueue<T>>,
    close: sync::Arc<sync::atomic::AtomicBool>,
    wake: sync::Arc<Notify>,
}

pub struct MqReceiver<T: Send + Clone + std::fmt::Debug + tracing::Value> {
    queue: sync::Arc<MessageQueue<T>>,
    wake: sync::Arc<Notify>,
}

#[must_use]
pub struct MqGuard<'a, T: Send + Clone + std::fmt::Debug + tracing::Value> {
    ack: bool,
    queue: sync::Arc<MessageQueue<T>>,
    cell_ptr: std::ptr::NonNull<AckCell<T>>,
    _phantom: std::marker::PhantomData<&'a ()>,
}

struct MessageQueue<T: Send + Clone + std::fmt::Debug + tracing::Value> {
    ring: std::ptr::NonNull<AckCell<T>>,
    senders: sync::atomic::AtomicUsize,
    writer: WriteRegion,
    reader: ReadRegion,
    cap: u32,
}

/// An atomic acknowledge cell, use to ensure an element has been read.
struct AckCell<T: Clone + std::fmt::Debug + tracing::Value> {
    elem: std::mem::MaybeUninit<T>,
    ack: sync::atomic::AtomicBool,
}

/// A region in a pointer array in which we are allowed to [read].
///
/// [read]: Self::read
struct ReadRegion {
    /// We use a single atomic in which we store the start and size of a region as two [`u32`] ints.
    /// This is done so that updates to the start and the size of the region happen as a single
    /// atomic unit, with no possibility of another thread slotting a `store` or `load` in between.
    strt_and_size: sync::atomic::AtomicU64,
}

/// A region in a pointer array in which we are allowed to [write].
///
/// [write]: Self::write
struct WriteRegion {
    strt_and_size: sync::atomic::AtomicU64,
}

unsafe impl<T: Send + Clone + std::fmt::Debug + tracing::Value> Send for MqSender<T> {}
unsafe impl<T: Send + Clone + std::fmt::Debug + tracing::Value> Sync for MqSender<T> {}

unsafe impl<T: Send + Clone + std::fmt::Debug + tracing::Value> Send for MqReceiver<T> {}
unsafe impl<T: Send + Clone + std::fmt::Debug + tracing::Value> Sync for MqReceiver<T> {}

unsafe impl<'a, T: Send + Clone + std::fmt::Debug + tracing::Value> Send for MqGuard<'a, T> {}
unsafe impl<'a, T: Send + Clone + std::fmt::Debug + tracing::Value> Sync for MqGuard<'a, T> {}

impl<T: Send + Clone + std::fmt::Debug + tracing::Value> std::fmt::Debug for MqSender<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MqSender").field("queue", &self.queue).finish()
    }
}

impl<T: Send + Clone + std::fmt::Debug + tracing::Value> std::fmt::Debug for MqReceiver<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MqSender").field("queue", &self.queue).finish()
    }
}

impl<'a, T: Send + Clone + std::fmt::Debug + tracing::Value> std::fmt::Debug for MqGuard<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MqGuard").field("ack", &self.ack).field("elem", &self.read()).finish()
    }
}

impl<T: Send + Clone + std::fmt::Debug + tracing::Value> std::fmt::Debug for MessageQueue<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let senders = self.senders.load(sync::atomic::Ordering::Acquire);
        f.debug_struct("MessageQueue")
            .field("senders", &senders)
            .field("writer", &self.writer)
            .field("reader", &self.reader)
            .field("cap", &self.cap)
            .finish()
    }
}

impl std::fmt::Debug for ReadRegion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let raw_bytes = self.strt_and_size.load(sync::atomic::Ordering::Acquire);
        let strt = get_strt(raw_bytes);
        let size = get_size(raw_bytes);
        f.debug_struct("ReadRegion").field("strt", &strt).field("size", &size).finish()
    }
}

impl std::fmt::Debug for WriteRegion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let raw_bytes = self.strt_and_size.load(sync::atomic::Ordering::Acquire);
        let strt = get_strt(raw_bytes);
        let size = get_size(raw_bytes);
        f.debug_struct("WriteRegion").field("strt", &strt).field("size", &size).finish()
    }
}

impl<T: Send + Clone + std::fmt::Debug + tracing::Value> Drop for MqSender<T> {
    fn drop(&mut self) {
        if !self.close.load(sync::atomic::Ordering::Acquire) {
            self.queue.sender_unregister();

            #[cfg(test)]
            self.wake.notify();
            #[cfg(not(test))]
            self.wake.notify_waiters();
        }
    }
}

impl<'a, T: Send + Clone + std::fmt::Debug + tracing::Value> Drop for MqGuard<'a, T> {
    fn drop(&mut self) {
        // If the element was not acknowledged, we add it back to the queue to be picked up again,
        // taking special care to drop the element in the process. We do not need to do this if the
        // element was acknowledged since the drop logic is implemented in `elem_drop` and this is
        // handled by `acknowledge` if it is called (which also sets the `ack` flag so we don't
        // double free here).
        if !self.ack {
            // Tangential fact but view types could simplify this by allowing for
            // `self.queue.write(self.read())` :)
            //
            // https://smallcultfollowing.com/babysteps/blog/2025/02/25/view-types-redux/
            let elem = self.read();
            self.elem_drop();

            // This can still fail, but without async drop there isn't really anything better we can
            // do. Should we panic?
            self.queue.write(elem);
        }
    }
}

impl<T: Send + Clone + std::fmt::Debug + tracing::Value> Drop for MessageQueue<T> {
    fn drop(&mut self) {
        let raw_bytes = self.reader.strt_and_size.swap(0, sync::atomic::Ordering::Release);

        let strt = get_strt(raw_bytes);
        let size = get_size(raw_bytes);
        // FIXME: this risks overflowing!
        let stop = strt + size;

        for i in (strt..stop).map(|i| fast_mod(i, self.cap)) {
            unsafe { self.ring.add(i as usize).read() };
        }

        let layout = alloc::Layout::array::<AckCell<T>>(self.cap as usize).unwrap();
        unsafe { alloc::dealloc(self.ring.as_ptr() as *mut u8, layout) }
    }
}

#[tracing::instrument]
pub fn channel<T: Send + Clone + std::fmt::Debug + tracing::Value>(cap: u32) -> (MqSender<T>, MqReceiver<T>) {
    tracing::debug!("Creating new channel");

    let queue_s = sync::Arc::new(MessageQueue::new(cap));
    let queue_r = sync::Arc::clone(&queue_s);

    let wake_s = sync::Arc::new(Notify::new());
    let wake_r = sync::Arc::clone(&wake_s);

    let sx = MqSender::new(queue_s, wake_s);
    let rx = MqReceiver::new(queue_r, wake_r);

    (sx, rx)
}

impl<T: Send + Clone + std::fmt::Debug + tracing::Value> MqSender<T> {
    fn new(queue: sync::Arc<MessageQueue<T>>, wake: sync::Arc<Notify>) -> Self {
        queue.sender_register();
        Self { queue, close: sync::Arc::new(sync::atomic::AtomicBool::new(false)), wake }
    }

    #[tracing::instrument(skip(self))]
    pub fn send(&self, elem: T) -> Option<T> {
        tracing::debug!("Trying to send value");
        match self.queue.write(elem) {
            // Failed to write to the queue. This can happen if the next element right after the
            // write region has been read but not acknowledge yet.
            Some(elem) => {
                tracing::debug!("Failed to send value");
                Some(elem)
            }
            None => {
                tracing::debug!("Value sent successfully");

                #[cfg(test)]
                self.wake.notify();
                #[cfg(not(test))]
                self.wake.notify_one();

                None
            }
        }
    }

    pub fn close(self) {
        self.close.store(true, sync::atomic::Ordering::Release);
        self.queue.sender_unregister_all();

        #[cfg(test)]
        self.wake.notify();
        #[cfg(not(test))]
        self.wake.notify_waiters();
    }
}

impl<T: Send + Clone + std::fmt::Debug + tracing::Value> MqReceiver<T> {
    fn new(queue: sync::Arc<MessageQueue<T>>, wake: sync::Arc<Notify>) -> Self {
        Self { queue, wake }
    }

    fn resubscribe(&self) -> Self {
        Self { queue: sync::Arc::clone(&self.queue), wake: sync::Arc::clone(&self.wake) }
    }

    #[tracing::instrument(skip(self))]
    pub async fn recv(&self) -> Option<MqGuard<T>> {
        tracing::debug!("Trying to receive value");
        loop {
            match self.queue.read() {
                Some(cell_ptr) => {
                    let guard = MqGuard::new(sync::Arc::clone(&self.queue), cell_ptr);
                    tracing::debug!(elem = guard.read(), "Received value");
                    break Some(guard);
                }
                None => {
                    tracing::debug!("Failed to receive value");
                    if !self.queue.sender_available() && self.queue.reader.size() == 0 {
                        tracing::debug!("No sender available");
                        break None;
                    } else {
                        tracing::debug!("Waiting for a send");

                        #[cfg(test)]
                        self.wake.wait();
                        #[cfg(not(test))]
                        self.wake.notified().await;

                        tracing::debug!("A send was detected");
                    }
                }
            }
        }
    }
}

impl<'a, T: Send + Clone + std::fmt::Debug + tracing::Value> MqGuard<'a, T> {
    pub fn new(queue: sync::Arc<MessageQueue<T>>, cell_ptr: std::ptr::NonNull<AckCell<T>>) -> Self {
        MqGuard { ack: false, queue, cell_ptr, _phantom: std::marker::PhantomData }
    }

    pub fn read(&self) -> T {
        unsafe {
            let cell = self.cell_ptr.as_ref();
            cell.elem.assume_init_ref().clone()
        }
    }

    pub fn read_acknowledge(self) -> T {
        let elem = self.read();
        self.acknowledge();
        elem
    }

    pub fn acknowledge(mut self) {
        self.ack = true;
        self.elem_drop()
    }

    // Invariant: calling this twice will result in a double free
    fn elem_drop(&mut self) {
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
        // guaranteeing that `assume_init_drop` happens before other threads can see the effects of
        // the store.
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
        // as ready for write while its contents have not been freed yet! Note we use `MaybeUnit`
        // to express that elements which are not in the read region are not safe to read yet.
        //
        // Finally, notice the lifetime attached to `MqGuard`: this ensure that the guard's lifetime
        // is tied to its receiver, and hence to the underlying `MessageQueue`. This ensures that
        // the message queue and its underlying array are not dropped and freed while or before we
        // are dropping and (potentially) freeing this element.
        unsafe {
            let cell = self.cell_ptr.as_mut();
            cell.elem.assume_init_drop();
            cell.elem = std::mem::MaybeUninit::uninit();
            cell.ack.store(true, sync::atomic::Ordering::Release);
        }
    }
}

impl<T: Send + Clone + std::fmt::Debug + tracing::Value> MessageQueue<T> {
    #[tracing::instrument]
    fn new(cap: u32) -> Self {
        assert_ne!(std::mem::size_of::<T>(), 0, "T cannot be a ZST");
        assert!(cap > 0, "Tried to create a message queue with a capacity < 1");

        let cap = cap.checked_next_power_of_two().expect("failed to retrieve the next power of 2 to cap");
        tracing::debug!(cap, "Determining array layout");
        let layout = alloc::Layout::array::<AckCell<T>>(cap as usize).unwrap();

        // From the `Layout` docs: "All layouts have an associated size and a power-of-two alignment.
        // The size, when rounded up to the nearest multiple of align, does not overflow isize (i.e.
        // the rounded value will always be less than or equal to isize::MAX)."
        //
        // I could not find anything in the source code of this method that checks that so making
        // sure here, is this really necessary?
        assert!(layout.size() <= std::isize::MAX as usize);

        let ptr = unsafe { alloc::alloc(layout) };
        let ring = match std::ptr::NonNull::new(ptr as *mut AckCell<T>) {
            Some(p) => p,
            None => std::alloc::handle_alloc_error(layout),
        };

        let senders = sync::atomic::AtomicUsize::new(0);
        let writter = WriteRegion::new(cap);
        let reader = ReadRegion::new();

        Self { ring, cap, senders, writer: writter, reader }
    }

    #[tracing::instrument(skip(self))]
    fn sender_register(&self) {
        let senders = self.senders.fetch_add(1, sync::atomic::Ordering::AcqRel);
        tracing::debug!(senders = senders + 1, "Increasing sender count");
        debug_assert_ne!(senders, usize::MAX);
    }

    #[tracing::instrument(skip(self))]
    fn sender_unregister(&self) {
        let senders = self.senders.fetch_sub(1, sync::atomic::Ordering::AcqRel);
        tracing::debug!(senders = senders - 1, "Decreasing sender count");
        debug_assert_ne!(senders, 0);
    }

    fn sender_unregister_all(&self) {
        let senders = self.senders.swap(0, sync::atomic::Ordering::Release);
        debug_assert_ne!(senders, 0);
    }

    #[tracing::instrument(skip(self))]
    fn sender_available(&self) -> bool {
        let senders = self.senders.load(sync::atomic::Ordering::Acquire);
        tracing::debug!(senders, "Senders available");
        senders > 0
    }

    fn read(&self) -> Option<std::ptr::NonNull<AckCell<T>>> {
        self.reader.read(self.ring, self.cap)
    }

    fn write(&self, elem: T) -> Option<T> {
        let res = self.writer.write(elem, self.ring, self.cap);
        if res.is_none() {
            self.reader.grow(self.cap);
        }
        res
    }
}

impl<T: Clone + std::fmt::Debug + tracing::Value> AckCell<T> {
    fn new(elem: T) -> Self {
        Self { elem: std::mem::MaybeUninit::new(elem), ack: sync::atomic::AtomicBool::new(false) }
    }
}

impl ReadRegion {
    #[tracing::instrument]
    fn new() -> Self {
        tracing::debug!("Creating new read region");
        Self { strt_and_size: sync::atomic::AtomicU64::new(0) }
    }

    fn strt(&self) -> u32 {
        get_strt(self.strt_and_size.load(sync::atomic::Ordering::Acquire))
    }

    fn size(&self) -> u32 {
        get_size(self.strt_and_size.load(sync::atomic::Ordering::Acquire))
    }

    #[tracing::instrument(skip(self, buf, cap))]
    fn read<T: Send + Clone + std::fmt::Debug + tracing::Value>(
        &self,
        buf: std::ptr::NonNull<AckCell<T>>,
        cap: u32,
    ) -> Option<std::ptr::NonNull<AckCell<T>>> {
        let mut raw_bytes = self.strt_and_size.load(sync::atomic::Ordering::Acquire);
        loop {
            let strt = get_strt(raw_bytes);
            let size = get_size(raw_bytes);
            tracing::debug!(strt, size, "Trying to read from buffer");

            if size == 0 {
                // Note that we do not try to grow the read region in case there is nothing left to
                // read. This is because while cells have and `ack` state to attest if they have
                // been read, we do not store any extra information concerning their write status.
                // Instead, it is the responsibility of the queue to grow the read region whenever
                // it writes a new value.
                tracing::debug!("Failed to read from buffer");
                break None;
            } else {
                tracing::debug!(strt, "Reading from buffer");

                let strt_new = fast_mod(strt + 1, cap);
                let size_new = size - 1; // checked above
                let raw_bytes_new = get_raw_bytes(strt_new, size_new);

                // So, this is a bit complicated. The issue is that we are mixing atomic (`load`,
                // `store`) with non atomic (mod, decrement) operations. Why is this a problem?
                // Well, when performing a `fetch_add` for example, the operation takes place as a
                // single atomic transaction (the fetch and the add happen simultaneously, and its
                // changes can be seen across threads as long as you use `AcRel` ordering). This is
                // not the case here: we `load` an atomic, we compute a change and then we `store`
                // it. Critically, we can only guarantee the ordering of atomic operations across
                // threads. We cannot guarantee that our (non-atomic) computation of `strt_new` and
                // `size_new` will be synchronized with other threads. In other words, it is
                // possible for the value of `start_and_size` to _change_ between our `load` and
                // `store`. Atomic fences will _not_ solve this problem since they only guarantee
                // relative ordering between atomic operations.
                //
                // `compare_exchange` allows us to work around this problem by updating an atomic
                // _only if its value has not changed from what we expect_. In other words, we ask
                // it to update `strt_and_size` only if `strt_and_size` has not been changed by
                // another thread in the meantime. If this is not the case, we re-try the whole
                // operations (checking `size`, computing `strt_new`, `size_new`) with the updated
                // information.
                //
                // We are making two assumptions here:
                //
                // 1. We will not loop indefinitely.
                // 2. The time it takes us to loop is very small, such that there is a good chance
                //    we will only ever loop a very small number of times before settling on a
                //    decision.
                //
                // Assumption [1] is satisfied by the fact that if other readers or writers keep
                // updating the message queue, we will eventually reach the condition `size == 0` or
                // we will succeed in a write. We can assume this since the operations between loop
                // cycles are very simple (in the order of single instructions), therefore it is
                // reasonable to expect we will NOT keep missing the store, which satisfiesS
                // assumption [2].
                if let Err(bytes) = self.strt_and_size.compare_exchange(
                    raw_bytes,
                    raw_bytes_new,
                    sync::atomic::Ordering::Release,
                    sync::atomic::Ordering::Acquire,
                ) {
                    tracing::debug!(bytes, "Inter-thread update on read region, trying again");
                    raw_bytes = bytes;
                    continue;
                };

                tracing::debug!(start = strt_new, size = size_new, "Updated read region");
                break Some(unsafe { buf.add(strt as usize) });
            }
        }
    }

    #[tracing::instrument(skip(self, cap))]
    fn grow(&self, cap: u32) {
        tracing::debug!("Growing read region");
        let size = get_size(self.strt_and_size.fetch_add(SIZE_INC, sync::atomic::Ordering::AcqRel));
        tracing::debug!(size = size + 1, "Growing successful");

        debug_assert_ne!(size, cap);
    }
}

impl WriteRegion {
    #[tracing::instrument]
    fn new(size: u32) -> Self {
        tracing::debug!("Creating new write region");
        assert!(size > 0);
        Self { strt_and_size: sync::atomic::AtomicU64::new(size as u64) }
    }

    fn strt(&self) -> u32 {
        get_strt(self.strt_and_size.load(sync::atomic::Ordering::Acquire))
    }

    fn size(&self) -> u32 {
        get_size(self.strt_and_size.load(sync::atomic::Ordering::Acquire))
    }

    #[tracing::instrument(skip(self, buf, cap))]
    fn write<T: Clone + std::fmt::Debug + tracing::Value>(
        &self,
        elem: T,
        buf: std::ptr::NonNull<AckCell<T>>,
        cap: u32,
    ) -> Option<T> {
        let mut raw_bytes = self.strt_and_size.load(sync::atomic::Ordering::Acquire);
        loop {
            let strt = get_strt(raw_bytes);
            let size = get_size(raw_bytes);
            tracing::debug!(size, "Trying to write to buffer");

            if size == 0 {
                if let Ok(bytes) = self.grow(buf, raw_bytes, cap) {
                    raw_bytes = bytes;
                    continue;
                }

                tracing::debug!("Failed to grow write region");
                break Some(elem);
            } else {
                tracing::debug!(strt, "Writing to buffer");

                // size - 1 is checked above and `grow` will increment size by 1 if it succeeds, so
                // whatever happens size > 0
                let strt_new = fast_mod(strt + 1, cap);
                let size_new = size - 1;
                let raw_bytes_new = get_raw_bytes(strt_new, size_new);

                if let Err(bytes) = self.strt_and_size.compare_exchange(
                    raw_bytes,
                    raw_bytes_new,
                    sync::atomic::Ordering::Release,
                    sync::atomic::Ordering::Acquire,
                ) {
                    tracing::debug!(bytes, "Inter-thread update on write region, trying again");
                    raw_bytes = bytes;
                    continue;
                };

                tracing::debug!(start = strt_new, size = size_new, "Updated write region");
                let cell = AckCell::new(elem);
                unsafe { buf.add(strt as usize).write(cell) };
                break None;
            }
        }
    }

    fn grow<T: Clone + std::fmt::Debug + tracing::Value>(
        &self,
        buf: std::ptr::NonNull<AckCell<T>>,
        raw_bytes: u64,
        cap: u32,
    ) -> Result<u64, &'static str> {
        // We are indexing the element right AFTER the end of the write region to see if we can
        // overwrite it (ie: it has been read and acknowledged)
        let strt = get_strt(raw_bytes);
        let size = get_size(raw_bytes);
        let stop = fast_mod(strt + size, cap);

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
        //
        // See the note in `MqGuard` to understand why we only read the `ack` state!
        let cell = unsafe { buf.add(stop as usize).as_ref() };

        // Why would this fail? Consider the following buffer state:
        //
        //    ┌───┬───┬───┬───┬───┬───┬───┬───┬───┐
        // B: │!a │ a │ a │ r │ r │ w │ w │ w │ w │
        //    └───┴───┴───┴───┴───┴───┴───┴───┴───┘
        //      0   1   2   3   4   5   6   7   8
        //
        //    ┌───────────────────────────────────┐
        //    │ .B: buffer                        │
        //    │ .r: read region                   │
        //    │ .w: write region                  │
        //    │ .a: acknowledged                  │
        //    │ !a: NOT acknowledged              │
        //    └───────────────────────────────────┘
        //
        // Notice how the element at index 0 has been read but not acknowledge yet: this means we
        // cannot overwrite it as another thread might read it in the future! In contrary, the
        // elements at index 1 and 2 have been read and acknowledged, meaning they are safe to
        // overwrite. However, since element 0 precedes them, we cannot grow the write region to
        // encompass them.
        //
        // This is done to avoid fragmenting the buffer and keep read and write operations simple
        // and efficient.
        if cell.ack.load(sync::atomic::Ordering::Acquire) {
            let strt_new = strt;
            let size_new = size + 1;
            let raw_bytes_new = get_raw_bytes(strt_new, size_new);
            debug_assert!(size_new <= cap);

            // Notice that we are not re-trying this operation in case the atomic has been updated
            // since we last loaded it. This is because we only ever call this method as part of
            // `write` and we use the retry loop there to handle failures in `grow`.
            match self.strt_and_size.compare_exchange(
                raw_bytes,
                raw_bytes_new,
                sync::atomic::Ordering::Release,
                sync::atomic::Ordering::Acquire,
            ) {
                Err(bytes) => Ok(bytes),
                Ok(bytes) => Ok(bytes),
            }
        } else {
            Err("Failed to grow write region, next element has not been acknowledged yet")
        }
    }
}

fn fast_mod(n: u32, pow_of_2: u32) -> u32 {
    n & (pow_of_2 - 1)
}

fn get_strt(raw_bytes: u64) -> u32 {
    ((raw_bytes & STRT_MSK) >> 32) as u32
}

fn get_size(raw_bytes: u64) -> u32 {
    (raw_bytes & SIZE_MSK) as u32
}

fn get_raw_bytes(strt: u32, size: u32) -> u64 {
    (strt as u64) << 32 | size as u64
}

#[cfg(test)]
mod test {
    use super::*;

    /// [loom] is a deterministic concurrent permutation simulator. From the loom docs:
    ///
    /// > _"At a high level, it runs tests many times, permuting the possible concurrent executions
    /// > of each test according to what constitutes valid executions under the C11 memory model. It
    /// > then uses state reduction techniques to avoid combinatorial explosion of the number of
    /// > possible executions."_
    ///
    /// # Running Loom
    ///
    /// To run the tests below, first enter:
    ///
    /// ```bash
    /// LOOM_LOCATION=1 \
    ///     LOOM_CHECKPOINT_INTERVAL=1 \
    ///     LOOM_CHECKPOINT_FILE=test_name.json \
    ///     cargo test test_name --release
    /// ```
    ///
    /// This will begin by running loom with no logs, checking all possible permutations of
    /// multithreaded operations for our program (actually this tests _most_ permutations, with
    /// limitations in regard to [SeqCst] and [Relaxed] ordering, but since we do not use those loom
    /// will be exploring the full concurrent permutations). If an invariant is violated, this will
    /// cause the test to fail and the fail state will be saved under `LOOM_CHECKPOINT_FILE`.
    ///
    /// > We do not enable logs for this first run as loom might simulate many thousand permutations
    /// > before finding a single failing case, and this would polute `stdout`. Also, we run in
    /// > `release` mode to make this process faster.
    ///
    /// Once a failing case has been identified, resume the tests with:
    ///
    /// ```bash
    /// LOOM_LOG=debug \
    ///     LOOM_LOCATION=1 \
    ///     LOOM_CHECKPOINT_INTERVAL=1 \
    ///     LOOM_CHECKPOINT_FILE=test_name.json \
    ///     cargo test test_name --release
    /// ```
    ///
    /// This will resume testing with the previously failing case. We enable logging this time as
    /// only a single iteration of the test will be run before the failure is caught.
    ///
    /// > Note that if ever you update the code of a test, you will then need to delete
    /// > `LOOM_CHECKPOINT_FILE` before running the tests again. Otherwise loom will complain about
    /// > having reached an unexpected execution path.
    ///
    /// # Complexity explosion
    ///
    /// Due to the way in which loom checks for concurrent access permutations, execution time will
    /// grow exponentially with the size of the model. For this reason, it might be necessary to
    /// limit the breath of checks done by loom.
    ///
    /// ```bash
    /// LOOM_MAX_PREEMPTIONS=3 \
    ///     LOOM_LOCATION=1 \
    ///     LOOM_CHECKPOINT_INTERVAL=1 \
    ///     LOOM_CHECKPOINT_FILE=test_name.json \
    ///     cargo test test_name --release
    /// ```
    ///
    /// From the loom docs:
    ///
    /// > _"you may need to not run an exhaustive check, and instead tell loom to prune out
    /// > interleavings that are unlikely to reveal additional bugs. You do this by providing loom
    /// > with a thread pre-emption bound. If you set such a bound, loom will check all possible
    /// > executions that include at most n thread pre-emptions (where one thread is forcibly
    /// > stopped and another one runs in its place. In practice, setting the thread pre-emption
    /// > bound to 2 or 3 is enough to catch most bugs while significantly reducing the number of
    /// > possible executions."_
    ///
    /// [SeqCst]: std::sync::atomic::Ordering::SeqCst
    /// [Relaxed]: std::sync::atomic::Ordering::Relaxed

    #[test]
    fn send_single() {
        loom::model(|| {
            let (sx, rx) = channel(1);

            assert_eq!(sx.queue.writer.strt(), 0);
            assert_eq!(sx.queue.writer.size(), 1);
            assert_eq!(sx.queue.reader.strt(), 0);
            assert_eq!(sx.queue.reader.size(), 0);

            let handle = loom::thread::spawn(move || {
                assert_matches::assert_matches!(
                    sx.send(42),
                    None,
                    "Failed to send value, message queue is {:#?}",
                    sx.queue
                )
            });

            loom::future::block_on(async move {
                let guard = rx.recv().await;
                assert_matches::assert_matches!(
                    guard,
                    Some(guard) => { assert_eq!(guard.read_acknowledge(), 42) },
                    "Failed to acquire acknowledge guard, message queue is {:#?}",
                    rx.queue
                );

                let guard = rx.recv().await;
                assert!(guard.is_none(), "Guard acquired on supposedly empty message queue: {:?}", guard.unwrap());
            });

            handle.join().unwrap();
        })
    }

    #[test]
    fn send_multiple() {
        loom::model(|| {
            let (sx, rx) = channel(3);

            assert_eq!(sx.queue.writer.strt(), 0);
            assert_eq!(sx.queue.writer.size(), 4); // closest power of 2
            assert_eq!(sx.queue.reader.strt(), 0);
            assert_eq!(sx.queue.reader.size(), 0);

            let handle = loom::thread::spawn(move || {
                for i in 0..4 {
                    assert_matches::assert_matches!(
                        sx.send(i),
                        None,
                        "Failed to send {i}, message queue is {:#?}",
                        sx.queue
                    )
                }
            });

            loom::future::block_on(async move {
                for i in 0..4 {
                    tracing::info!(i, "Waiting for element");
                    let guard = rx.recv().await;
                    assert_matches::assert_matches!(
                        guard,
                        Some(guard) => { assert_eq!(guard.read_acknowledge(), i) },
                        "Failed to acquire acknowledge guard {i}, message queue is {:#?}",
                        rx.queue
                    );
                }

                let guard = rx.recv().await;
                assert!(guard.is_none(), "Guard acquired on supposedly empty message queue: {:?}", guard.unwrap());
            });

            handle.join().unwrap();
        })
    }

    #[test]
    fn receive_multiple() {
        loom::model(|| {
            let (sx, rx1) = channel(3);
            let rx2 = rx1.resubscribe();
            let rx3 = rx1.resubscribe();
            let witness1 = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::default()));
            let witness2 = std::sync::Arc::clone(&witness1);
            let witness3 = std::sync::Arc::clone(&witness1);

            assert_eq!(sx.queue.writer.strt(), 0);
            assert_eq!(sx.queue.writer.size(), 4); // closest power of 2
            assert_eq!(sx.queue.reader.strt(), 0);
            assert_eq!(sx.queue.reader.size(), 0);

            loom::thread::spawn(move || {
                for i in 0..2 {
                    assert_matches::assert_matches!(
                        sx.send(i),
                        None,
                        "Failed to send {i}, message queue is {:#?}",
                        sx.queue
                    )
                }
            });

            let handle1 = loom::thread::spawn(move || {
                loom::future::block_on(async move {
                    tracing::info!("Waiting for element");
                    let guard = rx1.recv().await;
                    assert_matches::assert_matches!(
                        guard,
                        Some(guard) => { witness1.lock().await.push(guard.read_acknowledge()) },
                        "Failed to acquire acknowledge guard, message queue is {:#?}",
                        rx1.queue
                    );
                })
            });

            let handle2 = loom::thread::spawn(move || {
                loom::future::block_on(async move {
                    tracing::info!("Waiting for element");
                    let guard = rx2.recv().await;
                    assert_matches::assert_matches!(
                        guard,
                        Some(guard) => { witness2.lock().await.push(guard.read_acknowledge()) },
                        "Failed to acquire acknowledge guard, message queue is {:#?}",
                        rx2.queue
                    );
                })
            });

            handle1.join().unwrap();
            handle2.join().unwrap();

            loom::future::block_on(async move {
                tracing::info!(?rx3, "Checking close correctness");

                let guard = rx3.recv().await;
                assert!(guard.is_none(), "Guard acquired on supposedly empty message queue: {:?}", guard.unwrap());

                let mut witness = witness3.lock().await;
                tracing::info!(witness = ?*witness, "Checking receive correctness");

                witness.sort();
                for (expected, actual) in witness.iter().enumerate() {
                    assert_eq!(*actual, expected)
                }
            });
        })
    }
}
