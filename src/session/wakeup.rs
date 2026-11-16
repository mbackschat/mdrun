use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub(super) struct Notifier(Arc<UnixStream>);

impl Notifier {
    pub(super) fn notify(&self) {
        loop {
            match (&*self.0).write(&[1]) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                // A full socket already carries a wakeup; a closed receiver has finished.
                _ => return,
            }
        }
    }
}

pub(super) struct Wakeup {
    reader: UnixStream,
    notifier: Notifier,
}

impl Wakeup {
    pub(super) fn new() -> io::Result<Self> {
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;
        Ok(Self {
            reader,
            notifier: Notifier(Arc::new(writer)),
        })
    }

    pub(super) fn notifier(&self) -> Notifier {
        self.notifier.clone()
    }

    pub(super) fn clear(&self) -> io::Result<()> {
        let mut bytes = [0; 4096];
        match (&self.reader).read(&mut bytes) {
            Ok(_) => Ok(()),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn wait<'a>(
        &self,
        timeout: Duration,
        sources: impl IntoIterator<Item = BorrowedFd<'a>>,
    ) -> io::Result<bool> {
        let mut entries = vec![libc::pollfd {
            fd: self.reader.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        entries.extend(sources.into_iter().map(|source| libc::pollfd {
            fd: source.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }));
        let milliseconds = timeout.as_millis().min(i32::MAX as u128) as i32;
        let result = unsafe { libc::poll(entries.as_mut_ptr(), entries.len() as _, milliseconds) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(false);
            }
            return Err(error);
        }
        Ok(result != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn enqueue_between_scan_and_wait_remains_observable() {
        let wakeup = Wakeup::new().unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        wakeup.clear().unwrap();
        assert!(receiver.try_recv().is_err());
        sender.send(7).unwrap();
        wakeup.notifier().notify();
        assert!(wakeup.wait(Duration::ZERO, []).unwrap());
        wakeup.clear().unwrap();
        assert_eq!(receiver.try_recv().unwrap(), 7);
        assert!(!wakeup.wait(Duration::ZERO, []).unwrap());
    }

    #[test]
    fn clearing_notifications_preserves_every_already_queued_source() {
        let wakeup = Wakeup::new().unwrap();
        let (first, first_events) = mpsc::sync_channel(1);
        let (second, second_events) = mpsc::sync_channel(1);
        first.send(7).unwrap();
        wakeup.notifier().notify();
        second.send(9).unwrap();
        wakeup.notifier().notify();
        wakeup.clear().unwrap();
        assert_eq!(first_events.try_recv().unwrap(), 7);
        assert_eq!(second_events.try_recv().unwrap(), 9);
        assert!(!wakeup.wait(Duration::ZERO, []).unwrap());
    }

    #[test]
    fn saturated_notifications_neither_block_nor_replace_bounded_source_queues() {
        let wakeup = Wakeup::new().unwrap();
        let notifier = wakeup.notifier();
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.send(7).unwrap();
        for _ in 0..100_000 {
            notifier.notify();
        }
        assert!(sender.try_send(8).is_err());
        assert!(wakeup.wait(Duration::ZERO, []).unwrap());
        assert_eq!(receiver.try_recv().unwrap(), 7);
        drop(wakeup);
        notifier.notify();
    }
}
