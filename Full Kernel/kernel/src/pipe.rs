//! Pipes: a 4 KiB buffer between a writing process and a reading one
//! (`ls /bin | grep h`).
//!
//! A reader that finds the pipe empty sleeps (`sched::park`) until a writer
//! adds bytes or the last writer goes away (then it reads end-of-file); a
//! writer that finds it full sleeps until the reader makes room, and gets an
//! error if every reader has gone. The two ends are reference counted: an end
//! is closed when its last holder (a descriptor in some process) drops it.

use crate::log::SpinLock;
use crate::sched;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;

const CAPACITY: usize = 4096;

struct Inner {
    buf: VecDeque<u8>,
    readers: usize,
    writers: usize,
    /// Threads sleeping for data / for room.
    read_waiter: Option<usize>,
    write_waiter: Option<usize>,
}

struct Pipe {
    inner: SpinLock<Inner>,
}

/// The reading end.
pub struct Reader(Arc<Pipe>);
/// The writing end.
pub struct Writer(Arc<Pipe>);

/// A new pipe: (reading end, writing end).
pub fn new() -> (Arc<Reader>, Arc<Writer>) {
    let p = Arc::new(Pipe { inner: SpinLock::new(Inner { buf: VecDeque::new(), readers: 1, writers: 1, read_waiter: None, write_waiter: None }) });
    (Arc::new(Reader(p.clone())), Arc::new(Writer(p)))
}

fn wake(t: Option<usize>) {
    if let Some(id) = t {
        sched::unpark(id);
    }
}

impl Reader {
    /// Up to `max` bytes; empty means end of file (no writers left and nothing buffered).
    pub fn read(&self, max: usize) -> Vec<u8> {
        loop {
            let (out, writer) = self.0.inner.with(|p| {
                if !p.buf.is_empty() {
                    let n = max.min(p.buf.len());
                    let out: Vec<u8> = p.buf.drain(..n).collect();
                    return (Some(out), p.write_waiter.take());
                }
                if p.writers == 0 {
                    return (Some(Vec::new()), None);
                }
                p.read_waiter = Some(sched::current_id());
                (None, None)
            });
            wake(writer);
            if let Some(out) = out {
                return out;
            }
            sched::park();
        }
    }
}

impl Writer {
    /// Writes all of `data` (sleeping while the pipe is full). Err = nobody reads any more.
    pub fn write(&self, mut data: &[u8]) -> Result<usize, ()> {
        let total = data.len();
        while !data.is_empty() {
            let (progress, reader) = self.0.inner.with(|p| {
                if p.readers == 0 {
                    return (Err(()), None);
                }
                let room = CAPACITY - p.buf.len();
                if room == 0 {
                    p.write_waiter = Some(sched::current_id());
                    return (Ok(0), None);
                }
                let n = room.min(data.len());
                p.buf.extend(&data[..n]);
                (Ok(n), p.read_waiter.take())
            });
            wake(reader);
            match progress {
                Err(()) => return Err(()),
                Ok(0) => sched::park(),
                Ok(n) => data = &data[n..],
            }
        }
        Ok(total)
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        let w = self.0.inner.with(|p| {
            p.readers -= 1;
            p.write_waiter.take()
        });
        wake(w);
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let r = self.0.inner.with(|p| {
            p.writers -= 1;
            p.read_waiter.take()
        });
        wake(r);
    }
}
