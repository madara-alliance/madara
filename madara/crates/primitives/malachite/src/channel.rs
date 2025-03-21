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

pub struct MqSender<T: Send + Clone + std::fmt::Debug + tracing::Value> {
    queue: sync::Arc<MessageQueue<T>>,
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
    cap: usize,
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
    start: sync::atomic::AtomicUsize,
    size: sync::atomic::AtomicUsize,
}

/// A region in a pointer array in which we are allowed to [write].
///
/// [write]: Self::write
struct WriteRegion {
    start: sync::atomic::AtomicUsize,
    size: sync::atomic::AtomicUsize,
}

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
        let start = self.start.load(sync::atomic::Ordering::Acquire);
        let size = self.start.load(sync::atomic::Ordering::Acquire);
        sync::atomic::fence(sync::atomic::Ordering::Acquire);
        f.debug_struct("ReadRegion").field("start", &start).field("size", &size).finish()
    }
}

impl std::fmt::Debug for WriteRegion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let start = self.start.load(sync::atomic::Ordering::Acquire);
        let size = self.start.load(sync::atomic::Ordering::Acquire);
        sync::atomic::fence(sync::atomic::Ordering::Acquire);
        f.debug_struct("WriteRegion").field("start", &start).field("size", &size).finish()
    }
}

impl<T: Send + Clone + std::fmt::Debug + tracing::Value> Drop for MqSender<T> {
    fn drop(&mut self) {
        self.queue.sender_unregister();

        #[cfg(test)]
        self.wake.notify();
        #[cfg(not(test))]
        self.wake.notify_one();
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
        self.reader.size.store(0, sync::atomic::Ordering::Release);

        let start = self.reader.start();
        let stop = start + self.reader.size();

        for i in (start..stop).map(|i| fast_mod(i, self.cap)) {
            unsafe { self.ring.add(i).read() };
        }

        let layout = alloc::Layout::array::<AckCell<T>>(self.cap).unwrap();
        unsafe { alloc::dealloc(self.ring.as_ptr() as *mut u8, layout) }
    }
}

pub fn channel<T: Send + Clone + std::fmt::Debug + tracing::Value>(cap: usize) -> (MqSender<T>, MqReceiver<T>) {
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
        Self { queue, wake }
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
}

impl<T: Send + Clone + std::fmt::Debug + tracing::Value> MqReceiver<T> {
    fn new(queue: sync::Arc<MessageQueue<T>>, wake: sync::Arc<Notify>) -> Self {
        Self { queue, wake }
    }

    #[tracing::instrument(skip(self))]
    pub async fn rcv(&self) -> Option<MqGuard<T>> {
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

    pub fn read_acknowledge(mut self) -> T {
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
    fn new(cap: usize) -> Self {
        assert_ne!(std::mem::size_of::<T>(), 0, "T cannot be a ZST");
        assert!(cap > 0, "Tried to create a message queue with a capacity < 1");

        let cap = cap.checked_next_power_of_two().expect("failed to retrieve the next power of 2 to cap");
        let layout = alloc::Layout::array::<AckCell<T>>(cap).unwrap();

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

    fn sender_available(&self) -> bool {
        self.senders.load(sync::atomic::Ordering::Acquire) > 0
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
    fn new() -> Self {
        Self { start: sync::atomic::AtomicUsize::new(0), size: sync::atomic::AtomicUsize::new(0) }
    }

    fn start(&self) -> usize {
        self.start.load(sync::atomic::Ordering::Acquire)
    }

    fn size(&self) -> usize {
        self.size.load(sync::atomic::Ordering::Acquire)
    }

    #[tracing::instrument(skip(self, buf, cap))]
    fn read<T: Send + Clone + std::fmt::Debug + tracing::Value>(
        &self,
        buf: std::ptr::NonNull<AckCell<T>>,
        cap: usize,
    ) -> Option<std::ptr::NonNull<AckCell<T>>> {
        let size = self.size();
        tracing::debug!(size, "Trying to read from buffer");

        let res = if size == 0 {
            // Note that we do not try to grow the read region in case there is nothing left to
            // read. This is because while cells have and `ack` state to attest if they have been
            // read, we do not store any extra information concerning their write status. Instead,
            // it is the responsibility of the queue to grow the read region whenever it writes a
            // new value.
            tracing::info!("Failed to read from buffer");
            None
        } else {
            let start = self.start();
            tracing::debug!(start, "Reading from buffer");

            let s = fast_mod(start + 1, cap);
            self.start.store(s, sync::atomic::Ordering::Release);
            tracing::debug!(start = s, "Updated read region start");

            let s = size - 1;
            self.size.store(s, sync::atomic::Ordering::Release); // checked above
            tracing::debug!(size = s, "Updated read region size");

            // Notice how we use the value of `start` before the store. Here the store acts as a
            // locking mechanism, reserving this slot in the queue so that other threads will not
            // read it.
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
        sync::atomic::fence(sync::atomic::Ordering::Acquire);
        res
    }

    #[tracing::instrument(skip(self, cap))]
    fn grow(&self, cap: usize) {
        tracing::debug!("Growing read region");
        let size = self.size.fetch_add(1, sync::atomic::Ordering::AcqRel);

        tracing::debug!(size = size + 1, "Growing successful");
        debug_assert_ne!(size, cap);
    }
}

impl WriteRegion {
    fn new(size: usize) -> Self {
        assert!(size > 0);
        Self { start: sync::atomic::AtomicUsize::new(0), size: sync::atomic::AtomicUsize::new(size) }
    }

    fn start(&self) -> usize {
        self.start.load(sync::atomic::Ordering::Acquire)
    }

    fn size(&self) -> usize {
        self.size.load(sync::atomic::Ordering::Acquire)
    }

    #[tracing::instrument(skip(self, buf, cap))]
    fn write<T: Clone + std::fmt::Debug + tracing::Value>(
        &self,
        elem: T,
        buf: std::ptr::NonNull<AckCell<T>>,
        cap: usize,
    ) -> Option<T> {
        let size = self.size();
        tracing::debug!(size, "Trying to write to buffer");

        let res = if size == 0 && self.grow(buf, cap).is_err() {
            tracing::debug!("Failed to grow write region");
            Some(elem)
        } else {
            let start = self.start();
            tracing::debug!(start, "Writing to buffer");

            // size - 1 is checked above and `grow` will increment size by 1 if it succeeds, so
            // whatever happens size > 0
            let s = fast_mod(start + 1, cap);
            self.start.store(s, sync::atomic::Ordering::Release);
            tracing::debug!(start = s, "Updated write region start");

            let s = size - 1;
            self.size.store(s, sync::atomic::Ordering::Release);
            tracing::debug!(size = s, "Updated write region size");

            let cell = AckCell::new(elem);
            unsafe { buf.add(start).write(cell) };
            None
        };

        sync::atomic::fence(sync::atomic::Ordering::Acquire);
        res
    }

    fn grow<T: Clone + std::fmt::Debug + tracing::Value>(
        &self,
        buf: std::ptr::NonNull<AckCell<T>>,
        cap: usize,
    ) -> Result<(), &'static str> {
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
        //
        // See the note in `MqGuard` to understand why we only read the `ack` state!
        let cell = unsafe { buf.add(stop).as_ref() };

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
        let res = if cell.ack.load(sync::atomic::Ordering::Acquire) {
            debug_assert!(size + 1 <= cap);
            self.size.store(size + 1, sync::atomic::Ordering::Release);
            Ok(())
        } else {
            Err("Failed to grow write region, next element has not been acknowledged yet")
        };

        sync::atomic::fence(sync::atomic::Ordering::Acquire);
        res
    }
}

fn fast_mod(n: usize, pow_of_2: usize) -> usize {
    n & (pow_of_2 - 1)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn simple_send() {
        loom::model(|| {
            let (sx, rx) = channel(1);

            assert_eq!(sx.queue.writer.start(), 0);
            assert_eq!(sx.queue.writer.size(), 1);
            assert_eq!(sx.queue.reader.start(), 0);
            assert_eq!(sx.queue.reader.size(), 0);

            loom::thread::spawn(move || {
                assert_matches::assert_matches!(
                    sx.send(42),
                    None,
                    "Failed to send value, message queue is {:#?}",
                    sx.queue
                )
            });

            loom::future::block_on(async move {
                let guard = rx.rcv().await;
                assert_matches::assert_matches!(
                    guard,
                    Some(guard) => {
                        assert_eq!(guard.read_acknowledge(), 42);
                    },
                    "Failed to acquire acknowledge guard, message queue is {:#?}",
                    rx.queue
                );

                let guard = rx.rcv().await;
                assert!(guard.is_none());
            })
        })
    }
}
